//! A MySQL table's or view's structure from information_schema. The DDL is the
//! server's own `SHOW CREATE TABLE` (or VIEW), plus its triggers.

use std::collections::BTreeMap;

use sqlx::{AssertSqlSafe, Row};

use super::Conn;
use super::{connect, is_mariadb, quote_ident};
use crate::connections::model::Connection;
use crate::drivers::{ColumnInfo, ForeignKey, IndexInfo, ObjectKind, TableRef, TableStructure, TriggerInfo};
use crate::error::{AppError, Result};

pub async fn table_structure(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
) -> Result<TableStructure> {
    let mut conn = connect(connection, password, Some(&table.database)).await?;

    type TableRow = (String, Option<i64>, Option<i64>, Option<i64>, Option<String>);
    let found: Option<TableRow> = sqlx::query_as(
        "SELECT CAST(table_type AS CHAR), CAST(table_rows AS SIGNED), CAST(data_length AS SIGNED),
                CAST(index_length AS SIGNED), CAST(table_comment AS CHAR)
         FROM information_schema.tables WHERE table_schema = ? AND table_name = ?",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_optional(&mut conn)
    .await?;
    let (kind, row_estimate, data_bytes, index_bytes, comment) =
        found.ok_or_else(|| AppError::Invalid(format!("Table {}.{} not found", table.schema, table.name)))?;
    let is_view = kind == "VIEW";

    let structure = TableStructure {
        kind: if is_view { ObjectKind::View } else { ObjectKind::Table },
        row_estimate: if is_view { None } else { row_estimate },
        data_bytes: data_bytes.unwrap_or(0),
        index_bytes: index_bytes.unwrap_or(0),
        comment: comment.filter(|c| !c.is_empty()),
        columns: columns(&mut conn, table, is_mariadb(connection)).await?,
        indexes: indexes(&mut conn, table, is_mariadb(connection)).await?,
        foreign_keys: foreign_keys(&mut conn, table).await?,
        triggers: triggers(&mut conn, table).await?,
        ddl: String::new(),
    };
    let ddl = show_create(&mut conn, table, is_view).await?;
    conn.close().await?;
    Ok(TableStructure { ddl: with_triggers(ddl, &structure.triggers), ..structure })
}

async fn columns(conn: &mut Conn, table: &TableRef, mariadb: bool) -> Result<Vec<ColumnInfo>> {
    type Row = (String, String, String, Option<String>, String, Option<String>, String);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT CAST(column_name AS CHAR), CAST(column_type AS CHAR), CAST(is_nullable AS CHAR),
                CAST(column_default AS CHAR), CAST(extra AS CHAR), CAST(generation_expression AS CHAR),
                CAST(column_comment AS CHAR)
         FROM information_schema.columns
         WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(conn)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(name, data_type, nullable, default, extra, generated, comment)| ColumnInfo {
            options: enum_options(&data_type),
            name,
            data_type,
            nullable: nullable == "YES",
            // MariaDB writes an explicit `DEFAULT NULL` as the text NULL.
            default: column_default(default.filter(|d| !(mariadb && d == "NULL")), &extra, generated),
            comment: Some(comment).filter(|c| !c.is_empty()),
        })
        .collect())
}

/// The values of an `enum('a','b')` column, unquoted (`''` is a quote); empty for other types.
fn enum_options(column_type: &str) -> Vec<String> {
    let Some(list) = column_type.strip_prefix("enum(").and_then(|rest| rest.strip_suffix(')')) else {
        return Vec::new();
    };
    let mut options = Vec::new();
    let mut chars = list.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\'' {
            continue;
        }
        let mut option = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\'' if chars.peek() == Some(&'\'') => {
                    chars.next();
                    option.push('\'');
                }
                '\'' => break,
                c => option.push(c),
            }
        }
        options.push(option);
    }
    options
}

