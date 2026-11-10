//! A SQL Server table's or view's structure from the sys catalog, and a
//! schema's ER diagram. SQL Server keeps no `CREATE TABLE` text, so a table's
//! DDL is rebuilt from the catalog; a view's and a trigger's are their own.

use std::collections::BTreeMap;

use super::{Client, connect, query, quote_ident, text_rows, type_name};
use crate::connections::model::Connection;
use crate::drivers::{
    ColumnInfo, DiagramColumn, DiagramTable, ForeignKey, IndexInfo, ObjectKind, Relationship, SchemaDiagram, TableRef,
    TableStructure, TriggerInfo,
};
use crate::error::{AppError, Result};

/// The object id of `[@P1].[@P2]`, for queries that take the schema and name.
const OBJECT: &str = "OBJECT_ID(QUOTENAME(@P1) + '.' + QUOTENAME(@P2))";

/// Separates the items of a `STRING_AGG`; can't appear in a name typed by hand.
const SEPARATOR: char = '\u{1f}';

/// Marks a descending index key in a `STRING_AGG` of key columns.
const DESCENDING: char = '\u{1e}';

type Rows = Vec<Vec<Option<String>>>;

fn text(row: &[Option<String>], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

fn list(row: &[Option<String>], i: usize) -> Vec<String> {
    row.get(i).cloned().flatten().map(|items| items.split(SEPARATOR).map(String::from).collect()).unwrap_or_default()
}

async fn rows_of(client: &mut Client, sql: &str, table: &TableRef) -> Result<Rows> {
    text_rows(client, query(sql.replace("{OBJECT}", OBJECT), &[&table.schema, &table.name])).await
}

pub async fn table_structure(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
) -> Result<TableStructure> {
    let mut client = connect(connection, password, Some(&table.database)).await?;
    let found = rows_of(
        &mut client,
        "SELECT o.type, CAST(ep.value AS nvarchar(max)),
                (SELECT SUM(rows) FROM sys.partitions WHERE object_id = o.object_id AND index_id IN (0, 1)),
                (SELECT SUM(a.total_pages) * 8192
                 FROM sys.partitions p JOIN sys.allocation_units a ON a.container_id = p.partition_id
                 WHERE p.object_id = o.object_id AND p.index_id IN (0, 1)),
                (SELECT SUM(a.total_pages) * 8192
                 FROM sys.partitions p JOIN sys.allocation_units a ON a.container_id = p.partition_id
                 WHERE p.object_id = o.object_id AND p.index_id > 1)
         FROM sys.objects o
         LEFT JOIN sys.extended_properties ep
             ON ep.class = 1 AND ep.major_id = o.object_id AND ep.minor_id = 0 AND ep.name = 'MS_Description'
         WHERE o.object_id = {OBJECT} AND o.type IN ('U', 'V')",
        table,
    )
    .await?;
    let row =
        found.first().ok_or_else(|| AppError::Invalid(format!("Table {}.{} not found", table.schema, table.name)))?;
    let is_view = text(row, 0).trim() == "V";
    let comment = row[1].clone();
    let bytes = |i: usize| text(row, i).parse().unwrap_or(0);

    let columns = columns(&mut client, table).await?;
    let indexes = indexes(&mut client, table).await?;
    let foreign_keys = foreign_keys(&mut client, table).await?;
    let triggers: Vec<TriggerInfo> = rows_of(
        &mut client,
        "SELECT name, is_disabled, OBJECT_DEFINITION(object_id) FROM sys.triggers
         WHERE parent_id = {OBJECT} ORDER BY name",
        table,
    )
    .await?
    .iter()
    .map(|row| TriggerInfo {
        name: text(row, 0),
        enabled: text(row, 1) == "0",
        definition: text(row, 2).trim().to_string(),
    })
    .collect();
    let checks = rows_of(
        &mut client,
        "SELECT name, definition FROM sys.check_constraints WHERE parent_object_id = {OBJECT} ORDER BY name",
        table,
    )
    .await?;
    let view_definition = if is_view {
        rows_of(&mut client, "SELECT OBJECT_DEFINITION({OBJECT})", table).await?.first().map(|r| text(r, 0))
    } else {
        None
    };
    client.close().await?;

    let relation = format!("{}.{}", quote_ident(&table.schema), quote_ident(&table.name));
    let mut sections = match view_definition {
        Some(definition) => vec![definition.trim().to_string()],
        None => table_ddl(&relation, &columns, &indexes, &foreign_keys, &checks, table),
    };
    sections.extend(comments(table, is_view, &comment, &columns));
    sections.extend(triggers.iter().map(|t| t.definition.trim().to_string()));

    Ok(TableStructure {
        kind: if is_view { ObjectKind::View } else { ObjectKind::Table },
        row_estimate: if is_view { None } else { row[2].as_deref().and_then(|r| r.parse().ok()) },
        data_bytes: bytes(3),
        index_bytes: bytes(4),
        comment,
        columns: columns.into_iter().map(|c| c.info).collect(),
        indexes: indexes.into_iter().map(|i| i.info).collect(),
        foreign_keys,
        triggers,
        // Statements that must start their own batch (views, triggers) are kept
        // apart by `GO`, which the console understands.
        ddl: sections.join("\nGO\n\n"),
    })
}

struct CatalogColumn {
    info: ColumnInfo,
    /// What follows the name in `CREATE TABLE`: type, identity or computation,
    /// default and nullability.
    definition: String,
}

async fn columns(client: &mut Client, table: &TableRef) -> Result<Vec<CatalogColumn>> {
    let rows = rows_of(
        client,
        "SELECT c.name, TYPE_NAME(c.user_type_id), c.max_length, c.precision, c.scale, c.is_nullable,
                dc.definition, cc.definition, cc.is_persisted, c.is_identity,
                CAST(ic.seed_value AS nvarchar(40)), CAST(ic.increment_value AS nvarchar(40)),
                CAST(ep.value AS nvarchar(max))
         FROM sys.columns c
         LEFT JOIN sys.default_constraints dc ON dc.object_id = c.default_object_id
         LEFT JOIN sys.computed_columns cc ON cc.object_id = c.object_id AND cc.column_id = c.column_id
         LEFT JOIN sys.identity_columns ic ON ic.object_id = c.object_id AND ic.column_id = c.column_id
         LEFT JOIN sys.extended_properties ep
             ON ep.class = 1 AND ep.major_id = c.object_id AND ep.minor_id = c.column_id AND ep.name = 'MS_Description'
         WHERE c.object_id = {OBJECT}
         ORDER BY c.column_id",
        table,
    )
    .await?;
    Ok(rows
        .iter()
        .map(|row| {
            let data_type = type_name(&text(row, 1), &text(row, 2), &text(row, 3), &text(row, 4));
            let nullable = text(row, 5) == "1";
            let default = row[6].clone();
            let computed = row[7]
                .clone()
                .map(|expression| format!("AS {expression}{}", if text(row, 8) == "1" { " PERSISTED" } else { "" }));
            let identity = (text(row, 9) == "1").then(|| format!("IDENTITY({},{})", text(row, 10), text(row, 11)));

            let definition = match &computed {
                Some(computed) => computed.clone(),
                None => {
                    let mut parts = vec![data_type.clone()];
                    parts.extend(identity.clone());
                    parts.extend(default.as_ref().map(|d| format!("DEFAULT {d}")));
                    parts.push(if nullable { "NULL" } else { "NOT NULL" }.into());
                    parts.join(" ")
                }
            };
            CatalogColumn {
                info: ColumnInfo {
                    options: Vec::new(),
                    name: text(row, 0),
                    data_type,
                    nullable,
                    default: computed.or(identity).or(default.map(|d| unwrap_parens(&d).to_string())),
                    comment: row[12].clone(),
                },
                definition,
            }
        })
        .collect())
}

/// SQL Server stores `DEFAULT 5` as `((5))`; the parentheses are dropped for display.
fn unwrap_parens(text: &str) -> &str {
    let mut text = text.trim();
    while text.starts_with('(') && text.ends_with(')') && encloses_all(text) {
        text = text[1..text.len() - 1].trim();
    }
    text
}

/// Whether the opening parenthesis closes at the very end, as in `(a + b)` but not `(a) + (b)`.
fn encloses_all(text: &str) -> bool {
    let mut depth = 0;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return i == text.len() - 1;
        }
    }
    false
}

