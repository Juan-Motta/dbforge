//! Writes rows as CSV, TSV, JSON, SQL INSERT statements or a Markdown table.
//! Rows are written one at a time, so a whole table can stream to disk in
//! constant memory.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use serde::Deserialize;
use tauri::State;

use crate::connections::store::ConnectionStore;
use crate::drivers::{self, Column, RowFilter, RowSink, TableRef};
use crate::error::Result;
use crate::statement_log::{Origin, StatementLog};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Csv,
    Tsv,
    Json,
    Sql,
    Markdown,
}

/// How a value is written where formats care about types (JSON, SQL).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Number,
    Boolean,
    Json,
    Text,
}

/// Values arrive as the server's text, so the declared type decides the kind.
/// Accepts both catalog names (`integer`, `numeric(10,2)`) and wire names (`int4`).
fn kind_of(data_type: &str) -> Kind {
    let base = data_type.split('(').next().unwrap_or("").trim();
    match base {
        "smallint" | "integer" | "bigint" | "real" | "double precision" | "numeric" | "decimal" | "int2" | "int4"
        | "int8" | "float4" | "float8" | "oid" => Kind::Number,
        "boolean" | "bool" => Kind::Boolean,
        "json" | "jsonb" => Kind::Json,
        _ => Kind::Text,
    }
}

pub struct RowWriter<W: Write> {
    out: W,
    format: Format,
    /// Used by SQL INSERT; already quoted where needed, e.g. `public.orders`.
    table_name: String,
    columns: Vec<Column>,
    kinds: Vec<Kind>,
    rows: u64,
}

impl<W: Write> RowWriter<W> {
    pub fn new(out: W, format: Format, table_name: &str) -> Self {
        Self { out, format, table_name: table_name.to_string(), columns: Vec::new(), kinds: Vec::new(), rows: 0 }
    }

    /// Closes the output (the JSON array) and returns how many rows were written.
    pub fn finish(mut self) -> Result<u64> {
        if self.format == Format::Json {
            writeln!(self.out, "{}", if self.rows == 0 { "[]" } else { "\n]" })?;
        }
        self.out.flush()?;
        Ok(self.rows)
    }

    fn separated(&mut self, separator: char, values: impl Iterator<Item = String>) -> Result<()> {
        let line: Vec<String> = values.collect();
        writeln!(self.out, "{}", line.join(&separator.to_string()))?;
        Ok(())
    }
}

impl<W: Write> RowSink for RowWriter<W> {
    fn columns(&mut self, columns: &[Column]) -> Result<()> {
        self.columns = columns.to_vec();
        self.kinds = columns.iter().map(|c| kind_of(&c.data_type)).collect();
        let names = columns.iter().map(|c| c.name.clone());
        match self.format {
            Format::Csv => self.separated(',', names.map(|n| csv_field(&n, ','))),
            Format::Tsv => self.separated('\t', names.map(|n| csv_field(&n, '\t'))),
            Format::Markdown => {
                let header: Vec<String> = names.map(|n| markdown_cell(&n)).collect();
                writeln!(self.out, "| {} |", header.join(" | "))?;
                writeln!(self.out, "|{}", " --- |".repeat(columns.len()))?;
                Ok(())
            }
            Format::Json | Format::Sql => Ok(()),
        }
    }

