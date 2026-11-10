//! Every query run from a console, kept on disk so it survives restarts.
//! Stored as JSON Lines: a run appends one line instead of rewriting the file.
//! The file may hold up to [`COMPACT_AFTER`] lines more than are kept; once it
//! does, it is written again with the kept ones only.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Older entries are dropped past this, so the file stays small.
const MAX_ENTRIES: usize = 5_000;
/// Lines the file may hold beyond the entries kept before it is compacted:
/// one rewrite every this many runs, rather than one for each.
const COMPACT_AFTER: usize = 1_000;
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
    /// The Cosmos DB container the console queried. Missing from older lines.
    #[serde(default)]
    pub container: Option<String>,
    pub sql: String,
    pub elapsed_ms: u64,
    pub statements: usize,
    /// Rows of the last statement: returned by a SELECT, changed by anything else.
    pub rows: u64,
    pub error: Option<String>,
}

pub struct HistoryStore {
    path: PathBuf,
    state: Mutex<State>,
}

struct State {
    /// The newest [`MAX_ENTRIES`], oldest first.
    entries: VecDeque<HistoryEntry>,
    /// Lines in the file, dropped and damaged ones included.
    lines: usize,
}

impl HistoryStore {
    /// Loads the history from `path`. A missing file is an empty history, and a
    /// damaged line (say, from a crash mid-write) is skipped rather than fatal.
    pub fn load(path: PathBuf) -> Result<Self> {
        let (mut entries, lines) = match fs::read_to_string(&path) {
            Ok(text) => (
                text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect::<VecDeque<_>>(),
                text.lines().count(),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (VecDeque::new(), 0),
            Err(error) => return Err(error.into()),
        };
        let excess = entries.len().saturating_sub(MAX_ENTRIES);
        entries.drain(..excess);
        Ok(Self { path, state: Mutex::new(State { entries, lines }) })
    }

    pub fn record(&self, entry: HistoryEntry) -> Result<()> {
        let mut state = self.lock();
        if state.entries.len() == MAX_ENTRIES {
            state.entries.pop_front();
        }
        state.entries.push_back(entry);
        if state.lines >= MAX_ENTRIES + COMPACT_AFTER {
            self.rewrite(&state.entries)?;
            state.lines = state.entries.len();
            return Ok(());
        }
        self.append(state.entries.back().expect("just pushed"))?;
        state.lines += 1;
        Ok(())
    }

    /// Newest first. `text` matches anywhere in the SQL, ignoring case; with
    /// `connection_id`, only that connection's runs.
    pub fn search(
        &self,
        text: &str,
        connection_id: Option<&str>,
        errors_only: bool,
        limit: usize,
    ) -> Vec<HistoryEntry> {
        let text = text.to_lowercase();
        self.lock()
            .entries
            .iter()
            .rev()
            .filter(|e| connection_id.is_none_or(|id| e.connection_id == id))
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
    fn rewrite(&self, entries: &VecDeque<HistoryEntry>) -> Result<()> {
        let tmp = self.path.with_extension("jsonl.tmp");
        let mut text = String::new();
        for entry in entries {
            text.push_str(&serde_json::to_string(entry)?);
            text.push('\n');
        }
        let mut file = fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A poisoned lock only means another thread panicked; the data is still valid.
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Past runs, newest first, optionally filtered by SQL text, by connection or to failed runs.
#[tauri::command]
pub fn list_history(
    history: tauri::State<'_, HistoryStore>,
    search: String,
    connection_id: Option<String>,
    errors_only: bool,
) -> Vec<HistoryEntry> {
    history.search(&search, connection_id.as_deref(), errors_only, SEARCH_LIMIT)
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
            container: None,
            sql: sql.into(),
            elapsed_ms: 1,
            statements: 1,
            rows: 0,
            error: error.map(Into::into),
        }
    }

    #[test]
    fn reads_lines_saved_before_containers_were_recorded() {
        let old = r#"{"id":"1","executedAt":1,"connectionId":"c1","connectionName":"local","database":"shop","sql":"SELECT 1","elapsedMs":1,"statements":1,"rows":1,"error":null}"#;
        let entry: HistoryEntry = serde_json::from_str(old).unwrap();
        assert_eq!((entry.database.as_deref(), entry.container), (Some("shop"), None));
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
        assert_eq!(ids(&reloaded.search("", None, false, 10)), ["2", "1"]);
    }

    #[test]
    fn damaged_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let good = serde_json::to_string(&entry(1, "SELECT 1", None)).unwrap();
        fs::write(&path, format!("{good}\n{{\"id\": \"2\", \"sql\n")).unwrap();

        let store = HistoryStore::load(path).unwrap();
        assert_eq!(ids(&store.search("", None, false, 10)), ["1"]);
    }

    #[test]
    fn search_filters_by_text_and_errors_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::load(dir.path().join("history.jsonl")).unwrap();
        store.record(entry(1, "SELECT * FROM users", None)).unwrap();
        store.record(entry(2, "select * from orders", Some("boom"))).unwrap();
        store.record(entry(3, "SELECT * FROM Orders", None)).unwrap();

        assert_eq!(ids(&store.search("ORDERS", None, false, 10)), ["3", "2"]);
        assert_eq!(ids(&store.search("", None, true, 10)), ["2"]);
        assert_eq!(ids(&store.search("", None, false, 2)), ["3", "2"]);
    }

    #[test]
    fn the_file_is_compacted_once_in_a_while_not_on_every_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let lines = || fs::read_to_string(&path).unwrap().lines().count();
        let store = HistoryStore::load(path.clone()).unwrap();
        for n in 0..(MAX_ENTRIES + COMPACT_AFTER) as u64 {
            store.record(entry(n, "SELECT 1", None)).unwrap();
        }
        // Past the limit, runs are still appended; the oldest are only hidden.
        assert_eq!(lines(), MAX_ENTRIES + COMPACT_AFTER);
        assert_eq!(store.search("", None, false, usize::MAX).len(), MAX_ENTRIES);
        let reloaded = HistoryStore::load(path.clone()).unwrap();
        assert_eq!(reloaded.search("", None, false, usize::MAX).len(), MAX_ENTRIES);

        // The next run writes the kept ones only, and appending starts again.
        reloaded.record(entry(9_999_999, "SELECT 2", None)).unwrap();
        assert_eq!(lines(), MAX_ENTRIES);
        reloaded.record(entry(10_000_000, "SELECT 3", None)).unwrap();
        assert_eq!(lines(), MAX_ENTRIES + 1);
        let newest = HistoryStore::load(path).unwrap().search("", None, false, 2);
        assert_eq!(ids(&newest), ["10000000", "9999999"]);
    }

    #[test]
    fn search_can_keep_to_one_connection_before_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::load(dir.path().join("history.jsonl")).unwrap();
        store.record(entry(1, "SELECT 1", None)).unwrap();
        for n in 2..10 {
            store.record(HistoryEntry { connection_id: "c2".into(), ..entry(n, "SELECT 2", None) }).unwrap();
        }
        // The newest 3 are all c2's: c1's run is still found when asked for.
        assert_eq!(ids(&store.search("", Some("c1"), false, 3)), ["1"]);
        assert_eq!(store.search("", Some("c2"), false, 3).len(), 3);
        assert!(store.search("", Some("c3"), false, 3).is_empty());
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
        let all = reloaded.search("", None, false, usize::MAX);
        assert_eq!(all.len(), MAX_ENTRIES);
        assert_eq!(all.last().unwrap().id, "1", "entry 0 was the oldest and is gone");
    }
}
