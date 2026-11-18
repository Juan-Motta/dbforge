//! Pending edits to a table, kept until the user applies or discards them.
//! Loaded rows are identified by their primary key, so edits survive a
//! reload of the page; new rows by a number of their own. Document
//! databases also rewrite a row's whole document, or add new ones.

use std::collections::BTreeMap;

use dbforge_core::drivers::{CellValue, Column, RowChange};

/// A row's primary key values, by column name; `None` is NULL.
pub type Key = BTreeMap<String, Option<String>>;

/// A row as the grid shows it: a loaded one by its index in the page, or a new one by its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RowRef {
    Loaded(usize),
    New(u64),
}

/// The changed cells of a loaded row.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub key: Key,
    pub values: BTreeMap<String, CellValue>,
}

/// A new row: only the columns the user set (the rest take their DEFAULT).
#[derive(Debug, Clone, PartialEq)]
pub struct NewRow {
    pub id: u64,
    pub values: BTreeMap<String, Option<String>>,
}

/// A loaded row's whole document, rewritten.
#[derive(Debug, Clone, PartialEq)]
pub struct Replacement {
    pub key: Key,
    pub document: String,
}

/// A new document, in the engine's own syntax.
#[derive(Debug, Clone, PartialEq)]
pub struct NewDocument {
    pub id: u64,
    pub document: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Edits {
    /// Changed cells of loaded rows, by row key.
    pub updates: BTreeMap<String, Update>,
    /// Rows marked for deletion, by row key.
    pub deletes: BTreeMap<String, Key>,
    /// New rows, newest first.
    pub inserts: Vec<NewRow>,
    /// Document databases: loaded rows whose whole document was rewritten, by row key.
    pub replacements: BTreeMap<String, Replacement>,
    /// Document databases: new documents, newest first.
    pub new_documents: Vec<NewDocument>,
}

/// How many rows the edits change, by kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub updated: usize,
    pub inserted: usize,
    pub deleted: usize,
}

impl Counts {
    pub fn total(self) -> usize {
        self.updated + self.inserted + self.deleted
    }
}

/// A cell as the grid shows it, its pending edit included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub value: Option<String>,
    /// Left to its DEFAULT: a new row's column not set, or a loaded one set to DEFAULT.
    pub unset: bool,
    /// Its row is marked for deletion.
    pub deleted: bool,
}

/// The primary key values of a row, by column name.
pub fn key_of(row: &[Option<String>], columns: &[Column]) -> Key {
    columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.primary_key)
        .map(|(index, column)| (column.name.clone(), row.get(index).cloned().flatten()))
        .collect()
}

/// A row's identity among the edits: its key, whatever order its columns come in.
pub fn row_key(key: &Key) -> String {
    serde_json::to_string(key).unwrap_or_default()
}

/// Each loaded row's [`row_key`], by its position: worked out once for a page.
pub fn row_keys(rows: &[Vec<Option<String>>], columns: &[Column]) -> Vec<String> {
    rows.iter().map(|row| row_key(&key_of(row, columns))).collect()
}

impl Edits {
    /// Sets a cell of a loaded row; setting it back to what was loaded drops the change.
    pub fn edit_cell(&mut self, key: &Key, column: &str, value: CellValue, original: Option<&str>) {
        let id = row_key(key);
        let mut values = self.updates.remove(&id).map(|update| update.values).unwrap_or_default();
        if value == CellValue::from(original.map(str::to_owned)) {
            values.remove(column);
        } else {
            values.insert(column.to_owned(), value);
        }
        if !values.is_empty() {
            self.updates.insert(id, Update { key: key.clone(), values });
        }
    }

    /// Marks a loaded row for deletion, or unmarks it. Its cell edits and
    /// rewritten document are dropped either way.
    pub fn toggle_delete(&mut self, key: &Key) {
        let id = row_key(key);
        if self.deletes.remove(&id).is_none() {
            self.deletes.insert(id.clone(), key.clone());
        }
        self.updates.remove(&id);
        self.replacements.remove(&id);
    }

    /// Rewrites a loaded row's whole document; writing back what was loaded drops the change.
    pub fn replace_document(&mut self, key: &Key, document: &str, original: &str) {
        let id = row_key(key);
        if document == original {
            self.replacements.remove(&id);
        } else {
            self.replacements.insert(id, Replacement { key: key.clone(), document: document.to_owned() });
        }
    }

    /// Starts a new document with `document` as its text. Its id.
    pub fn add_document(&mut self, document: &str) -> u64 {
        let id = self.new_documents.iter().map(|new| new.id).max().unwrap_or(0) + 1;
        self.new_documents.insert(0, NewDocument { id, document: document.to_owned() });
        id
    }

