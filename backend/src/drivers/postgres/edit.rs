//! Row edits from the table grid: updates, inserts and deletes, keyed by primary key.
//!
//! Values travel as parameters, never spliced into the SQL, and are cast to each
//! column's real type as the catalog reports it. Everything runs in one
//! transaction, and every UPDATE or DELETE must hit exactly one row: if a row was
//! changed or deleted since it was loaded, nothing is applied.

use std::collections::{BTreeMap, BTreeSet};

use sqlx::postgres::PgConnection;
use sqlx::{AssertSqlSafe, Connection as _};

use super::{connect, quote_ident, with_timeout};
use crate::connections::model::Connection;
use crate::drivers::{ApplySummary, Column, RowChange, TableRef};
use crate::error::{AppError, Result};

/// One statement with its parameters, `$1`, `$2`… in order.
#[derive(Debug, PartialEq)]
struct Statement {
    sql: String,
    params: Vec<Option<String>>,
    /// Set for UPDATE and DELETE: the key that must match exactly one row.
    key: Option<String>,
}

pub async fn apply_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<ApplySummary> {
    let mut conn = with_timeout(connect(connection, password, Some(&table.database))).await?;
    let statements = build(&with_timeout(editable_columns(&mut conn, table)).await?, table, changes)?;

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
            // Dropping the transaction rolls everything back.
            return Err(AppError::Invalid(format!(
                "The row with {key} was changed or deleted since it was loaded. Nothing was applied; reload and try again."
            )));
        }
    }
    transaction.commit().await?;
    conn.close().await?;
    Ok(summarize(changes))
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
    Ok(build(&columns, table, changes)?.iter().map(inline).collect())
}

/// Columns with their type's internal, schema-qualified name: `"pg_catalog"."varchar"`,
/// not `character varying(5)`. An explicit cast to a length-limited type silently
/// truncates, and the SQL spelling `character` even means `character(1)`; the
/// internal name has no length, so the column's own limit applies on assignment
/// and a value that doesn't fit is rejected.
async fn editable_columns(conn: &mut PgConnection, table: &TableRef) -> Result<Vec<Column>> {
    let rows: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT a.attname::text, format('%I.%I', tn.nspname, t.typname),
                COALESCE(a.attnum = ANY((i.indkey::int2[])[0:i.indnkeyatts - 1]), false)
         FROM pg_attribute a
         JOIN pg_type t ON t.oid = a.atttypid
         JOIN pg_namespace tn ON tn.oid = t.typnamespace
         LEFT JOIN pg_index i ON i.indrelid = a.attrelid AND i.indisprimary
         WHERE a.attrelid = format('%I.%I', $1::text, $2::text)::regclass AND a.attnum > 0 AND NOT a.attisdropped
         ORDER BY a.attnum",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|(name, data_type, primary_key)| Column { name, data_type, primary_key }).collect())
}

fn build(columns: &[Column], table: &TableRef, changes: &[RowChange]) -> Result<Vec<Statement>> {
    let types: BTreeMap<&str, &str> = columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str())).collect();
    let primary_key: BTreeSet<&str> = columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
    if primary_key.is_empty() {
        return Err(AppError::Invalid(format!(
            "{}.{} has no primary key, so its rows can't be identified safely for editing.",
            table.schema, table.name
        )));
    }
    let relation = format!("{}.{}", quote_ident(&table.schema), quote_ident(&table.name));

    changes
        .iter()
        .map(|change| {
            let mut params = Vec::new();
            match change {
                RowChange::Update { key, values } => {
                    if values.is_empty() {
                        return Err(AppError::Invalid("An update must change at least one column.".into()));
                    }
                    let set = assignments(values, &types, &mut params, ", ")?;
                    let filter = key_filter(key, &primary_key, &types, &mut params)?;
                    Ok(Statement {
                        sql: format!("UPDATE {relation} SET {set} WHERE {filter}"),
                        params,
                        key: Some(describe_key(key)),
                    })
                }
                RowChange::Delete { key } => {
                    let filter = key_filter(key, &primary_key, &types, &mut params)?;
                    Ok(Statement {
                        sql: format!("DELETE FROM {relation} WHERE {filter}"),
                        params,
                        key: Some(describe_key(key)),
                    })
                }
                RowChange::Insert { values } if values.is_empty() => {
                    Ok(Statement { sql: format!("INSERT INTO {relation} DEFAULT VALUES"), params, key: None })
                }
                RowChange::Insert { values } => {
                    let names: Vec<String> = values.keys().map(|name| quote_ident(name)).collect();
                    let casts = values
                        .iter()
                        .map(|(name, value)| cast(name, value, &types, &mut params))
                        .collect::<Result<Vec<_>>>()?;
                    Ok(Statement {
                        sql: format!("INSERT INTO {relation} ({}) VALUES ({})", names.join(", "), casts.join(", ")),
                        params,
                        key: None,
                    })
                }
            }
        })
        .collect()
}