    fn row(&mut self, values: &[Option<String>]) -> Result<()> {
        match self.format {
            Format::Csv => self.separated(',', values.iter().map(|v| csv_field(v.as_deref().unwrap_or(""), ','))),
            Format::Tsv => self.separated('\t', values.iter().map(|v| csv_field(v.as_deref().unwrap_or(""), '\t'))),
            Format::Markdown => {
                let cells: Vec<String> =
                    values.iter().map(|v| v.as_deref().map_or("NULL".to_string(), markdown_cell)).collect();
                writeln!(self.out, "| {} |", cells.join(" | "))?;
                Ok(())
            }
            Format::Json => {
                let object: serde_json::Map<String, serde_json::Value> = self
                    .columns
                    .iter()
                    .zip(&self.kinds)
                    .zip(values)
                    .map(|((column, kind), value)| (column.name.clone(), json_value(value.as_deref(), *kind)))
                    .collect();
                let prefix = if self.rows == 0 { "[\n  " } else { ",\n  " };
                write!(self.out, "{prefix}{}", serde_json::Value::Object(object))?;
                Ok(())
            }
            Format::Sql => {
                let columns: Vec<String> = self.columns.iter().map(|c| quote_ident(&c.name)).collect();
                let literals: Vec<String> =
                    values.iter().zip(&self.kinds).map(|(value, kind)| sql_literal(value.as_deref(), *kind)).collect();
                writeln!(
                    self.out,
                    "INSERT INTO {} ({}) VALUES ({});",
                    self.table_name,
                    columns.join(", "),
                    literals.join(", ")
                )?;
                Ok(())
            }
        }?;
        self.rows += 1;
        Ok(())
    }
}