/// The default as the structure view shows it: the value, `auto_increment`, or
/// the expression of a generated column.
fn column_default(default: Option<String>, extra: &str, generated: Option<String>) -> Option<String> {
    let extra = extra.to_uppercase();
    if extra.contains("AUTO_INCREMENT") {
        return Some("auto_increment".into());
    }
    if let Some(expression) = generated.filter(|g| !g.is_empty()) {
        let kind = if extra.contains("STORED") { "STORED" } else { "VIRTUAL" };
        return Some(format!("GENERATED ALWAYS AS ({expression}) {kind}"));
    }
    default
}

async fn indexes(conn: &mut Conn, table: &TableRef, mariadb: bool) -> Result<Vec<IndexInfo>> {
    // One row per indexed column (or expression), in order within each index.
    type IndexRow = (String, i64, String, Option<String>, Option<String>);
    // MySQL's functional indexes list their expression; MariaDB has no such column.
    let expression = if mariadb { "NULL" } else { "CAST(expression AS CHAR)" };
    let rows: Vec<IndexRow> = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT CAST(index_name AS CHAR), CAST(non_unique AS SIGNED), CAST(index_type AS CHAR),
                CAST(column_name AS CHAR), {expression}
         FROM information_schema.statistics
         WHERE table_schema = ? AND table_name = ?
         ORDER BY index_name = 'PRIMARY' DESC, index_name, seq_in_index"
    )))
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(&mut *conn)
    .await?;

    // Sizes need the mysql schema, which not every user may read; without it they stay 0.
    let sizes: BTreeMap<String, i64> = sqlx::query_as(
        "SELECT CAST(index_name AS CHAR), CAST(stat_value * @@innodb_page_size AS SIGNED)
         FROM mysql.innodb_index_stats
         WHERE database_name = ? AND table_name = ? AND stat_name = 'size'",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(conn)
    .await
    .map(|rows: Vec<(String, i64)>| rows.into_iter().collect())
    .unwrap_or_default();

    let mut indexes: Vec<IndexInfo> = Vec::new();
    for (name, non_unique, method, column, expression) in rows {
        let part = column.or(expression).unwrap_or_default();
        match indexes.last_mut() {
            Some(index) if index.name == name => index.columns.push(part),
            _ => indexes.push(IndexInfo {
                bytes: sizes.get(&name).copied().unwrap_or(0),
                primary: name == "PRIMARY",
                name,
                method: method.to_lowercase(),
                unique: non_unique == 0,
                columns: vec![part],
                // MySQL has no partial indexes.
                condition: None,
            }),
        }
    }
    Ok(indexes)
}

async fn foreign_keys(conn: &mut Conn, table: &TableRef) -> Result<Vec<ForeignKey>> {
    type Row = (String, String, String, String, String, String, String);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT CAST(k.constraint_name AS CHAR), CAST(k.column_name AS CHAR),
                CAST(k.referenced_table_schema AS CHAR), CAST(k.referenced_table_name AS CHAR),
                CAST(k.referenced_column_name AS CHAR), CAST(r.update_rule AS CHAR), CAST(r.delete_rule AS CHAR)
         FROM information_schema.key_column_usage k
         JOIN information_schema.referential_constraints r
           ON r.constraint_schema = k.constraint_schema AND r.constraint_name = k.constraint_name
         WHERE k.table_schema = ? AND k.table_name = ? AND k.referenced_table_name IS NOT NULL
         ORDER BY k.constraint_name, k.ordinal_position",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(conn)
    .await?;

    let mut keys: Vec<ForeignKey> = Vec::new();
    for (name, column, schema, referenced, referenced_column, on_update, on_delete) in rows {
        match keys.last_mut() {
            Some(key) if key.name == name => {
                key.columns.push(column);
                key.referenced_columns.push(referenced_column);
            }
            _ => keys.push(ForeignKey {
                name,
                columns: vec![column],
                // Like PostgreSQL's regclass text: qualified only when in another database.
                referenced_table: if schema == table.schema { referenced } else { format!("{schema}.{referenced}") },
                referenced_columns: vec![referenced_column],
                on_update,
                on_delete,
            }),
        }
    }
    Ok(keys)
}

