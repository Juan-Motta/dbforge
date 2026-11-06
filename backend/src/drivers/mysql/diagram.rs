//! Tables and foreign keys of one MySQL database, for the ER diagram.

use std::collections::BTreeMap;

use sqlx::Connection as _;

use super::connect;
use crate::connections::model::Connection;
use crate::drivers::{DiagramColumn, DiagramTable, Relationship, SchemaDiagram};
use crate::error::Result;

/// `schema` is the database: in MySQL they are the same thing.
pub async fn schema_diagram(
    connection: &Connection,
    password: Option<&str>,
    _database: &str,
    schema: &str,
) -> Result<SchemaDiagram> {
    let mut conn = connect(connection, password, Some(schema)).await?;

    type ColumnRow = (String, Option<i64>, String, String, String, String);
    let rows: Vec<ColumnRow> = sqlx::query_as(
        "SELECT CAST(t.table_name AS CHAR), CAST(t.table_rows AS SIGNED), CAST(c.column_name AS CHAR),
                CAST(c.column_type AS CHAR), CAST(c.is_nullable AS CHAR), CAST(c.column_key AS CHAR)
         FROM information_schema.tables t
         JOIN information_schema.columns c ON c.table_schema = t.table_schema AND c.table_name = t.table_name
         WHERE t.table_schema = ? AND t.table_type = 'BASE TABLE'
         ORDER BY t.table_name, c.ordinal_position",
    )
    .bind(schema)
    .fetch_all(&mut conn)
    .await?;

    type KeyRow = (String, String, String, String, String, String);
    let keys: Vec<KeyRow> = sqlx::query_as(
        "SELECT CAST(constraint_name AS CHAR), CAST(table_name AS CHAR), CAST(column_name AS CHAR),
                CAST(referenced_table_schema AS CHAR), CAST(referenced_table_name AS CHAR),
                CAST(referenced_column_name AS CHAR)
         FROM information_schema.key_column_usage
         WHERE table_schema = ? AND referenced_table_name IS NOT NULL
         ORDER BY constraint_name, table_name, ordinal_position",
    )
    .bind(schema)
    .fetch_all(&mut conn)
    .await?;
    conn.close().await?;

    let mut tables: BTreeMap<String, DiagramTable> = BTreeMap::new();
    for (name, rows, column, data_type, nullable, key) in rows {
        tables
            .entry(name.clone())
            .or_insert_with(|| DiagramTable { name, row_estimate: rows, columns: Vec::new() })
            .columns
            .push(DiagramColumn { name: column, data_type, nullable: nullable == "YES", primary_key: key == "PRI" });
    }

    // Constraint names are unique per table, so a key is grouped by both.
    let mut relationships: Vec<Relationship> = Vec::new();
    for (name, table, column, referenced_schema, referenced_table, referenced_column) in keys {
        match relationships.last_mut() {
            Some(r) if r.name == name && r.table == table => {
                r.columns.push(column);
                r.referenced_columns.push(referenced_column);
            }
            _ => relationships.push(Relationship {
                name,
                table,
                columns: vec![column],
                referenced_schema,
                referenced_table,
                referenced_columns: vec![referenced_column],
            }),
        }
    }
    Ok(SchemaDiagram { tables: tables.into_values().collect(), relationships })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, run, servers};
    use super::*;

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn reads_tables_and_relationships() {
        for server in servers() {
            // Each test reads `local()`; here it means the server under test.
            let local = || server.clone();
            let _conn = run(
                &server,
                "DROP DATABASE IF EXISTS dbforge_mdiagram;
             CREATE DATABASE dbforge_mdiagram;
             CREATE TABLE dbforge_mdiagram.customers (id int PRIMARY KEY, referrer_id int,
                 FOREIGN KEY (referrer_id) REFERENCES dbforge_mdiagram.customers (id));
             CREATE TABLE dbforge_mdiagram.orders (id int PRIMARY KEY, customer_id int NOT NULL,
                 CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES dbforge_mdiagram.customers (id));
             CREATE VIEW dbforge_mdiagram.recent AS SELECT id FROM dbforge_mdiagram.orders;",
            )
            .await;

            let diagram = schema_diagram(&local(), PASSWORD, "dbforge_mdiagram", "dbforge_mdiagram").await.unwrap();

            let tables: Vec<_> = diagram.tables.iter().map(|t| t.name.as_str()).collect();
            assert_eq!(tables, ["customers", "orders"], "views are left out");
            assert!(diagram.tables[1].columns[0].primary_key);

            let relationships: Vec<_> = diagram
                .relationships
                .iter()
                .map(|r| (r.table.as_str(), r.columns.join(","), r.referenced_table.as_str()))
                .collect();
            assert!(relationships.contains(&("orders", "customer_id".into(), "customers")), "{relationships:?}");
            assert!(relationships.contains(&("customers", "referrer_id".into(), "customers")), "{relationships:?}");
            assert!(diagram.relationships.iter().all(|r| r.referenced_schema == "dbforge_mdiagram"));
        }
    }
}
