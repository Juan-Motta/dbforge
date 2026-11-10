//! One statement that may only read, for callers that must not change data.
//!
//! SQL Server has no read-only session, so the text is checked first and the
//! server is a second layer:
//!
//! 1. The statement must be one `SELECT`, optionally after `WITH`. T-SQL needs
//!    no `;` between statements (`SELECT 1 DELETE FROM t` is two), so besides
//!    refusing a `;` followed by more text, every word that starts a statement
//!    or writes is refused wherever it appears: `INTO`, `INSERT`, `UPDATE`,
//!    `DELETE`, `MERGE`, `EXEC`, `SET`, `BEGIN`, `COMMIT`, … These are reserved
//!    words, which can't be names unless quoted, so strings, `[names]`,
//!    `"names"` and comments are skipped while looking for them. `OPENQUERY`,
//!    `OPENROWSET` and `OPENDATASOURCE` are refused too: they run statements
//!    on linked or remote servers, where nothing here can undo them.
//! 2. It runs with `SET XACT_ABORT ON` inside a transaction that is always
//!    rolled back, so a write that got past the check is undone.
//!
//! A single `SELECT` can't change data: functions can't modify tables. Not
//! prevented: what a function or a non-transactional feature does anyway, such
//! as a sequence advanced by `NEXT VALUE FOR`, an extended stored procedure
//! called by a user function, or a four-part name reading a linked server.

use std::time::Instant;

use super::{Client, connect, read_results};
use crate::connections::model::Connection;
use crate::drivers::{QueryOutcome, StatementResult, with_timeout};
use crate::error::{AppError, Result};

/// Words that begin another statement or write, refused anywhere in the query.
/// None can appear in a single `SELECT` (`ELSE`, `END` and `FETCH` can, so they
/// are not here; the statements they begin are undone by the rollback).
const REFUSED: &[&str] = &[
    "ADD",
    "ALTER",
    "BACKUP",
    "BEGIN",
    "BREAK",
    "BULK",
    "CHECKPOINT",
    "CLOSE",
    "COMMIT",
    "CONTINUE",
    "CREATE",
    "DBCC",
    "DEALLOCATE",
    "DECLARE",
    "DELETE",
    "DENY",
    "DROP",
    "EXEC",
    "EXECUTE",
    "GOTO",
    "GRANT",
    "IF",
    "INSERT",
    "INTO",
    "KILL",
    "MERGE",
    "OPEN",
    "OPENDATASOURCE",
    "OPENQUERY",
    "OPENROWSET",
    "PRINT",
    "RAISERROR",
    "READTEXT",
    "RECONFIGURE",
    "RESTORE",
    "RETURN",
    "REVOKE",
    "ROLLBACK",
    "SAVE",
    "SET",
    "SETUSER",
    "SHUTDOWN",
    "TRAN",
    "TRANSACTION",
    "TRUNCATE",
    "UPDATE",
    "UPDATETEXT",
    "USE",
    "WAITFOR",
    "WHILE",
    "WRITETEXT",
];

/// Checks `statement`, then runs it in a rolled-back transaction on a new
/// connection that is closed afterwards.
pub async fn run_read_only(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    statement: &str,
    row_limit: usize,
) -> Result<QueryOutcome> {
    check(statement)?;
    let started = Instant::now();
    let mut client = with_timeout(connect(connection, password, database)).await?;
    let results = run_rolled_back(&mut client, statement, row_limit).await;
    client.close().await?;
    Ok(QueryOutcome::complete(results?, started))
}

async fn run_rolled_back(client: &mut Client, statement: &str, row_limit: usize) -> Result<Vec<StatementResult>> {
    // Any error rolls the whole transaction back and ends the batch.
    client.simple_query("SET XACT_ABORT ON; BEGIN TRANSACTION").await?.into_results().await?;
    let results = read_results(client, statement, row_limit).await;
    let rolled_back = async {
        client.simple_query("IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION").await?.into_results().await?;
        Ok::<_, AppError>(())
    }
    .await;
    // The statement's own error says more; closing the connection rolls back anyway.
    let results = results?;
    rolled_back?;
    Ok(results)
}

