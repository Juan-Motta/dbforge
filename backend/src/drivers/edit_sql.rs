//! Builds the statements behind row editing, the same way for every SQL engine:
//! only the quoting, the parameter markers and how a parameter takes a column's
//! type differ, and those come from a [`Dialect`].
//!
//! Guarantees kept here for all engines: rows are identified by their full
//! primary key, tables without one are refused, values are parameters (never
//! spliced into the SQL), and only known columns are touched.

use std::collections::{BTreeMap, BTreeSet};

use super::{ApplySummary, Column, RowChange, TableRef};
use crate::error::{AppError, Result};

pub struct Dialect {
    /// Quotes identifiers: `"` for PostgreSQL, `` ` `` for MySQL.
    pub quote: char,
    /// `$1`, `$2`… when true; `?` when false.
    pub numbered: bool,
    /// Turns a parameter marker into a value of a column's type.
    pub value: fn(marker: &str, data_type: &str) -> String,
    /// What follows `INSERT INTO t` for a row that only takes defaults.
    pub default_row: &'static str,
    /// Whether a backslash escapes in string literals (MySQL's default).
    pub backslash_escapes: bool,
}

/// One statement with its parameters, in marker order.
#[derive(Debug, PartialEq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<Option<String>>,
    /// Set for UPDATE and DELETE: the key that must match exactly one row.
    pub key: Option<String>,
}

/// The error when an UPDATE or DELETE did not hit exactly one row.
pub fn changed_elsewhere(key: &str) -> AppError {
    AppError::Invalid(format!(
        "The row with {key} was changed or deleted since it was loaded. Nothing was applied; reload and try again."
    ))
}

pub fn build(dialect: &Dialect, columns: &[Column], table: &TableRef, changes: &[RowChange]) -> Result<Vec<Statement>> {
    let types: BTreeMap<&str, &str> = columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str())).collect();
    let primary_key: BTreeSet<&str> = columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
    if primary_key.is_empty() {
        return Err(AppError::Invalid(format!(
            "{}.{} has no primary key, so its rows can't be identified safely for editing.",
            table.schema, table.name
        )));
    }
    let builder = Builder { dialect, types };
    let relation = format!("{}.{}", builder.quote(&table.schema), builder.quote(&table.name));

    changes
        .iter()
        .map(|change| {
            let mut params = Vec::new();
            match change {
                RowChange::Update { key, values } => {
                    if values.is_empty() {
                        return Err(AppError::Invalid("An update must change at least one column.".into()));
                    }
                    let set = builder.assignments(values, &mut params, ", ")?;
                    let filter = builder.key_filter(key, &primary_key, &mut params)?;
                    Ok(Statement {
                        sql: format!("UPDATE {relation} SET {set} WHERE {filter}"),
                        params,
                        key: Some(describe_key(key)),
                    })
                }
                RowChange::Delete { key } => {
                    let filter = builder.key_filter(key, &primary_key, &mut params)?;
                    Ok(Statement {
                        sql: format!("DELETE FROM {relation} WHERE {filter}"),
                        params,
                        key: Some(describe_key(key)),
                    })
                }
                RowChange::Insert { values } if values.is_empty() => {
                    Ok(Statement { sql: format!("INSERT INTO {relation} {}", dialect.default_row), params, key: None })
                }
                RowChange::Insert { values } => {
                    let names: Vec<String> = values.keys().map(|name| builder.quote(name)).collect();
                    let markers = values
                        .iter()
                        .map(|(name, value)| builder.value(name, value, &mut params))
                        .collect::<Result<Vec<_>>>()?;
                    Ok(Statement {
                        sql: format!("INSERT INTO {relation} ({}) VALUES ({})", names.join(", "), markers.join(", ")),
                        params,
                        key: None,
                    })
                }
            }
        })
        .collect()
}

struct Builder<'a> {
    dialect: &'a Dialect,
    types: BTreeMap<&'a str, &'a str>,
}

