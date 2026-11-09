//! Builds the statements behind row editing, the same way for every SQL engine:
//! only the quoting, the parameter markers and how a parameter takes a column's
//! type differ, and those come from a [`Dialect`].
//!
//! Guarantees kept here for all engines: rows are identified by their full
//! primary key, tables without one are refused, values are parameters (never
//! spliced into the SQL), and only known columns are touched.

use std::collections::{BTreeMap, BTreeSet};

use super::{ApplySummary, CellValue, Column, RowChange, TableRef};
use crate::error::{AppError, Result};

pub struct Dialect {
    /// Opens and closes a quoted identifier: `"` for PostgreSQL, `[` `]` for SQL Server.
    pub quote: [char; 2],
    /// The prefix of numbered parameters (`$` makes `$1`, `$2`…); `None` uses `?`.
    pub numbered: Option<&'static str>,
    /// Turns a parameter marker into a value of a column's type.
    pub value: fn(marker: &str, data_type: &str) -> String,
    /// What follows `INSERT INTO t` for a row that only takes defaults.
    pub default_row: &'static str,
    /// Whether a backslash escapes in string literals (MySQL's default).
    pub backslash_escapes: bool,
    /// Written before a string literal, like SQL Server's `N` for Unicode text.
    pub string_prefix: &'static str,
    /// How a column takes its DEFAULT, given its default expression: the `DEFAULT`
    /// keyword, or the expression itself where `SET c = DEFAULT` doesn't exist (SQLite).
    pub default_value: fn(expression: Option<&str>) -> String,
}

/// The `DEFAULT` keyword, for engines that accept it in UPDATE and INSERT.
pub fn default_keyword(_: Option<&str>) -> String {
    "DEFAULT".to_string()
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
    let known: BTreeMap<&str, &Column> = columns.iter().map(|c| (c.name.as_str(), c)).collect();
    let primary_key: BTreeSet<&str> = columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
    if primary_key.is_empty() {
        return Err(AppError::Invalid(format!(
            "{}.{} has no primary key, so its rows can't be identified safely for editing.",
            table.schema, table.name
        )));
    }
    // Defense in depth: the grid never offers these, but a value for a computed
    // column is an error, or worse, silently ignored on a lenient server.
    let computed: BTreeSet<&str> = columns.iter().filter(|c| c.generated).map(|c| c.name.as_str()).collect();
    for change in changes {
        let values = match change {
            RowChange::Update { values, .. } | RowChange::Insert { values } => values,
            RowChange::Delete { .. } => continue,
            RowChange::Replace { .. } | RowChange::InsertDocument { .. } => return Err(not_documents()),
        };
        if let Some(name) = values.keys().find(|name| computed.contains(name.as_str())) {
            return Err(AppError::Invalid(format!("{name} is computed by the database and can't be set.")));
        }
    }
    let builder = Builder { dialect, columns: known };
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
                RowChange::Replace { .. } | RowChange::InsertDocument { .. } => Err(not_documents()),
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
    columns: BTreeMap<&'a str, &'a Column>,
}

