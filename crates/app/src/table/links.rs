//! Foreign keys as links: a cell of a key column opens the table it points
//! to, filtered to the row its value names.

use std::collections::HashMap;

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::{ForeignKey, RowFilter, TableRef};

use super::filter::quote;

/// The table a foreign key points to. Engines name it bare when it is in the
/// same schema, else `schema.table`, with PostgreSQL's quotes where needed.
pub fn referenced_table(from: &TableRef, name: &str) -> TableRef {
    let parts = split_qualified(name);
    match parts.as_slice() {
        [schema, table] => TableRef { database: from.database.clone(), schema: schema.clone(), name: table.clone() },
        _ => TableRef {
            database: from.database.clone(),
            schema: from.schema.clone(),
            name: parts.last().cloned().unwrap_or_default(),
        },
    }
}

/// `a.b` or `"a.x"."b"` into its parts, unquoted.
fn split_qualified(name: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            '.' if !quoted => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
}

/// The filter that shows the row a foreign key points to: each referenced
/// column equal to this row's value. `None` while any key column is NULL,
/// since such a row points nowhere.
pub fn link_filter(engine: Engine, key: &ForeignKey, row: &HashMap<&str, Option<&str>>) -> Option<RowFilter> {
    let mut conditions = Vec::new();
    for (column, referenced) in key.columns.iter().zip(&key.referenced_columns) {
        let value = row.get(column.as_str()).copied().flatten()?;
        conditions.push(format!("{} = {}", quote(engine, referenced), literal(engine, value)));
    }
    Some(RowFilter { filter: conditions.join(" AND "), order: String::new() })
}

/// A string literal; the server converts it to the column's type.
pub fn literal(engine: Engine, value: &str) -> String {
    let escaped =
        if matches!(engine, Engine::Mysql | Engine::Mariadb) { value.replace('\\', "\\\\") } else { value.to_owned() };
    format!("'{}'", escaped.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orders() -> TableRef {
        TableRef { database: "shop".into(), schema: "public".into(), name: "orders".into() }
    }

    fn fk(columns: &[&str], table: &str, referenced: &[&str]) -> ForeignKey {
        ForeignKey {
            name: "fk".into(),
            columns: columns.iter().map(|c| c.to_string()).collect(),
            referenced_table: table.into(),
            referenced_columns: referenced.iter().map(|c| c.to_string()).collect(),
            on_update: "NO ACTION".into(),
            on_delete: "NO ACTION".into(),
        }
    }

    fn parts(table: TableRef) -> (String, String, String) {
        (table.database, table.schema, table.name)
    }

    #[test]
    fn stays_in_the_table_s_schema_unless_the_key_names_another() {
        let owned = |a: &str, b: &str, c: &str| (a.to_owned(), b.to_owned(), c.to_owned());
        assert_eq!(parts(referenced_table(&orders(), "customers")), owned("shop", "public", "customers"));
        assert_eq!(parts(referenced_table(&orders(), "billing.invoices")), owned("shop", "billing", "invoices"));
        assert_eq!(
            parts(referenced_table(&orders(), "\"Odd.Schema\".\"My \"\"T\"\"\"")),
            owned("shop", "Odd.Schema", "My \"T\"")
        );
    }

    #[test]
    fn filters_the_referenced_table_by_the_row_s_key_values_quoted_for_the_engine() {
        let row = HashMap::from([("customer_id", Some("877")), ("region", Some("O'Hara")), ("note", None)]);
        let link = |engine, key: &ForeignKey| link_filter(engine, key, &row).map(|filter| filter.filter);
        assert_eq!(
            link(Engine::Postgres, &fk(&["customer_id"], "customers", &["id"])).as_deref(),
            Some("\"id\" = '877'")
        );
        assert_eq!(
            link(Engine::Mysql, &fk(&["customer_id", "region"], "customers", &["id", "region"])).as_deref(),
            Some("`id` = '877' AND `region` = 'O''Hara'")
        );
        assert_eq!(
            link(Engine::Sqlserver, &fk(&["customer_id"], "customers", &["id"])).as_deref(),
            Some("[id] = '877'")
        );
        assert_eq!(link_filter(Engine::Postgres, &fk(&["customer_id"], "c", &["id"]), &row).unwrap().order, "");
    }

    #[test]
    fn has_no_link_while_a_key_column_is_null() {
        let row = HashMap::from([("note", None)]);
        assert_eq!(link_filter(Engine::Postgres, &fk(&["note"], "notes", &["id"]), &row), None);
    }
}
