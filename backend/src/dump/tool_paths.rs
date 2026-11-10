//! The tools the user chose in place of the ones DBForge finds, kept in `tools.json`.

use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use crate::error::Result;

pub struct ToolPaths {
    file: PathBuf,
    paths: Mutex<BTreeMap<String, PathBuf>>,
}

impl ToolPaths {
    /// The choices saved at `file`; none until the user makes one.
    pub fn load(file: PathBuf) -> Result<Self> {
        let paths = match fs::read_to_string(&file) {
            Ok(json) => serde_json::from_str(&json)?,
            Err(error) if error.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { file, paths: Mutex::new(paths) })
    }

    /// The path chosen for tool `name`, if any.
    pub fn get(&self, name: &str) -> Option<PathBuf> {
        self.lock().get(name).cloned()
    }

    /// Chooses `path` for `name`, or goes back to the one DBForge finds (`None`).
    pub fn set(&self, name: &str, path: Option<PathBuf>) -> Result<()> {
        let mut paths = self.lock();
        let mut next = paths.clone();
        match path {
            Some(path) => next.insert(name.to_string(), path),
            None => next.remove(name),
        };
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.file.with_extension("json.tmp");
        let mut file = fs::File::create(&tmp)?;
        file.write_all(serde_json::to_string_pretty(&next)?.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &self.file)?;
        *paths = next;
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, PathBuf>> {
        self.paths.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_chosen_path_until_it_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tools.json");
        let paths = ToolPaths::load(file.clone()).unwrap();
        assert_eq!(paths.get("pg_dump"), None);
        paths.set("pg_dump", Some("/opt/pg/bin/pg_dump".into())).unwrap();
        assert_eq!(ToolPaths::load(file.clone()).unwrap().get("pg_dump"), Some("/opt/pg/bin/pg_dump".into()));
        paths.set("pg_dump", None).unwrap();
        assert_eq!(ToolPaths::load(file).unwrap().get("pg_dump"), None);
    }
}