    pub fn edit_new_document(&mut self, id: u64, document: &str) {
        if let Some(new) = self.new_documents.iter_mut().find(|new| new.id == id) {
            new.document = document.to_owned();
        }
    }

    pub fn remove_new_document(&mut self, id: u64) {
        self.new_documents.retain(|new| new.id != id);
    }

    /// The new document `id`'s text, if it is still there.
    pub fn new_document(&self, id: u64) -> Option<&str> {
        self.new_documents.iter().find(|new| new.id == id).map(|new| new.document.as_str())
    }

    pub fn is_deleted(&self, id: &str) -> bool {
        self.deletes.contains_key(id)
    }

    pub fn add_row(&mut self) -> u64 {
        self.add_row_with(BTreeMap::new())
    }

    /// A new row holding `values`, e.g. a loaded row's, to duplicate it; the
    /// other columns take their DEFAULT. Its id.
    pub fn add_row_with(&mut self, values: BTreeMap<String, Option<String>>) -> u64 {
        let id = self.inserts.iter().map(|row| row.id).max().unwrap_or(0) + 1;
        self.inserts.insert(0, NewRow { id, values });
        id
    }

    pub fn edit_new_cell(&mut self, id: u64, column: &str, value: Option<String>) {
        if let Some(row) = self.inserts.iter_mut().find(|row| row.id == id) {
            row.values.insert(column.to_owned(), value);
        }
    }

    /// Drops a column from a new row, so it takes its DEFAULT again.
    pub fn unset_new_cell(&mut self, id: u64, column: &str) {
        if let Some(row) = self.inserts.iter_mut().find(|row| row.id == id) {
            row.values.remove(column);
        }
    }

    pub fn remove_new_row(&mut self, id: u64) {
        self.inserts.retain(|row| row.id != id);
    }

    pub fn counts(&self) -> Counts {
        Counts {
            updated: self.updates.len() + self.replacements.len(),
            inserted: self.inserts.len() + self.new_documents.len(),
            deleted: self.deletes.len(),
        }
    }

    pub fn has_edits(&self) -> bool {
        self.counts().total() > 0
    }

    /// The changes in the order they run: deletes first so a new row can
    /// reuse a deleted row's unique value, then updates and rewritten
    /// documents, then new rows and new documents, oldest first.
    pub fn to_changes(&self) -> Vec<RowChange> {
        let deletes = self.deletes.values().map(|key| RowChange::Delete { key: key.clone() });
        let updates = self
            .updates
            .values()
            .map(|update| RowChange::Update { key: update.key.clone(), values: update.values.clone() });
        let replacements = self.replacements.values().map(|replacement| RowChange::Replace {
            key: replacement.key.clone(),
            document: replacement.document.clone(),
        });
        let inserts = self.inserts.iter().rev().map(|row| RowChange::Insert {
            values: row.values.iter().map(|(column, value)| (column.clone(), value.clone().into())).collect(),
        });
        let new_documents =
            self.new_documents.iter().rev().map(|new| RowChange::InsertDocument { document: new.document.clone() });
        deletes.chain(updates).chain(replacements).chain(inserts).chain(new_documents).collect()
    }