impl Builder<'_> {
    fn quote(&self, name: &str) -> String {
        let [open, close] = self.dialect.quote;
        format!("{open}{}{close}", name.replace(close, &format!("{close}{close}")))
    }

    /// `"name" = <value>, …`
    fn assignments(
        &self,
        values: &BTreeMap<String, CellValue>,
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
        let key: BTreeMap<String, CellValue> =
            key.iter().map(|(name, value)| (name.clone(), value.clone().into())).collect();
        self.assignments(&key, params, " AND ")
    }

    /// Adds the value as a parameter of a known column; DEFAULT is written out instead.
    fn value(&self, name: &str, value: &CellValue, params: &mut Vec<Option<String>>) -> Result<String> {
        let column = self.columns.get(name).ok_or_else(|| AppError::Invalid(format!("Unknown column: {name}")))?;
        params.push(match value {
            CellValue::Text(text) => Some(text.clone()),
            CellValue::Null => None,
            CellValue::Default => return Ok((self.dialect.default_value)(column.default.as_deref())),
        });
        let data_type = column.data_type.as_str();
        let marker = match self.dialect.numbered {
            Some(prefix) => format!("{prefix}{}", params.len()),
            None => "?".to_string(),
        };
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
            format!("{}'{}'", dialect.string_prefix, text.replace('\'', "''"))
        }
    };
    let [open, close] = dialect.quote;

    let mut out = String::with_capacity(statement.sql.len());
    let mut rest = statement.sql.as_str();
    let mut in_name = false;
    let mut next = 0;
    while let Some(c) = rest.chars().next() {
        let width = c.len_utf8();
        if in_name || c == open {
            // A doubled closing quote is part of the name.
            if in_name && c == close && rest[width..].starts_with(close) {
                out.push_str(&rest[..2 * width]);
                rest = &rest[2 * width..];
                continue;
            }
            in_name = if in_name { c != close } else { true };
            out.push(c);
            rest = &rest[width..];
            continue;
        }
        // The marker's length and the index of its parameter.
        let marker = match dialect.numbered {
            Some(prefix) => rest.strip_prefix(prefix).and_then(|after| {
                let digits = after.len() - after.trim_start_matches(|d: char| d.is_ascii_digit()).len();
                let number: usize = after[..digits].parse().ok()?;
                Some((prefix.len() + digits, number.checked_sub(1)?))
            }),
            None if c == '?' => {
                next += 1;
                Some((1, next - 1))
            }
            None => None,
        };
        match marker.and_then(|(length, index)| Some((length, statement.params.get(index)?))) {
            Some((length, value)) => {
                out.push_str(&literal(value));
                rest = &rest[length..];
            }
            None => {
                out.push(c);
                rest = &rest[width..];
            }
        }
    }
    out
}

/// Whole documents are for document databases; SQL rows change cell by cell.
fn not_documents() -> AppError {
    AppError::Invalid("Rows of a SQL table are edited cell by cell, not as whole documents.".into())
}