struct CatalogIndex {
    info: IndexInfo,
    /// Key columns with whether each is descending.
    keys: Vec<(String, bool)>,
    included: Vec<String>,
}

impl CatalogIndex {
    /// The key list of `CREATE INDEX`: `[a], [b] DESC`.
    fn key_list(&self) -> String {
        let keys: Vec<String> = self
            .keys
            .iter()
            .map(|(name, descending)| format!("{}{}", quote_ident(name), if *descending { " DESC" } else { "" }))
            .collect();
        keys.join(", ")
    }
}

async fn indexes(client: &mut Client, table: &TableRef) -> Result<Vec<CatalogIndex>> {
    let rows = rows_of(
        client,
        "SELECT i.name, LOWER(i.type_desc), i.is_unique, i.is_primary_key, i.filter_definition,
                (SELECT SUM(a.total_pages) * 8192 FROM sys.partitions p
                 JOIN sys.allocation_units a ON a.container_id = p.partition_id
                 WHERE p.object_id = i.object_id AND p.index_id = i.index_id),
                (SELECT STRING_AGG(COL_NAME(ic.object_id, ic.column_id)
                                   + CASE WHEN ic.is_descending_key = 1 THEN NCHAR(30) ELSE '' END, NCHAR(31))
                        WITHIN GROUP (ORDER BY ic.key_ordinal)
                 FROM sys.index_columns ic
                 WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id AND ic.key_ordinal > 0),
                (SELECT STRING_AGG(COL_NAME(ic.object_id, ic.column_id), NCHAR(31))
                        WITHIN GROUP (ORDER BY ic.index_column_id)
                 FROM sys.index_columns ic
                 WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id AND ic.is_included_column = 1)
         FROM sys.indexes i
         WHERE i.object_id = {OBJECT} AND i.type > 0
         ORDER BY i.is_primary_key DESC, i.name",
        table,
    )
    .await?;
    Ok(rows
        .iter()
        .map(|row| {
            let keys: Vec<(String, bool)> = list(row, 6)
                .into_iter()
                .map(|key| match key.strip_suffix(DESCENDING) {
                    Some(name) => (name.to_string(), true),
                    None => (key, false),
                })
                .collect();
            CatalogIndex {
                info: IndexInfo {
                    name: text(row, 0),
                    method: text(row, 1),
                    unique: text(row, 2) == "1",
                    primary: text(row, 3) == "1",
                    columns: keys
                        .iter()
                        .map(|(name, descending)| if *descending { format!("{name} DESC") } else { name.clone() })
                        .collect(),
                    condition: row[4].clone().map(|c| unwrap_parens(&c).to_string()),
                    bytes: text(row, 5).parse().unwrap_or(0),
                },
                keys,
                included: list(row, 7),
            }
        })
        .collect())
}

