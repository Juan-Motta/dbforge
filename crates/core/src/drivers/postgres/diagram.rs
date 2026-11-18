//! Tables and foreign keys of one schema, for the ER diagram.

use std::collections::BTreeMap;

use super::connect;
use crate::connections::model::Connection;
use crate::drivers::{DiagramColumn, DiagramTable, Relationship, SchemaDiagram};
use crate::error::Result;

pub async fn schema_diagram(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    schema: &str,
) -> Result<SchemaDiagram> {
    let mut conn = connect(connection, password, Some(database)).await?;

    // One row per column; reltuples is the planner's estimate, -1 when never analyzed.
    type ColumnRow = (String, i64, String, String, bool, bool);
    let rows: Vec<ColumnRow> = sqlx::query_as(
        "SELECT c.relname::text, c.reltuples::int8, a.attname::text, format_type(a.atttypid, a.atttypmod),
                NOT a.attnotnull, COALESCE(a.attnum = ANY(i.indkey), false)
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
         LEFT JOIN pg_index i ON i.indrelid = c.oid AND i.indisprimary
         WHERE n.nspname = $1 AND c.relkind IN ('r', 'p')
         ORDER BY c.relname, a.attnum",
    )
    .bind(schema)
    .fetch_all(&mut conn)
    .await?;

    type RelationshipRow = (String, String, Vec<String>, String, String, Vec<String>);
    let relationships: Vec<RelationshipRow> = sqlx::query_as(
        "SELECT con.conname::text, src.relname::text,
                ARRAY(SELECT a.attname::text FROM unnest(con.conkey) WITH ORDINALITY k(num, ord)
                      JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.num ORDER BY k.ord),
                tn.nspname::text, tgt.relname::text,
                ARRAY(SELECT a.attname::text FROM unnest(con.confkey) WITH ORDINALITY k(num, ord)
                      JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.num ORDER BY k.ord)
         FROM pg_constraint con
         JOIN pg_class src ON src.oid = con.conrelid
         JOIN pg_namespace sn ON sn.oid = src.relnamespace
         JOIN pg_class tgt ON tgt.oid = con.confrelid
         JOIN pg_namespace tn ON tn.oid = tgt.relnamespace
         WHERE con.contype = 'f' AND sn.nspname = $1
         ORDER BY con.conname",
    )
    .bind(schema)
    .fetch_all(&mut conn)
    .await?;
    conn.close().await?;

    // Rows arrive sorted by table, so grouping keeps table and column order.
    let mut tables: BTreeMap<String, DiagramTable> = BTreeMap::new();
    for (table, estimate, name, data_type, nullable, primary_key) in rows {
        tables
            .entry(table.clone())
            .or_insert_with(|| DiagramTable {
                name: table,
                row_estimate: (estimate >= 0).then_some(estimate),
                columns: Vec::new(),
            })
            .columns
            .push(DiagramColumn { name, data_type, nullable, primary_key });
    }

    Ok(SchemaDiagram {
        tables: tables.into_values().collect(),
        relationships: relationships
            .into_iter()
            .map(|(name, table, columns, referenced_schema, referenced_table, referenced_columns)| Relationship {
                name,
                table,
                columns,
                referenced_schema,
                referenced_table,
                referenced_columns,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, local};
    use super::*;

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn reads_tables_columns_and_relationships() {
        let mut conn = connect(&local(), PASSWORD, None).await.unwrap();
        sqlx::raw_sql(
            "DROP SCHEMA IF EXISTS dbforge_diagram CASCADE;
             CREATE SCHEMA dbforge_diagram;
             CREATE TABLE dbforge_diagram.customers (id int PRIMARY KEY, referrer_id int REFERENCES dbforge_diagram.customers);
             CREATE TABLE dbforge_diagram.orders (
                 id int PRIMARY KEY,
                 customer_id int NOT NULL REFERENCES dbforge_diagram.customers (id)
             );
             CREATE VIEW dbforge_diagram.recent AS SELECT * FROM dbforge_diagram.orders;",
        )
        .execute(&mut conn)
        .await
        .unwrap();

        let diagram = schema_diagram(&local(), PASSWORD, "shop", "dbforge_diagram").await.unwrap();

        let tables: Vec<_> = diagram.tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(tables, ["customers", "orders"], "views are left out");
        let orders = &diagram.tables[1].columns;
        assert_eq!(
            orders.iter().map(|c| (c.name.as_str(), c.primary_key, c.nullable)).collect::<Vec<_>>(),
            [("id", true, false), ("customer_id", false, false)]
        );

        let relationships: Vec<_> = diagram
            .relationships
            .iter()
            .map(|r| {
                (r.table.as_str(), r.columns.join(","), r.referenced_table.as_str(), r.referenced_columns.join(","))
            })
            .collect();
        assert_eq!(
            relationships,
            [
                ("customers", "referrer_id".into(), "customers", "id".into()),
                ("orders", "customer_id".into(), "customers", "id".into()),
            ]
        );
        assert!(diagram.relationships.iter().all(|r| r.referenced_schema == "dbforge_diagram"));

        sqlx::query("DROP SCHEMA dbforge_diagram CASCADE").execute(&mut conn).await.unwrap();
    }
}
