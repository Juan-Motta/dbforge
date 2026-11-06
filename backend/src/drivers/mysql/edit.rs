//! Row edits from the table grid for MySQL, built by the shared `edit_sql` builder.
//!
//! MySQL converts a text parameter to the column's type on assignment, so values
//! need no cast. Two things keep the PostgreSQL guarantees: the session is put in
//! strict mode, so a value that doesn't fit fails instead of being truncated with
//! a warning, and only transactional tables can be edited, since a ROLLBACK can't
//! undo changes to MyISAM or MEMORY tables.

use sqlx::mysql::MySqlConnection;
use sqlx::{AssertSqlSafe, Connection as _};

use super::{connect, with_timeout};
use crate::connections::model::Connection;
use crate::drivers::edit_sql::{self, Dialect};
use crate::drivers::{ApplySummary, Column, RowChange, TableRef};
use crate::error::{AppError, Result};

const MYSQL: Dialect = Dialect {
    quote: '`',
    numbered: false,
    value: |marker, _| marker.to_string(),
    default_row: "() VALUES ()",
    backslash_escapes: true,
};

pub async fn apply_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<ApplySummary> {
    let mut conn = with_timeout(connect(connection, password, Some(&table.database))).await?;
    let columns = with_timeout(editable_columns(&mut conn, table)).await?;
    let statements = edit_sql::build(&MYSQL, &columns, table, changes)?;
    sqlx::query("SET SESSION sql_mode = CONCAT_WS(',', @@SESSION.sql_mode, 'STRICT_ALL_TABLES')")
        .execute(&mut conn)
        .await?;

    let mut transaction = conn.begin().await?;
    for statement in &statements {
        let mut query = sqlx::query(AssertSqlSafe(statement.sql.clone()));
        for param in &statement.params {
            query = query.bind(param.clone());
        }
        // The client asks for matched rows (FOUND_ROWS), so an UPDATE that sets a
        // value to what it already was still counts its row.
        let affected = query.execute(&mut *transaction).await?.rows_affected();
        if let Some(key) = &statement.key
            && affected != 1
        {
            return Err(edit_sql::changed_elsewhere(key));
        }
    }
    transaction.commit().await?;
    conn.close().await?;
    Ok(edit_sql::summarize(changes))
}

/// The statements `apply_changes` would run, with values written in, for review.
pub async fn preview_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<Vec<String>> {
    let mut conn = with_timeout(connect(connection, password, Some(&table.database))).await?;
    let columns = with_timeout(editable_columns(&mut conn, table)).await?;
    conn.close().await?;
    let statements = edit_sql::build(&MYSQL, &columns, table, changes)?;
    Ok(statements.iter().map(|statement| edit_sql::inline(&MYSQL, statement)).collect())
}

