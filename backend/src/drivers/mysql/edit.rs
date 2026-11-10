//! Row edits from the table grid for MySQL, built by the shared `edit_sql` builder.
//!
//! MySQL converts a text parameter to the column's type on assignment, so values
//! need no cast, but for binary and bit columns: those go back from the form the
//! grid shows them in (`TextForm`). Two things keep the PostgreSQL guarantees: the session is put in
//! strict mode, so a value that doesn't fit fails instead of being truncated with
//! a warning, and only transactional tables can be edited, since a ROLLBACK can't
//! undo changes to MyISAM or MEMORY tables.

use super::super::sqlx_edits;
use super::{Conn, TextForm, connect, table_columns, with_timeout};
use crate::connections::model::Connection;
use crate::drivers::edit_sql::{self, Dialect, default_keyword};
use crate::drivers::{ApplySummary, CellValue, Column, RowChange, TableRef};
use crate::error::{AppError, Result};

const MYSQL: Dialect = Dialect {
    quote: ['`', '`'],
    numbered: None,
    value: |marker, data_type| TextForm::of(data_type).write(marker),
    default_row: "() VALUES ()",
    backslash_escapes: true,
    string_prefix: "",
    default_value: default_keyword,
};

pub async fn apply_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<ApplySummary> {
    let mut conn = with_timeout(connect(connection, password, Some(&table.database))).await?;
    let columns = with_timeout(editable_columns(&mut conn, table)).await?;
    check_binary_values(&columns, changes)?;
    let statements = edit_sql::build(&MYSQL, &columns, table, changes)?;
    sqlx::query("SET SESSION sql_mode = CONCAT_WS(',', @@SESSION.sql_mode, 'STRICT_ALL_TABLES')")
        .execute(&mut conn)
        .await?;

    let mut transaction = conn.begin().await?;
    sqlx_edits::run::<sqlx::MySql>(&mut transaction, &MYSQL, &statements).await?;
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
    check_binary_values(&columns, changes)?;
    let statements = edit_sql::build(&MYSQL, &columns, table, changes)?;
    Ok(statements.iter().map(|statement| edit_sql::inline(&MYSQL, statement)).collect())
}

/// Refuses a value for a binary column that isn't `0x` and pairs of hex digits:
/// `UNHEX` would turn it into NULL without a word.
fn check_binary_values(columns: &[Column], changes: &[RowChange]) -> Result<()> {
    let is_hex = |text: &str| {
        text.strip_prefix("0x")
            .is_some_and(|digits| digits.len() % 2 == 0 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    let values = changes.iter().flat_map(|change| match change {
        RowChange::Update { values, .. } | RowChange::Insert { values } => Some(values),
        // Deletes carry no values, and whole documents aren't a SQL table's.
        _ => None,
    });
    for (name, value) in values.flatten() {
        let binary = columns.iter().any(|c| &c.name == name && TextForm::of(&c.data_type) == TextForm::Hex);
        if let CellValue::Text(text) = value
            && binary
            && !is_hex(text)
        {
            return Err(AppError::Invalid(format!(
                "{name} takes binary values as 0x followed by hex digits, like 0x0A1B."
            )));
        }
    }
    Ok(())
}

/// The table's columns, as the grid reads them. Refuses tables whose storage
/// engine has no transactions.
async fn editable_columns(conn: &mut Conn, table: &TableRef) -> Result<Vec<Column>> {
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

    table_columns(conn, table).await
}

#[cfg(test)]
mod tests {
    use sqlx::AssertSqlSafe;

    use std::collections::BTreeMap;

    use super::super::tests::{PASSWORD, run, servers, table};
    use super::*;

    /// Keys (`Option<String>`) or values (`CellValue`) from text, `None` being NULL.
    fn map<V: From<Option<String>>>(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, V> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.map(String::from).into())).collect()
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

    async fn rows(conn: &mut Conn, database: &str) -> Vec<(i32, String, i32, Option<String>)> {
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
    async fn binary_and_bit_values_are_read_and_written_back_as_shown() {
        for server in servers() {
            let database = "dbforge_medit_bin";
            run(
                &server,
                &format!(
                    "DROP DATABASE IF EXISTS {database};
                     CREATE DATABASE {database};
                     CREATE TABLE {database}.bins (id binary(4) PRIMARY KEY, flags bit(3) NOT NULL, data varbinary(8));
                     INSERT INTO {database}.bins VALUES (x'ACED0005', b'101', x'FF00'), (x'00000001', b'000', NULL);"
                ),
            )
            .await;
            let bins = table(database, "bins");
            let shown = || async {
                super::super::fetch_rows(&server, PASSWORD, &bins, &crate::drivers::RowFilter::default(), 0, 50)
                    .await
                    .unwrap()
                    .rows
            };
            let some = |v: &str| Some(v.to_string());
            assert_eq!(
                shown().await,
                [[some("0x00000001"), some("0"), None], [some("0xACED0005"), some("5"), some("0xFF00")]]
            );

            // The key as the grid shows it finds its row; values go back in the same form.
            let changes = vec![
                RowChange::Update {
                    key: map(&[("id", Some("0xACED0005"))]),
                    values: map(&[("data", Some("0x00FE")), ("flags", Some("6"))]),
                },
                RowChange::Delete { key: map(&[("id", Some("0x00000001"))]) },
                RowChange::Insert { values: map(&[("id", Some("0xFFFFFFFF")), ("flags", Some("1"))]) },
            ];
            let preview = preview_changes(&server, PASSWORD, &bins, &changes).await.unwrap();
            assert_eq!(
                preview[0],
                "UPDATE `dbforge_medit_bin`.`bins` SET `data` = UNHEX(SUBSTR('0x00FE', 3)), \
                 `flags` = CAST('6' AS UNSIGNED) WHERE `id` = UNHEX(SUBSTR('0xACED0005', 3))"
            );
            apply_changes(&server, PASSWORD, &bins, &changes).await.unwrap();
            assert_eq!(
                shown().await,
                [[some("0xACED0005"), some("6"), some("0x00FE")], [some("0xFFFFFFFF"), some("1"), None]]
            );

            // Text that isn't hex is refused, never stored as NULL.
            let bad = [RowChange::Update {
                key: map(&[("id", Some("0xACED0005"))]),
                values: map(&[("data", Some("hello"))]),
            }];
            let error = apply_changes(&server, PASSWORD, &bins, &bad).await.unwrap_err();
            assert!(error.to_string().contains("data takes binary values as 0x"), "{error}");
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn a_unique_key_is_not_a_primary_key_for_the_grid_either() {
        for server in servers() {
            let database = "dbforge_medit_unique";
            run(
                &server,
                &format!(
                    "DROP DATABASE IF EXISTS {database};
                     CREATE DATABASE {database};
                     CREATE TABLE {database}.tags (code varchar(5) NOT NULL UNIQUE, label text);
                     INSERT INTO {database}.tags VALUES ('a', 'x');"
                ),
            )
            .await;
            let tags = table(database, "tags");
            // MySQL shows `code` as PRI; the grid must not offer it as the key to edit by.
            let page = super::super::fetch_rows(&server, PASSWORD, &tags, &crate::drivers::RowFilter::default(), 0, 10)
                .await
                .unwrap();
            assert!(page.columns.iter().all(|c| !c.primary_key), "{:?}", page.columns);
            let diagram = super::super::schema_diagram(&server, PASSWORD, database, database).await.unwrap();
            assert!(diagram.tables[0].columns.iter().all(|c| !c.primary_key));
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