/// Refuses anything but one `SELECT`; see the module documentation.
fn check(statement: &str) -> Result<()> {
    let tokens = tokens(statement)?;
    let mut words = tokens.iter().filter_map(|token| match token {
        Token::Word(word) => Some(*word),
        Token::Semicolon => None,
    });
    let first = words.next().unwrap_or_default();
    if !first.eq_ignore_ascii_case("SELECT") && !first.eq_ignore_ascii_case("WITH") {
        return Err(AppError::Invalid("Read-only queries on SQL Server must be a SELECT (or WITH … SELECT).".into()));
    }
    if let Some(semicolon) = tokens.iter().position(|token| *token == Token::Semicolon)
        && tokens[semicolon..].iter().any(|token| *token != Token::Semicolon)
    {
        return Err(AppError::Invalid("Read-only queries run one statement at a time.".into()));
    }
    let refused = |word: &&str| REFUSED.contains(&word.to_ascii_uppercase().as_str());
    if let Some(word) = words.find(refused) {
        return Err(AppError::Invalid(format!(
            "`{}` isn't allowed in a read-only query: it writes, starts another statement or reaches another server.",
            word.to_ascii_uppercase()
        )));
    }
    Ok(())
}

/// What the check looks at: words and `;`. Strings, quoted names, numbers,
/// comments and punctuation are read past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    Word(&'a str),
    Semicolon,
}

/// Splits T-SQL into [`Token`]s the way the server reads it: `'…'` and `N'…'`
/// strings with `''`, `[names]` with `]]`, `"names"` with `""`, `--` comments
/// and nested `/* */` comments. Text the server would read differently from
/// here is refused rather than guessed: an unclosed string, name or comment,
/// and a word glued to a number (`1INTO`, which the server reads as `1 INTO`).
fn tokens(sql: &str) -> Result<Vec<Token<'_>>> {
    let unclosed = |what: &str| AppError::Invalid(format!("The query has an unclosed {what}."));
    let mut tokens = Vec::new();
    let mut chars = sql.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        match c {
            ';' => tokens.push(Token::Semicolon),
            '\'' | '"' | '[' => {
                let close = if c == '[' { ']' } else { c };
                loop {
                    match chars.next() {
                        None => return Err(unclosed(if c == '\'' { "string" } else { "quoted name" })),
                        // A doubled closing character stands for itself.
                        Some((_, next)) if next == close => {
                            if chars.next_if(|&(_, after)| after == close).is_none() {
                                break;
                            }
                        }
                        Some(_) => {}
                    }
                }
            }
            // The server ends a line comment at a carriage return as well as a newline.
            '-' if chars.next_if(|&(_, next)| next == '-').is_some() => {
                while chars.next_if(|&(_, next)| next != '\n' && next != '\r').is_some() {}
            }
            '/' if chars.next_if(|&(_, next)| next == '*').is_some() => {
                let mut depth = 1;
                while depth > 0 {
                    match chars.next() {
                        None => return Err(unclosed("comment")),
                        Some((_, '/')) if chars.next_if(|&(_, next)| next == '*').is_some() => depth += 1,
                        Some((_, '*')) if chars.next_if(|&(_, next)| next == '/').is_some() => depth -= 1,
                        Some(_) => {}
                    }
                }
            }
            // `$1.50` is a money literal: the server reads `$1COMMIT` as `$1` then `COMMIT`.
            '$' if chars.peek().is_some_and(|(_, next)| next.is_ascii_digit() || *next == '.') => {
                let (_, first) = chars.next().expect("peeked");
                number(first, &mut chars)?;
            }
            '0'..='9' | '.' if c != '.' || chars.peek().is_some_and(|(_, next)| next.is_ascii_digit()) => {
                number(c, &mut chars)?;
            }
            c if is_word_start(c) => {
                let mut end = start + c.len_utf8();
                while let Some((at, next)) = chars.next_if(|&(_, next)| is_word_char(next)) {
                    end = at + next.len_utf8();
                }
                let word = &sql[start..end];
                // `N'…'` is a Unicode string; the `N` is part of it.
                if !(word.eq_ignore_ascii_case("n") && chars.peek().is_some_and(|&(_, next)| next == '\'')) {
                    tokens.push(Token::Word(word));
                }
            }
            // Blanks and punctuation separate words the same way here and on the server.
            c if c.is_ascii() && (c.is_ascii_whitespace() || c.is_ascii_punctuation()) => {}
            // Anything else outside a string, a [name] or a comment (a byte-order
            // mark, a non-breaking or zero-width space, a letter outside ASCII) the
            // server might read differently from here, so it isn't guessed at.
            _ => {
                return Err(AppError::Invalid(
                    "Outside strings and comments, read-only queries take plain ASCII; put other names in [brackets]."
                        .into(),
                ));
            }
        }
    }
    Ok(tokens)
}