    /// The cell at `row` and `column` as the grid shows it; `None` when it is no longer there.
    pub fn cell_at(
        &self,
        rows: &[Vec<Option<String>>],
        columns: &[Column],
        row: RowRef,
        column: usize,
    ) -> Option<Shown> {
        let name = &columns.get(column)?.name;
        match row {
            RowRef::New(id) => {
                let row = self.inserts.iter().find(|row| row.id == id)?;
                let value = row.values.get(name);
                Some(Shown { value: value.cloned().flatten(), unset: value.is_none(), deleted: false })
            }
            RowRef::Loaded(index) => {
                let loaded = rows.get(index)?;
                let id = row_key(&key_of(loaded, columns));
                let deleted = self.deletes.contains_key(&id);
                let changed = self.updates.get(&id).and_then(|update| update.values.get(name));
                Some(match changed {
                    Some(CellValue::Default) => Shown { value: None, unset: true, deleted },
                    Some(CellValue::Null) => Shown { value: None, unset: false, deleted },
                    Some(CellValue::Text(text)) => Shown { value: Some(text.clone()), unset: false, deleted },
                    None => Shown { value: loaded.get(column).cloned().flatten(), unset: false, deleted },
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, primary_key: bool) -> Column {
        Column { name: name.into(), data_type: "text".into(), primary_key, generated: false, default: None }
    }

    fn columns() -> Vec<Column> {
        vec![column("id", true), column("name", false), column("note", false)]
    }

    fn key(id: &str) -> Key {
        Key::from([("id".to_owned(), Some(id.to_owned()))])
    }

    fn text(value: &str) -> CellValue {
        CellValue::Text(value.into())
    }

    fn row(values: &[Option<&str>]) -> Vec<Option<String>> {
        values.iter().map(|value| value.map(str::to_owned)).collect()
    }

    #[test]
    fn takes_the_primary_key_columns_of_a_row_in_any_order() {
        assert_eq!(key_of(&row(&[Some("1"), Some("Ann"), None]), &columns()), key("1"));
        let ab = Key::from([("b".to_owned(), Some("2".to_owned())), ("a".to_owned(), Some("1".to_owned()))]);
        let ba = Key::from([("a".to_owned(), Some("1".to_owned())), ("b".to_owned(), Some("2".to_owned()))]);
        assert_eq!(row_key(&ab), row_key(&ba));
    }

    #[test]
    fn records_a_changed_cell_null_included() {
        let mut edits = Edits::default();
        edits.edit_cell(&key("1"), "name", text("Bea"), Some("Ann"));
        edits.edit_cell(&key("1"), "note", CellValue::Null, Some("hi"));
        let values = BTreeMap::from([("name".to_owned(), text("Bea")), ("note".to_owned(), CellValue::Null)]);
        assert_eq!(edits.to_changes(), [RowChange::Update { key: key("1"), values }]);
    }

    #[test]
    fn drops_a_change_that_goes_back_to_the_loaded_value() {
        let mut edits = Edits::default();
        edits.edit_cell(&key("1"), "name", text("Bea"), Some("Ann"));
        edits.edit_cell(&key("1"), "name", text("Ann"), Some("Ann"));
        assert!(!edits.has_edits());
        edits.edit_cell(&key("1"), "note", CellValue::Null, None);
        assert!(!edits.has_edits(), "NULL over NULL is no change");
    }

    #[test]
    fn toggles_deletes_and_drops_the_row_s_cell_edits() {
        let mut edits = Edits::default();
        edits.edit_cell(&key("1"), "name", text("Bea"), Some("Ann"));
        edits.toggle_delete(&key("1"));
        assert_eq!(edits.counts(), Counts { updated: 0, inserted: 0, deleted: 1 });
        edits.toggle_delete(&key("1"));
        assert!(!edits.has_edits());
    }

    #[test]
    fn adds_rows_on_top_and_keeps_only_the_columns_that_were_set() {
        let mut edits = Edits::default();
        edits.add_row();
        edits.add_row();
        assert_eq!(edits.inserts.iter().map(|row| row.id).collect::<Vec<_>>(), [2, 1]);
        edits.edit_new_cell(1, "name", Some("Cy".into()));
        assert_eq!(edits.inserts[1].values, BTreeMap::from([("name".to_owned(), Some("Cy".to_owned()))]));
        edits.remove_new_row(2);
        assert_eq!(edits.inserts.iter().map(|row| row.id).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn can_put_a_column_of_a_new_row_back_to_its_default() {
        let mut edits = Edits::default();
        let id = edits.add_row();
        edits.edit_new_cell(id, "name", Some("Cy".into()));
        edits.unset_new_cell(id, "name");
        assert_eq!(edits.to_changes(), [RowChange::Insert { values: BTreeMap::new() }]);
    }

    #[test]
    fn runs_deletes_then_updates_then_inserts_oldest_first() {
        let mut edits = Edits::default();
        let first = edits.add_row();
        edits.edit_new_cell(first, "name", Some("first".into()));
        let second = edits.add_row();
        edits.edit_new_cell(second, "name", Some("second".into()));
        edits.edit_cell(&key("2"), "name", text("x"), Some("y"));
        edits.toggle_delete(&key("3"));
        let changes = edits.to_changes();
        assert!(matches!(changes[0], RowChange::Delete { .. }));
        assert!(matches!(changes[1], RowChange::Update { .. }));
        assert_eq!(
            changes[2..],
            [
                RowChange::Insert { values: BTreeMap::from([("name".to_owned(), text("first"))]) },
                RowChange::Insert { values: BTreeMap::from([("name".to_owned(), text("second"))]) },
            ]
        );
    }

    #[test]
    fn reads_a_loaded_cell_with_its_pending_edit() {
        let rows = vec![row(&[Some("1"), Some("Ann"), None]), row(&[Some("2"), Some("Bob"), Some("hi")])];
        let mut edits = Edits::default();
        edits.edit_cell(&key("1"), "name", text("Anna"), Some("Ann"));
        assert_eq!(
            edits.cell_at(&rows, &columns(), RowRef::Loaded(0), 1),
            Some(Shown { value: Some("Anna".into()), unset: false, deleted: false })
        );
        assert_eq!(edits.cell_at(&rows, &columns(), RowRef::Loaded(1), 2).unwrap().value.as_deref(), Some("hi"));
        edits.toggle_delete(&key("1"));
        assert!(edits.cell_at(&rows, &columns(), RowRef::Loaded(0), 1).unwrap().deleted);
    }

    #[test]
    fn reads_a_new_row_with_columns_left_to_their_default() {
        let mut edits = Edits::default();
        let id = edits.add_row();
        edits.edit_new_cell(id, "name", Some("Cy".into()));
        assert_eq!(
            edits.cell_at(&[], &columns(), RowRef::New(id), 1),
            Some(Shown { value: Some("Cy".into()), unset: false, deleted: false })
        );
        assert_eq!(
            edits.cell_at(&[], &columns(), RowRef::New(id), 2),
            Some(Shown { value: None, unset: true, deleted: false })
        );
    }

    #[test]
    fn has_no_cell_that_is_gone() {
        let rows = vec![row(&[Some("1"), Some("Ann"), None])];
        let edits = Edits::default();
        assert_eq!(edits.cell_at(&rows, &columns(), RowRef::Loaded(5), 1), None);
        assert_eq!(edits.cell_at(&rows, &columns(), RowRef::New(9), 1), None);
        assert_eq!(edits.cell_at(&rows, &columns(), RowRef::Loaded(0), 7), None);
    }

    #[test]
    fn keeps_default_as_a_pending_change_read_back_as_a_default_cell() {
        let rows = vec![row(&[Some("1"), Some("Ann"), None])];
        let mut edits = Edits::default();
        edits.edit_cell(&key("1"), "note", CellValue::Default, None);
        let values = BTreeMap::from([("note".to_owned(), CellValue::Default)]);
        assert_eq!(edits.to_changes(), [RowChange::Update { key: key("1"), values }]);
        assert_eq!(
            edits.cell_at(&rows, &columns(), RowRef::Loaded(0), 2),
            Some(Shown { value: None, unset: true, deleted: false })
        );
    }

    #[test]
    fn adds_a_row_holding_another_row_s_values() {
        let mut edits = Edits::default();
        edits.add_row();
        let values = BTreeMap::from([("name".to_owned(), Some("Ann".to_owned())), ("email".to_owned(), None)]);
        let id = edits.add_row_with(values.clone());
        assert_eq!(edits.inserts[0], NewRow { id: 2, values });
        assert!(edits.to_changes().contains(&RowChange::Insert {
            values: BTreeMap::from([("name".to_owned(), text("Ann")), ("email".to_owned(), CellValue::Null)])
        }));
        assert_eq!(id, 2);
    }

    #[test]
    fn replaces_a_document_until_it_is_written_back_as_loaded() {
        let mut edits = Edits::default();
        edits.replace_document(&key("1"), r#"{"id": "1", "n": 2}"#, r#"{"id": "1", "n": 1}"#);
        assert_eq!(edits.counts(), Counts { updated: 1, inserted: 0, deleted: 0 });
        assert_eq!(
            edits.to_changes(),
            [RowChange::Replace { key: key("1"), document: r#"{"id": "1", "n": 2}"#.into() }]
        );
        edits.replace_document(&key("1"), r#"{"id": "1", "n": 1}"#, r#"{"id": "1", "n": 1}"#);
        assert!(!edits.has_edits());
    }

    #[test]
    fn drops_a_replacement_when_the_row_is_marked_for_deletion() {
        let mut edits = Edits::default();
        edits.replace_document(&key("1"), "{}", r#"{"id": "1"}"#);
        edits.toggle_delete(&key("1"));
        assert_eq!(edits.counts(), Counts { updated: 0, inserted: 0, deleted: 1 });
    }

    #[test]
    fn adds_new_documents_sent_oldest_first_after_everything_else() {
        let mut edits = Edits::default();
        edits.add_document("{}");
        edits.add_document("{}");
        edits.edit_new_document(1, r#"{"first": true}"#);
        edits.replace_document(&key("1"), r#"{"id": "1"}"#, r#"{"id": "1", "old": true}"#);
        let changes = edits.to_changes();
        assert!(matches!(
            changes[..],
            [RowChange::Replace { .. }, RowChange::InsertDocument { .. }, RowChange::InsertDocument { .. }]
        ));
        assert_eq!(changes[1], RowChange::InsertDocument { document: r#"{"first": true}"#.into() });
        edits.remove_new_document(2);
        assert_eq!(edits.counts(), Counts { updated: 1, inserted: 1, deleted: 0 });
        assert_eq!(edits.new_document(1), Some(r#"{"first": true}"#));
    }
}