impl Builder<'_> {
    fn quote(&self, name: &str) -> String {
        let q = self.dialect.quote;
        format!("{q}{}{q}", name.replace(q, &format!("{q}{q}")))
    }

    /// `"name" = <value>, …`
    fn assignments(
        &self,
        values: &BTreeMap<String, Option<String>>,
        params: &mut Vec<Option<String>>,
        separator: &str,
    ) -> Result<String> {
        let parts = values
            .iter()
            .map(|(name, value)| Ok(format!("{} = {}", self.quote(name), self.value(name, value, params)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(parts.join(separator))
    }

    /// The WHERE clause that pins one row: every primary key column, and only those.
    fn key_filter(
        &self,
        key: &BTreeMap<String, Option<String>>,
        primary_key: &BTreeSet<&str>,
        params: &mut Vec<Option<String>>,
    ) -> Result<String> {
        let given: BTreeSet<&str> = key.keys().map(String::as_str).collect();
        if &given != primary_key || key.values().any(Option::is_none) {
            return Err(AppError::Invalid("A row must be identified by its full primary key.".into()));
        }
        self.assignments(key, params, " AND ")
    }

    /// Adds the value as a parameter of a known column.
    fn value(&self, name: &str, value: &Option<String>, params: &mut Vec<Option<String>>) -> Result<String> {
        let data_type = self.types.get(name).ok_or_else(|| AppError::Invalid(format!("Unknown column: {name}")))?;
        params.push(value.clone());
        let marker = if self.dialect.numbered { format!("${}", params.len()) } else { "?".to_string() };
        Ok((self.dialect.value)(&marker, data_type))
    }
}

fn describe_key(key: &BTreeMap<String, Option<String>>) -> String {
    let parts: Vec<String> =
        key.iter().map(|(name, value)| format!("{name} = {}", value.as_deref().unwrap_or("NULL"))).collect();
    parts.join(", ")
}

/// Writes the parameters into the SQL as literals. Only for showing the user.
/// One pass left to right, so text already written in (a value holding `$1` or
/// `?`) is never read again, and quoted names like `"t$1"` are copied as they are.
pub fn inline(dialect: &Dialect, statement: &Statement) -> String {
    let literal = |value: &Option<String>| match value {
        None => "NULL".to_string(),
        Some(text) => {
            let text = if dialect.backslash_escapes { text.replace('\\', "\\\\") } else { text.clone() };
            format!("'{}'", text.replace('\'', "''"))
        }
    };

    let mut out = String::with_capacity(statement.sql.len());
    let mut chars = statement.sql.chars().peekable();
    let mut in_name = false;
    let mut next = 0;
    while let Some(c) = chars.next() {
        if c == dialect.quote {
            in_name = !in_name;
        }
        let index = match c {
            _ if in_name => None,
            '$' if dialect.numbered && chars.peek().is_some_and(char::is_ascii_digit) => {
                let mut number = String::new();
                while let Some(digit) = chars.next_if(char::is_ascii_digit) {
                    number.push(digit);
                }
                number.parse::<usize>().ok().map(|n| n.wrapping_sub(1))
            }
            '?' if !dialect.numbered => {
                next += 1;
                Some(next - 1)
            }
            _ => None,
        };
        match index.and_then(|i| statement.params.get(i)) {
            Some(value) => out.push_str(&literal(value)),
            None => out.push(c),
        }
    }
    out
}

pub fn summarize(changes: &[RowChange]) -> ApplySummary {
    let count = |f: fn(&RowChange) -> bool| changes.iter().filter(|c| f(c)).count() as u64;
    ApplySummary {
        updated: count(|c| matches!(c, RowChange::Update { .. })),
        inserted: count(|c| matches!(c, RowChange::Insert { .. })),
        deleted: count(|c| matches!(c, RowChange::Delete { .. })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOLLAR: Dialect = Dialect {
        quote: '"',
        numbered: true,
        value: |marker, _| marker.to_string(),
        default_row: "DEFAULT VALUES",
        backslash_escapes: false,
    };
    const QUESTION: Dialect = Dialect {
        quote: '`',
        numbered: false,
        value: |marker, _| marker.to_string(),
        default_row: "() VALUES ()",
        backslash_escapes: true,
    };

    fn statement(sql: &str, params: &[Option<&str>]) -> Statement {
        Statement { sql: sql.into(), params: params.iter().map(|p| p.map(String::from)).collect(), key: None }
    }

    #[test]
    fn inlining_leaves_markers_in_values_and_names_alone() {
        let numbered = statement(r#"UPDATE "t$1" SET "a" = $1, "b" = $2"#, &[Some("x"), Some("$1")]);
        assert_eq!(inline(&DOLLAR, &numbered), r#"UPDATE "t$1" SET "a" = 'x', "b" = '$1'"#);

        let positional = statement("UPDATE `t?` SET `a` = ?, `b` = ?", &[Some("?"), None]);
        assert_eq!(inline(&QUESTION, &positional), "UPDATE `t?` SET `a` = '?', `b` = NULL");
    }

    #[test]
    fn inlining_handles_ten_or_more_parameters() {
        let sql = (1..=10).map(|i| format!("${i}")).collect::<Vec<_>>().join(",");
        let params: Vec<Option<&str>> = vec![Some("1"); 9].into_iter().chain([Some("it's")]).collect();
        assert_eq!(inline(&DOLLAR, &statement(&sql, &params)), "'1','1','1','1','1','1','1','1','1','it''s'");
    }

    #[test]
    fn mysql_literals_escape_backslashes() {
        assert_eq!(inline(&QUESTION, &statement("?", &[Some(r"a\b")])), r"'a\\b'");
    }
}
