//! Row edits from the table grid for SQL Server, built by the shared `edit_sql`
//! builder. Values are sent as nvarchar parameters, which SQL Server converts to
//! each column's type; no cast with a length, so a value too long for its column
//! is an error instead of being cut. Every change runs in one transaction, and
//! each UPDATE or DELETE must hit exactly one row.

use super::logged::Statement;
use super::{Client, connect, table_columns, with_timeout};
use crate::connections::model::Connection;
use crate::drivers::edit_sql::{self, Dialect, default_keyword};
use crate::drivers::{ApplySummary, RowChange, TableRef};
use crate::error::Result;

const SQLSERVER: Dialect = Dialect {
    quote: ['[', ']'],
    numbered: Some("@P"),
    value,
    default_row: "DEFAULT VALUES",
    backslash_escapes: false,
    string_prefix: "N",
    default_value: default_keyword,
};

/// Text converts implicitly to every type but these: binary takes the `0x…`
/// form the grid shows, and datetime reads `yyyy-mm-dd hh:mi:ss.mmm` whatever
/// the session's language and date format.
fn value(marker: &str, data_type: &str) -> String {
    let base = data_type.split('(').next().unwrap_or(data_type);
    match base {
        "binary" | "varbinary" | "image" => format!("CONVERT(varbinary(max), {marker}, 1)"),
        "datetime" | "smalldatetime" => format!("CONVERT({base}, {marker}, 121)"),
        _ => marker.to_string(),
    }
}

pub async fn apply_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<ApplySummary> {
    let mut client = with_timeout(connect(connection, password, Some(&table.database))).await?;
    let columns = with_timeout(table_columns(&mut client, table)).await?;
    let statements = edit_sql::build(&SQLSERVER, &columns, table, changes)?;

    client.simple_query("BEGIN TRANSACTION").await?.into_results().await?;
    let applied = run_all(&mut client, &statements).await;
    let ending = if applied.is_ok() { "COMMIT" } else { "ROLLBACK" };
    client.simple_query(ending).await?.into_results().await?;
    client.close().await?;
    applied?;
    Ok(edit_sql::summarize(changes))
}

async fn run_all(client: &mut Client, statements: &[edit_sql::Statement]) -> Result<()> {
    for statement in statements {
        let bound = statement
            .params
            .iter()
            .fold(Statement::new(statement.sql.as_str()), |bound, param| bound.bind(param.as_deref()));
        let affected = client.execute(bound).await?.total();
        if let Some(key) = &statement.key
            && affected != 1
        {
            return Err(edit_sql::changed_elsewhere(key));
        }
    }
    Ok(())
}

