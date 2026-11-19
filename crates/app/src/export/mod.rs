//! Exporting rows, as the web UI's `features/export`: a whole table streamed
//! from the server (only the rows its filter keeps, when it has one), or
//! rows the window already holds (a query result), written as CSV, TSV,
//! JSON, SQL INSERT statements or a Markdown table, to a file or the
//! clipboard. The format and options of the last export are offered first
//! next time ([`keys::EXPORT`](crate::preferences::keys::EXPORT)).

mod dialog;

use std::sync::Arc;

use dbforge_core::drivers::{Column, RowFilter, TableRef};
pub use dbforge_core::export::{ExportOptions, Format, JsonStyle};
use serde_json::{Value, json};

pub use dialog::{ExportDialog, key_bindings};

use crate::i18n::ExportMessages;

/// What is exported: a whole table, streamed from the server, or rows the window already holds.
#[derive(Debug, Clone)]
pub enum ExportSource {
    Table {
        connection_id: String,
        table: TableRef,
        filter: RowFilter,
    },
    /// `name` names the file and is the table of SQL INSERTs.
    Rows {
        name: String,
        columns: Vec<Column>,
        rows: Arc<Vec<Vec<Option<String>>>>,
    },
}

/// A format as the dialog offers it; its description is in [`description`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatInfo {
    pub format: Format,
    /// As the preferences keep it.
    pub id: &'static str,
    pub label: &'static str,
    pub extension: &'static str,
    pub icon: &'static str,
}

/// The formats, in the order the dialog shows them.
pub const FORMATS: [FormatInfo; 5] = [
    FormatInfo { format: Format::Csv, id: "csv", label: "CSV", extension: "csv", icon: "table" },
    FormatInfo { format: Format::Tsv, id: "tsv", label: "TSV", extension: "tsv", icon: "column" },
    FormatInfo { format: Format::Json, id: "json", label: "JSON", extension: "json", icon: "code" },
    FormatInfo { format: Format::Sql, id: "sql", label: "SQL INSERT", extension: "sql", icon: "database" },
    FormatInfo { format: Format::Markdown, id: "markdown", label: "Markdown", extension: "md", icon: "edit" },
];

pub fn info(format: Format) -> &'static FormatInfo {
    FORMATS.iter().find(|info| info.format == format).expect("every format is listed")
}

/// What a format is for, under its name; JSON says how it is laid out.
pub fn description(format: Format, json_style: JsonStyle, m: &ExportMessages) -> &'static str {
    match format {
        Format::Csv => m.csv_description,
        Format::Tsv => m.tsv_description,
        Format::Json if json_style == JsonStyle::Lines => m.json_lines_description,
        Format::Json => m.json_description,
        Format::Sql => m.sql_description,
        Format::Markdown => m.markdown_description,
    }
}

/// `orders.csv`; runs of characters that file systems reject become `_`.
pub fn default_file_name(name: &str, format: Format) -> String {
    let mut safe = String::with_capacity(name.len());
    let mut in_run = false;
    for c in name.chars() {
        if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            if !in_run {
                safe.push('_');
            }
            in_run = true;
        } else {
            safe.push(c);
            in_run = false;
        }
    }
    let safe = safe.trim();
    format!("{}.{}", if safe.is_empty() { "export" } else { safe }, info(format).extension)
}

/// The sizes offered for SQL; the core caps it at the largest.
pub const ROWS_PER_INSERT: [u32; 4] = [1, 100, 500, 1000];

/// What a format writes besides the values; each option matters only to the formats its comment names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatOptions {
    /// CSV and TSV: a first line with the column names.
    pub header: bool,
    /// CSV and TSV: what a NULL is written as.
    pub null_text: String,
    pub json_style: JsonStyle,
    /// SQL: how many rows each INSERT adds.
    pub rows_per_insert: u32,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self { header: true, null_text: String::new(), json_style: JsonStyle::Array, rows_per_insert: 1 }
    }
}

/// The format and options of the last export, offered first next time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportChoice {
    pub format: Format,
    pub options: FormatOptions,
}

impl Default for ExportChoice {
    fn default() -> Self {
        Self { format: Format::Csv, options: FormatOptions::default() }
    }
}

/// What was kept, with anything missing or not understood back at its default.
pub fn read_choice(saved: Option<&str>) -> ExportChoice {
    let Some(value) = saved.map(serde_json::from_str::<Value>) else { return ExportChoice::default() };
    let Ok(value) = value else { return ExportChoice::default() };
    let defaults = FormatOptions::default();
    let options = value.get("options");
    let option = |key: &str| options.and_then(|options| options.get(key));
    let format = value
        .get("format")
        .and_then(Value::as_str)
        .and_then(|id| FORMATS.iter().find(|info| info.id == id))
        .map_or(Format::Csv, |info| info.format);
    let rows_per_insert = option("rowsPerInsert")
        .and_then(Value::as_f64)
        .and_then(|count| ROWS_PER_INSERT.into_iter().find(|offered| f64::from(*offered) == count))
        .unwrap_or(defaults.rows_per_insert);
    ExportChoice {
        format,
        options: FormatOptions {
            header: option("header").and_then(Value::as_bool).unwrap_or(defaults.header),
            null_text: option("nullText").and_then(Value::as_str).map_or(defaults.null_text, str::to_owned),
            json_style: if option("jsonStyle").and_then(Value::as_str) == Some("lines") {
                JsonStyle::Lines
            } else {
                JsonStyle::Array
            },
            rows_per_insert,
        },
    }
}

