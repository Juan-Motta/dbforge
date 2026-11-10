//! The CLI panel: commands typed as in redis-cli, run on a connection kept
//! open between runs (so `SELECT`, `MULTI` and `CLIENT SETNAME` carry over),
//! with replies printed the way redis-cli prints them.
//!
//! A script is one command per line. A command the server refuses prints its
//! error, as redis-cli does, and the next line still runs.

use std::time::Instant;

use redis::Value;

use super::connect;
use super::logged::{LoggedConnection, quote};
use crate::connections::model::Connection;
use crate::drivers::Ran;
use crate::drivers::{Column, QueryOutcome, StatementResult};
use crate::error::{AppError, Result};

/// Commands that turn the connection into a stream of pushes, which a panel
/// that prints one reply per command can't show.
pub(super) const STREAMING: [&str; 7] =
    ["SUBSCRIBE", "PSUBSCRIBE", "SSUBSCRIBE", "MONITOR", "SYNC", "PSYNC", "SHUTDOWN"];

pub struct Session {
    conn: LoggedConnection,
    /// The server's id for this connection, which `CLIENT UNBLOCK` takes.
    client_id: i64,
}

pub async fn open_session(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<Session> {
    let db = database.map(super::database_number).transpose()?;
    // No response timeout: a blocking command waits as long as it was told to.
    let mut conn = connect(connection, password, db, None).await?;
    let client_id: i64 = redis::cmd("CLIENT").arg("ID").query_async(&mut conn).await?;
    Ok(Session { conn, client_id })
}

impl Session {
    pub fn client_id(&self) -> i64 {
        self.client_id
    }

    /// Each command's reply as lines in a one-column result named `reply`.
    pub async fn run(&mut self, script: &str, row_limit: usize) -> Result<QueryOutcome> {
        let started = Instant::now();
        let commands = script
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| split(line).map_err(AppError::Invalid))
            .collect::<Result<Vec<_>>>()?;
        if commands.is_empty() {
            return Err(AppError::Invalid("Nothing to run.".into()));
        }
        let mut ran = Ran::default();
        for words in commands {
            if !ran.add(run_command(&mut self.conn, &words, row_limit).await) {
                break;
            }
        }
        QueryOutcome::of(ran, started)
    }

    pub async fn is_alive(&mut self) -> bool {
        redis::cmd("PING").query_async::<String>(&mut self.conn).await.is_ok()
    }
}

/// Runs one command, given as its words, and returns its reply as lines in a
/// one-column result named `reply`, as redis-cli prints it, the first
/// `row_limit` of them. An error the server replies with is printed too; only
/// a lost connection is an `Err`.
pub(super) async fn run_command(
    conn: &mut LoggedConnection,
    words: &[Vec<u8>],
    row_limit: usize,
) -> Result<StatementResult> {
    let name = String::from_utf8_lossy(&words[0]).to_ascii_uppercase();
    let lines = if STREAMING.contains(&name.as_str()) {
        vec![format!("(error) {name} isn't supported in the CLI panel.")]
    } else {
        let mut command = redis::cmd(&name);
        command.arg(&words[1..]);
        match command.query_async::<Value>(conn).await {
            Ok(reply) => format(&reply),
            // The server refused the command: printed, and a script goes on.
            Err(error) if error.code().is_some() => vec![format!("(error) {}", AppError::from(error))],
            Err(error) => return Err(error.into()),
        }
    };
    let column =
        Column { name: "reply".into(), data_type: "redis".into(), primary_key: false, generated: false, default: None };
    let truncated = lines.len() > row_limit;
    Ok(StatementResult {
        columns: vec![column],
        rows: lines.into_iter().take(row_limit).map(|line| vec![Some(line)]).collect(),
        truncated,
        rows_affected: 1,
    })
}

/// Ends a blocking command (`BLPOP`, `XREAD BLOCK`, …) the session is waiting
/// on. Other commands run to the end: Redis can't stop them.
pub async fn cancel(connection: &Connection, password: Option<&str>, client_id: i64) -> Result<()> {
    let mut conn = super::browse(connection, password, None).await?;
    let unblocked: i64 = redis::cmd("CLIENT").arg("UNBLOCK").arg(client_id).arg("ERROR").query_async(&mut conn).await?;
    if unblocked == 0 {
        return Err(AppError::Invalid(
            "Only blocking commands (BLPOP, XREAD BLOCK, …) can be cancelled; Redis finishes the others.".into(),
        ));
    }
    Ok(())
}

