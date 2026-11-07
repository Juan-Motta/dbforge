use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use super::model::Connection;
use crate::error::{AppError, Result};

/// Saved connections, kept in memory and persisted to a JSON file.
pub struct ConnectionStore {
    path: PathBuf,
    connections: Mutex<Vec<Connection>>,
}

impl ConnectionStore {
    /// Loads the store from `path`. A missing file means no connections yet.
    pub fn load(path: PathBuf) -> Result<Self> {
        let connections = match fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, connections: Mutex::new(connections) })
    }

    pub fn list(&self) -> Vec<Connection> {
        self.lock().clone()
    }

    pub fn get(&self, id: &str) -> Result<Connection> {
        self.lock().iter().find(|c| c.id == id).cloned().ok_or_else(|| AppError::NotFound(id.into()))
    }

    /// Adds the connection, or replaces the one with the same id.
    pub fn upsert(&self, connection: Connection) -> Result<()> {
        let mut connections = self.lock();
        match connections.iter_mut().find(|c| c.id == connection.id) {
            Some(existing) => *existing = connection,
            None => connections.push(connection),
        }
        self.persist(&connections)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let mut connections = self.lock();
        let before = connections.len();
        connections.retain(|c| c.id != id);
        if connections.len() == before {
            return Err(AppError::NotFound(id.into()));
        }
        self.persist(&connections)
    }

    /// Writes to a temporary file first so a crash never leaves a half-written file.
    fn persist(&self, connections: &[Connection]) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(connections)?)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Connection>> {
        // A poisoned lock only means another thread panicked; the data is still valid.
        self.connections.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::model::Engine;

    fn connection(id: &str, name: &str) -> Connection {
        Connection {
            id: id.into(),
            name: name.into(),
            engine: Engine::Postgres,
            environment: None,
            host: "localhost".into(),
            port: 5432,
            database: "shop".into(),
            username: "app".into(),
            tls: false,
            allow_self_signed: false,
            srv: false,
        }
    }

    fn temp_store() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("connections.json");
        (dir, path)
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let (_dir, path) = temp_store();
        assert!(ConnectionStore::load(path).unwrap().list().is_empty());
    }

    #[test]
    fn upsert_adds_then_replaces_and_persists() {
        let (_dir, path) = temp_store();
        let store = ConnectionStore::load(path.clone()).unwrap();
        store.upsert(connection("a", "first")).unwrap();
        store.upsert(connection("a", "renamed")).unwrap();
        store.upsert(connection("b", "second")).unwrap();

        let reloaded = ConnectionStore::load(path).unwrap();
        let names: Vec<_> = reloaded.list().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["renamed", "second"]);
    }

    #[test]
    fn remove_deletes_and_persists() {
        let (_dir, path) = temp_store();
        let store = ConnectionStore::load(path.clone()).unwrap();
        store.upsert(connection("a", "first")).unwrap();
        store.remove("a").unwrap();
        assert!(ConnectionStore::load(path).unwrap().list().is_empty());
    }

    #[test]
    fn unknown_ids_are_not_found() {
        let (_dir, path) = temp_store();
        let store = ConnectionStore::load(path).unwrap();
        assert!(matches!(store.get("x"), Err(AppError::NotFound(_))));
        assert!(matches!(store.remove("x"), Err(AppError::NotFound(_))));
    }

    #[test]
    fn corrupt_file_is_reported() {
        let (_dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not json").unwrap();
        assert!(matches!(ConnectionStore::load(path), Err(AppError::Format(_))));
    }
}
