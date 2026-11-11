//! Writes rows as CSV, TSV, JSON, SQL INSERT statements or a Markdown table.
//! Rows are written one at a time, so a whole table can stream to disk in
//! constant memory.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::connections::store::ConnectionStore;
use crate::drivers::{self, Column, RowFilter, RowSink, TableRef};
use crate::error::{AppError, Result};
use crate::files::write_finished;
use crate::jobs::{Every, Jobs, unless_cancelled};
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

/// How JSON lays out the rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JsonStyle {
    /// One array of objects.
    #[default]
    Array,
    /// One object per line (NDJSON), which tools read a row at a time.
    Lines,
}

/// What a format writes besides the values, and which columns. Each option
/// only matters to the formats its comment names; the others ignore it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportOptions {
    /// CSV and TSV: a first line with the column names.
    pub header: bool,
    /// CSV and TSV: what a NULL is written as; nothing by default.
    pub null_text: String,
    pub json_style: JsonStyle,
    /// SQL: how many rows each INSERT adds.
    pub rows_per_insert: u32,
    /// The columns to write, in this order; every column when absent.
    pub columns: Option<Vec<String>>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self { header: true, null_text: String::new(), json_style: JsonStyle::Array, rows_per_insert: 1, columns: None }
    }
}

/// The most rows one INSERT adds, so a statement stays a size servers accept.
const MAX_ROWS_PER_INSERT: u32 = 1000;

/// How a value is written where formats care about types (JSON, SQL).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Number,
    Boolean,
    Json,
    /// Bytes, which MySQL and SQL Server show as `0x` and hex digits.
    Binary,
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
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" | "image" => Kind::Binary,
        _ => Kind::Text,
    }
}