/// The words of a command line, as redis-cli reads them: separated by spaces,
/// with `"…"` (escapes `\n`, `\t`, `\"`, `\\`, `\xHH`) and `'…'` (only `\'`).
/// Words are bytes: `"\xff"` is the byte 0xff.
pub fn split(line: &str) -> std::result::Result<Vec<Vec<u8>>, String> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else { return Ok(words) };
        let mut word: Vec<u8> = Vec::new();
        let push = |word: &mut Vec<u8>, c: char| word.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
        if first == '"' || first == '\'' {
            chars.next();
            loop {
                match chars.next() {
                    None => return Err(format!("Unbalanced quotes in: {line}")),
                    Some(c) if c == first => break,
                    Some('\\') if first == '\'' => match chars.next() {
                        Some('\'') => push(&mut word, '\''),
                        Some(other) => {
                            push(&mut word, '\\');
                            push(&mut word, other);
                        }
                        None => return Err(format!("Unbalanced quotes in: {line}")),
                    },
                    Some('\\') => match chars.next() {
                        Some('n') => push(&mut word, '\n'),
                        Some('r') => push(&mut word, '\r'),
                        Some('t') => push(&mut word, '\t'),
                        Some('b') => push(&mut word, '\u{8}'),
                        Some('a') => push(&mut word, '\u{7}'),
                        Some('x') => {
                            let hex: String = [chars.next(), chars.next()].into_iter().flatten().collect();
                            match u8::from_str_radix(&hex, 16) {
                                Ok(byte) => word.push(byte),
                                Err(_) => {
                                    word.extend_from_slice(b"\\x");
                                    word.extend_from_slice(hex.as_bytes());
                                }
                            }
                        }
                        Some(other) => push(&mut word, other),
                        None => return Err(format!("Unbalanced quotes in: {line}")),
                    },
                    Some(c) => push(&mut word, c),
                }
            }
            // redis-cli wants a space (or the end) after a closing quote.
            if chars.peek().is_some_and(|c| !c.is_whitespace()) {
                return Err(format!("A closing quote must be followed by a space in: {line}"));
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                push(&mut word, c);
            }
        }
        words.push(word);
    }
}

/// A reply as redis-cli prints it.
pub fn format(reply: &Value) -> Vec<String> {
    match reply {
        Value::Nil => vec!["(nil)".into()],
        Value::Int(n) => vec![format!("(integer) {n}")],
        Value::Okay => vec!["OK".into()],
        Value::SimpleString(text) => vec![text.clone()],
        // Multi-line text (INFO, CLIENT LIST, LATENCY DOCTOR) reads better as it is.
        Value::BulkString(bytes) if bytes.contains(&b'\n') && std::str::from_utf8(bytes).is_ok() => {
            String::from_utf8_lossy(bytes).lines().map(|line| line.trim_end_matches('\r').to_string()).collect()
        }
        Value::BulkString(bytes) => vec![quote(bytes)],
        Value::VerbatimString { text, .. } => text.lines().map(String::from).collect(),
        Value::Double(n) => vec![format!("(double) {n}")],
        Value::Boolean(b) => vec![format!("({b})")],
        Value::BigNumber(n) => vec![format!("(big number) {n:?}")],
        Value::ServerError(error) => {
            vec![format!("(error) {} {}", error.code(), error.details().unwrap_or_default())]
        }
        Value::Array(items) | Value::Set(items) | Value::Push { data: items, .. } => numbered(items.iter().map(format)),
        // As RESP2 sends maps: keys and values alternating.
        Value::Map(pairs) => {
            let items: Vec<Vec<String>> = pairs.iter().flat_map(|(key, value)| [format(key), format(value)]).collect();
            numbered(items.into_iter())
        }
        Value::Attribute { data, .. } => format(data),
        _ => vec![format!("{reply:?}")],
    }
}

