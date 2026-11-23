//! The exports DBForge wrote into a folder: how much they take, and clearing
//! them. Only files named the way an export is named
//! (`shop-2026-11-10-1432.sql`, a MongoDB `….archive.gz`, a SQLite copy
//! `….db`, or an unfinished `….dbforge-partial` of one) count, at the top of the folder; anything else there is left alone, since
//! the folder may be one the user keeps other files in.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

use chrono::{Datelike, NaiveDateTime, Timelike};
use serde::Serialize;

use crate::error::Result;

/// How many exports, and their size in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFiles {
    pub count: u64,
    pub bytes: u64,
}

/// The extensions of the files exports write, as [`super::DumpOptions::extension`] gives them.
const EXTENSIONS: [&str; 3] = [".sql", ".archive.gz", ".db"];

/// Whether `name` is an export's file name: anything, then `-YYYY-MM-DD-HHMM`
/// and one of the [`EXTENSIONS`]; or an unfinished one.
fn is_export(name: &str) -> bool {
    finished_export(name.strip_suffix(".dbforge-partial").unwrap_or(name)).is_some()
}

/// The extension of `name` when it is a finished export's file name, dot included.
pub fn export_extension(name: &str) -> Option<&'static str> {
    finished_export(name)
}

fn finished_export(name: &str) -> Option<&'static str> {
    let (extension, stem) =
        EXTENSIONS.iter().find_map(|extension| name.strip_suffix(extension).map(|stem| (*extension, stem)))?;
    // "-2026-11-10-1432": a dash, then digits and dashes in this shape.
    const SHAPE: &[u8] = b"-dddd-dd-dd-dddd";
    let bytes = stem.as_bytes();
    let dated = bytes.len() > SHAPE.len()
        && bytes[bytes.len() - SHAPE.len()..]
            .iter()
            .zip(SHAPE)
            .all(|(byte, shape)| if *shape == b'd' { byte.is_ascii_digit() } else { byte == shape });
    dated.then_some(extension)
}

/// A file name for a database's export, unique to the minute, with the
/// engine's `extension`: `shop-2026-11-10-1432.sql`, `….archive.gz`.
pub fn export_file_name(database: &str, at: NaiveDateTime, extension: &str) -> String {
    // Characters a file name can't hold on any system become dashes.
    let safe: String = database
        .chars()
        .map(|c| if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0') { '-' } else { c })
        .collect();
    format!("{safe}-{}-{:02}-{:02}-{:02}{:02}{extension}", at.year(), at.month(), at.day(), at.hour(), at.minute())
}

/// The name a SQLite connection's exports go by: its file's, without the extension.
pub fn sqlite_name(file: &str) -> String {
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => name.to_owned(),
    }
}

/// The folder exports go to when the user chose none: `DBForge` in their
/// Documents; `None` on a system without a Documents folder.
pub fn default_export_folder() -> Option<PathBuf> {
    dirs::document_dir().map(|documents| documents.join("DBForge"))
}

/// Where exports go, as the app's preference says: the app sets it at start
/// and whenever the preference changes, and the MCP tools read it.
pub struct ExportFolder(RwLock<Option<PathBuf>>);

impl Default for ExportFolder {
    fn default() -> Self {
        Self(RwLock::new(default_export_folder()))
    }
}

impl ExportFolder {
    /// The folder now; `None` when none was chosen and there is no default.
    pub fn get(&self) -> Option<PathBuf> {
        self.0.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Exports go to `folder` from now on; `None` (or an empty path) goes back to the default.
    pub fn set(&self, folder: Option<PathBuf>) {
        let folder = folder.filter(|folder| !folder.as_os_str().is_empty()).or_else(default_export_folder);
        *self.0.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = folder;
    }
}

/// A finished export at the top of the exports folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFile {
    pub name: String,
    /// Dot included, as [`super::DumpOptions::extension`] gives it.
    pub extension: &'static str,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
}

/// The finished exports at the top of `folder`, by name; none when it doesn't exist yet.
pub fn list_exports(folder: &Path) -> Result<Vec<ExportFile>> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        // Not followed: a link named like an export is not one DBForge wrote.
        let metadata = entry.metadata()?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        if let Some(extension) = export_extension(&name).filter(|_| metadata.is_file()) {
            files.push(ExportFile { name, extension, bytes: metadata.len(), modified: metadata.modified().ok() });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

/// Visits each export file at the top of `folder`, with its size. A folder
/// that doesn't exist yet holds none.
fn each_export(folder: &Path, mut visit: impl FnMut(&Path, u64) -> std::io::Result<()>) -> Result<()> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        // Not followed: a link named like an export is not one DBForge wrote.
        let metadata = entry.metadata()?;
        if metadata.is_file() && entry.file_name().to_str().is_some_and(is_export) {
            visit(&entry.path(), metadata.len())?;
        }
    }
    Ok(())
}

/// The exports in `folder`.
pub fn usage(folder: &Path) -> Result<ExportFiles> {
    let mut files = ExportFiles::default();
    each_export(folder, |_, bytes| {
        files.count += 1;
        files.bytes += bytes;
        Ok(())
    })?;
    Ok(files)
}

