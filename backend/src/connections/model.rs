use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Postgres,
    Mysql,
    Mariadb,
    Sqlite,
    Sqlserver,
    Mongodb,
    Redis,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::Postgres => "PostgreSQL",
            Engine::Mysql => "MySQL",
            Engine::Mariadb => "MariaDB",
            Engine::Sqlite => "SQLite",
            Engine::Sqlserver => "SQL Server",
            Engine::Mongodb => "MongoDB",
            Engine::Redis => "Redis",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Production,
    Staging,
    Development,
}

/// A saved connection. The password is never part of it: it lives in the OS keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub engine: Engine,
    pub environment: Option<Environment>,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
}

/// What the UI sends to create, update or test a connection.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInput {
    /// Missing for a new connection.
    pub id: Option<String>,
    pub name: String,
    pub engine: Engine,
    pub environment: Option<Environment>,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    /// `None` keeps the stored password; `Some("")` clears it.
    pub password: Option<String>,
}

impl ConnectionInput {
    /// Validates the input and splits it into the connection to save and the password.
    pub fn into_parts(self) -> Result<(Connection, Option<String>)> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Invalid("Name is required".into()));
        }
        // A SQLite connection is its file: without it there is nothing to open.
        if self.engine == Engine::Sqlite && self.database.trim().is_empty() {
            return Err(AppError::Invalid("Choose the SQLite database file".into()));
        }
        let connection = Connection {
            id: self.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            engine: self.engine,
            environment: self.environment,
            host: self.host.trim().to_string(),
            port: self.port,
            database: self.database.trim().to_string(),
            username: self.username.trim().to_string(),
        };
        Ok((connection, self.password))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, id: Option<&str>) -> ConnectionInput {
        ConnectionInput {
            id: id.map(String::from),
            name: name.into(),
            engine: Engine::Postgres,
            environment: None,
            host: " localhost ".into(),
            port: 5432,
            database: "shop".into(),
            username: "app".into(),
            password: Some("secret".into()),
        }
    }

    #[test]
    fn new_connection_gets_an_id_and_trimmed_fields() {
        let (connection, password) = input(" local ", None).into_parts().unwrap();
        assert!(!connection.id.is_empty());
        assert_eq!(connection.name, "local");
        assert_eq!(connection.host, "localhost");
        assert_eq!(password.as_deref(), Some("secret"));
    }

    #[test]
    fn existing_connection_keeps_its_id() {
        let (connection, _) = input("local", Some("abc")).into_parts().unwrap();
        assert_eq!(connection.id, "abc");
    }

    #[test]
    fn sqlite_needs_its_file() {
        let sqlite = ConnectionInput { engine: Engine::Sqlite, database: "  ".into(), ..input("local", None) };
        assert!(sqlite.into_parts().unwrap_err().to_string().contains("SQLite database file"));
    }

    #[test]
    fn empty_name_is_rejected() {
        let error = input("  ", None).into_parts().unwrap_err();
        assert_eq!(error.to_string(), "Name is required");
    }

    #[test]
    fn engines_serialize_as_the_ui_expects() {
        assert_eq!(serde_json::to_string(&Engine::Sqlserver).unwrap(), "\"sqlserver\"");
    }
}
