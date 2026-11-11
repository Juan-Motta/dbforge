//! Row edits from the table grid for SQLite, built by the shared `edit_sql`
//! builder. SQLite converts a text parameter by the column's affinity, so values
//! need no cast; every change runs in one transaction, and each UPDATE or DELETE
//! must hit exactly one row.

use super::super::sqlx_edits;
use super::{connect, table_columns, with_timeout};
use crate::connections::model::Connection;
use crate::drivers::edit_sql::{self, Dialect};
use crate::drivers::{ApplySummary, ImportBatches, RowChange, TableRef};
use crate::error::Result;

const SQLITE: Dialect = Dialect {
    quote: ['"', '"'],
    numbered: None,
    value: |marker, _| marker.to_string(),
    default_row: "DEFAULT VALUES",
    backslash_escapes: false,
    string_prefix: "",
    // SQLite has no `SET c = DEFAULT`: the column's own DEFAULT expression, as the
    // schema stores it, is written out instead; a column without one defaults to NULL.
    default_value: |expression| expression.map_or_else(|| "NULL".to_string(), |e| format!("({e})")),
};

pub async fn apply_changes(connection: &Connection, table: &TableRef, changes: &[RowChange]) -> Result<ApplySummary> {
    let mut conn = with_timeout(connect(connection)).await?;
    let columns = with_timeout(table_columns(&mut conn, &table.name)).await?;
    // The builder qualifies the table with its schema, which is `main` here.
    let statements = edit_sql::build(&SQLITE, &columns, table, changes)?;

    let mut transaction = conn.begin().await?;
    sqlx_edits::run::<sqlx::Sqlite>(&mut transaction, &SQLITE, &statements).await?;
    transaction.commit().await?;
    conn.close().await?;
    Ok(edit_sql::summarize(changes))
}

/// The most parameters SQLite binds to one statement (its default limit).
const MAX_PARAMS: usize = 32_766;