/// `"name" = CAST($1::text AS text), …`
fn assignments(
    values: &BTreeMap<String, Option<String>>,
    types: &BTreeMap<&str, &str>,
    params: &mut Vec<Option<String>>,
    separator: &str,
) -> Result<String> {
    let parts = values
        .iter()
        .map(|(name, value)| Ok(format!("{} = {}", quote_ident(name), cast(name, value, types, params)?)))
        .collect::<Result<Vec<_>>>()?;
    Ok(parts.join(separator))
}

/// The WHERE clause that pins one row: every primary key column, and only those.
fn key_filter(
    key: &BTreeMap<String, Option<String>>,
    primary_key: &BTreeSet<&str>,
    types: &BTreeMap<&str, &str>,
    params: &mut Vec<Option<String>>,
) -> Result<String> {
    let given: BTreeSet<&str> = key.keys().map(String::as_str).collect();
    if &given != primary_key || key.values().any(Option::is_none) {
        return Err(AppError::Invalid("A row must be identified by its full primary key.".into()));
    }
    assignments(key, types, params, " AND ")
}

/// Sends the value as text and lets the server convert it to the column's type.
fn cast(
    name: &str,
    value: &Option<String>,
    types: &BTreeMap<&str, &str>,
    params: &mut Vec<Option<String>>,
) -> Result<String> {
    let data_type = types.get(name).ok_or_else(|| AppError::Invalid(format!("Unknown column: {name}")))?;
    params.push(value.clone());
    Ok(format!("CAST(${}::text AS {data_type})", params.len()))
}

fn describe_key(key: &BTreeMap<String, Option<String>>) -> String {
    let parts: Vec<String> =
        key.iter().map(|(name, value)| format!("{name} = {}", value.as_deref().unwrap_or("NULL"))).collect();
    parts.join(", ")
}

/// Writes the parameters into the SQL as literals. Only for showing the user.
/// One pass left to right, so text already written in (a value holding `$1`) is
/// never read again, and quoted names like `"t$1"` are copied as they are.
fn inline(statement: &Statement) -> String {
    let mut out = String::with_capacity(statement.sql.len());
    let mut chars = statement.sql.chars().peekable();
    let mut in_name = false;
    while let Some(c) = chars.next() {
        if c == '"' {
            in_name = !in_name;
        }
        if c != '$' || in_name || !chars.peek().is_some_and(char::is_ascii_digit) {
            out.push(c);
            continue;
        }
        let mut number = String::new();
        while let Some(digit) = chars.next_if(char::is_ascii_digit) {
            number.push(digit);
        }
        let param = number.parse::<usize>().ok().and_then(|n| statement.params.get(n.wrapping_sub(1)));
        match param {
            Some(Some(value)) => out.push_str(&format!("'{}'", value.replace('\'', "''"))),
            Some(None) => out.push_str("NULL"),
            None => out.push_str(&format!("${number}")),
        }
    }
    out
}