pub async fn preview_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<Vec<String>> {
    let mut client = with_timeout(connect(connection, password, Some(&table.database))).await?;
    let columns = with_timeout(table_columns(&mut client, table)).await?;
    client.close().await?;
    let statements = edit_sql::build(&SQLSERVER, &columns, table, changes)?;
    Ok(statements.iter().map(|statement| edit_sql::inline(&SQLSERVER, statement)).collect())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::tests::{PASSWORD, database, table};
    use super::*;
    use crate::statement_log::store::tests::Collected;
    use crate::statement_log::{self, Origin, StatementLog};

    /// Keys (`Option<String>`) or values (`CellValue`) from text, `None` being NULL.
    fn map<V: From<Option<String>>>(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, V> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.map(String::from).into())).collect()
    }

    const ITEMS: &str = "
        CREATE TABLE items (
            id int PRIMARY KEY, name nvarchar(10) NOT NULL, qty int NOT NULL DEFAULT 5,
            blob varbinary(4), seen datetime, doubled AS qty * 2
        );
        INSERT INTO items (id, name, qty) VALUES (1, 'one', 1), (2, 'two', 2), (3, 'three', 3);
        CREATE TABLE counted (id int IDENTITY PRIMARY KEY, name nvarchar(10));
        CREATE TABLE loose (name nvarchar(10));";

    async fn rows(connection: &Connection, name: &str) -> Vec<Vec<Option<String>>> {
        super::super::fetch_rows(
            connection,
            PASSWORD,
            &table(&connection.database, name),
            &crate::drivers::RowFilter::default(),
            0,
            50,
        )
        .await
        .unwrap()
        .rows
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn applies_updates_inserts_and_deletes_together() {
        let connection = database("dbforge_edit", ITEMS).await;
        let changes = vec![
            RowChange::Delete { key: map(&[("id", Some("3"))]) },
            RowChange::Update {
                key: map(&[("id", Some("1"))]),
                values: map(&[
                    ("name", Some("über")),
                    ("blob", Some("0x0A0B")),
                    ("seen", Some("2024-03-04 05:06:07.123")),
                ]),
            },
            RowChange::Insert { values: map(&[("id", Some("4")), ("name", Some("it's"))]) },
        ];
        let items = table("dbforge_edit", "items");
        let summary = apply_changes(&connection, PASSWORD, &items, &changes).await.unwrap();
        assert_eq!(summary, ApplySummary { updated: 1, inserted: 1, deleted: 1 });

        let items = rows(&connection, "items").await;
        let some = |v: &str| Some(v.to_string());
        assert_eq!(
            items[0],
            [some("1"), some("über"), some("1"), some("0x0A0B"), some("2024-03-04 05:06:07.123"), some("2")]
        );
        assert_eq!(items[2], [some("4"), some("it's"), some("5"), None, None, some("10")]);
        assert_eq!(items.len(), 3);

        // An identity column takes its value from the server.
        let insert =
            [RowChange::Insert { values: map(&[("name", Some("x"))]) }, RowChange::Insert { values: map(&[]) }];
        apply_changes(&connection, PASSWORD, &table("dbforge_edit", "counted"), &insert).await.unwrap();
        let counted = rows(&connection, "counted").await;
        assert_eq!(counted, [[some("1"), some("x")], [some("2"), None]]);
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn a_failed_change_rolls_everything_back() {
        let connection = database("dbforge_rollback", ITEMS).await;
        let items = table("dbforge_rollback", "items");
        let missing = vec![
            RowChange::Delete { key: map(&[("id", Some("1"))]) },
            RowChange::Update { key: map(&[("id", Some("99"))]), values: map(&[("name", Some("gone"))]) },
        ];
        let error = apply_changes(&connection, PASSWORD, &items, &missing).await.unwrap_err();
        assert!(error.to_string().contains("id = 99 was changed or deleted"), "{error}");

        // Too long for nvarchar(10): an error, never a cut value.
        let long = vec![
            RowChange::Delete { key: map(&[("id", Some("1"))]) },
            RowChange::Update { key: map(&[("id", Some("2"))]), values: map(&[("name", Some("much too long"))]) },
        ];
        let error = apply_changes(&connection, PASSWORD, &items, &long).await.unwrap_err();
        assert!(error.to_string().contains("truncated"), "{error}");
        assert_eq!(rows(&connection, "items").await.len(), 3);
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn records_the_transaction_with_values_written_in() {
        let connection = database("dbforge_edit_log", ITEMS).await;
        let items = table("dbforge_edit_log", "items");
        let missing = vec![
            RowChange::Delete { key: map(&[("id", Some("1"))]) },
            RowChange::Update { key: map(&[("id", Some("99"))]), values: map(&[("name", Some("it's"))]) },
        ];
        let log = StatementLog::new(50, Collected::default());

        let applied =
            statement_log::scoped(&log, "ms", Origin::Edit, apply_changes(&connection, PASSWORD, &items, &missing));
        assert!(applied.await.is_err());

        let entries = log.entries();
        let columns = &entries[0];
        assert!(columns.text.contains("QUOTENAME(N'dbo') + '.' + QUOTENAME(N'items')"), "{}", columns.text);
        let ran: Vec<_> =
            entries.iter().skip(1).map(|e| (e.text.split_whitespace().next().unwrap_or_default(), e.rows)).collect();
        assert_eq!(ran, [("BEGIN", None), ("DELETE", Some(1)), ("UPDATE", Some(0)), ("ROLLBACK", None)]);
        assert!(entries[3].text.contains("N'it''s'"), "{}", entries[3].text);
        assert!(entries.iter().all(|e| e.database.as_deref() == Some("dbforge_edit_log") && e.error.is_none()));
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn refuses_computed_columns_and_tables_without_a_primary_key() {
        let connection = database("dbforge_refuse", ITEMS).await;
        let computed = [RowChange::Update { key: map(&[("id", Some("1"))]), values: map(&[("doubled", Some("9"))]) }];
        let error =
            apply_changes(&connection, PASSWORD, &table("dbforge_refuse", "items"), &computed).await.unwrap_err();
        assert!(error.to_string().contains("doubled is computed"), "{error}");

        let identity = [RowChange::Insert { values: map(&[("id", Some("9"))]) }];
        let error =
            apply_changes(&connection, PASSWORD, &table("dbforge_refuse", "counted"), &identity).await.unwrap_err();
        assert!(error.to_string().contains("id is computed"), "{error}");

        let insert = [RowChange::Insert { values: map(&[("name", Some("x"))]) }];
        let error = apply_changes(&connection, PASSWORD, &table("dbforge_refuse", "loose"), &insert).await.unwrap_err();
        assert!(error.to_string().contains("has no primary key"), "{error}");
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn previews_the_exact_statements() {
        let connection = database("dbforge_preview", ITEMS).await;
        let changes = vec![
            RowChange::Update {
                key: map(&[("id", Some("1"))]),
                values: map(&[("blob", Some("0x01")), ("name", Some("o'ne"))]),
            },
            RowChange::Insert { values: map(&[]) },
        ];
        let preview =
            preview_changes(&connection, PASSWORD, &table("dbforge_preview", "items"), &changes).await.unwrap();
        assert_eq!(
            preview,
            [
                "UPDATE [dbo].[items] SET [blob] = CONVERT(varbinary(max), N'0x01', 1), [name] = N'o''ne' WHERE [id] = N'1'",
                "INSERT INTO [dbo].[items] DEFAULT VALUES",
            ]
        );
    }
}
