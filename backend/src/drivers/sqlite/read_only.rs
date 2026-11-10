//! One statement that may only read, for callers that must not change data.
//!
//! SQLite enforces it: the file is opened read-only, so every write to it, its
//! schema or its settings fails, and an `ATTACH` can't create a file. One
//! write reaches past that and is refused here: `VACUUM INTO` copies the
//! database to a new file even from a read-only connection. Since SQLite runs
//! every statement of a string, a script is refused too, which also keeps
//! anything a statement sets up (an `ATTACH`, a `PRAGMA`) from outliving it.

use std::time::Instant;

use sqlx::AssertSqlSafe;

use super::{connect_with, has_several_statements, read_results};
use crate::connections::model::Connection;
use crate::drivers::{QueryOutcome, with_timeout};
use crate::error::{AppError, Result};

/// Runs `statement` on the file opened read-only, then closes it.
pub async fn run_read_only(connection: &Connection, statement: &str, row_limit: usize) -> Result<QueryOutcome> {
    check(statement)?;
    let started = Instant::now();
    let mut conn = with_timeout(connect_with(connection, |options| options.read_only(true))).await?;
    let results = read_results(sqlx::raw_sql(AssertSqlSafe(statement)).fetch_many(&mut conn), row_limit).await;
    conn.close().await?;
    Ok(QueryOutcome { results: results?, elapsed_ms: started.elapsed().as_millis() as u64 })
}

/// Refuses scripts and `VACUUM`, which can write a new file.
fn check(statement: &str) -> Result<()> {
    if has_several_statements(statement) {
        return Err(AppError::Invalid("Read-only queries run one statement at a time.".into()));
    }
    if first_word(statement).eq_ignore_ascii_case("vacuum") {
        return Err(AppError::Invalid("VACUUM isn't allowed in a read-only query: it can write a new file.".into()));
    }
    Ok(())
}

/// The statement's first word, after blanks and comments.
fn first_word(statement: &str) -> &str {
    let mut rest = statement;
    loop {
        rest = rest.trim_start();
        if let Some(comment) = rest.strip_prefix("--") {
            rest = comment.find('\n').map_or("", |end| &comment[end..]);
        } else if let Some(comment) = rest.strip_prefix("/*") {
            rest = comment.find("*/").map_or("", |end| &comment[end + 2..]);
        } else {
            let end = rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len());
            return &rest[..end];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::database;
    use super::*;

    #[test]
    fn refuses_scripts_and_vacuum_after_comments() {
        assert_eq!(first_word("  -- note\n /* a */ select 1"), "select");
        assert_eq!(first_word("/* unclosed"), "");
        assert!(check("SELECT ';' FROM t;  ").is_ok());
        assert!(check("SELECT 1; DELETE FROM t").is_err());
        assert!(check("/* x */ VACUUM INTO '/tmp/copy.db'").is_err());
        assert!(check("vacuum").is_err());
        assert!(check("EXPLAIN VACUUM").is_ok());
    }

    #[tokio::test]
    async fn read_only_reads_and_refuses_every_write() {
        let (dir, connection) = database(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, note TEXT); INSERT INTO items VALUES (1, 'a'), (2, NULL);",
        )
        .await;
        let outcome = run_read_only(&connection, "SELECT * FROM items ORDER BY id", 1).await.unwrap();
        let [result] = outcome.results.as_slice() else { panic!("{outcome:?}") };
        assert_eq!(result.rows, [vec![Some("1".into()), Some("a".into())]]);
        assert!(result.truncated);

        let copy = dir.path().join("copy.db");
        let attempts = [
            "INSERT INTO items VALUES (3, 'c')".to_string(),
            "UPDATE items SET note = 'x'".to_string(),
            "DELETE FROM items".to_string(),
            "DROP TABLE items".to_string(),
            "CREATE TABLE created (a)".to_string(),
            "PRAGMA user_version = 7".to_string(),
            "SELECT 1; DELETE FROM items".to_string(),
            "PRAGMA query_only = 0; DELETE FROM items".to_string(),
            format!("VACUUM INTO '{}'", copy.display()),
            format!("ATTACH '{}' AS other", dir.path().join("new.db").display()),
        ];
        for attempt in &attempts {
            assert!(run_read_only(&connection, attempt, 100).await.is_err(), "ran: {attempt}");
        }
        let after = run_read_only(&connection, "SELECT count(*), (SELECT count(*) FROM sqlite_master) FROM items", 9);
        assert_eq!(after.await.unwrap().results[0].rows, [vec![Some("2".into()), Some("1".into())]]);
        let version = run_read_only(&connection, "PRAGMA user_version", 9).await.unwrap();
        assert_eq!(version.results[0].rows, [vec![Some("0".into())]]);
        assert!(!copy.exists());
        assert!(!dir.path().join("new.db").exists());
    }
}
