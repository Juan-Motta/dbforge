//! What the filter bar narrows and orders a table's rows by. SQL engines read
//! a `WHERE` condition and an `ORDER BY` list as typed, Cosmos DB the same on
//! its items, called `c`; MongoDB reads a filter and a sort document in
//! mongosh syntax. A header click sorts by one column: ascending, then
//! descending, then not at all.

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::RowFilter;

use crate::i18n::TableMessages;
use crate::sql::Dialect;
use crate::sql::snippets;

/// Which way a column is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}

/// How the bar labels and hints its two fields for an engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterLabels {
    pub filter: &'static str,
    pub filter_hint: &'static str,
    pub order: &'static str,
    pub order_hint: &'static str,
}

/// Whether `filter` narrows or orders the rows at all.
pub fn is_filtered(filter: &RowFilter) -> bool {
    !filter.filter.trim().is_empty() || !filter.order.trim().is_empty()
}

/// The bar's labels and hints for `engine`, in the language of `m`; SQL keeps its keywords.
pub fn filter_labels(engine: Engine, m: &TableMessages) -> FilterLabels {
    match engine {
        Engine::Mongodb => FilterLabels {
            filter: m.filter,
            filter_hint: "{status: 'paid', total: {$gt: 100}}",
            order: m.sort,
            order_hint: "{created: -1}",
        },
        Engine::Cosmosdb => FilterLabels {
            filter: "WHERE",
            filter_hint: "c.status = 'paid' AND c.total > 100",
            order: "ORDER BY",
            order_hint: "c.created DESC",
        },
        _ => FilterLabels {
            filter: "WHERE",
            filter_hint: "status = 'paid' AND total > 100",
            order: "ORDER BY",
            order_hint: "created_at DESC",
        },
    }
}

/// A column name as `engine` writes it in a condition: quoted in SQL, a key
/// in a MongoDB document, a property of Cosmos DB's `c`.
pub fn quote(engine: Engine, column: &str) -> String {
    if engine == Engine::Mongodb {
        let plain = column.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$')
            && column.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
        return if plain { column.to_owned() } else { serde_json::Value::from(column).to_string() };
    }
    snippets::quote(Dialect::of(engine).unwrap_or(Dialect::Postgres), column)
}

/// How an order sorts by `column` alone in `direction`, as `engine` writes it.
pub fn order_text(engine: Engine, column: &str, direction: Direction) -> String {
    let name = quote(engine, column);
    match (engine, direction) {
        (Engine::Mongodb, Direction::Asc) => format!("{{{name}: 1}}"),
        (Engine::Mongodb, Direction::Desc) => format!("{{{name}: -1}}"),
        (_, Direction::Asc) => name,
        (_, Direction::Desc) => format!("{name} DESC"),
    }
}

/// Which way `order` sorts by `column` alone, as the header shows it.
pub fn sort_of(engine: Engine, column: &str, order: &str) -> Option<Direction> {
    let text = order.trim();
    [Direction::Asc, Direction::Desc].into_iter().find(|direction| text == order_text(engine, column, *direction))
}

/// A header click: ascending, then descending, then no order; it replaces any other order.
pub fn sort_by(engine: Engine, column: &str, current: &RowFilter) -> RowFilter {
    let order = match sort_of(engine, column, &current.order) {
        None => order_text(engine, column, Direction::Asc),
        Some(Direction::Asc) => order_text(engine, column, Direction::Desc),
        Some(Direction::Desc) => String::new(),
    };
    RowFilter { filter: current.filter.clone(), order }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn filter(filter: &str, order: &str) -> RowFilter {
        RowFilter { filter: filter.into(), order: order.into() }
    }

    #[test]
    fn cycles_a_column_through_ascending_descending_and_no_order_quoted_for_the_engine() {
        let asc = sort_by(Engine::Postgres, "total", &RowFilter::default());
        assert_eq!(asc.order, "\"total\"");
        let desc = sort_by(Engine::Postgres, "total", &asc);
        assert_eq!(desc.order, "\"total\" DESC");
        assert_eq!(sort_by(Engine::Postgres, "total", &desc).order, "");
        assert_eq!(sort_by(Engine::Mysql, "we`ird", &RowFilter::default()).order, "`we``ird`");
        assert_eq!(sort_by(Engine::Sqlserver, "a]b", &RowFilter::default()).order, "[a]]b]");
    }

    #[test]
    fn writes_mongodb_sorts_as_documents_and_keeps_the_filter() {
        let filtered = filter("{status: 'paid'}", "");
        assert_eq!(sort_by(Engine::Mongodb, "total", &filtered), filter("{status: 'paid'}", "{total: 1}"));
        assert_eq!(sort_by(Engine::Mongodb, "total", &filter("", "{total: 1}")).order, "{total: -1}");
        assert_eq!(sort_by(Engine::Mongodb, "created at", &RowFilter::default()).order, "{\"created at\": 1}");
    }

    #[test]
    fn writes_cosmos_db_sorts_on_the_item_c() {
        let asc = sort_by(Engine::Cosmosdb, "total", &RowFilter::default());
        assert_eq!(asc.order, "c.total");
        assert_eq!(sort_by(Engine::Cosmosdb, "total", &asc).order, "c.total DESC");
        assert_eq!(sort_by(Engine::Cosmosdb, "created at", &RowFilter::default()).order, "c[\"created at\"]");
        assert_eq!(sort_of(Engine::Cosmosdb, "total", "c.total DESC"), Some(Direction::Desc));
    }

    #[test]
    fn replaces_another_column_s_order() {
        assert_eq!(sort_by(Engine::Postgres, "id", &filter("", "total DESC, id")).order, "\"id\"");
    }

    #[test]
    fn tells_the_header_which_way_a_column_is_sorted() {
        assert_eq!(sort_of(Engine::Postgres, "total", "\"total\" DESC"), Some(Direction::Desc));
        assert_eq!(sort_of(Engine::Postgres, "total", "\"total\""), Some(Direction::Asc));
        assert_eq!(sort_of(Engine::Mongodb, "total", "{total: -1}"), Some(Direction::Desc));
        assert_eq!(sort_of(Engine::Postgres, "total", "total DESC"), None);
    }

    #[test]
    fn names_mongodb_s_fields_in_the_interface_language_and_keeps_sql_keywords() {
        let en = &Language::En.messages().table;
        let es = &Language::Es.messages().table;
        assert_eq!(
            (filter_labels(Engine::Mongodb, en).filter, filter_labels(Engine::Mongodb, en).order),
            ("Filter", "Sort")
        );
        assert_eq!(
            (filter_labels(Engine::Mongodb, es).filter, filter_labels(Engine::Mongodb, es).order),
            ("Filtro", "Orden")
        );
        assert_eq!(
            (filter_labels(Engine::Postgres, es).filter, filter_labels(Engine::Postgres, es).order),
            ("WHERE", "ORDER BY")
        );
    }

    #[test]
    fn knows_when_rows_are_filtered() {
        assert!(!is_filtered(&filter("  ", "")));
        assert!(is_filtered(&filter("", "id")));
    }
}