/// RFC 4180: quote a field when it holds the separator, a quote or a line break.
fn csv_field(value: &str, separator: char) -> String {
    if value.contains([separator, '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// A pipe would end the cell and a line break the row.
fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn json_value(value: Option<&str>, kind: Kind) -> serde_json::Value {
    use serde_json::Value;
    let Some(text) = value else { return Value::Null };
    let parsed = match kind {
        // NaN and Infinity aren't JSON numbers; they stay strings.
        Kind::Number => serde_json::from_str::<serde_json::Number>(text).ok().map(Value::Number),
        Kind::Boolean => Some(Value::Bool(text == "t" || text == "true")),
        Kind::Json => serde_json::from_str(text).ok(),
        Kind::Text => None,
    };
    parsed.unwrap_or_else(|| Value::String(text.to_string()))
}

fn sql_literal(value: Option<&str>, kind: Kind) -> String {
    match (value, kind) {
        (None, _) => "NULL".to_string(),
        (Some(text), Kind::Number) if text.parse::<f64>().is_ok_and(f64::is_finite) => text.to_string(),
        (Some(text), Kind::Boolean) => if text == "t" || text == "true" { "TRUE" } else { "FALSE" }.to_string(),
        (Some(text), _) => format!("'{}'", text.replace('\'', "''")),
    }
}

/// Always quoted: that is valid for any name, reserved words like `order` included.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn write_rows<W: Write>(
    out: W,
    format: Format,
    name: &str,
    columns: &[Column],
    rows: &[Vec<Option<String>>],
) -> Result<u64> {
    let mut writer = RowWriter::new(out, format, name);
    writer.columns(columns)?;
    for row in rows {
        writer.row(row)?;
    }
    writer.finish()
}

/// Formats rows already in the UI (a query result), e.g. for the clipboard.
#[tauri::command]
pub fn format_rows(
    format: Format,
    name: String,
    columns: Vec<Column>,
    rows: Vec<Vec<Option<String>>>,
) -> Result<String> {
    let mut out = Vec::new();
    write_rows(&mut out, format, &quote_ident(&name), &columns, &rows)?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Saves rows already in the UI (a query result) to `path`.
#[tauri::command]
pub fn export_rows(
    path: PathBuf,
    format: Format,
    name: String,
    columns: Vec<Column>,
    rows: Vec<Vec<Option<String>>>,
) -> Result<u64> {
    write_rows(BufWriter::new(File::create(path)?), format, &quote_ident(&name), &columns, &rows)
}

/// Streams every row of a table or view to `path`, only those `filter` keeps
/// when given; returns how many were written.
#[tauri::command]
pub async fn export_table(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    table: TableRef,
    filter: Option<RowFilter>,
    format: Format,
    path: PathBuf,
) -> Result<u64> {
    let opened = store.open(&connection_id)?;
    let name = format!("{}.{}", quote_ident(&table.schema), quote_ident(&table.name));
    let mut writer = RowWriter::new(BufWriter::new(File::create(path)?), format, &name);
    let filter = filter.unwrap_or_default();
    opened
        .run(&log, Origin::Export, async |connection, password| {
            drivers::stream_table(connection, password, &table, &filter, &mut writer).await
        })
        .await?;
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Vec<Column> {
        [("id", "integer"), ("Name", "text"), ("active", "boolean"), ("meta", "jsonb")]
            .map(|(name, data_type)| Column {
                name: name.into(),
                data_type: data_type.into(),
                primary_key: false,
                generated: false,
                default: None,
            })
            .to_vec()
    }

    fn rows() -> Vec<Vec<Option<String>>> {
        vec![
            vec![Some("1".into()), Some("Ann, \"A\"".into()), Some("t".into()), Some(r#"{"a": 1}"#.into())],
            vec![Some("2".into()), None, Some("f".into()), None],
        ]
    }

    fn render(format: Format) -> String {
        let mut out = Vec::new();
        let written = write_rows(&mut out, format, "\"public\".\"people\"", &columns(), &rows()).unwrap();
        assert_eq!(written, 2);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn csv_quotes_only_when_needed_and_leaves_null_empty() {
        assert_eq!(render(Format::Csv), "id,Name,active,meta\n1,\"Ann, \"\"A\"\"\",t,\"{\"\"a\"\": 1}\"\n2,,f,\n");
    }

    #[test]
    fn tsv_separates_with_tabs() {
        assert_eq!(
            render(Format::Tsv),
            "id\tName\tactive\tmeta\n1\t\"Ann, \"\"A\"\"\"\tt\t\"{\"\"a\"\": 1}\"\n2\t\tf\t\n"
        );
    }

    #[test]
    fn json_keeps_numbers_booleans_and_nested_json_typed() {
        let parsed: serde_json::Value = serde_json::from_str(&render(Format::Json)).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!([
                { "id": 1, "Name": "Ann, \"A\"", "active": true, "meta": { "a": 1 } },
                { "id": 2, "Name": null, "active": false, "meta": null }
            ])
        );
    }

    #[test]
    fn json_with_no_rows_is_an_empty_array() {
        let mut out = Vec::new();
        write_rows(&mut out, Format::Json, "t", &columns(), &[]).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "[]\n");
    }

    #[test]
    fn sql_writes_one_insert_per_row() {
        assert_eq!(
            render(Format::Sql),
            "INSERT INTO \"public\".\"people\" (\"id\", \"Name\", \"active\", \"meta\") \
             VALUES (1, 'Ann, \"A\"', TRUE, '{\"a\": 1}');\n\
             INSERT INTO \"public\".\"people\" (\"id\", \"Name\", \"active\", \"meta\") VALUES (2, NULL, FALSE, NULL);\n"
        );
    }

    #[test]
    fn markdown_escapes_pipes_and_line_breaks() {
        let mut out = Vec::new();
        let columns = vec![Column {
            name: "note".into(),
            data_type: "text".into(),
            primary_key: false,
            generated: false,
            default: None,
        }];
        write_rows(&mut out, Format::Markdown, "t", &columns, &[vec![Some("a|b\nc".into())], vec![None]]).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "| note |\n| --- |\n| a\\|b c |\n| NULL |\n");
    }

    #[test]
    fn kinds_follow_catalog_and_wire_type_names() {
        assert_eq!(kind_of("numeric(10,2)"), Kind::Number);
        assert_eq!(kind_of("int8"), Kind::Number);
        assert_eq!(kind_of("bool"), Kind::Boolean);
        assert_eq!(kind_of("character varying(80)"), Kind::Text);
    }

    #[test]
    fn special_numbers_stay_strings() {
        assert_eq!(json_value(Some("NaN"), Kind::Number), serde_json::json!("NaN"));
        assert_eq!(sql_literal(Some("NaN"), Kind::Number), "'NaN'");
    }

    #[test]
    fn identifiers_are_always_quoted() {
        assert_eq!(quote_ident("order"), "\"order\"");
        assert_eq!(quote_ident("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
