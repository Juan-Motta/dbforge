//! A SQLite table's structure from its pragmas, and the file's ER diagram.
//! The DDL is what SQLite stored when the objects were created.

use std::collections::BTreeMap;

use sqlx::Connection as _;
use sqlx::sqlite::SqliteConnection;

use super::{MAIN, connect};
use crate::connections::model::Connection;
use crate::drivers::{
    ColumnInfo, DiagramColumn, DiagramTable, ForeignKey, IndexInfo, ObjectKind, Relationship, SchemaDiagram, TableRef,
    TableStructure, TriggerInfo,
};
use crate::error::{AppError, Result};

pub async fn table_structure(connection: &Connection, table: &TableRef) -> Result<TableStructure> {
    let mut conn = connect(connection).await?;
    let found: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT type, sql FROM sqlite_schema WHERE name = ? AND type IN ('table', 'view')")
            .bind(&table.name)
            .fetch_optional(&mut conn)
            .await?;
    let (kind, sql) = found.ok_or_else(|| AppError::Invalid(format!("Table {} not found", table.name)))?;

    let columns = columns(&mut conn, &table.name).await?;
    let indexes = indexes(&mut conn, &table.name, &columns).await?;
    let foreign_keys = foreign_keys(&mut conn, &table.name).await?;
    let triggers: Vec<TriggerInfo> = sqlx::query_as::<_, (String, String)>(
        "SELECT name, sql FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = ? ORDER BY name",
    )
    .bind(&table.name)
    .fetch_all(&mut conn)
    .await?
    .into_iter()
    // SQLite triggers can't be disabled.
    .map(|(name, definition)| TriggerInfo { name, enabled: true, definition })
    .collect();
    let index_sql: Vec<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type = 'index' AND tbl_name = ? AND sql IS NOT NULL ORDER BY name",
    )
    .bind(&table.name)
    .fetch_all(&mut conn)
    .await?;
    conn.close().await?;

    let ddl = [sql.into_iter().collect::<Vec<_>>(), index_sql, triggers.iter().map(|t| t.definition.clone()).collect()]
        .concat()
        .iter()
        .map(|statement| format!("{statement};"))
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok(TableStructure {
        kind: if kind == "view" { ObjectKind::View } else { ObjectKind::Table },
        // Counting would read the whole table, and SQLite keeps no estimate or sizes.
        row_estimate: None,
        data_bytes: 0,
        index_bytes: 0,
        comment: None,
        columns: columns.into_iter().map(|c| c.info).collect(),
        indexes,
        foreign_keys,
        triggers,
        ddl,
    })
}

struct PragmaColumn {
    info: ColumnInfo,
    primary_key: bool,
}

async fn columns(conn: &mut SqliteConnection, table: &str) -> Result<Vec<PragmaColumn>> {
    let rows: Vec<(String, String, i64, Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo(?) WHERE hidden <> 1 ORDER BY cid",
    )
    .bind(table)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, data_type, not_null, default, pk, hidden)| PragmaColumn {
            info: ColumnInfo {
                options: Vec::new(),
                name,
                data_type: if data_type.is_empty() { "any".into() } else { data_type.to_lowercase() },
                nullable: not_null == 0 && pk == 0,
                // The pragma doesn't give a generated column's expression; the DDL shows it.
                default: if hidden >= 2 {
                    Some(format!("generated ({})", if hidden == 3 { "stored" } else { "virtual" }))
                } else {
                    default
                },
                comment: None,
            },
            primary_key: pk > 0,
        })
        .collect())
}

async fn indexes(conn: &mut SqliteConnection, table: &str, columns: &[PragmaColumn]) -> Result<Vec<IndexInfo>> {
    let list: Vec<(String, i64, String, i64)> = sqlx::query_as(
        "SELECT name, \"unique\", origin, partial FROM pragma_index_list(?) ORDER BY origin = 'pk' DESC, name",
    )
    .bind(table)
    .fetch_all(&mut *conn)
    .await?;
    let mut indexes = Vec::new();
    for (name, unique, origin, partial) in list {
        // Expression parts have no column name.
        let parts: Vec<Option<String>> = sqlx::query_scalar("SELECT name FROM pragma_index_info(?) ORDER BY seqno")
            .bind(&name)
            .fetch_all(&mut *conn)
            .await?;
        let condition = if partial == 1 {
            let sql: Option<String> =
                sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?")
                    .bind(&name)
                    .fetch_optional(&mut *conn)
                    .await?
                    .flatten();
            sql.and_then(|sql| where_clause(&sql))
        } else {
            None
        };
        indexes.push(IndexInfo {
            name,
            method: "btree".into(),
            unique: unique == 1,
            primary: origin == "pk",
            columns: parts.into_iter().map(|p| p.unwrap_or_else(|| "(expression)".into())).collect(),
            condition,
            bytes: 0,
        });
    }
    // An INTEGER PRIMARY KEY is the rowid itself and has no index of its own.
    let key: Vec<String> = columns.iter().filter(|c| c.primary_key).map(|c| c.info.name.clone()).collect();
    if !key.is_empty() && !indexes.iter().any(|i| i.primary) {
        indexes.insert(
            0,
            IndexInfo {
                name: "(rowid)".into(),
                method: "btree".into(),
                unique: true,
                primary: true,
                columns: key,
                condition: None,
                bytes: 0,
            },
        );
    }
    Ok(indexes)
}

/// The condition of a partial index, from its `CREATE INDEX … WHERE …`.
fn where_clause(sql: &str) -> Option<String> {
    let at = sql.to_uppercase().rfind(" WHERE ")?;
    Some(sql[at + 7..].trim().to_string())
}