/// The table's columns, marking only the real primary key (not a unique index
/// MySQL would show as `PRI` when there is no primary key). Refuses tables whose
/// storage engine has no transactions.
async fn editable_columns(conn: &mut MySqlConnection, table: &TableRef) -> Result<Vec<Column>> {
    let engine: Option<Option<String>> = sqlx::query_scalar(
        "SELECT CAST(engine AS CHAR) FROM information_schema.tables WHERE table_schema = ? AND table_name = ?",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_optional(&mut *conn)
    .await?;
    match engine.flatten().as_deref() {
        None => return Err(AppError::Invalid(format!("Table {}.{} not found", table.schema, table.name))),
        Some("InnoDB") => {}
        Some(other) => {
            return Err(AppError::Invalid(format!(
                "{}.{} uses the {other} engine, which has no transactions, so edits can't be applied all-or-nothing.",
                table.schema, table.name
            )));
        }
    }

    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT CAST(c.column_name AS CHAR), CAST(c.column_type AS CHAR),
                CAST(EXISTS (SELECT 1 FROM information_schema.statistics s
                             WHERE s.table_schema = c.table_schema AND s.table_name = c.table_name
                               AND s.index_name = 'PRIMARY' AND s.column_name = c.column_name) AS SIGNED)
         FROM information_schema.columns c
         WHERE c.table_schema = ? AND c.table_name = ? ORDER BY c.ordinal_position",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, data_type, primary)| Column { name, data_type, primary_key: primary != 0 })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::tests::{PASSWORD, run, servers, table};
    use super::*;

    fn map(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.map(String::from))).collect()
    }

    fn setup(database: &str) -> String {
        format!(
            "DROP DATABASE IF EXISTS {database};
             CREATE DATABASE {database};
             CREATE TABLE {database}.items (
                 id int PRIMARY KEY,
                 name varchar(5) NOT NULL,
                 qty int NOT NULL DEFAULT 5,
                 status enum('new', 'done')
             );
             INSERT INTO {database}.items VALUES (1, 'one', 1, 'new'), (2, 'two', 2, NULL), (3, 'three', 3, NULL);
             CREATE TABLE {database}.loose (name text);
             CREATE TABLE {database}.legacy (id int PRIMARY KEY) ENGINE = MyISAM;"
        )
    }

    async fn rows(conn: &mut MySqlConnection, database: &str) -> Vec<(i32, String, i32, Option<String>)> {
        sqlx::query_as(AssertSqlSafe(format!(
            "SELECT id, name, qty, CAST(status AS CHAR) FROM {database}.items ORDER BY id"
        )))
        .fetch_all(conn)
        .await
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn applies_updates_inserts_and_deletes_together() {
        for server in servers() {
            // Each test reads `local()`; here it means the server under test.
            let local = || server.clone();
            let mut conn = run(&server, &setup("dbforge_medit1")).await;
            let changes = vec![
                RowChange::Delete { key: map(&[("id", Some("3"))]) },
                // Setting a value to what it already was must still count as one row.
                RowChange::Update {
                    key: map(&[("id", Some("1"))]),
                    values: map(&[("name", Some("one")), ("status", None)]),
                },
                RowChange::Update { key: map(&[("id", Some("2"))]), values: map(&[("status", Some("done"))]) },
                RowChange::Insert { values: map(&[("id", Some("4")), ("name", Some("it's"))]) },
            ];

            let summary = apply_changes(&local(), PASSWORD, &table("dbforge_medit1", "items"), &changes).await.unwrap();

            assert_eq!(summary, ApplySummary { updated: 2, inserted: 1, deleted: 1 });
            assert_eq!(
                rows(&mut conn, "dbforge_medit1").await,
                [(1, "one".into(), 1, None), (2, "two".into(), 2, Some("done".into())), (4, "it's".into(), 5, None)]
            );
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn a_row_changed_elsewhere_rolls_everything_back() {
        for server in servers() {
            // Each test reads `local()`; here it means the server under test.
            let local = || server.clone();
            let mut conn = run(&server, &setup("dbforge_medit2")).await;
            let changes = vec![
                RowChange::Delete { key: map(&[("id", Some("1"))]) },
                RowChange::Update { key: map(&[("id", Some("99"))]), values: map(&[("name", Some("gone"))]) },
            ];
            let error =
                apply_changes(&local(), PASSWORD, &table("dbforge_medit2", "items"), &changes).await.unwrap_err();
            assert!(error.to_string().contains("id = 99 was changed or deleted"), "{error}");
            assert_eq!(rows(&mut conn, "dbforge_medit2").await.len(), 3, "the delete was rolled back");
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn rejects_values_too_long_and_unsafe_tables() {
        for server in servers() {
            // Each test reads `local()`; here it means the server under test.
            let local = || server.clone();
            let mut conn = run(&server, &setup("dbforge_medit3")).await;
            // Strict mode even when the server's own mode isn't.
            sqlx::query("SET GLOBAL sql_mode = ''").execute(&mut conn).await.unwrap();
            let too_long =
                [RowChange::Update { key: map(&[("id", Some("1"))]), values: map(&[("name", Some("abcdefghij"))]) }];
            let result = apply_changes(&local(), PASSWORD, &table("dbforge_medit3", "items"), &too_long).await;
            sqlx::query("SET GLOBAL sql_mode = DEFAULT").execute(&mut conn).await.unwrap();
            assert!(result.unwrap_err().to_string().contains("Data too long"));
            assert_eq!(rows(&mut conn, "dbforge_medit3").await[0].1, "one");

            let insert = [RowChange::Insert { values: map(&[]) }];
            let error =
                apply_changes(&local(), PASSWORD, &table("dbforge_medit3", "loose"), &insert).await.unwrap_err();
            assert!(error.to_string().contains("has no primary key"), "{error}");
            let error =
                apply_changes(&local(), PASSWORD, &table("dbforge_medit3", "legacy"), &insert).await.unwrap_err();
            assert!(error.to_string().contains("MyISAM engine"), "{error}");
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn previews_the_exact_statements() {
        for server in servers() {
            // Each test reads `local()`; here it means the server under test.
            let local = || server.clone();
            let _conn = run(&server, &setup("dbforge_medit4")).await;
            let changes = vec![
                RowChange::Update { key: map(&[("id", Some("1"))]), values: map(&[("name", Some("o'n\\e"))]) },
                RowChange::Insert { values: map(&[]) },
            ];
            let preview =
                preview_changes(&local(), PASSWORD, &table("dbforge_medit4", "items"), &changes).await.unwrap();
            assert_eq!(
                preview,
                [
                    r"UPDATE `dbforge_medit4`.`items` SET `name` = 'o''n\\e' WHERE `id` = '1'",
                    "INSERT INTO `dbforge_medit4`.`items` () VALUES ()",
                ]
            );
        }
    }
}
