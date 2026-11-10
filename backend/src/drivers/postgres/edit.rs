//! Row edits from the table grid for PostgreSQL. The statements come from the
//! shared `edit_sql` builder; this file adds how PostgreSQL converts a text
//! parameter to a column's type, and runs everything in one transaction where
//! every UPDATE or DELETE must hit exactly one row.

use super::super::sqlx_edits;
use super::Conn;
use super::{connect, with_timeout};
use crate::connections::model::Connection;
use crate::drivers::edit_sql::{self, Dialect, default_keyword};
use crate::drivers::{ApplySummary, Column, RowChange, TableRef};
use crate::error::Result;

/// Values are sent as text and cast to the column's type by the server.
const POSTGRES: Dialect = Dialect {
    quote: ['"', '"'],
    numbered: Some("$"),
    value: |marker, data_type| format!("CAST({marker}::text AS {data_type})"),
    default_row: "DEFAULT VALUES",
    backslash_escapes: false,
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
    let statements = edit_sql::build(&POSTGRES, &columns, table, changes)?;

    let mut transaction = conn.begin().await?;
    sqlx_edits::run::<sqlx::Postgres>(&mut transaction, &POSTGRES, &statements).await?;
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
    let statements = edit_sql::build(&POSTGRES, &columns, table, changes)?;
    Ok(statements.iter().map(|statement| edit_sql::inline(&POSTGRES, statement)).collect())
}

/// Columns with their type's internal, schema-qualified name: `"pg_catalog"."varchar"`,
/// not `character varying(5)`. An explicit cast to a length-limited type silently
/// truncates, and the SQL spelling `character` even means `character(1)`; the
/// internal name has no length, so the column's own limit applies on assignment
/// and a value that doesn't fit is rejected.
async fn editable_columns(conn: &mut Conn, table: &TableRef) -> Result<Vec<Column>> {
    let rows: Vec<(String, String, bool, bool)> = sqlx::query_as(
        "SELECT a.attname::text, format('%I.%I', tn.nspname, t.typname),
                COALESCE(a.attnum = ANY((i.indkey::int2[])[0:i.indnkeyatts - 1]), false),
                (a.attgenerated <> '' OR a.attidentity = 'a')
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
    Ok(rows
        .into_iter()
        .map(|(name, data_type, primary_key, generated)| Column {
            name,
            data_type,
            primary_key,
            generated,
            default: None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use sqlx::AssertSqlSafe;

    use super::super::tests::{PASSWORD, local};
    use std::collections::BTreeMap;

    use super::*;
    use crate::drivers::CellValue;
    use crate::statement_log::store::tests::Collected;
    use crate::statement_log::{self, Origin, StatementLog};

    /// Keys (`Option<String>`) or values (`CellValue`) from text, `None` being NULL.
    fn map<V: From<Option<String>>>(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, V> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.map(String::from).into())).collect()
    }

    fn items() -> TableRef {
        TableRef { database: "shop".into(), schema: "dbforge_edit".into(), name: "items".into() }
    }

    async fn run(sql: &str) -> Conn {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut conn).await.unwrap();
        conn
    }

    async fn rows(conn: &mut Conn) -> Vec<(i32, String, i32, Option<String>)> {
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

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn records_the_transaction_with_values_written_in() {
        run(&SETUP.replace("dbforge_edit", "dbforge_edit_log")).await;
        let changes =
            [RowChange::Update { key: map(&[("id", Some("1"))]), values: map(&[("name", Some("it's one"))]) }];
        let table = TableRef { schema: "dbforge_edit_log".into(), ..items() };
        let log = StatementLog::new(50, Collected::default());

        statement_log::scoped(&log, "pg", Origin::Edit, apply_changes(&local(), PASSWORD, &table, &changes))
            .await
            .unwrap();

        let entries = log.entries();
        let texts: Vec<_> = entries.iter().map(|e| e.text.as_str()).skip_while(|t| *t != "BEGIN").collect();
        assert_eq!(
            texts,
            [
                "BEGIN",
                r#"UPDATE "dbforge_edit_log"."items" SET "name" = CAST('it''s one'::text AS pg_catalog.text) WHERE "id" = CAST('1'::text AS pg_catalog.int4)"#,
                "COMMIT"
            ]
        );
        let update = entries.iter().find(|e| e.text.starts_with("UPDATE")).unwrap();
        assert_eq!((update.database.as_deref(), update.origin, update.rows), (Some("shop"), Origin::Edit, Some(1)));
        assert!(entries.iter().all(|e| e.connection_id == "pg" && e.error.is_none()));
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn puts_a_column_back_to_its_default() {
        let mut conn = run(&SETUP.replace("dbforge_edit", "dbforge_edit6")).await;
        let changes = [RowChange::Update {
            key: map(&[("id", Some("1"))]),
            values: [("qty".to_string(), CellValue::Default)].into(),
        }];
        let table = TableRef { schema: "dbforge_edit6".into(), ..items() };
        apply_changes(&local(), PASSWORD, &table, &changes).await.unwrap();
        let qty: i32 =
            sqlx::query_scalar("SELECT qty FROM dbforge_edit6.items WHERE id = 1").fetch_one(&mut conn).await.unwrap();
        assert_eq!(qty, 5);
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