async fn foreign_keys(conn: &mut SqliteConnection, table: &str) -> Result<Vec<ForeignKey>> {
    let rows: Vec<(i64, String, String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT id, \"table\", \"from\", \"to\", on_update, on_delete FROM pragma_foreign_key_list(?) ORDER BY id, seq",
    )
    .bind(table)
    .fetch_all(&mut *conn)
    .await?;
    let mut keys: BTreeMap<i64, ForeignKey> = BTreeMap::new();
    for (id, referenced, from, to, on_update, on_delete) in rows {
        // A reference without columns points at the other table's primary key.
        let to = match to {
            Some(to) => to,
            None => primary_key_of(conn, &referenced).await?.join(", "),
        };
        let key = keys.entry(id).or_insert_with(|| ForeignKey {
            // SQLite foreign keys have no names; number them like the pragma does.
            name: format!("fk_{table}_{id}"),
            columns: Vec::new(),
            referenced_table: referenced,
            referenced_columns: Vec::new(),
            on_update,
            on_delete,
        });
        key.columns.push(from);
        key.referenced_columns.push(to);
    }
    Ok(keys.into_values().collect())
}

async fn primary_key_of(conn: &mut SqliteConnection, table: &str) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar("SELECT name FROM pragma_table_info(?) WHERE pk > 0 ORDER BY pk")
        .bind(table)
        .fetch_all(conn)
        .await?)
}

/// Every table of the file with its columns, and the foreign keys between them.
pub async fn schema_diagram(connection: &Connection) -> Result<SchemaDiagram> {
    let mut conn = connect(connection).await?;
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name",
    )
    .fetch_all(&mut conn)
    .await?;
    let mut tables = Vec::new();
    let mut relationships = Vec::new();
    for name in names {
        let columns = columns(&mut conn, &name).await?;
        tables.push(DiagramTable {
            name: name.clone(),
            row_estimate: None,
            columns: columns
                .into_iter()
                .map(|c| DiagramColumn {
                    name: c.info.name,
                    data_type: c.info.data_type,
                    nullable: c.info.nullable,
                    primary_key: c.primary_key,
                })
                .collect(),
        });
        for key in foreign_keys(&mut conn, &name).await? {
            relationships.push(Relationship {
                name: key.name,
                table: name.clone(),
                columns: key.columns,
                referenced_schema: MAIN.into(),
                referenced_table: key.referenced_table,
                referenced_columns: key.referenced_columns,
            });
        }
    }
    conn.close().await?;
    Ok(SchemaDiagram { tables, relationships })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{database, table};
    use super::*;

    const SHOP: &str = "
        CREATE TABLE customers (id INTEGER PRIMARY KEY, email TEXT UNIQUE, country TEXT DEFAULT 'US');
        CREATE TABLE orders (
            id INTEGER PRIMARY KEY,
            customer_id INTEGER NOT NULL REFERENCES customers ON DELETE CASCADE,
            total REAL,
            doubled REAL AS (total * 2) STORED
        );
        CREATE INDEX ix_big ON orders (customer_id) WHERE total > 100;
        CREATE TRIGGER touch AFTER UPDATE ON orders BEGIN SELECT 1; END;";

    #[tokio::test]
    async fn reads_columns_indexes_keys_triggers_and_ddl() {
        let (_dir, connection) = database(SHOP).await;
        let orders = table_structure(&connection, &table("orders")).await.unwrap();

        let columns: Vec<_> =
            orders.columns.iter().map(|c| (c.name.as_str(), c.nullable, c.default.as_deref())).collect();
        assert_eq!(
            columns,
            [
                ("id", false, None),
                ("customer_id", false, None),
                ("total", true, None),
                ("doubled", true, Some("generated (stored)"))
            ]
        );

        let indexes: Vec<_> =
            orders.indexes.iter().map(|i| (i.name.as_str(), i.primary, i.condition.as_deref())).collect();
        assert_eq!(indexes, [("(rowid)", true, None), ("ix_big", false, Some("total > 100"))]);

        let [fk] = orders.foreign_keys.as_slice() else { panic!("{:?}", orders.foreign_keys) };
        assert_eq!(
            (fk.referenced_table.as_str(), fk.referenced_columns[0].as_str(), fk.on_delete.as_str()),
            ("customers", "id", "CASCADE")
        );

        assert_eq!(orders.triggers[0].name, "touch");
        assert!(orders.ddl.starts_with("CREATE TABLE orders"), "{}", orders.ddl);
        assert!(
            orders.ddl.contains("CREATE INDEX ix_big") && orders.ddl.contains("CREATE TRIGGER touch"),
            "{}",
            orders.ddl
        );

        let customers = table_structure(&connection, &table("customers")).await.unwrap();
        assert!(customers.indexes.iter().any(|i| i.unique && i.columns == ["email"]), "{:?}", customers.indexes);
        assert_eq!(customers.columns[2].default.as_deref(), Some("'US'"));
    }

    #[tokio::test]
    async fn draws_tables_and_relationships() {
        let (_dir, connection) = database(SHOP).await;
        let diagram = schema_diagram(&connection).await.unwrap();
        let tables: Vec<_> = diagram.tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(tables, ["customers", "orders"]);
        let [relationship] = diagram.relationships.as_slice() else { panic!("{:?}", diagram.relationships) };
        assert_eq!((relationship.table.as_str(), relationship.referenced_table.as_str()), ("orders", "customers"));
        assert_eq!(relationship.referenced_schema, "main");
    }
}
