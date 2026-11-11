//! Reads the rows of a CSV, TSV or JSON file one at a time, so a file of any
//! size is read in constant memory. It reads what `export` writes: a file
//! exported with some options imports back with the same ones.

use std::io::BufRead;

use serde::de::{DeserializeSeed, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportFormat {
    Csv,
    Tsv,
    /// An array of objects, or one object per line (NDJSON).
    Json,
}

/// How a CSV or TSV file is read; JSON ignores these.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReadOptions {
    /// The first line holds the column names.
    pub header: bool,
    /// An unquoted value equal to this is NULL; an empty one by default.
    pub null_text: String,
    /// CSV: what separates the values. TSV always uses a tab.
    pub delimiter: char,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self { header: true, null_text: String::new(), delimiter: ',' }
    }
}

/// One row: a value for each field, `None` being NULL.
pub type Row = Vec<Option<String>>;

/// Whether to go on reading after a row.
pub type Go = bool;

/// The file's fields, from its header (or `Column 1`, `Column 2`… without
/// one), or for JSON the keys of the objects read, in the order they first
/// appear.
pub fn fields_of(format: ImportFormat, options: &ReadOptions, input: impl BufRead, sample: usize) -> Result<Preview> {
    let mut rows = Vec::new();
    match format {
        ImportFormat::Json => {
            let mut fields: Vec<String> = Vec::new();
            let mut objects = Vec::new();
            json_objects(input, |object| {
                for key in object.keys() {
                    if !fields.contains(key) {
                        fields.push(key.clone());
                    }
                }
                objects.push(object);
                Ok(objects.len() < sample)
            })?;
            rows = objects.iter().map(|object| json_row(object, &fields)).collect();
            Ok(Preview { fields, rows })
        }
        ImportFormat::Csv | ImportFormat::Tsv => {
            let mut fields = None;
            separated(input, separator(format, options), |record| {
                if fields.is_none() {
                    if options.header {
                        fields = Some(record.into_iter().map(|field| field.text).collect());
                        return Ok(true);
                    }
                    fields = Some((1..=record.len()).map(|n| format!("Column {n}")).collect());
                }
                rows.push(row_of(record, &options.null_text));
                Ok(rows.len() < sample)
            })?;
            Ok(Preview { fields: fields.unwrap_or_default(), rows })
        }
    }
}

/// The fields of a file and its first rows.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Preview {
    pub fields: Vec<String>,
    pub rows: Vec<Row>,
}

/// Hands every row of the file to `on_row`, its values in the order of
/// `fields` (for JSON, the keys to read; for CSV and TSV, checked against the
/// header so a row of another width is an error), until it says to stop.
pub fn read_rows(
    format: ImportFormat,
    options: &ReadOptions,
    fields: &[String],
    input: impl BufRead,
    mut on_row: impl FnMut(Row) -> Result<Go>,
) -> Result<()> {
    match format {
        ImportFormat::Json => json_objects(input, |object| on_row(json_row(&object, fields))),
        ImportFormat::Csv | ImportFormat::Tsv => {
            let mut line = 0u64;
            separated(input, separator(format, options), |record| {
                line += 1;
                if line == 1 && options.header {
                    return Ok(true);
                }
                if record.len() != fields.len() {
                    let values =
                        if record.len() == 1 { "1 value".to_string() } else { format!("{} values", record.len()) };
                    return Err(AppError::Invalid(format!(
                        "Row {line} has {values}, but the file has {} columns.",
                        fields.len()
                    )));
                }
                on_row(row_of(record, &options.null_text))
            })
        }
    }
}

fn separator(format: ImportFormat, options: &ReadOptions) -> char {
    if format == ImportFormat::Tsv { '\t' } else { options.delimiter }
}

/// A value as written in the file, and whether it was in quotes: a quoted
/// value is never NULL, whatever its text.
struct Field {
    text: String,
    quoted: bool,
}

fn row_of(record: Vec<Field>, null_text: &str) -> Row {
    record.into_iter().map(|field| (field.quoted || field.text != null_text).then_some(field.text)).collect()
}