/// Reads the rest of a number that starts with `first`: digits, a decimal
/// point, an exponent, or `0x` hex. A word right after it is refused: the
/// server reads `1INTO` as `1 INTO`.
fn number(first: char, chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) -> Result<()> {
    let hex = first == '0' && chars.next_if(|&(_, next)| next == 'x' || next == 'X').is_some();
    while chars.next_if(|&(_, next)| next.is_ascii_hexdigit() && (hex || next.is_ascii_digit())).is_some() {}
    if !hex {
        while chars.next_if(|&(_, next)| next.is_ascii_digit() || next == '.').is_some() {}
        // An exponent: `1e5`, `1E-3`.
        let mut ahead = chars.clone();
        if ahead.next_if(|&(_, next)| next == 'e' || next == 'E').is_some() {
            ahead.next_if(|&(_, next)| next == '+' || next == '-');
            if ahead.peek().is_some_and(|(_, next)| next.is_ascii_digit()) {
                *chars = ahead;
                while chars.next_if(|&(_, next)| next.is_ascii_digit()).is_some() {}
            }
        }
    }
    if chars.peek().is_some_and(|&(_, next)| is_word_char(next)) {
        return Err(AppError::Invalid("Put a space between a number and the word after it.".into()));
    }
    Ok(())
}

/// What a name or variable starts with: a letter, `_`, `@` or `#` (never a digit or `$`).
fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, '_' | '@' | '#')
}

