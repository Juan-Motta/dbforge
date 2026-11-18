//! The exports DBForge wrote into a folder: how much they take, and clearing
//! them. Only files named the way an export is named
//! (`shop-2026-11-10-1432.sql`, or an unfinished `….sql.dbforge-partial`)
//! count, at the top of the folder; anything else there is left alone, since
//! the folder may be one the user keeps other files in.

use std::io::ErrorKind;
use std::path::Path;

use serde::Serialize;

use crate::error::Result;

/// How many exports, and their size in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFiles {
    pub count: u64,
    pub bytes: u64,
}

/// Whether `name` is an export's file name: anything, then `-YYYY-MM-DD-HHMM.sql`.
fn is_export(name: &str) -> bool {
    let name = name.strip_suffix(".dbforge-partial").unwrap_or(name);
    let Some(stem) = name.strip_suffix(".sql") else { return false };
    // "-2026-11-10-1432": a dash, then digits and dashes in this shape.
    const SHAPE: &[u8] = b"-dddd-dd-dd-dddd";
    let bytes = stem.as_bytes();
    bytes.len() > SHAPE.len()
        && bytes[bytes.len() - SHAPE.len()..]
            .iter()
            .zip(SHAPE)
            .all(|(byte, shape)| if *shape == b'd' { byte.is_ascii_digit() } else { byte == shape })
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
    fn counts_and_clears_only_exports() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        std::fs::write(folder.join("shop-2026-11-10-1432.sql"), "12345").unwrap();
        std::fs::write(folder.join("crm-2026-11-09-0900.sql.dbforge-partial"), "123").unwrap();
        std::fs::write(folder.join("schema.sql"), "keep").unwrap();
        std::fs::create_dir(folder.join("old-2026-01-01-0000.sql")).unwrap();
        std::fs::write(folder.join("nested-2026-01-01-0000.txt"), "keep").unwrap();

        assert_eq!(usage(folder).unwrap(), ExportFiles { count: 2, bytes: 8 });
        assert_eq!(clear(folder).unwrap(), ExportFiles { count: 2, bytes: 8 });
        assert_eq!(usage(folder).unwrap(), ExportFiles::default());

        let mut left: Vec<_> =
            std::fs::read_dir(folder).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["nested-2026-01-01-0000.txt", "old-2026-01-01-0000.sql", "schema.sql"]);
    }

    #[test]
    fn a_folder_not_made_yet_holds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("DBForge");
        assert_eq!(usage(&missing).unwrap(), ExportFiles::default());
        assert_eq!(clear(&missing).unwrap(), ExportFiles::default());
    }
}