/// RFC 4180 records: a value in quotes may hold the separator, quotes (written
/// twice) and line breaks. Blank lines are skipped and a leading BOM dropped.
fn separated(input: impl BufRead, separator: char, mut on_record: impl FnMut(Vec<Field>) -> Result<Go>) -> Result<()> {
    let mut record: Vec<Field> = Vec::new();
    let mut field = Field { text: String::new(), quoted: false };
    let mut in_quotes = false;
    for (number, line) in input.lines().enumerate() {
        let line = line.map_err(|error| AppError::Invalid(format!("The file can't be read as text: {error}")))?;
        let line = if number == 0 { line.strip_prefix('\u{feff}').unwrap_or(&line).to_string() } else { line };
        if !in_quotes && line.is_empty() {
            continue;
        }
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if in_quotes {
                if c != '"' {
                    field.text.push(c);
                } else if chars.peek() == Some(&'"') {
                    field.text.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else if c == '"' && field.text.is_empty() && !field.quoted {
                in_quotes = true;
                field.quoted = true;
            } else if c == separator {
                record.push(std::mem::replace(&mut field, Field { text: String::new(), quoted: false }));
            } else {
                field.text.push(c);
            }
        }
        if in_quotes {
            field.text.push('\n');
            continue;
        }
        record.push(std::mem::replace(&mut field, Field { text: String::new(), quoted: false }));
        if !on_record(std::mem::take(&mut record))? {
            return Ok(());
        }
    }
    if in_quotes {
        return Err(AppError::Invalid("A quoted value is never closed before the file ends.".into()));
    }
    Ok(())
}

type Object = serde_json::Map<String, serde_json::Value>;

/// The values of `object` for `fields`: JSON null or a missing key is NULL,
/// text is itself, numbers and booleans their JSON text, and nested objects
/// and arrays their JSON.
fn json_row(object: &Object, fields: &[String]) -> Row {
    fields
        .iter()
        .map(|field| match object.get(field) {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(text)) => Some(text.clone()),
            Some(other) => Some(other.to_string()),
        })
        .collect()
}

/// Every object of a JSON array, or of one object per line, read one at a
/// time until `on_object` says to stop.
fn json_objects(input: impl BufRead, mut on_object: impl FnMut(Object) -> Result<Go>) -> Result<()> {
    let mut input = input;
    // Whether the file is an array is told by its first character.
    let array = loop {
        let buffer = input.fill_buf().map_err(|error| AppError::Invalid(format!("The file can't be read: {error}")))?;
        let Some(&first) = buffer.first() else { return Ok(()) };
        if first.is_ascii_whitespace() || buffer.starts_with("\u{feff}".as_bytes()) {
            let skip = if first.is_ascii_whitespace() { 1 } else { 3 };
            input.consume(skip);
            continue;
        }
        break first == b'[';
    };
    let mut stopped = false;
    let mut failure = None;
    let mut handle = |value: serde_json::Value| -> std::result::Result<Go, String> {
        let serde_json::Value::Object(object) = value else {
            return Err("Every value of a JSON file must be an object, one per row.".into());
        };
        match on_object(object) {
            Ok(go) => Ok(go),
            Err(error) => {
                failure = Some(error);
                Err(String::new())
            }
        }
    };
    let result = if array {
        let mut deserializer = serde_json::Deserializer::from_reader(input);
        Elements { handle: &mut handle, stopped: &mut stopped }.deserialize(&mut deserializer).map(|_| ())
    } else {
        let mut outcome = Ok(());
        for value in serde_json::Deserializer::from_reader(input).into_iter::<serde_json::Value>() {
            match value.map_err(|error| error.to_string()).and_then(&mut handle) {
                Ok(true) => {}
                Ok(false) => break,
                Err(message) => {
                    outcome = Err(message);
                    break;
                }
            }
        }
        return match (failure, outcome) {
            (Some(error), _) => Err(error),
            (None, Err(message)) => Err(AppError::Invalid(format!("The file isn't valid JSON: {message}"))),
            (None, Ok(())) => Ok(()),
        };
    };
    match (failure, result) {
        (Some(error), _) => Err(error),
        // Stopping early leaves the rest of the array unread, which serde reports.
        (None, Err(_)) if stopped => Ok(()),
        (None, Err(error)) => Err(AppError::Invalid(format!("The file isn't valid JSON: {error}"))),
        (None, Ok(())) => Ok(()),
    }
}

/// Reads a JSON array an element at a time, handing each to `handle`.
struct Elements<'a, F> {
    handle: &'a mut F,
    stopped: &'a mut bool,
}

impl<'de, F: FnMut(serde_json::Value) -> std::result::Result<Go, String>> DeserializeSeed<'de> for Elements<'_, F> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de, F: FnMut(serde_json::Value) -> std::result::Result<Go, String>> Visitor<'de> for Elements<'_, F> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an array of objects")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        while let Some(value) = seq.next_element::<serde_json::Value>()? {
            match (self.handle)(value) {
                Ok(true) => {}
                Ok(false) => {
                    *self.stopped = true;
                    return Err(serde::de::Error::custom("stopped"));
                }
                Err(message) => return Err(serde::de::Error::custom(message)),
            }
        }
        Ok(())
    }
}