async fn foreign_keys(client: &mut Client, table: &TableRef) -> Result<Vec<ForeignKey>> {
    let rows = rows_of(
        client,
        "SELECT fk.name, OBJECT_SCHEMA_NAME(fk.referenced_object_id), OBJECT_NAME(fk.referenced_object_id),
                fk.update_referential_action_desc, fk.delete_referential_action_desc,
                STRING_AGG(COL_NAME(fkc.parent_object_id, fkc.parent_column_id), NCHAR(31))
                    WITHIN GROUP (ORDER BY fkc.constraint_column_id),
                STRING_AGG(COL_NAME(fkc.referenced_object_id, fkc.referenced_column_id), NCHAR(31))
                    WITHIN GROUP (ORDER BY fkc.constraint_column_id)
         FROM sys.foreign_keys fk
         JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id
         WHERE fk.parent_object_id = {OBJECT}
         GROUP BY fk.name, fk.referenced_object_id, fk.update_referential_action_desc, fk.delete_referential_action_desc
         ORDER BY fk.name",
        table,
    )
    .await?;
    Ok(rows
        .iter()
        .map(|row| {
            let schema = text(row, 1);
            ForeignKey {
                name: text(row, 0),
                columns: list(row, 5),
                referenced_table: if schema == table.schema {
                    text(row, 2)
                } else {
                    format!("{schema}.{}", text(row, 2))
                },
                referenced_columns: list(row, 6),
                on_update: text(row, 3).replace('_', " "),
                on_delete: text(row, 4).replace('_', " "),
            }
        })
        .collect())
}