pub fn summarize(changes: &[RowChange]) -> ApplySummary {
    let count = |f: fn(&RowChange) -> bool| changes.iter().filter(|c| f(c)).count() as u64;
    ApplySummary {
        updated: count(|c| matches!(c, RowChange::Update { .. } | RowChange::Replace { .. })),
        inserted: count(|c| matches!(c, RowChange::Insert { .. } | RowChange::InsertDocument { .. })),
        deleted: count(|c| matches!(c, RowChange::Delete { .. })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOLLAR: Dialect = Dialect {
        quote: ['"', '"'],
        numbered: Some("$"),
        value: |marker, _| marker.to_string(),
        default_row: "DEFAULT VALUES",
        backslash_escapes: false,
        string_prefix: "",
        default_value: default_keyword,
    };
    const QUESTION: Dialect = Dialect {
        quote: ['`', '`'],
        numbered: None,
        value: |marker, _| marker.to_string(),
        default_row: "() VALUES ()",
        backslash_escapes: true,
        string_prefix: "",
        default_value: default_keyword,
    };
    const BRACKETS: Dialect = Dialect {
        quote: ['[', ']'],
        numbered: Some("@P"),
        value: |marker, _| marker.to_string(),
        default_row: "DEFAULT VALUES",
        backslash_escapes: false,
        string_prefix: "N",
        default_value: default_keyword,
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
    fn inlining_reads_bracketed_names_and_prefixed_markers() {
        let sql = "UPDATE [t]]@P1] SET [a] = @P1, [b] = @P2 WHERE [id] = @P3";
        let bracketed = statement(sql, &[Some("café"), Some("@P1"), None]);
        assert_eq!(inline(&BRACKETS, &bracketed), "UPDATE [t]]@P1] SET [a] = N'café', [b] = N'@P1' WHERE [id] = NULL");
    }

    #[test]
    fn quoting_doubles_the_closing_character() {
        let column =
            Column { name: "a]b".into(), data_type: "int".into(), primary_key: true, generated: false, default: None };
        let table = TableRef { database: "db".into(), schema: "dbo".into(), name: "t".into() };
        let key: BTreeMap<String, Option<String>> = [("a]b".to_string(), Some("1".to_string()))].into();
        let [delete] = build(&BRACKETS, &[column], &table, &[RowChange::Delete { key }]).unwrap().try_into().unwrap();
        assert_eq!(delete.sql, "DELETE FROM [dbo].[t] WHERE [a]]b] = @P1");
    }

    #[test]
    fn inlining_handles_ten_or_more_parameters() {
        let sql = (1..=10).map(|i| format!("${i}")).collect::<Vec<_>>().join(",");
        let params: Vec<Option<&str>> = vec![Some("1"); 9].into_iter().chain([Some("it's")]).collect();
        assert_eq!(inline(&DOLLAR, &statement(&sql, &params)), "'1','1','1','1','1','1','1','1','1','it''s'");
    }

    #[test]
    fn refuses_values_for_computed_columns() {
        let column = |name: &str, primary_key, generated| Column {
            name: name.into(),
            data_type: "int".into(),
            primary_key,
            generated,
            default: None,
        };
        let columns = [column("id", true, false), column("doubled", false, true)];
        let table = TableRef { database: "db".into(), schema: "s".into(), name: "t".into() };
        let values: BTreeMap<String, CellValue> = [("doubled".to_string(), CellValue::Text("2".into()))].into();

        let error = build(&QUESTION, &columns, &table, &[RowChange::Insert { values }]).unwrap_err();
        assert!(error.to_string().contains("doubled is computed"), "{error}");

        // Its key may still identify a row, and a delete never sets values.
        let key: BTreeMap<String, Option<String>> = [("id".to_string(), Some("1".to_string()))].into();
        assert!(build(&QUESTION, &columns, &table, &[RowChange::Delete { key }]).is_ok());
    }

    #[test]
    fn refuses_whole_documents() {
        let columns =
            [Column { name: "id".into(), data_type: "int".into(), primary_key: true, generated: false, default: None }];
        let table = TableRef { database: "db".into(), schema: "s".into(), name: "t".into() };
        let key: BTreeMap<String, Option<String>> = [("id".to_string(), Some("1".to_string()))].into();
        let replace = RowChange::Replace { key, document: "{}".into() };
        let error = build(&QUESTION, &columns, &table, &[replace]).unwrap_err();
        assert!(error.to_string().contains("edited cell by cell"), "{error}");
        let insert = RowChange::InsertDocument { document: "{}".into() };
        assert!(build(&QUESTION, &columns, &table, &[insert]).is_err());
    }

    #[test]
    fn default_is_written_out_never_sent_as_a_parameter() {
        let column = |name: &str, primary_key| Column {
            name: name.into(),
            data_type: "int".into(),
            primary_key,
            generated: false,
            default: Some("7".into()),
        };
        let columns = [column("id", true), column("qty", false)];
        let table = TableRef { database: "db".into(), schema: "s".into(), name: "t".into() };
        let key: BTreeMap<String, Option<String>> = [("id".to_string(), Some("1".to_string()))].into();
        let values: BTreeMap<String, CellValue> = [("qty".to_string(), CellValue::Default)].into();
        let changes = [RowChange::Update { key, values: values.clone() }, RowChange::Insert { values }];

        let [update, insert] = build(&DOLLAR, &columns, &table, &changes).unwrap().try_into().unwrap();
        assert_eq!(update.sql, r#"UPDATE "s"."t" SET "qty" = DEFAULT WHERE "id" = $1"#);
        assert_eq!(update.params, [Some("1".to_string())]);
        assert_eq!(insert.sql, r#"INSERT INTO "s"."t" ("qty") VALUES (DEFAULT)"#);

        // An engine without the keyword writes the column's expression.
        let written_out = Dialect { default_value: |e| format!("({})", e.unwrap()), ..DOLLAR };
        let [update, _] = build(&written_out, &columns, &table, &changes).unwrap().try_into().unwrap();
        assert_eq!(update.sql, r#"UPDATE "s"."t" SET "qty" = (7) WHERE "id" = $1"#);
    }

    #[test]
    fn a_cell_value_reads_text_null_or_default() {
        let read = |json: &str| serde_json::from_str::<CellValue>(json).unwrap();
        assert_eq!(read(r#""x""#), CellValue::Text("x".into()));
        assert_eq!(read("null"), CellValue::Null);
        assert_eq!(read(r#"{"kind":"default"}"#), CellValue::Default);
        // The text DEFAULT is a value like any other.
        assert_eq!(read(r#""DEFAULT""#), CellValue::Text("DEFAULT".into()));
    }

    #[test]
    fn mysql_literals_escape_backslashes() {
        assert_eq!(inline(&QUESTION, &statement("?", &[Some(r"a\b")])), r"'a\\b'");
    }
}
