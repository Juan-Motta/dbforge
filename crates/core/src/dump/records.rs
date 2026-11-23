//! Which connection wrote each export DBForge made, kept in `exports.json`:
//! agents are offered only exports of connections they may reach, and a
//! file DBForge has no record of (made elsewhere, or before records were
//! kept) is offered to none.
//!
//! An export is recorded by its path, so one written under the same name in
//! another folder is another export. Records of files gone are dropped when read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::files::JsonFile;

/// Where an export came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRecord {
    pub connection_id: String,
    /// The database exported; for SQLite, the connection's file.
    pub database: String,
}

pub struct ExportRecords {
    records: JsonFile<BTreeMap<PathBuf, ExportRecord>>,
}

impl ExportRecords {
    /// The records saved at `file`; none until an export is made.
    pub fn load(file: PathBuf) -> Result<Self> {
        Ok(Self { records: JsonFile::load(file, BTreeMap::new)? })
    }

    /// Where the file was moved when it couldn't be read at start.
    pub fn set_aside(&self) -> Option<&Path> {
        self.records.set_aside()
    }

    /// Records that the export at `path` holds `database` of connection `connection_id`.
    pub fn record(&self, path: &Path, connection_id: &str, database: &str) -> Result<()> {
        let record = ExportRecord { connection_id: connection_id.into(), database: database.into() };
        self.records.update(|records| {
            records.retain(|path, _| path.is_file());
            records.insert(path.to_owned(), record);
            Ok(())
        })
    }

    /// Where the export at `path` came from; `None` when DBForge has no record of it.
    pub fn of(&self, path: &Path) -> Option<ExportRecord> {
        self.prune();
        self.records.read(|records| records.get(path).cloned())
    }

    /// Drops the records of files that are gone. Kept as they are when the file can't be written.
    fn prune(&self) {
        if self.records.read(|records| records.keys().all(|path| path.is_file())) {
            return;
        }
        let _ = self.records.update(|records| {
            records.retain(|path, _| path.is_file());
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_who_made_each_export_until_its_file_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("exports.json");
        let export = dir.path().join("shop-2026-11-10-1432.sql");
        let other = dir.path().join("crm-2026-11-10-1432.sql");
        std::fs::write(&export, "SELECT 1;").unwrap();
        std::fs::write(&other, "SELECT 1;").unwrap();
        let records = ExportRecords::load(file.clone()).unwrap();
        assert_eq!(records.of(&export), None, "not made by DBForge");
        records.record(&export, "shop", "shop").unwrap();
        records.record(&other, "crm", "crm").unwrap();

        let loaded = ExportRecords::load(file.clone()).unwrap();
        let shop = ExportRecord { connection_id: "shop".into(), database: "shop".into() };
        assert_eq!(loaded.of(&export), Some(shop));
        assert_eq!(loaded.of(&dir.path().join("elsewhere").join("shop-2026-11-10-1432.sql")), None);

        std::fs::remove_file(&other).unwrap();
        assert_eq!(loaded.of(&other), None);
        let saved = std::fs::read_to_string(&file).unwrap();
        assert!(!saved.contains("crm-2026"), "a gone file's record is dropped: {saved}");
        assert!(saved.contains("shop-2026"), "{saved}");
    }
}
