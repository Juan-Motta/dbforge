//! What a cell's menu writes: a condition on its value (to narrow the rows
//! or copy), a one-column order, and its row as JSON or as an INSERT.

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::{RowFilter, TableRef};

use super::filter::{Direction, order_text, quote};
use super::links::literal;
use crate::sql::Dialect;
use crate::sql::snippets::qualified_name;

/// How a cell's menu narrows the rows by its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueTest {
    Equal,
    NotEqual,
    IsNull,
    IsNotNull,
}

/// Engines whose filter is a condition on columns. MongoDB's is a document,
/// and a cell's text doesn't say whether its value is a number, a string or
/// an ObjectId, so it gets no filter from a value.
pub fn filters_by_value(engine: Engine) -> bool {
    engine != Engine::Mongodb
}

/// Relational engines, whose rows copy as an INSERT.
pub fn is_relational(engine: Engine) -> bool {
    matches!(engine, Engine::Postgres | Engine::Mysql | Engine::Mariadb | Engine::Sqlite | Engine::Sqlserver)
}

/// The condition that keeps the rows whose `column` passes `test` against `value`.
pub fn value_condition(engine: Engine, column: &str, test: ValueTest, value: Option<&str>) -> String {
    let name = quote(engine, column);
    let cosmos = engine == Engine::Cosmosdb;
    match (test, value) {
        (ValueTest::IsNull, _) | (ValueTest::Equal, None) => {
            if cosmos {
                format!("IS_NULL({name})")
            } else {
                format!("{name} IS NULL")
            }
        }
        (ValueTest::IsNotNull, _) | (ValueTest::NotEqual, None) => {
            if cosmos {
                format!("NOT IS_NULL({name})")
            } else {
                format!("{name} IS NOT NULL")
            }
        }
        (test, Some(value)) => {
            let operator = if test == ValueTest::Equal { "=" } else { "<>" };
            format!("{name} {operator} {}", literal(engine, value))
        }
    }
}

/// `current` narrowed by `condition`, keeping what it already filtered.
pub fn narrowed_by(current: &RowFilter, condition: &str) -> RowFilter {
    let existing = current.filter.trim();
    let filter = if existing.is_empty() { condition.to_owned() } else { format!("({existing}) AND {condition}") };
    RowFilter { filter, order: current.order.clone() }
}

/// `current` sorted by `column` alone.
pub fn sorted_by(engine: Engine, column: &str, direction: Direction, current: &RowFilter) -> RowFilter {
    RowFilter { filter: current.filter.clone(), order: order_text(engine, column, direction) }
}

/// A row as a JSON object, column by column, indented by two spaces; NULL is `null`.
pub fn row_as_json(columns: &[String], row: &[Option<String>]) -> String {
    let mut out = String::from("{");
    for (index, column) in columns.iter().enumerate() {
        let value = row.get(index).cloned().flatten().map_or(serde_json::Value::Null, serde_json::Value::String);
        out.push_str(if index == 0 { "\n  " } else { ",\n  " });
        out.push_str(&format!("{}: {value}", serde_json::Value::from(column.as_str())));
    }
    out.push_str(if columns.is_empty() { "}" } else { "\n}" });
    out
}

/// The INSERT that would add `row` again, with its values. Relational engines only.
pub fn row_as_insert(engine: Engine, table: &TableRef, columns: &[String], row: &[Option<String>]) -> String {
    let dialect = Dialect::of(engine).unwrap_or(Dialect::Postgres);
    let names: Vec<String> = columns.iter().map(|column| quote(engine, column)).collect();
    let values: Vec<String> =
        row.iter().map(|value| value.as_deref().map_or("NULL".to_owned(), |value| literal(engine, value))).collect();
    format!("INSERT INTO {} ({})\nVALUES ({});", qualified_name(dialect, table), names.join(", "), values.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(filter: &str, order: &str) -> RowFilter {
        RowFilter { filter: filter.into(), order: order.into() }
    }

    #[test]
    fn write_conditions_on_a_value_in_each_engine() {
        assert_eq!(
            value_condition(Engine::Postgres, "name", ValueTest::Equal, Some("O'Brien")),
            "\"name\" = 'O''Brien'"
        );
        assert_eq!(value_condition(Engine::Mysql, "name", ValueTest::NotEqual, Some("a\\b")), "`name` <> 'a\\\\b'");
        assert_eq!(value_condition(Engine::Sqlserver, "name", ValueTest::Equal, None), "[name] IS NULL");
        assert_eq!(value_condition(Engine::Postgres, "name", ValueTest::NotEqual, None), "\"name\" IS NOT NULL");
        assert_eq!(value_condition(Engine::Cosmosdb, "name", ValueTest::IsNull, Some("x")), "IS_NULL(c.name)");
        assert_eq!(value_condition(Engine::Cosmosdb, "total", ValueTest::Equal, Some("12")), "c.total = '12'");
        assert!(!filters_by_value(Engine::Mongodb));
    }

    #[test]
    fn narrow_an_existing_filter_and_sort_by_one_column() {
        assert_eq!(narrowed_by(&filter("", "x"), "a = 1"), filter("a = 1", "x"));
        assert_eq!(narrowed_by(&filter("b = 2 OR c = 3", ""), "a = 1").filter, "(b = 2 OR c = 3) AND a = 1");
        assert_eq!(
            sorted_by(Engine::Postgres, "name", Direction::Desc, &filter("a", "b")),
            filter("a", "\"name\" DESC")
        );
        assert_eq!(sorted_by(Engine::Mongodb, "name", Direction::Asc, &filter("", "")).order, "{name: 1}");
    }

    #[test]
    fn copy_a_row_as_json_or_as_an_insert() {
        let columns = ["id".to_owned(), "name".to_owned()];
        assert_eq!(row_as_json(&columns, &[Some("1".into()), None]), "{\n  \"id\": \"1\",\n  \"name\": null\n}");
        let users = TableRef { database: "shop".into(), schema: "public".into(), name: "users".into() };
        assert_eq!(
            row_as_insert(Engine::Postgres, &users, &columns, &[Some("1".into()), Some("O'Brien".into())]),
            "INSERT INTO \"public\".\"users\" (\"id\", \"name\")\nVALUES ('1', 'O''Brien');"
        );
    }
}