fn summarize(changes: &[RowChange]) -> ApplySummary {
    let count = |f: fn(&RowChange) -> bool| changes.iter().filter(|c| f(c)).count() as u64;
    ApplySummary {
        updated: count(|c| matches!(c, RowChange::Update { .. })),
        inserted: count(|c| matches!(c, RowChange::Insert { .. })),
        deleted: count(|c| matches!(c, RowChange::Delete { .. })),
    }
}

#[cfg(test)]
mod tests {
    use sqlx::postgres::PgConnection;

    use super::super::tests::{PASSWORD, local};
    use super::*;

    fn map(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.map(String::from))).collect()
    }

    fn items() -> TableRef {
        TableRef { database: "shop".into(), schema: "dbforge_edit".into(), name: "items".into() }
    }

    async fn run(sql: &str) -> PgConnection {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut conn).await.unwrap();
        conn
    }

    async fn rows(conn: &mut PgConnection) -> Vec<(i32, String, i32, Option<String>)> {
        sqlx::query_as("SELECT id, name, qty, status::text FROM dbforge_edit.items ORDER BY id")
            .fetch_all(conn)
            .await
            .unwrap()
    }

    const SETUP: &str = "
        DROP SCHEMA IF EXISTS dbforge_edit CASCADE;
        CREATE SCHEMA dbforge_edit;
        CREATE TYPE dbforge_edit.state AS ENUM ('new', 'done');
        CREATE TABLE dbforge_edit.items (
            id int PRIMARY KEY,
            name text NOT NULL,
            qty int NOT NULL DEFAULT 5,
            status dbforge_edit.state
        );
        INSERT INTO dbforge_edit.items VALUES (1, 'one', 1, 'new'), (2, 'two', 2, NULL), (3, 'three', 3, NULL);
        CREATE TABLE dbforge_edit.loose (name text);";

    #[test]
    fn inlining_leaves_dollar_signs_in_values_and_names_alone() {
        let statement = Statement {
            sql: r#"UPDATE "t$1" SET "a" = $1, "b" = $2"#.into(),
            params: vec![Some("x".into()), Some("$1".into())],
            key: None,
        };
        assert_eq!(inline(&statement), r#"UPDATE "t$1" SET "a" = 'x', "b" = '$1'"#);
    }

    #[test]
    fn inlining_handles_ten_or_more_parameters() {
        let statement = Statement {
            sql: (1..=10).map(|i| format!("${i}")).collect::<Vec<_>>().join(","),
            params: (1..=9).map(|i| Some(i.to_string())).chain([Some("it's".to_string())]).collect(),
            key: None,
        };
        assert_eq!(inline(&statement), "'1','2','3','4','5','6','7','8','9','it''s'");
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn applies_updates_inserts_and_deletes_together() {
        let mut conn = run(SETUP).await;
        let changes = vec![
            RowChange::Delete { key: map(&[("id", Some("3"))]) },
            RowChange::Update {
                key: map(&[("id", Some("1"))]),
                values: map(&[("name", Some("uno")), ("status", None)]),
            },
            RowChange::Update { key: map(&[("id", Some("2"))]), values: map(&[("status", Some("done"))]) },
            // qty is left out, so its DEFAULT applies.
            RowChange::Insert { values: map(&[("id", Some("4")), ("name", Some("it's four"))]) },
        ];

        let summary = apply_changes(&local(), PASSWORD, &items(), &changes).await.unwrap();

        assert_eq!(summary, ApplySummary { updated: 2, inserted: 1, deleted: 1 });
        assert_eq!(
            rows(&mut conn).await,
            [(1, "uno".into(), 1, None), (2, "two".into(), 2, Some("done".into())), (4, "it's four".into(), 5, None),]
        );
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn a_row_changed_elsewhere_rolls_everything_back() {
        let mut conn = run(&SETUP.replace("dbforge_edit", "dbforge_edit2")).await;
        let table = TableRef { schema: "dbforge_edit2".into(), ..items() };
        let changes = vec![
            RowChange::Delete { key: map(&[("id", Some("1"))]) },
            RowChange::Update { key: map(&[("id", Some("99"))]), values: map(&[("name", Some("gone"))]) },
        ];

        let error = apply_changes(&local(), PASSWORD, &table, &changes).await.unwrap_err();

        assert!(error.to_string().contains("id = 99 was changed or deleted"), "{error}");
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM dbforge_edit2.items").fetch_one(&mut conn).await.unwrap();
        assert_eq!(count, 3, "the delete before the failing update was rolled back");
        sqlx::query("DROP SCHEMA dbforge_edit2 CASCADE").execute(&mut conn).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn refuses_tables_without_a_primary_key_and_partial_keys() {
        let _conn = run(&SETUP.replace("dbforge_edit", "dbforge_edit3")).await;
        let loose = TableRef { database: "shop".into(), schema: "dbforge_edit3".into(), name: "loose".into() };
        let error =
            apply_changes(&local(), PASSWORD, &loose, &[RowChange::Insert { values: map(&[("name", Some("x"))]) }])
                .await
                .unwrap_err();
        assert!(error.to_string().contains("has no primary key"), "{error}");

        let table = TableRef { schema: "dbforge_edit3".into(), ..items() };
        let wrong_key = [RowChange::Delete { key: map(&[("name", Some("one"))]) }];
        let error = apply_changes(&local(), PASSWORD, &table, &wrong_key).await.unwrap_err();
        assert!(error.to_string().contains("full primary key"), "{error}");
    }

    /// An explicit cast to `varchar(5)` silently cuts a longer value down to 5
    /// characters; the server must reject it instead, as a plain assignment would.
    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn values_too_long_for_a_column_are_rejected_not_truncated() {
        let mut conn = run("
            DROP SCHEMA IF EXISTS dbforge_edit5 CASCADE;
            CREATE SCHEMA dbforge_edit5;
            CREATE TABLE dbforge_edit5.codes (id int PRIMARY KEY, code varchar(5), flag char(3));
            INSERT INTO dbforge_edit5.codes VALUES (1, 'abc', 'x');")
        .await;
        let table = TableRef { database: "shop".into(), schema: "dbforge_edit5".into(), name: "codes".into() };

        for change in [
            RowChange::Update { key: map(&[("id", Some("1"))]), values: map(&[("code", Some("abcdefghij"))]) },
            RowChange::Insert { values: map(&[("id", Some("2")), ("flag", Some("abcdef"))]) },
        ] {
            let error = apply_changes(&local(), PASSWORD, &table, &[change]).await.unwrap_err();
            assert!(error.to_string().contains("value too long"), "{error}");
        }
        let rows: Vec<(i32, Option<String>)> =
            sqlx::query_as("SELECT id, code FROM dbforge_edit5.codes").fetch_all(&mut conn).await.unwrap();
        assert_eq!(rows, [(1, Some("abc".into()))]);
        sqlx::query("DROP SCHEMA dbforge_edit5 CASCADE").execute(&mut conn).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn previews_the_exact_statements() {
        let _conn = run(&SETUP.replace("dbforge_edit", "dbforge_edit4")).await;
        let table = TableRef { schema: "dbforge_edit4".into(), ..items() };
        let changes = vec![
            RowChange::Update { key: map(&[("id", Some("1"))]), values: map(&[("name", Some("o'ne"))]) },
            RowChange::Insert { values: map(&[]) },
        ];

        let preview = preview_changes(&local(), PASSWORD, &table, &changes).await.unwrap();

        assert_eq!(
            preview,
            [
                "UPDATE \"dbforge_edit4\".\"items\" SET \"name\" = CAST('o''ne'::text AS pg_catalog.text) \
                 WHERE \"id\" = CAST('1'::text AS pg_catalog.int4)",
                "INSERT INTO \"dbforge_edit4\".\"items\" DEFAULT VALUES",
            ]
        );
    }
}