/// `0x` and an even number of hex digits.
fn is_hex_literal(text: &str) -> bool {
    text.strip_prefix("0x").is_some_and(|digits| digits.len() % 2 == 0 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub struct RowWriter<W: Write> {
    out: W,
    format: Format,
    options: ExportOptions,
    /// Used by SQL INSERT; already quoted where needed, e.g. `public.orders`.
    table_name: String,
    /// Where each column written is in the rows that come in.
    pick: Vec<usize>,
    columns: Vec<Column>,
    kinds: Vec<Kind>,
    /// SQL: `INSERT INTO table (columns)`, and the rows waiting for their statement.
    insert: String,
    pending: Vec<String>,
    rows: u64,
}

impl<W: Write> RowWriter<W> {
    pub fn new(out: W, format: Format, table_name: &str, options: ExportOptions) -> Self {
        Self {
            out,
            format,
            options: ExportOptions {
                rows_per_insert: options.rows_per_insert.clamp(1, MAX_ROWS_PER_INSERT),
                ..options
            },
            table_name: table_name.to_string(),
            pick: Vec::new(),
            columns: Vec::new(),
            kinds: Vec::new(),
            insert: String::new(),
            pending: Vec::new(),
            rows: 0,
        }
    }

    /// Closes the output (the last INSERT, the JSON array) and returns how many rows were written.
    pub fn finish(mut self) -> Result<u64> {
        self.write_pending()?;
        if self.format == Format::Json && self.options.json_style == JsonStyle::Array {
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

    /// The columns written: those the options name, in their order, or all of them.
    fn picked(columns: &[Column], names: Option<&[String]>) -> Result<Vec<usize>> {
        let Some(names) = names else { return Ok((0..columns.len()).collect()) };
        if names.is_empty() {
            return Err(AppError::Invalid("Choose at least one column to export.".into()));
        }
        names
            .iter()
            .map(|name| {
                columns
                    .iter()
                    .position(|c| &c.name == name)
                    .ok_or_else(|| AppError::Invalid(format!("There is no column {name} to export.")))
            })
            .collect()
    }

    /// One INSERT with the rows waiting for it.
    fn write_pending(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        if self.options.rows_per_insert == 1 {
            writeln!(self.out, "{} VALUES {};", self.insert, self.pending.join(""))?;
        } else {
            writeln!(self.out, "{} VALUES\n  {};", self.insert, self.pending.join(",\n  "))?;
        }
        self.pending.clear();
        Ok(())
    }
}

impl<W: Write> RowSink for RowWriter<W> {
    fn columns(&mut self, columns: &[Column]) -> Result<()> {
        self.pick = Self::picked(columns, self.options.columns.as_deref())?;
        self.columns = self.pick.iter().map(|&i| columns[i].clone()).collect();
        self.kinds = self.columns.iter().map(|c| kind_of(&c.data_type)).collect();
        let names: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        match self.format {
            Format::Csv | Format::Tsv if !self.options.header => Ok(()),
            Format::Csv => self.separated(',', names.iter().map(|n| csv_field(n, ','))),
            Format::Tsv => self.separated('\t', names.iter().map(|n| csv_field(n, '\t'))),
            Format::Markdown => {
                let header: Vec<String> = names.iter().map(|n| markdown_cell(n)).collect();
                writeln!(self.out, "| {} |", header.join(" | "))?;
                writeln!(self.out, "|{}", " --- |".repeat(names.len()))?;
                Ok(())
            }
            Format::Sql => {
                let quoted: Vec<String> = names.iter().map(|n| quote_ident(n)).collect();
                self.insert = format!("INSERT INTO {} ({})", self.table_name, quoted.join(", "));
                Ok(())
            }
            Format::Json => Ok(()),
        }
    }

    fn row(&mut self, values: &[Option<String>]) -> Result<()> {
        let values: Vec<Option<&str>> = self.pick.iter().map(|&i| values.get(i).and_then(Option::as_deref)).collect();
        let null = self.options.null_text.clone();
        match self.format {
            Format::Csv => self.separated(',', values.iter().map(|v| csv_field(v.unwrap_or(&null), ','))),
            Format::Tsv => self.separated('\t', values.iter().map(|v| csv_field(v.unwrap_or(&null), '\t'))),
            Format::Markdown => {
                let cells: Vec<String> = values.iter().map(|v| v.map_or("NULL".to_string(), markdown_cell)).collect();
                writeln!(self.out, "| {} |", cells.join(" | "))?;
                Ok(())
            }
            Format::Json => {
                let object: serde_json::Map<String, serde_json::Value> = self
                    .columns
                    .iter()
                    .zip(&self.kinds)
                    .zip(&values)
                    .map(|((column, kind), value)| (column.name.clone(), json_value(*value, *kind)))
                    .collect();
                let object = serde_json::Value::Object(object);
                match self.options.json_style {
                    JsonStyle::Lines => writeln!(self.out, "{object}")?,
                    JsonStyle::Array => {
                        let prefix = if self.rows == 0 { "[\n  " } else { ",\n  " };
                        write!(self.out, "{prefix}{object}")?;
                    }
                }
                Ok(())
            }
            Format::Sql => {
                let literals: Vec<String> =
                    values.iter().zip(&self.kinds).map(|(value, kind)| sql_literal(*value, *kind)).collect();
                self.pending.push(format!("({})", literals.join(", ")));
                if self.pending.len() >= self.options.rows_per_insert as usize {
                    self.write_pending()?;
                }
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
        Kind::Binary | Kind::Text => None,
    };
    parsed.unwrap_or_else(|| Value::String(text.to_string()))
}

fn sql_literal(value: Option<&str>, kind: Kind) -> String {
    match (value, kind) {
        (None, _) => "NULL".to_string(),
        (Some(text), Kind::Number) if text.parse::<f64>().is_ok_and(f64::is_finite) => text.to_string(),
        (Some(text), Kind::Boolean) => if text == "t" || text == "true" { "TRUE" } else { "FALSE" }.to_string(),
        // A hex literal, as MySQL and SQL Server read it: quoted, it would be the text "0x…".
        (Some(text), Kind::Binary) if is_hex_literal(text) => text.to_string(),
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
    options: ExportOptions,
    columns: &[Column],
    rows: &[Vec<Option<String>>],
) -> Result<u64> {
    let mut writer = RowWriter::new(out, format, name, options);
    writer.columns(columns)?;
    for row in rows {
        writer.row(row)?;
    }
    writer.finish()
}

/// Formats rows already in the UI (a query result), e.g. for the clipboard.
#[tauri::command(async)]
pub fn format_rows(
    format: Format,
    options: Option<ExportOptions>,
    name: String,
    columns: Vec<Column>,
    rows: Vec<Vec<Option<String>>>,
) -> Result<String> {
    let mut out = Vec::new();
    write_rows(&mut out, format, &quote_ident(&name), options.unwrap_or_default(), &columns, &rows)?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Saves rows already in the UI (a query result) to `path`. A file already
/// there is replaced only once the new one is written whole.
#[tauri::command]
pub async fn export_rows(
    path: PathBuf,
    format: Format,
    options: Option<ExportOptions>,
    name: String,
    columns: Vec<Column>,
    rows: Vec<Vec<Option<String>>>,
) -> Result<u64> {
    let options = options.unwrap_or_default();
    write_finished(&path, async |partial| {
        write_rows(BufWriter::new(File::create(partial)?), format, &quote_ident(&name), options, &columns, &rows)
    })
    .await
}

/// How many rows an export has written so far, for its dialog.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    job_id: &'a str,
    rows: u64,
}

const PROGRESS: &str = "export-progress";
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// A sink that hands the rows on and reports how many have gone by.
struct Counted<S, F> {
    sink: S,
    rows: u64,
    report: F,
}

impl<S: RowSink, F: FnMut(u64)> RowSink for Counted<S, F> {
    fn columns(&mut self, columns: &[Column]) -> Result<()> {
        self.sink.columns(columns)
    }

    fn row(&mut self, values: &[Option<String>]) -> Result<()> {
        self.sink.row(values)?;
        self.rows += 1;
        (self.report)(self.rows);
        Ok(())
    }
}

/// Streams every row of a table or view to `path`, only those `filter` keeps
/// when given; returns how many were written. Job `job_id` reports its rows
/// as it goes and can be cancelled; a failed or cancelled export leaves a
/// file already at `path` as it was.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn export_table(
    app: AppHandle,
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    jobs: State<'_, Jobs>,
    job_id: String,
    connection_id: String,
    table: TableRef,
    filter: Option<RowFilter>,
    format: Format,
    options: Option<ExportOptions>,
    path: PathBuf,
) -> Result<u64> {
    let opened = store.open(&connection_id)?;
    let name = format!("{}.{}", quote_ident(&table.schema), quote_ident(&table.name));
    let filter = filter.unwrap_or_default();
    let job = jobs.start(&job_id);
    let mut every = Every::new(PROGRESS_EVERY);
    let report = |rows: u64| {
        if every.due() {
            let _ = app.emit(PROGRESS, Progress { job_id: &job_id, rows });
        }
    };
    write_finished(&path, async |partial| {
        let writer = RowWriter::new(BufWriter::new(File::create(partial)?), format, &name, options.unwrap_or_default());
        let mut sink = Counted { sink: writer, rows: 0, report };
        let stream = opened.run(&log, Origin::Export, async |connection, password| {
            drivers::stream_table(connection, password, &table, &filter, &mut sink).await
        });
        unless_cancelled(job.token(), stream).await?;
        sink.sink.finish()
    })
    .await
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
        let written = render_with(format, ExportOptions::default(), &mut out);
        assert_eq!(written, 2);
        String::from_utf8(out).unwrap()
    }

    fn render_with(format: Format, options: ExportOptions, out: &mut Vec<u8>) -> u64 {
        write_rows(out, format, "\"public\".\"people\"", options, &columns(), &rows()).unwrap()
    }

    fn rendered(format: Format, options: ExportOptions) -> String {
        let mut out = Vec::new();
        render_with(format, options, &mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn csv_can_leave_out_the_header_and_spell_null() {
        let options = ExportOptions { header: false, null_text: "NULL".into(), ..Default::default() };
        assert_eq!(
            rendered(Format::Csv, options.clone()),
            "1,\"Ann, \"\"A\"\"\",t,\"{\"\"a\"\": 1}\"\n2,NULL,f,NULL\n"
        );
        assert_eq!(rendered(Format::Tsv, options), "1\t\"Ann, \"\"A\"\"\"\tt\t\"{\"\"a\"\": 1}\"\n2\tNULL\tf\tNULL\n");
        // A NULL text that holds the separator is quoted like any value.
        let comma = ExportOptions { null_text: "a,b".into(), ..Default::default() };
        assert!(rendered(Format::Csv, comma).ends_with("2,\"a,b\",f,\"a,b\"\n"));
    }

    #[test]
    fn json_lines_writes_one_object_per_line() {
        let options = ExportOptions { json_style: JsonStyle::Lines, ..Default::default() };
        let text = rendered(Format::Json, options.clone());
        let lines: Vec<serde_json::Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(lines[1], serde_json::json!({ "id": 2, "Name": null, "active": false, "meta": null }));
        assert_eq!(lines.len(), 2);
        // No rows, no lines.
        let mut out = Vec::new();
        write_rows(&mut out, Format::Json, "t", options, &columns(), &[]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn sql_groups_rows_into_inserts_of_the_size_asked() {
        let three = [rows(), vec![vec![Some("3".into()), Some("Bo".into()), None, None]]].concat();
        let options = ExportOptions { rows_per_insert: 2, columns: Some(vec!["id".into()]), ..Default::default() };
        let mut out = Vec::new();
        write_rows(&mut out, Format::Sql, "t", options, &columns(), &three).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "INSERT INTO t (\"id\") VALUES\n  (1),\n  (2);\nINSERT INTO t (\"id\") VALUES\n  (3);\n"
        );
        // Past the most a statement takes, the size is kept to it.
        let big = RowWriter::new(
            Vec::new(),
            Format::Sql,
            "t",
            ExportOptions { rows_per_insert: 50_000, ..Default::default() },
        );
        assert_eq!(big.options.rows_per_insert, MAX_ROWS_PER_INSERT);
    }

    #[test]
    fn only_the_columns_chosen_are_written_in_their_order() {
        let options = ExportOptions { columns: Some(vec!["active".into(), "id".into()]), ..Default::default() };
        assert_eq!(rendered(Format::Csv, options), "active,id\nt,1\nf,2\n");
        let fail = |chosen: Vec<String>| {
            let options = ExportOptions { columns: Some(chosen), ..Default::default() };
            write_rows(Vec::new(), Format::Csv, "t", options, &columns(), &rows()).unwrap_err().to_string()
        };
        assert_eq!(fail(vec![]), "Choose at least one column to export.");
        assert_eq!(fail(vec!["nope".into()]), "There is no column nope to export.");
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
        write_rows(&mut out, Format::Json, "t", ExportOptions::default(), &columns(), &[]).unwrap();
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
        write_rows(
            &mut out,
            Format::Markdown,
            "t",
            ExportOptions::default(),
            &columns,
            &[vec![Some("a|b\nc".into())], vec![None]],
        )
        .unwrap();
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
        assert_eq!(kind_of("varbinary(16)"), Kind::Binary);
        assert_eq!(sql_literal(Some("0xACED0005"), Kind::Binary), "0xACED0005");
        assert_eq!(sql_literal(Some("0x"), Kind::Binary), "0x");
        // Anything else stays a string, never written in raw.
        assert_eq!(sql_literal(Some("0x1; DROP"), Kind::Binary), "'0x1; DROP'");
        assert_eq!(sql_literal(Some("bytes"), Kind::Binary), "'bytes'");
        assert_eq!(json_value(Some("0xFF"), Kind::Binary), serde_json::json!("0xFF"));
    }

    #[test]
    fn identifiers_are_always_quoted() {
        assert_eq!(quote_ident("order"), "\"order\"");
        assert_eq!(quote_ident("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
