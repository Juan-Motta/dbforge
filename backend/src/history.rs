//! Every query run from a console, kept on disk so it survives restarts.
//! Stored as JSON Lines: a run appends one line instead of rewriting the file.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::Result;

/// Older entries are dropped past this, so the file stays small.
const MAX_ENTRIES: usize = 5_000;
/// Entries returned per search; the newest win.
const SEARCH_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    /// Milliseconds since the Unix epoch.
    pub executed_at: u64,
    pub connection_id: String,
    /// The name when it ran, so the entry stays readable after a rename or delete.
    pub connection_name: String,
    pub database: Option<String>,
    pub sql: String,
    pub elapsed_ms: u64,
    pub statements: usize,
    /// Rows of the last statement: returned by a SELECT, changed by anything else.
    pub rows: u64,
    pub error: Option<String>,
}

pub struct HistoryStore {
    path: PathBuf,
    /// Oldest first, as in the file.
    entries: Mutex<Vec<HistoryEntry>>,
}

impl HistoryStore {
    /// Loads the history from `path`. A missing file is an empty history, and a
    /// damaged line (say, from a crash mid-write) is skipped rather than fatal.
    pub fn load(path: PathBuf) -> Result<Self> {
        let entries = match fs::read_to_string(&path) {
            Ok(text) => text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, entries: Mutex::new(entries) })
    }

    pub fn record(&self, entry: HistoryEntry) -> Result<()> {
        let mut entries = self.lock();
        entries.push(entry);
        if entries.len() > MAX_ENTRIES {
            let excess = entries.len() - MAX_ENTRIES;
            entries.drain(..excess);
            return self.rewrite(&entries);
        }
        self.append(entries.last().expect("just pushed"))
    }

    /// Newest first. `text` matches anywhere in the SQL, ignoring case.
    pub fn search(&self, text: &str, errors_only: bool, limit: usize) -> Vec<HistoryEntry> {
        let text = text.to_lowercase();
        self.lock()
            .iter()
            .rev()
            .filter(|e| !errors_only || e.error.is_some())
            .filter(|e| text.is_empty() || e.sql.to_lowercase().contains(&text))
            .take(limit)
            .cloned()
            .collect()
    }

    fn append(&self, entry: &HistoryEntry) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(file, "{}", serde_json::to_string(entry)?)?;
        Ok(())
    }

    /// Writes to a temporary file first so a crash never leaves a half-written file.
    fn rewrite(&self, entries: &[HistoryEntry]) -> Result<()> {
        let tmp = self.path.with_extension("jsonl.tmp");
        let mut text = String::new();
        for entry in entries {
            text.push_str(&serde_json::to_string(entry)?);
            text.push('\n');
        }
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<HistoryEntry>> {
        // A poisoned lock only means another thread panicked; the data is still valid.
        self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Past runs, newest first, optionally filtered by SQL text or to failed runs.
#[tauri::command]
pub fn list_history(history: State<'_, HistoryStore>, search: String, errors_only: bool) -> Vec<HistoryEntry> {
    history.search(&search, errors_only, SEARCH_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u64, sql: &str, error: Option<&str>) -> HistoryEntry {
        HistoryEntry {
            id: n.to_string(),
            executed_at: n,
            connection_id: "c1".into(),
            connection_name: "local".into(),
            database: None,
            sql: sql.into(),
            elapsed_ms: 1,
            statements: 1,
            rows: 0,
            error: error.map(Into::into),
        }
    }

    fn ids(entries: &[HistoryEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn records_survive_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let store = HistoryStore::load(path.clone()).unwrap();
        store.record(entry(1, "SELECT 1", None)).unwrap();
        store.record(entry(2, "SELECT 2", None)).unwrap();

        let reloaded = HistoryStore::load(path).unwrap();
        assert_eq!(ids(&reloaded.search("", false, 10)), ["2", "1"]);
    }

    #[test]
    fn damaged_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let good = serde_json::to_string(&entry(1, "SELECT 1", None)).unwrap();
        fs::write(&path, format!("{good}\n{{\"id\": \"2\", \"sql\n")).unwrap();

        let store = HistoryStore::load(path).unwrap();
        assert_eq!(ids(&store.search("", false, 10)), ["1"]);
    }

    #[test]
    fn search_filters_by_text_and_errors_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::load(dir.path().join("history.jsonl")).unwrap();
        store.record(entry(1, "SELECT * FROM users", None)).unwrap();
        store.record(entry(2, "select * from orders", Some("boom"))).unwrap();
        store.record(entry(3, "SELECT * FROM Orders", None)).unwrap();

        assert_eq!(ids(&store.search("ORDERS", false, 10)), ["3", "2"]);
        assert_eq!(ids(&store.search("", true, 10)), ["2"]);
        assert_eq!(ids(&store.search("", false, 2)), ["3", "2"]);
    }

    #[test]
    fn oldest_entries_are_dropped_past_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let store = HistoryStore::load(path.clone()).unwrap();
        for n in 0..=MAX_ENTRIES as u64 {
            store.record(entry(n, "SELECT 1", None)).unwrap();
        }

        let reloaded = HistoryStore::load(path).unwrap();
        let all = reloaded.search("", false, usize::MAX);
        assert_eq!(all.len(), MAX_ENTRIES);
        assert_eq!(all.last().unwrap().id, "1", "entry 0 was the oldest and is gone");
    }
}