/// Deletes the exports in `folder`, and says what was deleted.
pub fn clear(folder: &Path) -> Result<ExportFiles> {
    let mut removed = ExportFiles::default();
    each_export(folder, |path, bytes| {
        std::fs::remove_file(path)?;
        removed.count += 1;
        removed.bytes += bytes;
        Ok(())
    })?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_names_are_the_dated_sql_files() {
        assert!(is_export("shop-2026-11-10-1432.sql"));
        assert!(is_export("my-db-2026-01-02-0304.sql"));
        assert!(is_export("shop-2026-11-10-1432.sql.dbforge-partial"));
        assert!(!is_export("-2026-11-10-1432.sql"));
        assert!(!is_export("shop.sql"));
        assert!(!is_export("shop-2026-11-10.sql"));
        assert!(!is_export("shop-2026-11-10-1432.sql.bak"));
        assert!(!is_export("notes.txt"));
    }

    #[test]
    fn mongodb_archives_and_sqlite_copies_are_exports_too() {
        assert!(is_export("shop-2026-11-10-1432.archive.gz"));
        assert!(is_export("shop-2026-11-10-1432.archive.gz.dbforge-partial"));
        assert!(is_export("app-2026-11-10-1432.db"));
        assert!(is_export("app-2026-11-10-1432.db.dbforge-partial"));
        assert!(!is_export("shop-2026-11-10-1432.gz"));
        assert!(!is_export("shop.archive.gz"));
        assert!(!is_export("app.db"));
        assert!(!is_export("app-2026-11-10.db"));
        assert!(!is_export("shop-2026-11-10-1432.archive"));
        assert!(!is_export("-2026-11-10-1432.db"));
    }

    #[test]
    fn counts_and_clears_only_exports() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        std::fs::write(folder.join("shop-2026-11-10-1432.sql"), "12345").unwrap();
        std::fs::write(folder.join("crm-2026-11-09-0900.sql.dbforge-partial"), "123").unwrap();
        std::fs::write(folder.join("schema.sql"), "keep").unwrap();
        std::fs::create_dir(folder.join("old-2026-01-01-0000.sql")).unwrap();
        std::fs::write(folder.join("nested-2026-01-01-0000.txt"), "keep").unwrap();
        std::fs::write(folder.join("orders-2026-11-10-1432.archive.gz"), "12").unwrap();
        std::fs::write(folder.join("app-2026-11-10-1432.db"), "1").unwrap();
        std::fs::write(folder.join("app.db"), "keep").unwrap();

        assert_eq!(usage(folder).unwrap(), ExportFiles { count: 4, bytes: 11 });
        assert_eq!(clear(folder).unwrap(), ExportFiles { count: 4, bytes: 11 });
        assert_eq!(usage(folder).unwrap(), ExportFiles::default());

        let mut left: Vec<_> =
            std::fs::read_dir(folder).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["app.db", "nested-2026-01-01-0000.txt", "old-2026-01-01-0000.sql", "schema.sql"]);
    }

    #[test]
    fn only_a_finished_export_has_an_extension() {
        assert_eq!(export_extension("shop-2026-11-10-1432.sql"), Some(".sql"));
        assert_eq!(export_extension("shop-2026-11-10-1432.archive.gz"), Some(".archive.gz"));
        assert_eq!(export_extension("app-2026-11-10-1432.db"), Some(".db"));
        assert_eq!(export_extension("shop-2026-11-10-1432.sql.dbforge-partial"), None);
        assert_eq!(export_extension("shop.sql"), None);
        assert!(export_extension("../shop-2026-11-10-1432.sql").is_some(), "a name check, not a path check");
    }

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap().and_hms_opt(hour, minute, 0).unwrap()
    }

    #[test]
    fn names_an_export_after_its_database_and_the_minute() {
        assert_eq!(export_file_name("shop", at(2026, 11, 10, 14, 32), ".sql"), "shop-2026-11-10-1432.sql");
        assert_eq!(export_file_name("a/b:c\\d", at(2026, 1, 2, 3, 4), ".db"), "a-b-c-d-2026-01-02-0304.db");
        let name = export_file_name("../../etc", at(2026, 1, 2, 3, 4), ".sql");
        assert!(!name.contains('/'), "{name}");
        assert_eq!(sqlite_name("/data/app.v2.db"), "app.v2");
        assert_eq!(sqlite_name("C:\\data\\app.sqlite"), "app");
        assert_eq!(sqlite_name("/data/.hidden"), ".hidden");
    }

    #[test]
    fn lists_only_finished_exports_with_their_kind() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        std::fs::write(folder.join("shop-2026-11-10-1432.sql"), "12345").unwrap();
        std::fs::write(folder.join("crm-2026-11-09-0900.sql.dbforge-partial"), "123").unwrap();
        std::fs::write(folder.join("orders-2026-11-10-1432.archive.gz"), "12").unwrap();
        std::fs::write(folder.join("notes.txt"), "keep").unwrap();
        std::fs::create_dir(folder.join("old-2026-01-01-0000.sql")).unwrap();

        let listed: Vec<_> =
            list_exports(folder).unwrap().into_iter().map(|file| (file.name, file.extension, file.bytes)).collect();
        assert_eq!(
            listed,
            [
                ("orders-2026-11-10-1432.archive.gz".to_owned(), ".archive.gz", 2),
                ("shop-2026-11-10-1432.sql".to_owned(), ".sql", 5),
            ]
        );
        assert!(list_exports(&folder.join("missing")).unwrap().is_empty());
    }

    #[test]
    fn the_folder_setting_falls_back_to_the_default() {
        let folder = ExportFolder::default();
        assert_eq!(folder.get(), default_export_folder());
        folder.set(Some("/tmp/exports".into()));
        assert_eq!(folder.get(), Some(PathBuf::from("/tmp/exports")));
        folder.set(Some(PathBuf::new()));
        assert_eq!(folder.get(), default_export_folder());
    }

    #[test]
    fn a_folder_not_made_yet_holds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("DBForge");
        assert_eq!(usage(&missing).unwrap(), ExportFiles::default());
        assert_eq!(clear(&missing).unwrap(), ExportFiles::default());
    }
}
