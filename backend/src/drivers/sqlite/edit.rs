//! Row edits from the table grid for SQLite, built by the shared `edit_sql`
//! builder. SQLite converts a text parameter by the column's affinity, so values
//! need no cast; every change runs in one transaction, and each UPDATE or DELETE
//! must hit exactly one row.

use sqlx::{AssertSqlSafe, Connection as _};

use super::{connect, table_columns, with_timeout};
use crate::connections::model::Connection;
use crate::drivers::edit_sql::{self, Dialect};
use crate::drivers::{ApplySummary, RowChange, TableRef};
use crate::error::Result;

const SQLITE: Dialect = Dialect {
    quote: '"',
    numbered: false,
    value: |marker, _| marker.to_string(),
    default_row: "DEFAULT VALUES",
    backslash_escapes: false,
};

pub async fn apply_changes(connection: &Connection, table: &TableRef, changes: &[RowChange]) -> Result<ApplySummary> {
    let mut conn = with_timeout(connect(connection)).await?;
    let columns = with_timeout(table_columns(&mut conn, &table.name)).await?;
    // The builder qualifies the table with its schema, which is `main` here.
    let statements = edit_sql::build(&SQLITE, &columns, table, changes)?;

    let mut transaction = conn.begin().await?;
    for statement in &statements {
        let mut query = sqlx::query(AssertSqlSafe(statement.sql.clone()));
        for param in &statement.params {
            query = query.bind(param.clone());
        }
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

    fn map(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.map(String::from))).collect()
    }

    const ITEMS: &str = "
        CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, qty INTEGER NOT NULL DEFAULT 5,
                            doubled INTEGER AS (qty * 2));
        INSERT INTO items (id, name, qty) VALUES (1, 'one', 1), (2, 'two', 2), (3, 'three', 3);
        CREATE TABLE loose (name TEXT);";

    async fn rows(connection: &Connection) -> Vec<Vec<Option<String>>> {
        super::super::fetch_rows(connection, &table("items"), 0, 50).await.unwrap().rows
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
