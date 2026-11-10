use std::path::PathBuf;

use super::model::Connection;
use crate::credentials::Credentials;
use crate::error::{AppError, Result};
use crate::files::JsonFile;
use crate::statement_log::{self, Origin, StatementLog};

/// Saved connections, kept in memory and persisted to a JSON file, and their
/// passwords, kept where the user chose.
pub struct ConnectionStore {
    connections: JsonFile<Vec<Connection>>,
    credentials: Credentials,
}

/// A saved connection with its password: what a command needs to reach the database.
pub struct Opened {
    pub connection: Connection,
    pub password: Option<String>,
}

impl Opened {
    /// Runs `work` on the connection with its statements recorded in `log`, as `origin`.
    pub async fn run<T>(
        &self,
        log: &StatementLog,
        origin: Origin,
        work: impl AsyncFnOnce(&Connection, Option<&str>) -> T,
    ) -> T {
        let work = work(&self.connection, self.password.as_deref());
        statement_log::scoped(log, &self.connection.id, origin, work).await
    }
}

impl ConnectionStore {
    /// Connection `id` with its saved password.
    pub fn open(&self, id: &str) -> Result<Opened> {
        let connection = self.get(id)?;
        let password = self.password(&connection.id)?;
        Ok(Opened { connection, password })
    }

    /// Loads the store from `path`. A missing file means no connections yet.
    pub fn load(path: PathBuf, credentials: Credentials) -> Result<Self> {
        Ok(Self { connections: JsonFile::load(path, Vec::new)?, credentials })
    }

    /// The saved password of connection `id`, if any.
    pub fn password(&self, id: &str) -> Result<Option<String>> {
        self.credentials.get(id)
    }

    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    pub fn list(&self) -> Vec<Connection> {
        self.connections.get()
    }

    pub fn get(&self, id: &str) -> Result<Connection> {
        self.connections
            .read(|connections| connections.iter().find(|c| c.id == id).cloned())
            .ok_or_else(|| AppError::NotFound(id.into()))
    }

    /// Adds the connection, or replaces the one with the same id.
    pub fn upsert(&self, connection: Connection) -> Result<()> {
        self.connections.update(|connections| {
            match connections.iter_mut().find(|c| c.id == connection.id) {
                Some(existing) => *existing = connection,
                None => connections.push(connection),
            }
            Ok(())
        })
    }

    /// A copy of connection `id` named `name`, with a new id and the same
    /// saved password.
    pub fn duplicate(&self, id: &str, name: &str) -> Result<Connection> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Invalid("Name is required".into()));
        }
        let original = self.get(id)?;
        let copy = Connection { id: uuid::Uuid::new_v4().to_string(), name: name.to_string(), ..original };
        if let Some(password) = self.credentials.get(id)? {
            self.credentials.set(&copy.id, &password)?;
        }
        if let Err(error) = self.upsert(copy.clone()) {
            let _ = self.credentials.delete(&copy.id);
            return Err(error);
        }
        Ok(copy)
    }

    /// Connections whose tag is not one of `tags` (it was deleted) lose it.
    /// Checking against every tag that exists, not only the ones just
    /// deleted, also catches a clear that failed after an earlier delete.
    pub fn keep_only_tags(&self, tags: &[String]) -> Result<()> {
        let stale = |connection: &Connection| connection.environment.as_ref().is_some_and(|tag| !tags.contains(tag));
        if !self.connections.read(|connections| connections.iter().any(stale)) {
            return Ok(());
        }
        self.connections.update(|connections| {
            for connection in connections.iter_mut().filter(|c| stale(c)) {
                connection.environment = None;
            }
            Ok(())
        })
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        self.connections.update(|connections| {
            let before = connections.len();
            connections.retain(|c| c.id != id);
            if connections.len() == before {
                return Err(AppError::NotFound(id.into()));
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

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
            agent_access: Default::default(),
        }
    }

    /// The store at `path`, with its credentials next to it.
    fn load(path: PathBuf) -> ConnectionStore {
        let credentials = Credentials::load(path.parent().unwrap().to_path_buf()).unwrap();
        ConnectionStore::load(path, credentials).unwrap()
    }

    fn temp_store() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("connections.json");
        (dir, path)
    }

    #[test]
    fn duplicates_a_connection_with_its_password() {
        let (_dir, path) = temp_store();
        let store = load(path.clone());
        store.upsert(connection("a", "first")).unwrap();
        // A new install keeps passwords in the encrypted file next to the store.
        store.credentials().set("a", "secret").unwrap();

        let copy = store.duplicate("a", " first (copy) ").unwrap();
        assert_ne!(copy.id, "a");
        assert_eq!(copy.name, "first (copy)");
        assert_eq!(Connection { id: "a".into(), name: "first".into(), ..copy.clone() }, connection("a", "first"));
        assert_eq!(store.password(&copy.id).unwrap().as_deref(), Some("secret"));
        assert_eq!(load(path).list().len(), 2);

        assert!(matches!(store.duplicate("a", "  "), Err(AppError::Invalid(_))));
        assert!(matches!(store.duplicate("missing", "x"), Err(AppError::NotFound(_))));
    }

    #[test]
    fn deleted_tags_are_cleared_from_their_connections() {
        let (_dir, path) = temp_store();
        let store = load(path.clone());
        let tagged =
            |id: &str, tag: Option<&str>| Connection { environment: tag.map(String::from), ..connection(id, id) };
        store.upsert(tagged("a", Some("test"))).unwrap();
        store.upsert(tagged("b", Some("production"))).unwrap();
        store.upsert(tagged("c", None)).unwrap();
        store.upsert(tagged("d", Some("left from a failed clear"))).unwrap();
        store.keep_only_tags(&["production".into(), "staging".into()]).unwrap();
        let tags: Vec<_> = load(path).list().into_iter().map(|c| c.environment).collect();
        assert_eq!(tags, [None, Some("production".into()), None, None]);
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let (_dir, path) = temp_store();
        assert!(load(path).list().is_empty());
    }

    #[test]
    fn upsert_adds_then_replaces_and_persists() {
        let (_dir, path) = temp_store();
        let store = load(path.clone());
        store.upsert(connection("a", "first")).unwrap();
        store.upsert(connection("a", "renamed")).unwrap();
        store.upsert(connection("b", "second")).unwrap();

        let reloaded = load(path);
        let names: Vec<_> = reloaded.list().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["renamed", "second"]);
    }

    #[test]
    fn remove_deletes_and_persists() {
        let (_dir, path) = temp_store();
        let store = load(path.clone());
        store.upsert(connection("a", "first")).unwrap();
        store.remove("a").unwrap();
        assert!(load(path).list().is_empty());
    }

    #[test]
    fn unknown_ids_are_not_found() {
        let (_dir, path) = temp_store();
        let store = load(path);
        assert!(matches!(store.get("x"), Err(AppError::NotFound(_))));
        assert!(matches!(store.remove("x"), Err(AppError::NotFound(_))));
    }

    #[test]
    fn corrupt_file_is_reported() {
        let (_dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not json").unwrap();
        let credentials = Credentials::load(path.parent().unwrap().to_path_buf()).unwrap();
        assert!(matches!(ConnectionStore::load(path, credentials), Err(AppError::Format(_))));
    }
}