/// `CREATE TABLE` with its key, foreign keys and checks, then its other indexes.
fn table_ddl(
    relation: &str,
    columns: &[CatalogColumn],
    indexes: &[CatalogIndex],
    foreign_keys: &[ForeignKey],
    checks: &Rows,
    table: &TableRef,
) -> Vec<String> {
    let names = |names: &[String]| names.iter().map(|n| quote_ident(n)).collect::<Vec<_>>().join(", ");
    let mut body: Vec<String> =
        columns.iter().map(|c| format!("    {} {}", quote_ident(&c.info.name), c.definition)).collect();
    for index in indexes.iter().filter(|i| i.info.primary) {
        let kind = index.info.method.to_uppercase();
        body.push(format!(
            "    CONSTRAINT {} PRIMARY KEY {kind} ({})",
            quote_ident(&index.info.name),
            index.key_list()
        ));
    }
    for key in foreign_keys {
        let referenced = match key.referenced_table.split_once('.') {
            Some((schema, name)) => format!("{}.{}", quote_ident(schema), quote_ident(name)),
            None => format!("{}.{}", quote_ident(&table.schema), quote_ident(&key.referenced_table)),
        };
        let mut line = format!(
            "    CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {referenced} ({})",
            quote_ident(&key.name),
            names(&key.columns),
            names(&key.referenced_columns)
        );
        for (event, action) in [("UPDATE", &key.on_update), ("DELETE", &key.on_delete)] {
            if action != "NO ACTION" {
                line.push_str(&format!(" ON {event} {action}"));
            }
        }
        body.push(line);
    }
    for check in checks {
        body.push(format!("    CONSTRAINT {} CHECK {}", quote_ident(&text(check, 0)), text(check, 1)));
    }

    let mut sections = vec![format!("CREATE TABLE {relation} (\n{}\n);", body.join(",\n"))];
    let standalone: Vec<String> = indexes
        .iter()
        .filter(|i| !i.info.primary)
        .map(|index| {
            let mut statement = format!(
                "CREATE {}{} INDEX {} ON {relation} ({})",
                if index.info.unique { "UNIQUE " } else { "" },
                index.info.method.to_uppercase(),
                quote_ident(&index.info.name),
                index.key_list()
            );
            if !index.included.is_empty() {
                statement.push_str(&format!(" INCLUDE ({})", names(&index.included)));
            }
            if let Some(condition) = &index.info.condition {
                statement.push_str(&format!(" WHERE {condition}"));
            }
            format!("{statement};")
        })
        .collect();
    if !standalone.is_empty() {
        sections[0].push_str(&format!("\n\n{}", standalone.join("\n")));
    }
    sections
}

