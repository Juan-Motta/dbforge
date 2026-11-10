//! One statement that may only read, for callers that must not change data.
//!
//! The server enforces it, not the text: the session starts with
//! `default_transaction_read_only = on`, and the statement runs inside a
//! transaction block that is always rolled back. The block matters: a read-only
//! session alone still lets `DO $$ … COMMIT; INSERT … $$` end its transaction
//! and write in the next one, while inside a block that `COMMIT` fails. The
//! server's own parser also confirms the text is a single statement (a prepared
//! statement takes only one) before it runs, so a script such as
//! `SET default_transaction_read_only = off; COMMIT; DELETE …` never starts.
//!
//! Not prevented: what a read-only transaction allows but reaches outside the
//! database's tables, such as `pg_terminate_backend`, `pg_cancel_backend`,
//! `dblink`, or `COPY … TO PROGRAM` for superusers. Connect agents with a role
//! that has no such privileges.

use std::time::Instant;

use sqlx::{AssertSqlSafe, Executor, SqlSafeStr};

use super::{connect_with, read_results};
use crate::connections::model::Connection;
use crate::drivers::{QueryOutcome, with_timeout};
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
    let read_only =
        |options: sqlx::postgres::PgConnectOptions| options.options([("default_transaction_read_only", "on")]);
    let mut conn = with_timeout(connect_with(connection, password, database, read_only)).await?;
    let mut transaction = conn.begin().await?;
    let results = async {
        // Parsed as a prepared statement only to have the server refuse several.
        (&mut transaction).prepare(AssertSqlSafe(statement.to_owned()).into_sql_str()).await?;
        // The simple protocol returns every value as text, as the console shows it.
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
    use super::super::connect;
    use super::super::tests::{PASSWORD, local};
    use super::*;

    /// A fresh table `name` holding the rows 1 and 2.
    async fn table(name: &str) {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        let setup = format!(
            "DROP TABLE IF EXISTS {name}; CREATE TABLE {name} (id int PRIMARY KEY); INSERT INTO {name} VALUES (1), (2)"
        );
        sqlx::raw_sql(AssertSqlSafe(setup)).execute(&mut conn).await.unwrap();
    }

    async fn ids(name: &str) -> Vec<Vec<Option<String>>> {
        let query = format!("SELECT id FROM {name} ORDER BY id");
        run_read_only(&local(), PASSWORD, None, &query, 100).await.unwrap().results.remove(0).rows
    }

    fn rows(values: &[&str]) -> Vec<Vec<Option<String>>> {
        values.iter().map(|v| vec![Some(v.to_string())]).collect()
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn reads_rows_as_text() {
        table("dbforge_ro_read").await;
        let outcome =
            run_read_only(&local(), PASSWORD, None, "SELECT id, id * 1.5 AS half FROM dbforge_ro_read ORDER BY id", 1)
                .await
                .unwrap();
        let [result] = outcome.results.as_slice() else { panic!("{outcome:?}") };
        assert_eq!(result.rows, [vec![Some("1".into()), Some("1.5".into())]]);
        assert!(result.truncated);
        assert_eq!(result.rows_affected, 2);
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn refuses_writes_ddl_scripts_and_escapes() {
        let name = "dbforge_ro_write";
        table(name).await;
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        let setup = "DROP TABLE IF EXISTS dbforge_ro_new; CREATE SEQUENCE IF NOT EXISTS dbforge_ro_seq";
        sqlx::raw_sql(setup).execute(&mut conn).await.unwrap();
        let attempts = [
            format!("INSERT INTO {name} VALUES (3)"),
            format!("UPDATE {name} SET id = id + 10"),
            format!("DELETE FROM {name}"),
            format!("DROP TABLE {name}"),
            "CREATE TABLE dbforge_ro_new (a int)".to_string(),
            format!("SELECT 1; DELETE FROM {name}"),
            format!("SET default_transaction_read_only = off; DELETE FROM {name}"),
            format!("COMMIT; DELETE FROM {name}"),
            // Ends the transaction from inside: refused only because it runs in a block.
            format!(
                "DO $$ BEGIN PERFORM set_config('default_transaction_read_only', 'off', false); COMMIT; \
                 DELETE FROM {name}; END $$"
            ),
            format!("WITH gone AS (DELETE FROM {name} RETURNING id) SELECT * FROM gone"),
            "SELECT nextval('dbforge_ro_seq')".to_string(),
        ];
        for attempt in &attempts {
            let result = run_read_only(&local(), PASSWORD, None, attempt, 100).await;
            assert!(result.is_err(), "ran: {attempt}");
        }
        assert_eq!(ids(name).await, rows(&["1", "2"]));
        let created: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('dbforge_ro_new')::text").fetch_one(&mut conn).await.unwrap();
        assert_eq!(created, None);
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn a_statement_that_changes_the_session_changes_nothing_after_it() {
        let name = "dbforge_ro_session";
        table(name).await;
        // Each one is allowed to run, alone, and the connection is closed after it.
        for statement in ["SET default_transaction_read_only = off", "COMMIT", "SET TRANSACTION READ WRITE"] {
            run_read_only(&local(), PASSWORD, None, statement, 100).await.unwrap();
        }
        let error = run_read_only(&local(), PASSWORD, None, &format!("DELETE FROM {name}"), 100).await.unwrap_err();
        assert_eq!(error.to_string(), "cannot execute DELETE in a read-only transaction");
        assert_eq!(ids(name).await, rows(&["1", "2"]));
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn records_the_statement_in_the_log() {
        use crate::statement_log::store::tests::Collected;
        use crate::statement_log::{self, Origin, StatementLog};

        table("dbforge_ro_log").await;
        let log = StatementLog::new(50, Collected::default());
        let connection = local();
        let run = run_read_only(&connection, PASSWORD, None, "SELECT id FROM dbforge_ro_log", 100);
        statement_log::scoped(&log, "pg", Origin::Console, run).await.unwrap();
        let texts: Vec<_> = log.entries().iter().map(|e| e.text.clone()).collect();
        assert_eq!(texts, ["BEGIN", "SELECT id FROM dbforge_ro_log", "ROLLBACK"]);
    }
}