/// `1) …`, `2) …`, with the lines of nested replies aligned under their first.
fn numbered(items: impl ExactSizeIterator<Item = Vec<String>>) -> Vec<String> {
    let count = items.len();
    if count == 0 {
        return vec!["(empty array)".into()];
    }
    let width = count.to_string().len();
    let mut lines = Vec::new();
    for (i, item) in items.enumerate() {
        let label = format!("{:>width$}) ", i + 1);
        let indent = " ".repeat(label.len());
        for (j, line) in item.into_iter().enumerate() {
            lines.push(if j == 0 { format!("{label}{line}") } else { format!("{indent}{line}") });
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::tests::{PASSWORD, database};
    use super::*;

    fn words(line: &str) -> Vec<String> {
        split(line).unwrap().iter().map(|w| String::from_utf8_lossy(w).into_owned()).collect()
    }

    #[test]
    fn splits_words_like_redis_cli() {
        assert_eq!(words(r#"  SET  greeting "hello \"you\"\n" "#), ["SET", "greeting", "hello \"you\"\n"]);
        assert_eq!(words(r"SET k 'it\'s \n'"), ["SET", "k", "it's \\n"]);
        assert_eq!(words(r#"SET k "\x41\x42""#), ["SET", "k", "AB"]);
        assert_eq!(split(r#"SET k "\xff""#).unwrap()[2], [0xff]);
        assert_eq!(split(r#"SET k "open"#).unwrap_err(), r#"Unbalanced quotes in: SET k "open"#);
        assert!(split(r#"SET k "a"b"#).unwrap_err().starts_with("A closing quote must be followed by a space"));
    }

    #[test]
    fn formats_replies_like_redis_cli() {
        let nested = Value::Array(vec![
            Value::BulkString(b"a".to_vec()),
            Value::Array(vec![Value::Int(1), Value::Nil]),
            Value::Array(vec![]),
        ]);
        assert_eq!(format(&nested), ["1) \"a\"", "2) 1) (integer) 1", "   2) (nil)", "3) (empty array)"]);
        assert_eq!(format(&Value::BulkString(b"say \"hi\"\x01\xff".to_vec())), [r#""say \"hi\"\x01\xff""#]);
        assert_eq!(
            format(&Value::BulkString(b"# Server\r\nredis_version:7\r\n".to_vec())),
            ["# Server", "redis_version:7"]
        );
        assert_eq!(format(&Value::Okay), ["OK"]);
        let ten: Vec<Value> = (0..10).map(Value::Int).collect();
        assert_eq!(format(&Value::Array(ten))[0], " 1) (integer) 0");
    }

    fn lines(outcome: &QueryOutcome) -> Vec<Vec<String>> {
        outcome.results.iter().map(|r| r.rows.iter().map(|row| row[0].clone().unwrap()).collect()).collect()
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn a_long_reply_keeps_the_first_lines_up_to_the_limit() {
        let items: Vec<String> = (0..10).map(|n| n.to_string()).collect();
        let mut push = vec!["RPUSH", "long"];
        push.extend(items.iter().map(String::as_str));
        let (connection, _) = database(15, &[&push]).await;
        let mut session = open_session(&connection, PASSWORD, Some("db15")).await.unwrap();
        let outcome = session.run("LRANGE long 0 -1\nGET missing", 3).await.unwrap();
        let [long, short] = outcome.results.as_slice() else { panic!("{outcome:?}") };
        assert_eq!(long.rows.len(), 3);
        assert!(long.truncated);
        assert_eq!(long.rows[0], [Some(" 1) \"0\"".into())]);
        assert!(!short.truncated);
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn runs_commands_and_keeps_the_connection_between_runs() {
        // The script switches to database 8, so it starts empty too.
        database(8, &[]).await;
        let (connection, _) = database(7, &[&["HSET", "session:1", "user_id", "u_88412"]]).await;
        let mut session = open_session(&connection, PASSWORD, None).await.unwrap();
        let outcome = session
            .run(
                "HGET session:1 user_id\nHGET session:1 missing\nINCR counter\nLPUSH session:1 x\nRPUSH l a \"b c\"\nLRANGE l 0 -1",
                100,
            )
            .await
            .unwrap();
        assert_eq!(
            lines(&outcome),
            [
                vec![r#""u_88412""#.to_string()],
                vec!["(nil)".into()],
                vec!["(integer) 1".into()],
                vec!["(error) WRONGTYPE Operation against a key holding the wrong kind of value".into()],
                vec!["(integer) 2".into()],
                vec![r#"1) "a""#.into(), r#"2) "b c""#.into()],
            ]
        );
        // SELECT and MULTI belong to the session's connection.
        session.run("SELECT 8\nSET only_in_8 1", 100).await.unwrap();
        let outcome = session.run("GET only_in_8\nMULTI\nINCR n\nEXEC", 100).await.unwrap();
        assert_eq!(lines(&outcome), [vec![r#""1""#], vec!["OK"], vec!["QUEUED"], vec!["1) (integer) 1"]]);
        let refused = session.run("MONITOR", 100).await.unwrap();
        assert_eq!(lines(&refused), [vec!["(error) MONITOR isn't supported in the CLI panel."]]);
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn cancel_ends_a_blocking_command() {
        let (connection, _) = database(9, &[]).await;
        let mut session = open_session(&connection, PASSWORD, None).await.unwrap();
        let id = session.client_id();
        let canceller = {
            let connection = connection.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                cancel(&connection, PASSWORD, id).await
            })
        };
        let started = Instant::now();
        let outcome = session.run("BLPOP nothing 0", 100).await.unwrap();
        assert!(lines(&outcome)[0][0].starts_with("(error) UNBLOCKED"), "{outcome:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        canceller.await.unwrap().unwrap();

        let error = cancel(&connection, PASSWORD, id).await.unwrap_err();
        assert!(error.to_string().starts_with("Only blocking commands"), "{error}");
    }
}