/// Letters, digits and the symbols T-SQL allows inside names and variables.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '#' | '$')
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, database};
    use super::*;

    fn words(sql: &str) -> Vec<&str> {
        tokens(sql)
            .unwrap()
            .into_iter()
            .map(|token| match token {
                Token::Word(word) => word,
                Token::Semicolon => ";",
            })
            .collect()
    }

    #[test]
    fn tokens_skip_strings_names_and_comments() {
        assert_eq!(
            words("SELECT N'it''s; INTO', [we]]ird INTO], \"a\"\"INTO\" -- INTO\n/* a /* INTO */ b */ FROM t;"),
            ["SELECT", "FROM", "t", ";"]
        );
        assert_eq!(words("SELECT 1.5e-3, 0xFF, .5, @x, #t FROM t"), ["SELECT", "@x", "#t", "FROM", "t"]);
        assert_eq!(words("SELECT [a]INTO"), ["SELECT", "INTO"]);
        assert_eq!(words("SELECT 'a'INTO"), ["SELECT", "INTO"]);
        assert_eq!(words("SELECT [naïve]"), ["SELECT"]);
        assert!(tokens("SELECT naïve").is_err(), "names outside ASCII go in brackets");
        for unclosed in ["SELECT 'a", "SELECT [a", "SELECT \"a", "SELECT /* a /* b */"] {
            assert!(tokens(unclosed).is_err(), "{unclosed}");
        }
        assert!(tokens("SELECT 1INTO t").is_err());
        assert!(tokens("SELECT 1e INTO t").is_err());
    }

    #[test]
    fn statements_hidden_from_the_check_are_refused() {
        // The server reads `$1COMMIT` as the money literal `$1`, then COMMIT.
        assert!(check("SELECT $1COMMIT SELECT $1DELETE FROM items").is_err());
        assert!(check("SELECT $1ROLLBACK SELECT $1DELETE FROM items").is_err());
        // A line comment ends at a bare carriage return on the server.
        assert!(check("SELECT 1 --x\rCOMMIT DELETE FROM items").is_err());
        // Characters the server may read differently are refused, not guessed at.
        for hidden in ["\u{feff}SELECT 1", "SELECT\u{a0}1", "SELECT 1\u{200b}", "SELECT ñ FROM t"] {
            assert!(check(hidden).is_err(), "{hidden:?}");
        }
        // Money, $PARTITION and Unicode inside strings or brackets still read.
        check("SELECT $1.50, $PARTITION.f(1), N'ñ', [ñ] FROM t -- ñ\r\n").unwrap();
    }

    #[test]
    fn check_allows_one_select_only() {
        for allowed in [
            "SELECT * FROM t",
            "  -- note\n select a, 'DELETE' AS [update] FROM t ORDER BY a OFFSET 1 ROWS FETCH NEXT 5 ROWS ONLY;",
            "WITH x AS (SELECT 1 AS a) SELECT CASE WHEN a = 1 THEN 'x' ELSE 'y' END FROM x;;",
        ] {
            assert!(check(allowed).is_ok(), "{allowed}");
        }
        for refused in [
            "DELETE FROM t",
            "UPDATE t SET a = 1",
            "EXEC sp_who",
            "sp_who",
            "SELECT * INTO copy FROM t",
            "SELECT 1; DELETE FROM t",
            "SELECT 1 DELETE FROM t",
            "SELECT 1 /* x */ ; -- y\n DROP TABLE t",
            "WITH x AS (SELECT 1 AS a) DELETE FROM x",
            "WITH x AS (SELECT 1 AS a) INSERT INTO t SELECT a FROM x",
            "SELECT 1 COMMIT",
            "SELECT 1 SET XACT_ABORT OFF",
            "SELECT * FROM OPENQUERY(remote, 'DELETE FROM t')",
            "SELECT 1 WAITFOR DELAY '01:00'",
            "",
        ] {
            assert!(check(refused).is_err(), "{refused}");
        }
        assert_eq!(
            check("SELECT a INTO b FROM t").unwrap_err().to_string(),
            "`INTO` isn't allowed in a read-only query: it writes, starts another statement or reaches another server."
        );
    }

    async fn ids(connection: &Connection) -> Vec<Vec<Option<String>>> {
        let outcome = run_read_only(connection, PASSWORD, None, "SELECT id FROM items ORDER BY id", 100);
        outcome.await.unwrap().results.remove(0).rows
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn read_only_reads_and_refuses_every_write() {
        let connection = database(
            "dbforge_read_only",
            "CREATE TABLE items (id int PRIMARY KEY); INSERT INTO items VALUES (1), (2);",
        )
        .await;
        let outcome = run_read_only(&connection, PASSWORD, None, "SELECT id, 'x' AS tag FROM items ORDER BY id", 1)
            .await
            .unwrap();
        let [result] = outcome.results.as_slice() else { panic!("{outcome:?}") };
        assert_eq!(result.rows, [vec![Some("1".into()), Some("x".into())]]);
        assert!(result.truncated);

        for attempt in [
            "INSERT INTO items VALUES (3)",
            "UPDATE items SET id = id + 10",
            "DELETE FROM items",
            "DROP TABLE items",
            "CREATE TABLE created (a int)",
            "SELECT * INTO copied FROM items",
            "SELECT 1; DELETE FROM items",
            "SELECT 1 DELETE FROM items",
            "SELECT 1 COMMIT DELETE FROM items",
        ] {
            assert!(run_read_only(&connection, PASSWORD, None, attempt, 100).await.is_err(), "ran: {attempt}");
        }
        assert_eq!(ids(&connection).await, [vec![Some("1".to_string())], vec![Some("2".to_string())]]);
        let tables = run_read_only(&connection, PASSWORD, None, "SELECT name FROM sys.tables ORDER BY name", 10);
        assert_eq!(tables.await.unwrap().results[0].rows, [vec![Some("items".to_string())]]);
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn read_only_rolls_back_what_slips_past_the_check() {
        let connection = database(
            "dbforge_read_only_rollback",
            "CREATE TABLE items (id int PRIMARY KEY); INSERT INTO items VALUES (1);",
        )
        .await;
        let mut client = connect(&connection, PASSWORD, None).await.unwrap();
        // Not a SELECT, so `check` would refuse it; run directly to prove the second layer.
        let results = run_rolled_back(&mut client, "DELETE FROM items; SELECT COUNT(*) FROM items", 10).await.unwrap();
        assert_eq!(results.last().unwrap().rows, [vec![Some("0".to_string())]]);
        client.close().await.unwrap();
        assert_eq!(ids(&connection).await, [vec![Some("1".to_string())]]);
    }
}
