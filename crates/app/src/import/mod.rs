//! Importing a CSV, TSV or JSON file into a table, as the web UI's
//! `features/import`: the file's first rows, each field mapped to a column
//! (by name until the user picks), then its rows added. SQL engines take
//! them in one transaction; document databases a batch at a time. On a
//! connection whose tag asks first (PROD) the first press only warns, and
//! the second imports.

mod dialog;

use dbforge_core::connections::model::Connection;
use dbforge_core::drivers::TableRef;
pub use dbforge_core::import::ImportFormat;
use gpui::WeakEntity;

pub use dialog::{ImportDialog, ImportEvent, key_bindings};

use crate::table::TableView;

/// Where an import goes, and the rows view to read again once rows went in
/// (the table's tab it was opened from; none from the explorer, as in the web UI).
#[derive(Debug, Clone)]
pub struct ImportTarget {
    pub connection: Connection,
    pub table: TableRef,
    pub rows: Option<WeakEntity<TableView>>,
}

/// The format a file's name suggests; CSV when it suggests none.
pub fn format_of_path(path: &str) -> ImportFormat {
    let extension = path.rsplit_once('.').map(|(_, extension)| extension.to_lowercase());
    match extension.as_deref() {
        Some("tsv") => ImportFormat::Tsv,
        Some("json" | "ndjson" | "jsonl") => ImportFormat::Json,
        _ => ImportFormat::Csv,
    }
}

/// The column each field goes into: the one of the same name, or failing that
/// the same name ignoring case, or none. Exact names are matched first, so a
/// field named like a column always gets it; a column is filled from one field.
pub fn map_by_name(fields: &[String], columns: &[String]) -> Vec<Option<String>> {
    let exact: Vec<Option<String>> =
        fields.iter().map(|field| columns.contains(field).then(|| field.clone())).collect();
    let mut taken: Vec<String> = exact.iter().flatten().cloned().collect();
    exact
        .into_iter()
        .zip(fields)
        .map(|(column, field)| {
            column.or_else(|| {
                let loose = columns
                    .iter()
                    .find(|column| !taken.contains(column) && column.to_lowercase() == field.to_lowercase())?
                    .clone();
                taken.push(loose.clone());
                Some(loose)
            })
        })
        .collect()
}

/// Columns that more than one field goes into: the import would refuse them.
pub fn repeated_columns(mapping: &[Option<String>]) -> Vec<String> {
    let mut seen: Vec<&String> = Vec::new();
    let mut repeated: Vec<String> = Vec::new();
    for column in mapping.iter().flatten() {
        if seen.contains(&column) && !repeated.contains(column) {
            repeated.push(column.clone());
        }
        seen.push(column);
    }
    repeated
}

/// The columns a field can go into: the table's; a document also takes
/// fields the collection doesn't have yet, so those are offered too.
pub fn offered_columns(table_columns: &[String], fields: &[String], transactional: bool) -> Vec<String> {
    let mut columns = table_columns.to_vec();
    if !transactional {
        columns.extend(fields.iter().filter(|field| !table_columns.contains(field)).cloned());
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    fn mapping(list: &[Option<&str>]) -> Vec<Option<String>> {
        list.iter().map(|column| column.map(str::to_owned)).collect()
    }

    #[test]
    fn reads_the_format_from_the_extension() {
        assert_eq!(format_of_path("/a/rows.TSV"), ImportFormat::Tsv);
        assert_eq!(format_of_path("/a/rows.jsonl"), ImportFormat::Json);
        assert_eq!(format_of_path("/a/rows.json"), ImportFormat::Json);
        assert_eq!(format_of_path("/a/rows.txt"), ImportFormat::Csv);
        assert_eq!(format_of_path("/a/rows"), ImportFormat::Csv);
    }

    #[test]
    fn sends_each_field_to_the_column_of_its_name_exact_first_then_ignoring_case() {
        assert_eq!(
            map_by_name(&names(&["id", "NAME", "extra"]), &names(&["id", "name", "email"])),
            mapping(&[Some("id"), Some("name"), None])
        );
        assert_eq!(map_by_name(&names(&["Name", "name"]), &names(&["name"])), mapping(&[None, Some("name")]));
    }

    #[test]
    fn names_the_columns_two_fields_go_into() {
        assert_eq!(repeated_columns(&mapping(&[Some("a"), None, Some("b"), Some("a"), None])), names(&["a"]));
        assert!(repeated_columns(&mapping(&[Some("a"), Some("b")])).is_empty());
    }

    #[test]
    fn offers_a_document_the_file_s_new_fields_and_a_table_only_its_columns() {
        let table = names(&["_id", "name"]);
        let fields = names(&["name", "tags"]);
        assert_eq!(offered_columns(&table, &fields, true), names(&["_id", "name"]));
        assert_eq!(offered_columns(&table, &fields, false), names(&["_id", "name", "tags"]));
    }
}