/// Inserts an import's rows in one transaction: all of them or none.
pub async fn import_rows(
    connection: &Connection,
    table: &TableRef,
    names: &[String],
    batches: &mut ImportBatches,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<u64> {
    let mut conn = with_timeout(connect(connection)).await?;
    let columns = with_timeout(table_columns(&mut conn, &table.name)).await?;
    let mut transaction = conn.begin().await?;
    let inserted = sqlx_edits::insert_batches::<sqlx::Sqlite>(
        &mut transaction,
        &SQLITE,
        &columns,
        table,
        names,
        batches,
        MAX_PARAMS,
        |_, _| Ok(()),
        progress,
    )
    .await?;
    transaction.commit().await?;
    conn.close().await?;
    Ok(inserted)
}

pub async fn preview_changes(connection: &Connection, table: &TableRef, changes: &[RowChange]) -> Result<Vec<String>> {
    let mut conn = with_timeout(connect(connection)).await?;
    let columns = with_timeout(table_columns(&mut conn, &table.name)).await?;
    conn.close().await?;
    let statements = edit_sql::build(&SQLITE, &columns, table, changes)?;
    Ok(statements.iter().map(|statement| edit_sql::inline(&SQLITE, statement)).collect())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::tests::{database, table};
    use super::*;
    use crate::drivers::{CellValue, import_batches};

    /// Keys (`Option<String>`) or values (`CellValue`) from text, `None` being NULL.
    fn map<V: From<Option<String>>>(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, V> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.map(String::from).into())).collect()
    }

    const ITEMS: &str = "
        CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, qty INTEGER NOT NULL DEFAULT 5,
                            doubled INTEGER AS (qty * 2));
        INSERT INTO items (id, name, qty) VALUES (1, 'one', 1), (2, 'two', 2), (3, 'three', 3);
        CREATE TABLE loose (name TEXT);";

    async fn rows(connection: &Connection) -> Vec<Vec<Option<String>>> {
        super::super::fetch_rows(connection, &table("items"), &crate::drivers::RowFilter::default(), 0, 50)
            .await
            .unwrap()
            .rows
    }

    #[tokio::test]
    async fn applies_updates_inserts_and_deletes_together() {
        let (_dir, connection) = database(ITEMS).await;
        let changes = vec![
            RowChange::Delete { key: map(&[("id", Some("3"))]) },
            RowChange::Update { key: map(&[("id", Some("1"))]), values: map(&[("name", Some("uno"))]) },
            RowChange::Insert { values: map(&[("id", Some("4")), ("name", Some("it's"))]) },
        ];
        let summary = apply_changes(&connection, &table("items"), &changes).await.unwrap();
        assert_eq!(summary, ApplySummary { updated: 1, inserted: 1, deleted: 1 });

        let rows = rows(&connection).await;
        let names: Vec<_> =
            rows.iter().map(|r| (r[0].clone().unwrap(), r[1].clone().unwrap(), r[2].clone().unwrap())).collect();
        assert_eq!(
            names,
            [
                ("1".into(), "uno".into(), "1".into()),
                ("2".into(), "two".into(), "2".into()),
                ("4".into(), "it's".into(), "5".into())
            ]
        );
    }

    #[tokio::test]
    async fn a_row_changed_elsewhere_rolls_everything_back() {
        let (_dir, connection) = database(ITEMS).await;
        let changes = vec![
            RowChange::Delete { key: map(&[("id", Some("1"))]) },
            RowChange::Update { key: map(&[("id", Some("99"))]), values: map(&[("name", Some("gone"))]) },
        ];
        let error = apply_changes(&connection, &table("items"), &changes).await.unwrap_err();
        assert!(error.to_string().contains("id = 99 was changed or deleted"), "{error}");
        assert_eq!(rows(&connection).await.len(), 3);
    }

    #[tokio::test]
    async fn refuses_computed_columns_and_tables_without_a_primary_key() {
        let (_dir, connection) = database(ITEMS).await;
        let computed = [RowChange::Update { key: map(&[("id", Some("1"))]), values: map(&[("doubled", Some("9"))]) }];
        let error = apply_changes(&connection, &table("items"), &computed).await.unwrap_err();
        assert!(error.to_string().contains("doubled is computed"), "{error}");

        let insert = [RowChange::Insert { values: map(&[("name", Some("x"))]) }];
        let error = apply_changes(&connection, &table("loose"), &insert).await.unwrap_err();
        assert!(error.to_string().contains("has no primary key"), "{error}");
    }

    fn row(values: &[Option<&str>]) -> Vec<Option<String>> {
        values.iter().map(|v| v.map(String::from)).collect()
    }

    #[tokio::test]
    async fn imports_every_batch_into_a_table_without_a_primary_key() {
        let (_dir, connection) = database(ITEMS).await;
        let mut counted = Vec::new();
        let mut sent = import_batches(vec![Ok(vec![vec![Some("a")], vec![None]]), Ok(vec![vec![Some("it's")]])]);
        let inserted = import_rows(&connection, &table("loose"), &["name".into()], &mut sent, &mut |n| counted.push(n))
            .await
            .unwrap();
        assert_eq!((inserted, counted), (3, vec![2, 3]));
        let loose = super::super::fetch_rows(&connection, &table("loose"), &Default::default(), 0, 10).await.unwrap();
        assert_eq!(loose.rows, [row(&[Some("a")]), row(&[None]), row(&[Some("it's")])]);
    }

    #[tokio::test]
    async fn an_import_that_fails_partway_adds_nothing() {
        let (_dir, connection) = database(ITEMS).await;
        let names = ["id".to_string(), "name".to_string()];
        // The second batch breaks NOT NULL; the file reader failing would do the same.
        let mut sent = import_batches(vec![Ok(vec![vec![Some("10"), Some("ten")]]), Ok(vec![vec![Some("11"), None]])]);
        let error = import_rows(&connection, &table("items"), &names, &mut sent, &mut |_| {}).await.unwrap_err();
        assert!(error.to_string().contains("NOT NULL"), "{error}");
        let mut unreadable = import_batches(vec![
            Ok(vec![vec![Some("12"), Some("twelve")]]),
            Err(crate::error::AppError::Invalid("Row 3 has 3 values, but the file has 2 columns.".into())),
        ]);
        let error = import_rows(&connection, &table("items"), &names, &mut unreadable, &mut |_| {}).await.unwrap_err();
        assert_eq!(error.to_string(), "Row 3 has 3 values, but the file has 2 columns.");
        assert_eq!(rows(&connection).await.len(), 3);
    }

    #[tokio::test]
    async fn puts_a_column_back_to_its_default() {
        let (_dir, connection) = database(ITEMS).await;
        let changes = vec![RowChange::Update {
            key: map(&[("id", Some("1"))]),
            values: [("qty".to_string(), CellValue::Default), ("name".to_string(), CellValue::Text("uno".into()))]
                .into(),
        }];
        apply_changes(&connection, &table("items"), &changes).await.unwrap();
        let first = &rows(&connection).await[0];
        assert_eq!((first[1].as_deref(), first[2].as_deref()), (Some("uno"), Some("5")));

        let preview = preview_changes(&connection, &table("items"), &changes).await.unwrap();
        assert_eq!(preview, ["UPDATE \"main\".\"items\" SET \"name\" = 'uno', \"qty\" = (5) WHERE \"id\" = '1'"]);
    }

    #[tokio::test]
    async fn previews_the_exact_statements() {
        let (_dir, connection) = database(ITEMS).await;
        let changes = vec![
            RowChange::Update { key: map(&[("id", Some("1"))]), values: map(&[("name", Some("o'ne"))]) },
            RowChange::Insert { values: map(&[]) },
        ];
        let preview = preview_changes(&connection, &table("items"), &changes).await.unwrap();
        assert_eq!(
            preview,
            [
                "UPDATE \"main\".\"items\" SET \"name\" = 'o''ne' WHERE \"id\" = '1'",
                "INSERT INTO \"main\".\"items\" DEFAULT VALUES",
            ]
        );
    }
}
