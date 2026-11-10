//! One statement that may only read, for callers that must not change data.
//!
//! The server enforces it, not the text: the session is set to
//! `SET SESSION TRANSACTION READ ONLY` (the same syntax on MySQL and MariaDB,
//! whatever each calls the variable), under which both refuse DML and DDL,
//! and the statement runs inside a transaction that is always rolled back. The
//! server's own parser also confirms the text is a single statement (a
//! prepared statement takes only one) before it runs, so a script such as
//! `SET SESSION TRANSACTION READ WRITE; COMMIT; DELETE …` never starts.
//!
//! Not prevented: statements a read-only transaction allows that act on the
//! server rather than its tables, such as `SET GLOBAL`, `KILL`, or
//! `SELECT … INTO OUTFILE`; they need administrative privileges (SUPER,
//! SYSTEM_VARIABLES_ADMIN, CONNECTION_ADMIN, FILE). A stored procedure that
//! commits and switches its own session back to read-write is not prevented
//! either. Connect agents with an account that has none of these privileges.
//! Statements the prepared protocol doesn't take are refused by the server.

use std::time::Instant;

use sqlx::{AssertSqlSafe, Executor, SqlSafeStr};

use super::{connect, read_results};
use crate::connections::model::{Connection, Engine};
use crate::drivers::{QueryOutcome, READ_ONLY_TIME_LIMIT, with_timeout};
use crate::error::Result;

/// Runs `statement` on a new read-only connection that is closed afterwards.
pub async fn run_read_only(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    statement: &str,
    row_limit: usize,
) -> Result<QueryOutcome> {
    let started = Instant::now();
    let mut conn = with_timeout(connect(connection, password, database)).await?;
    sqlx::raw_sql("SET SESSION TRANSACTION READ ONLY").execute(&mut conn).await?;
    // The server stops the statement after the limit, so a dropped caller leaves nothing running.
    let limit = match connection.engine {
        Engine::Mariadb => format!("SET SESSION max_statement_time = {}", READ_ONLY_TIME_LIMIT.as_secs()),
        _ => format!("SET SESSION max_execution_time = {}", READ_ONLY_TIME_LIMIT.as_millis()),
    };
    sqlx::raw_sql(AssertSqlSafe(limit)).execute(&mut conn).await?;
    let mut transaction = conn.begin().await?;
    let results = async {
        // Prepared only to have the server refuse several statements.
        (&mut transaction).prepare(AssertSqlSafe(statement.to_owned()).into_sql_str()).await?;
        // The text protocol returns every value as text, as the console shows it.
        read_results(sqlx::raw_sql(AssertSqlSafe(statement)).fetch_many(&mut transaction), row_limit).await
    }
    .await;
    let rolled_back = transaction.rollback().await;
    let results = results?;
    rolled_back?;
    conn.close().await?;
    Ok(QueryOutcome { results, elapsed_ms: started.elapsed().as_millis() as u64 })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, run, servers};
    use super::*;

    /// A fresh database `name` with table `items` holding the rows 1 and 2.
    async fn database(server: &Connection, name: &str) -> Connection {
        let setup = format!(
            "DROP DATABASE IF EXISTS {name}; CREATE DATABASE {name};
             CREATE TABLE {name}.items (id int PRIMARY KEY); INSERT INTO {name}.items VALUES (1), (2);"
        );
        run(server, &setup).await;
        Connection { database: name.into(), ..server.clone() }
    }

    async fn ids(connection: &Connection) -> Vec<Vec<Option<String>>> {
        let outcome = run_read_only(connection, PASSWORD, None, "SELECT id FROM items ORDER BY id", 100);
        outcome.await.unwrap().results.remove(0).rows
    }

    fn rows(values: &[&str]) -> Vec<Vec<Option<String>>> {
        values.iter().map(|v| vec![Some(v.to_string())]).collect()
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn reads_rows_as_text() {
        for server in servers() {
            let connection = database(&server, "dbforge_ro_read").await;
            let outcome =
                run_read_only(&connection, PASSWORD, None, "SELECT id, id / 2 AS half FROM items ORDER BY id", 1)
                    .await
                    .unwrap();
            let [result] = outcome.results.as_slice() else { panic!("{outcome:?}") };
            assert_eq!(result.rows, [vec![Some("1".into()), Some("0.5000".into())]], "{}", server.engine.label());
            assert!(result.truncated);
            assert_eq!(result.rows_affected, 2);

            let other = run_read_only(&server, PASSWORD, Some("dbforge_ro_read"), "SELECT COUNT(*) FROM items", 10);
            assert_eq!(other.await.unwrap().results[0].rows, rows(&["2"]));
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn refuses_writes_ddl_scripts_and_escapes() {
        let attempts = [
            "INSERT INTO items VALUES (3)",
            "UPDATE items SET id = id + 10",
            "DELETE FROM items",
            "DROP TABLE items",
            "CREATE TABLE created (a int)",
            "TRUNCATE TABLE items",
            "SELECT 1; DELETE FROM items",
            "SET SESSION TRANSACTION READ WRITE; DELETE FROM items",
            "COMMIT; DELETE FROM items",
        ];
        for server in servers() {
            let connection = database(&server, "dbforge_ro_write").await;
            for attempt in attempts {
                let result = run_read_only(&connection, PASSWORD, None, attempt, 100).await;
                assert!(result.is_err(), "{} ran: {attempt}", server.engine.label());
            }
            assert_eq!(ids(&connection).await, rows(&["1", "2"]));
            let tables = run_read_only(&connection, PASSWORD, None, "SHOW TABLES", 10).await.unwrap();
            assert_eq!(tables.results[0].rows, rows(&["items"]));
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn a_statement_that_changes_the_session_changes_nothing_after_it() {
        for server in servers() {
            let connection = database(&server, "dbforge_ro_session").await;
            for statement in ["SET SESSION TRANSACTION READ WRITE", "COMMIT"] {
                run_read_only(&connection, PASSWORD, None, statement, 100).await.unwrap();
            }
            let error = run_read_only(&connection, PASSWORD, None, "DELETE FROM items", 100).await.unwrap_err();
            assert!(error.to_string().starts_with("Cannot execute statement in a READ ONLY transaction"), "{error}");
            assert_eq!(ids(&connection).await, rows(&["1", "2"]));
        }
    }
}