/// The choice as it is kept, in the web UI's shape.
pub fn write_choice(choice: &ExportChoice) -> String {
    let options = &choice.options;
    json!({
        "format": info(choice.format).id,
        "options": {
            "header": options.header,
            "nullText": options.null_text,
            "jsonStyle": if options.json_style == JsonStyle::Lines { "lines" } else { "array" },
            "rowsPerInsert": options.rows_per_insert,
        },
    })
    .to_string()
}

/// The options sent with an export: the format's, and the columns, in the
/// source's order, when only some are chosen.
pub fn export_options(options: &FormatOptions, chosen: &[String], all: &[String]) -> ExportOptions {
    let some = chosen.len() != all.len();
    ExportOptions {
        header: options.header,
        null_text: options.null_text.clone(),
        json_style: options.json_style,
        rows_per_insert: options.rows_per_insert,
        columns: some.then(|| all.iter().filter(|name| chosen.contains(name)).cloned().collect()),
    }
}

/// The source's columns: being read, known in order, or unknown (then every one is exported).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Columns {
    Loading,
    Known(Vec<String>),
    Unavailable,
}

impl Columns {
    pub fn known(&self) -> &[String] {
        match self {
            Self::Known(names) => names,
            Self::Loading | Self::Unavailable => &[],
        }
    }
}

/// The chosen columns once `name` is ticked (`on`) or not, in the source's order.
pub fn toggle_column(known: &[String], chosen: &[String], name: &str, on: bool) -> Vec<String> {
    if on {
        known.iter().filter(|column| *column == name || chosen.contains(column)).cloned().collect()
    } else {
        chosen.iter().filter(|column| *column != name).cloned().collect()
    }
}

/// The line that says which columns go in.
pub fn columns_summary(columns: &Columns, chosen: &[String], m: &ExportMessages) -> String {
    match columns {
        Columns::Loading => m.loading_columns.into(),
        Columns::Unavailable => m.columns_unavailable.into(),
        Columns::Known(_) if chosen.is_empty() => m.no_column_chosen.into(),
        Columns::Known(all) if chosen.len() == all.len() => (m.all_columns)(all.len()),
        Columns::Known(all) => (m.some_columns)(chosen.len(), all.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn names_the_file_after_the_source_with_the_format_s_extension() {
        assert_eq!(default_file_name("orders", Format::Markdown), "orders.md");
        assert_eq!(default_file_name("a/b:c", Format::Csv), "a_b_c.csv");
        assert_eq!(default_file_name("a//b", Format::Csv), "a_b.csv", "a run becomes one _");
        assert_eq!(default_file_name("  ", Format::Json), "export.json");
    }

    #[test]
    fn brings_back_the_last_format_and_options() {
        let kept = ExportChoice {
            format: Format::Sql,
            options: FormatOptions {
                header: false,
                null_text: "NULL".into(),
                json_style: JsonStyle::Lines,
                rows_per_insert: 500,
            },
        };
        assert_eq!(read_choice(Some(&write_choice(&kept))), kept);
        assert_eq!(
            write_choice(&ExportChoice::default()),
            r#"{"format":"csv","options":{"header":true,"nullText":"","jsonStyle":"array","rowsPerInsert":1}}"#,
            "the web UI's shape and key order"
        );
    }

    #[test]
    fn falls_back_to_csv_and_the_defaults_when_nothing_is_kept_or_it_is_not_understood() {
        let fallback = ExportChoice::default();
        assert_eq!(read_choice(None), fallback);
        assert_eq!(read_choice(Some("not json")), fallback);
        assert_eq!(read_choice(Some(r#"{"format":"xlsx","options":{"rowsPerInsert":7,"header":"yes"}}"#)), fallback);
        assert_eq!(read_choice(Some("null")), fallback);
    }

    #[test]
    fn sends_no_column_list_when_every_column_is_chosen_and_the_chosen_ones_in_the_source_s_order() {
        let all = names(&["id", "name", "email"]);
        let options = FormatOptions::default();
        assert_eq!(export_options(&options, &names(&["email", "id", "name"]), &all).columns, None);
        assert_eq!(export_options(&options, &names(&["email", "id"]), &all).columns, Some(names(&["id", "email"])));
    }

    #[test]
    fn ticks_a_column_back_into_its_place_and_unticks_it() {
        let known = names(&["id", "name", "email"]);
        assert_eq!(toggle_column(&known, &names(&["email"]), "id", true), names(&["id", "email"]));
        assert_eq!(toggle_column(&known, &names(&["id", "email"]), "id", false), names(&["email"]));
    }

    #[test]
    fn says_which_columns_go_in() {
        let m = &Language::En.messages().export;
        let all = Columns::Known(names(&["id", "name", "email"]));
        assert_eq!(columns_summary(&Columns::Loading, &[], m), "Loading the columns…");
        assert_eq!(columns_summary(&Columns::Unavailable, &[], m), "Every column is exported.");
        assert_eq!(columns_summary(&all, &[], m), "Choose at least one column.");
        assert_eq!(columns_summary(&all, all.known(), m), "All 3 columns");
        assert_eq!(columns_summary(&all, &names(&["id"]), m), "1 of 3 columns");
        let es = &Language::Es.messages().export;
        assert_eq!(columns_summary(&all, &names(&["id"]), es), "1 de 3 columnas");
    }

    #[test]
    fn describes_json_by_its_layout() {
        let m = &Language::En.messages().export;
        assert_eq!(description(Format::Json, JsonStyle::Array, m), "An array of objects");
        assert_eq!(description(Format::Json, JsonStyle::Lines, m), "One object per line");
        assert_eq!(description(Format::Csv, JsonStyle::Lines, m), "Spreadsheets and tools");
    }
}