/// Reads a whole small input, for tests and previews of short strings.
#[cfg(test)]
fn bytes(text: &str) -> impl BufRead + '_ {
    std::io::BufReader::new(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(format: ImportFormat, options: &ReadOptions, text: &str) -> Result<(Vec<String>, Vec<Row>)> {
        let preview = fields_of(format, options, bytes(text), usize::MAX)?;
        let mut read = Vec::new();
        read_rows(format, options, &preview.fields, bytes(text), |row| {
            read.push(row);
            Ok(true)
        })?;
        assert_eq!(read, preview.rows, "a preview shows the rows the import reads");
        Ok((preview.fields, read))
    }

    fn text(value: &str) -> Option<String> {
        Some(value.into())
    }

    #[test]
    fn csv_reads_what_export_writes() {
        // As `export` writes it: quotes only where needed, NULL left empty.
        let file = "\u{feff}id,Name,meta\n1,\"Ann, \"\"A\"\"\",\"{\"\"a\"\": 1}\"\n2,,\"two\nlines\"\n\n";
        let (fields, read) = rows(ImportFormat::Csv, &ReadOptions::default(), file).unwrap();
        assert_eq!(fields, ["id", "Name", "meta"]);
        assert_eq!(
            read,
            [vec![text("1"), text("Ann, \"A\""), text(r#"{"a": 1}"#)], vec![text("2"), None, text("two\nlines")]]
        );
    }

    #[test]
    fn a_quoted_value_is_never_null() {
        let options = ReadOptions { null_text: "NULL".into(), ..Default::default() };
        let (_, read) = rows(ImportFormat::Csv, &options, "a,b,c\nNULL,\"NULL\",\n").unwrap();
        assert_eq!(read, [vec![None, text("NULL"), text("")]]);
    }

    #[test]
    fn tsv_and_other_delimiters_and_no_header() {
        let (fields, read) = rows(ImportFormat::Tsv, &ReadOptions::default(), "a\tb\n1\t2,3\n").unwrap();
        assert_eq!((fields, read), (vec!["a".into(), "b".into()], vec![vec![text("1"), text("2,3")]]));
        let options = ReadOptions { header: false, delimiter: ';', ..Default::default() };
        let (fields, read) = rows(ImportFormat::Csv, &options, "1;x\n2;y\n").unwrap();
        assert_eq!(fields, ["Column 1", "Column 2"]);
        assert_eq!(read.len(), 2);
    }

    #[test]
    fn a_row_of_another_width_or_an_open_quote_is_an_error() {
        let wide = rows(ImportFormat::Csv, &ReadOptions::default(), "a,b\n1,2\n3,4,5\n").unwrap_err();
        assert_eq!(wide.to_string(), "Row 3 has 3 values, but the file has 2 columns.");
        let open = rows(ImportFormat::Csv, &ReadOptions::default(), "a\n\"never closed\n").unwrap_err();
        assert_eq!(open.to_string(), "A quoted value is never closed before the file ends.");
    }

    #[test]
    fn json_reads_arrays_and_lines_alike() {
        let array = r#" [ {"id": 1, "name": "Ann", "tags": ["a"]}, {"id": 2, "extra": true, "name": null} ] "#;
        let lines = "{\"id\": 1, \"name\": \"Ann\", \"tags\": [\"a\"]}\n{\"id\": 2, \"extra\": true, \"name\": null}\n";
        for file in [array, lines] {
            let (fields, read) = rows(ImportFormat::Json, &ReadOptions::default(), file).unwrap();
            assert_eq!(fields, ["id", "name", "tags", "extra"]);
            assert_eq!(
                read,
                [vec![text("1"), text("Ann"), text(r#"["a"]"#), None], vec![text("2"), None, None, text("true")]]
            );
        }
    }

    #[test]
    fn json_that_is_not_objects_is_an_error() {
        let error = rows(ImportFormat::Json, &ReadOptions::default(), "[1, 2]").unwrap_err();
        assert!(error.to_string().contains("must be an object"), "{error}");
        let broken = rows(ImportFormat::Json, &ReadOptions::default(), "[{\"a\": 1},").unwrap_err();
        assert!(broken.to_string().starts_with("The file isn't valid JSON"), "{broken}");
    }

    #[test]
    fn a_preview_stops_after_its_sample() {
        let file = "a\n1\n2\n3\n";
        let preview = fields_of(ImportFormat::Csv, &ReadOptions::default(), bytes(file), 2).unwrap();
        assert_eq!(preview.rows.len(), 2);
        let json = "[{\"a\":1},{\"a\":2},{\"a\":3}]";
        let preview = fields_of(ImportFormat::Json, &ReadOptions::default(), bytes(json), 2).unwrap();
        assert_eq!(preview.rows.len(), 2);
    }
}