/// Descriptions are the `MS_Description` extended property, as SSMS writes them.
fn comments(table: &TableRef, is_view: bool, comment: &Option<String>, columns: &[CatalogColumn]) -> Option<String> {
    let literal = |text: &str| format!("N'{}'", text.replace('\'', "''"));
    let level1 = format!(
        "'SCHEMA', {}, '{}', {}",
        literal(&table.schema),
        if is_view { "VIEW" } else { "TABLE" },
        literal(&table.name)
    );
    let mut lines: Vec<String> = comment
        .iter()
        .map(|text| format!("EXEC sys.sp_addextendedproperty 'MS_Description', {}, {level1};", literal(text)))
        .collect();
    lines.extend(columns.iter().filter_map(|c| {
        let text = c.info.comment.as_ref()?;
        Some(format!(
            "EXEC sys.sp_addextendedproperty 'MS_Description', {}, {level1}, 'COLUMN', {};",
            literal(text),
            literal(&c.info.name)
        ))
    }));
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Every table of a schema with its columns, and the foreign keys declared in it.
pub async fn schema_diagram(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    schema: &str,
) -> Result<SchemaDiagram> {
    let mut client = connect(connection, password, Some(database)).await?;
    let rows = text_rows(
        &mut client,
        query(
            "SELECT t.name,
                    (SELECT SUM(rows) FROM sys.partitions WHERE object_id = t.object_id AND index_id IN (0, 1)),
                    c.name, TYPE_NAME(c.user_type_id), c.max_length, c.precision, c.scale, c.is_nullable,
                    CASE WHEN EXISTS (
                        SELECT 1 FROM sys.indexes i
                        JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id
                        WHERE i.object_id = t.object_id AND i.is_primary_key = 1 AND ic.column_id = c.column_id
                    ) THEN 1 ELSE 0 END
             FROM sys.tables t JOIN sys.columns c ON c.object_id = t.object_id
             WHERE t.schema_id = SCHEMA_ID(@P1) AND t.is_ms_shipped = 0
             ORDER BY t.name, c.column_id",
            &[schema],
        ),
    )
    .await?;
    let relationships = text_rows(
        &mut client,
        query(
            "SELECT fk.name, OBJECT_NAME(fk.parent_object_id),
                    STRING_AGG(COL_NAME(fkc.parent_object_id, fkc.parent_column_id), NCHAR(31))
                        WITHIN GROUP (ORDER BY fkc.constraint_column_id),
                    OBJECT_SCHEMA_NAME(fk.referenced_object_id), OBJECT_NAME(fk.referenced_object_id),
                    STRING_AGG(COL_NAME(fkc.referenced_object_id, fkc.referenced_column_id), NCHAR(31))
                        WITHIN GROUP (ORDER BY fkc.constraint_column_id)
             FROM sys.foreign_keys fk
             JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id
             WHERE fk.schema_id = SCHEMA_ID(@P1)
             GROUP BY fk.name, fk.parent_object_id, fk.referenced_object_id
             ORDER BY fk.name",
            &[schema],
        ),
    )
    .await?;
    client.close().await?;

    // Rows arrive sorted by table, so grouping keeps table and column order.
    let mut tables: BTreeMap<String, DiagramTable> = BTreeMap::new();
    for row in &rows {
        let name = text(row, 0);
        tables
            .entry(name.clone())
            .or_insert_with(|| DiagramTable {
                name,
                row_estimate: row[1].as_deref().and_then(|r| r.parse().ok()),
                columns: Vec::new(),
            })
            .columns
            .push(DiagramColumn {
                name: text(row, 2),
                data_type: type_name(&text(row, 3), &text(row, 4), &text(row, 5), &text(row, 6)),
                nullable: text(row, 7) == "1",
                primary_key: text(row, 8) == "1",
            });
    }
    Ok(SchemaDiagram {
        tables: tables.into_values().collect(),
        relationships: relationships
            .iter()
            .map(|row| Relationship {
                name: text(row, 0),
                table: text(row, 1),
                columns: list(row, 2),
                referenced_schema: text(row, 3),
                referenced_table: text(row, 4),
                referenced_columns: list(row, 5),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, database, table};
    use super::super::{batches, connect};
    use super::*;

    const SHOP: &str = "
        CREATE TABLE customers (id int PRIMARY KEY, email nvarchar(80) UNIQUE, country char(2) DEFAULT 'US');
        CREATE TABLE orders (
            id int IDENTITY(10,5) NOT NULL,
            customer_id int NOT NULL,
            total decimal(10,2) DEFAULT 0 CHECK (total >= 0),
            [Note] nvarchar(max),
            doubled AS total * 2 PERSISTED,
            CONSTRAINT pk_orders PRIMARY KEY CLUSTERED (id),
            CONSTRAINT fk_orders_customer FOREIGN KEY (customer_id) REFERENCES customers (id) ON DELETE CASCADE
        );
        CREATE INDEX ix_big ON orders (customer_id DESC) INCLUDE (total) WHERE total > 100;
        EXEC sys.sp_addextendedproperty 'MS_Description', N'Customer''s orders', 'SCHEMA', N'dbo', 'TABLE', N'orders';
        EXEC sys.sp_addextendedproperty 'MS_Description', N'Free text', 'SCHEMA', N'dbo', 'TABLE', N'orders', 'COLUMN', N'Note';
        GO
        CREATE TRIGGER touch ON orders AFTER UPDATE AS BEGIN SET NOCOUNT ON; END
        GO
        CREATE VIEW big_orders AS SELECT id, total FROM orders WHERE total > 100";

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn reads_the_structure_and_rebuilds_it_from_the_ddl() {
        let connection = database("dbforge_structure", SHOP).await;
        let orders = table_structure(&connection, PASSWORD, &table("dbforge_structure", "orders")).await.unwrap();

        assert_eq!(orders.kind, ObjectKind::Table);
        assert_eq!(orders.comment.as_deref(), Some("Customer's orders"));
        let columns: Vec<_> = orders
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.data_type.as_str(), c.nullable, c.default.as_deref(), c.comment.as_deref()))
            .collect();
        assert_eq!(
            columns,
            [
                ("id", "int", false, Some("IDENTITY(10,5)"), None),
                ("customer_id", "int", false, None, None),
                ("total", "decimal(10,2)", true, Some("0"), None),
                ("Note", "nvarchar(max)", true, None, Some("Free text")),
                ("doubled", "decimal(12,2)", true, Some("AS ([total]*(2)) PERSISTED"), None),
            ]
        );

        let indexes: Vec<_> = orders
            .indexes
            .iter()
            .map(|i| (i.name.as_str(), i.method.as_str(), i.primary, i.columns.clone(), i.condition.as_deref()))
            .collect();
        assert_eq!(
            indexes,
            [
                ("pk_orders", "clustered", true, vec!["id".to_string()], None),
                ("ix_big", "nonclustered", false, vec!["customer_id DESC".to_string()], Some("[total]>(100)")),
            ]
        );

        let [fk] = orders.foreign_keys.as_slice() else { panic!("{:?}", orders.foreign_keys) };
        assert_eq!(
            (fk.name.as_str(), fk.referenced_table.as_str(), fk.on_delete.as_str(), fk.on_update.as_str()),
            ("fk_orders_customer", "customers", "CASCADE", "NO ACTION")
        );
        let [trigger] = orders.triggers.as_slice() else { panic!("{:?}", orders.triggers) };
        assert!(trigger.enabled && trigger.definition.starts_with("CREATE TRIGGER touch"), "{trigger:?}");

        let customers = table_structure(&connection, PASSWORD, &table("dbforge_structure", "customers")).await.unwrap();
        assert!(customers.indexes.iter().any(|i| i.unique && i.columns == ["email"]), "{:?}", customers.indexes);
        assert_eq!(customers.columns[2].default.as_deref(), Some("'US'"));

        let view = table_structure(&connection, PASSWORD, &table("dbforge_structure", "big_orders")).await.unwrap();
        assert_eq!((view.kind, view.row_estimate), (ObjectKind::View, None));
        assert!(view.ddl.starts_with("CREATE VIEW big_orders AS"), "{}", view.ddl);

        // Running the DDL in an empty database recreates the same table.
        let copy = database("dbforge_structure_copy", "CREATE TABLE customers (id int PRIMARY KEY)").await;
        let mut client = connect(&copy, PASSWORD, None).await.unwrap();
        for batch in batches(&orders.ddl) {
            client.simple_query(batch).await.unwrap().into_results().await.unwrap();
        }
        client.close().await.unwrap();
        let rebuilt = table_structure(&copy, PASSWORD, &table("dbforge_structure_copy", "orders")).await.unwrap();
        assert_eq!(rebuilt.ddl, orders.ddl);
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn draws_tables_and_relationships() {
        let connection = database("dbforge_diagram", SHOP).await;
        let diagram = schema_diagram(&connection, PASSWORD, "dbforge_diagram", "dbo").await.unwrap();
        let tables: Vec<_> = diagram.tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(tables, ["customers", "orders"]);
        let id = &diagram.tables[1].columns[0];
        assert_eq!((id.name.as_str(), id.data_type.as_str(), id.primary_key, id.nullable), ("id", "int", true, false));

        let [relationship] = diagram.relationships.as_slice() else { panic!("{:?}", diagram.relationships) };
        assert_eq!(
            (
                relationship.table.as_str(),
                relationship.referenced_schema.as_str(),
                relationship.referenced_table.as_str()
            ),
            ("orders", "dbo", "customers")
        );
        assert_eq!(
            (relationship.columns.clone(), relationship.referenced_columns.clone()),
            (vec!["customer_id".to_string()], vec!["id".to_string()])
        );
    }

    #[test]
    fn display_drops_only_enclosing_parentheses() {
        assert_eq!(unwrap_parens("((5))"), "5");
        assert_eq!(unwrap_parens("(N'US')"), "N'US'");
        assert_eq!(unwrap_parens("((1)+(2))"), "(1)+(2)");
        assert_eq!(unwrap_parens("(getdate())"), "getdate()");
    }
}
