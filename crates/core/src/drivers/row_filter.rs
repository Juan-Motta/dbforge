//! What the table view's filter bar narrows and orders rows by. SQL engines
//! take the text as a `WHERE` condition and an `ORDER BY` list, as typed;
//! MongoDB takes a filter document and a sort document in mongosh syntax.

use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RowFilter {
    /// `status = 'paid' AND total > 100`, or `{status: 'paid'}` for MongoDB.
    pub filter: String,
    /// `created_at DESC`, or `{created_at: -1}` for MongoDB.
    pub order: String,
}

/// The ` WHERE …` and ` ORDER BY …` of a `SELECT`; without an order the rows
/// keep `default_order` (the primary key), so pages stay stable.
///
/// The condition goes in parentheses so an `OR` can't escape it, and both are
/// on lines of their own so a trailing `-- comment` can't swallow what follows.
pub fn sql_clauses(filter: &RowFilter, default_order: &[String]) -> String {
    let mut sql = String::new();
    let condition = filter.filter.trim().trim_end_matches(';').trim();
    if !condition.is_empty() {
        sql.push_str(&format!(" WHERE (\n{condition}\n)"));
    }
    let order = filter.order.trim().trim_end_matches(';').trim();
    if !order.is_empty() {
        sql.push_str(&format!(" ORDER BY\n{order}\n"));
    } else if !default_order.is_empty() {
        sql.push_str(&format!(" ORDER BY {}", default_order.join(", ")));
    }
    sql
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(filter: &str, order: &str) -> RowFilter {
        RowFilter { filter: filter.into(), order: order.into() }
    }

    #[test]
    fn writes_the_condition_and_order_as_typed() {
        let key = ["\"id\"".to_string()];
        assert_eq!(sql_clauses(&RowFilter::default(), &key), " ORDER BY \"id\"");
        assert_eq!(sql_clauses(&RowFilter::default(), &[]), "");
        assert_eq!(
            sql_clauses(&filter("status = 'paid' OR total > 100;", " created_at DESC "), &key),
            " WHERE (\nstatus = 'paid' OR total > 100\n) ORDER BY\ncreated_at DESC\n"
        );
        assert_eq!(sql_clauses(&filter("a = 1 -- only a", ""), &key), " WHERE (\na = 1 -- only a\n) ORDER BY \"id\"");
    }
}