async fn triggers(conn: &mut Conn, table: &TableRef) -> Result<Vec<TriggerInfo>> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT CAST(trigger_name AS CHAR), CAST(action_timing AS CHAR), CAST(event_manipulation AS CHAR),
                CAST(action_statement AS CHAR)
         FROM information_schema.triggers
         WHERE event_object_schema = ? AND event_object_table = ? ORDER BY trigger_name",
    )
    .bind(&table.schema)
    .bind(&table.name)
    .fetch_all(conn)
    .await?;
    let relation = quote_ident(&table.name);
    Ok(rows
        .into_iter()
        .map(|(name, timing, event, statement)| TriggerInfo {
            definition: format!(
                "CREATE TRIGGER {} {timing} {event} ON {relation} FOR EACH ROW {statement}",
                quote_ident(&name)
            ),
            name,
            // MySQL triggers can't be disabled.
            enabled: true,
        })
        .collect())
}

async fn show_create(conn: &mut Conn, table: &TableRef, is_view: bool) -> Result<String> {
    let object = if is_view { "VIEW" } else { "TABLE" };
    let sql = format!("SHOW CREATE {object} {}.{}", quote_ident(&table.schema), quote_ident(&table.name));
    let row = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_one(conn).await?;
    // The statement is the second column ("Create Table" or "Create View").
    let bytes: Vec<u8> = row.try_get_unchecked(1)?;
    Ok(format!("{};", String::from_utf8_lossy(&bytes)))
}

