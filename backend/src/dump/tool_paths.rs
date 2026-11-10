//! The tools the user chose in place of the ones DBForge finds, kept in `tools.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::Result;
use crate::files::JsonFile;

pub struct ToolPaths {
    paths: JsonFile<BTreeMap<String, PathBuf>>,
}

impl ToolPaths {
    /// The choices saved at `file`; none until the user makes one.
    pub fn load(file: PathBuf) -> Result<Self> {
        Ok(Self { paths: JsonFile::load(file, BTreeMap::new)? })
    }

    /// Every choice, by tool name.
    pub fn all(&self) -> BTreeMap<String, PathBuf> {
        self.paths.get()
    }

    /// The path chosen for tool `name`, if any.
    pub fn get(&self, name: &str) -> Option<PathBuf> {
        self.paths.read(|paths| paths.get(name).cloned())
    }

    /// Chooses `path` for `name`, or goes back to the one DBForge finds (`None`).
    pub fn set(&self, name: &str, path: Option<PathBuf>) -> Result<()> {
        self.paths.update(|paths| {
            match path {
                Some(path) => paths.insert(name.to_string(), path),
                None => paths.remove(name),
            };
            Ok(())
        })
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