fn with_triggers(ddl: String, triggers: &[TriggerInfo]) -> String {
    if triggers.is_empty() {
        return ddl;
    }
    let definitions: Vec<String> = triggers.iter().map(|t| format!("{};", t.definition)).collect();
    format!("{ddl}\n\n{}", definitions.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, local, mariadb, run, table};
    use super::*;

    #[test]
    fn enum_types_give_their_values() {
        assert_eq!(enum_options("enum('new','it''s done','a,b')"), ["new", "it's done", "a,b"]);
        assert!(enum_options("varchar(20)").is_empty());
        assert!(enum_options("set('a','b')").is_empty());
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn enum_columns_list_their_values() {
        run(
            &local(),
            "DROP TABLE IF EXISTS shop.dbforge_enum_tasks;
             CREATE TABLE shop.dbforge_enum_tasks (id int PRIMARY KEY, state enum('new','it''s done'), done boolean)",
        )
        .await;
        let structure = table_structure(&local(), PASSWORD, &table("shop", "dbforge_enum_tasks")).await.unwrap();
        let options: Vec<_> = structure.columns.iter().map(|c| (c.name.as_str(), c.options.clone())).collect();
        assert_eq!(options, [("id", vec![]), ("state", vec!["new".to_string(), "it's done".into()]), ("done", vec![])]);
        run(&local(), "DROP TABLE shop.dbforge_enum_tasks").await;
    }

    #[test]
    fn defaults_show_auto_increment_and_generated_columns() {
        assert_eq!(column_default(None, "auto_increment", None).as_deref(), Some("auto_increment"));
        assert_eq!(
            column_default(None, "STORED GENERATED", Some("(`a` * 2)".into())).as_deref(),
            Some("GENERATED ALWAYS AS ((`a` * 2)) STORED")
        );
        assert_eq!(column_default(Some("US".into()), "", None).as_deref(), Some("US"));
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn reads_columns_indexes_keys_triggers_and_ddl() {
        let _conn = run(&local(),
            "DROP DATABASE IF EXISTS dbforge_mstruct;
             CREATE DATABASE dbforge_mstruct;
             CREATE TABLE dbforge_mstruct.customers (id int PRIMARY KEY);
             CREATE TABLE dbforge_mstruct.orders (
                 id int AUTO_INCREMENT PRIMARY KEY,
                 customer_id int NOT NULL,
                 total decimal(10,2) DEFAULT 0 COMMENT 'Gross',
                 note varchar(50),
                 UNIQUE KEY uq_note (note),
                 KEY ix_lower ((lower(note))),
                 CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES dbforge_mstruct.customers (id) ON DELETE CASCADE
             ) COMMENT 'Orders';
             CREATE TRIGGER dbforge_mstruct.touch BEFORE UPDATE ON dbforge_mstruct.orders FOR EACH ROW SET NEW.note = NEW.note;
             CREATE VIEW dbforge_mstruct.recent AS SELECT id FROM dbforge_mstruct.orders;",
        )
        .await;

        let orders = table_structure(&local(), PASSWORD, &table("dbforge_mstruct", "orders")).await.unwrap();

        assert_eq!(orders.kind, ObjectKind::Table);
        assert_eq!(orders.comment.as_deref(), Some("Orders"));
        let columns: Vec<_> =
            orders.columns.iter().map(|c| (c.name.as_str(), c.nullable, c.default.as_deref())).collect();
        assert_eq!(
            columns,
            [
                ("id", false, Some("auto_increment")),
                ("customer_id", false, None),
                ("total", true, Some("0.00")),
                ("note", true, None)
            ]
        );
        assert_eq!(orders.columns[2].comment.as_deref(), Some("Gross"));

        let indexes: Vec<_> = orders.indexes.iter().map(|i| (i.name.as_str(), i.primary, i.unique)).collect();
        assert_eq!(indexes[0], ("PRIMARY", true, true));
        assert!(indexes.contains(&("uq_note", false, true)), "{indexes:?}");
        let functional = orders.indexes.iter().find(|i| i.name == "ix_lower").unwrap();
        assert!(functional.columns[0].contains("lower"), "{:?}", functional.columns);

        let [fk] = orders.foreign_keys.as_slice() else { panic!("{:?}", orders.foreign_keys) };
        assert_eq!((fk.referenced_table.as_str(), fk.on_delete.as_str()), ("customers", "CASCADE"));

        assert_eq!(orders.triggers[0].name, "touch");
        assert!(orders.ddl.starts_with("CREATE TABLE `orders`"), "{}", orders.ddl);
        assert!(orders.ddl.contains("CREATE TRIGGER `touch` BEFORE UPDATE ON `orders`"), "{}", orders.ddl);

        let view = table_structure(&local(), PASSWORD, &table("dbforge_mstruct", "recent")).await.unwrap();
        assert_eq!(view.kind, ObjectKind::View);
        assert!(view.ddl.contains("VIEW `recent`"), "{}", view.ddl);
    }

    #[tokio::test]
    #[ignore = "needs a local MariaDB on port 53307"]
    async fn mariadb_reads_indexes_and_literal_defaults() {
        let _conn = run(
            &mariadb(),
            "DROP DATABASE IF EXISTS dbforge_mstruct;
             CREATE DATABASE dbforge_mstruct;
             CREATE TABLE dbforge_mstruct.places (
                 id int PRIMARY KEY,
                 code char(2) DEFAULT 'US',
                 note varchar(10) DEFAULT NULL,
                 doubled int AS (id * 2) STORED,
                 KEY ix_code (code)
             );",
        )
        .await;

        let places = table_structure(&mariadb(), PASSWORD, &table("dbforge_mstruct", "places")).await.unwrap();

        let defaults: Vec<_> = places.columns.iter().map(|c| (c.name.as_str(), c.default.as_deref())).collect();
        assert_eq!(
            defaults,
            [
                ("id", None),
                ("code", Some("'US'")),
                ("note", None),
                ("doubled", Some("GENERATED ALWAYS AS (`id` * 2) STORED"))
            ]
        );
        let indexes: Vec<_> = places.indexes.iter().map(|i| (i.name.as_str(), i.columns.join(","))).collect();
        assert_eq!(indexes, [("PRIMARY", "id".to_string()), ("ix_code", "code".to_string())]);
        assert!(places.ddl.starts_with("CREATE TABLE `places`"), "{}", places.ddl);
    }
}
