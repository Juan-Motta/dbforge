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
    Cosmosdb,
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
            Engine::Cosmosdb => "Azure Cosmos DB",
        }
    }
}

/// What an AI agent connected over MCP may do with a connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentAccess {
    /// Agents don't see the connection at all.
    #[default]
    None,
    /// Agents may read, on a session the server keeps from writing.
    Read,
    /// Agents may also change data; tags that ask before changes make them ask the user.
    Write,
}

/// A saved connection. The password is never part of it: it lives in the OS keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub engine: Engine,
    /// The id of its tag (see `tags`). Connections saved before tags hold
    /// `production`, `staging` or `development`, the default tags' ids.
    pub environment: Option<String>,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    /// Encrypts the connection (Redis, MongoDB and Cosmos DB, whose local emulator may use HTTP).
    #[serde(default)]
    pub tls: bool,
    /// Accepts a certificate no authority signed, as development servers have.
    #[serde(default)]
    pub allow_self_signed: bool,
    /// MongoDB: `host` names a DNS seed list (`mongodb+srv://`), which lists
    /// the servers and their port.
    #[serde(default)]
    pub srv: bool,
    /// What agents may do with it over MCP; none for connections saved before agents could.
    #[serde(default)]
    pub agent_access: AgentAccess,
}

/// What the UI sends to create, update or test a connection.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInput {
    /// Missing for a new connection.
    pub id: Option<String>,
    pub name: String,
    pub engine: Engine,
    pub environment: Option<String>,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    /// `None` keeps the stored password; `Some("")` clears it.
    pub password: Option<String>,
    #[serde(default)]
    pub tls: bool,
    #[serde(default)]
    pub allow_self_signed: bool,
    #[serde(default)]
    pub srv: bool,
    #[serde(default)]
    pub agent_access: AgentAccess,
}

impl ConnectionInput {
    /// The input for a connection test, which saves nothing: a name isn't needed yet.
    pub fn for_test(self) -> Self {
        if self.name.trim().is_empty() { Self { name: "Untitled".into(), ..self } } else { self }
    }

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
        let srv = self.engine == Engine::Mongodb && self.srv;
        let connection = Connection {
            id: self.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            engine: self.engine,
            environment: self.environment.filter(|tag| !tag.is_empty()),
            host: self.host.trim().to_string(),
            port: self.port,
            database: self.database.trim().to_string(),
            username: self.username.trim().to_string(),
            // A seed list is always reached over TLS, as mongodb+srv:// defines.
            tls: self.tls || srv,
            allow_self_signed: (self.tls || srv) && self.allow_self_signed,
            srv,
            agent_access: self.agent_access,
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
            tls: false,
            allow_self_signed: false,
            srv: false,
            agent_access: Default::default(),
        }
    }

    #[test]
    fn agents_get_no_access_until_given_some() {
        let saved = r#"{"id":"a","name":"n","engine":"postgres","environment":null,"host":"h","port":5432,"database":"","username":""}"#;
        let connection: Connection = serde_json::from_str(saved).unwrap();
        assert_eq!(connection.agent_access, AgentAccess::None);

        let (connection, _) =
            ConnectionInput { agent_access: AgentAccess::Write, ..input("n", None) }.into_parts().unwrap();
        assert_eq!(connection.agent_access, AgentAccess::Write);
        assert_eq!(serde_json::to_value(connection).unwrap()["agentAccess"], "write");
    }

    #[test]
    fn environments_saved_before_tags_are_the_default_tags_ids() {
        let json = r#"{"id":"a","name":"n","engine":"postgres","environment":"production","host":"h","port":5432,"database":"","username":""}"#;
        let connection: Connection = serde_json::from_str(json).unwrap();
        assert_eq!(connection.environment.as_deref(), Some("production"));
        let untagged = ConnectionInput { environment: Some(String::new()), ..input("r", None) };
        assert_eq!(untagged.into_parts().unwrap().0.environment, None);
    }

    #[test]
    fn connections_saved_before_tls_options_load_with_them_off() {
        let json = r#"{"id":"a","name":"n","engine":"redis","environment":null,"host":"h","port":6379,"database":"","username":""}"#;
        let connection: Connection = serde_json::from_str(json).unwrap();
        assert_eq!((connection.tls, connection.allow_self_signed, connection.srv), (false, false, false));
    }

    #[test]
    fn tls_options_only_apply_where_they_mean_something() {
        let redis = ConnectionInput {
            engine: Engine::Redis,
            tls: true,
            allow_self_signed: true,
            srv: true,
            agent_access: Default::default(),
            ..input("r", None)
        };
        let (connection, _) = redis.into_parts().unwrap();
        // SRV records are a MongoDB feature.
        assert_eq!((connection.tls, connection.allow_self_signed, connection.srv), (true, true, false));

        let plain = ConnectionInput { engine: Engine::Redis, tls: false, allow_self_signed: true, ..input("r", None) };
        assert!(!plain.into_parts().unwrap().0.allow_self_signed, "no certificate to accept without TLS");

        let atlas = ConnectionInput { engine: Engine::Mongodb, srv: true, tls: false, ..input("m", None) };
        let (connection, _) = atlas.into_parts().unwrap();
        assert_eq!((connection.srv, connection.tls), (true, true), "mongodb+srv always uses TLS");
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
    fn a_test_needs_no_name_but_saving_does() {
        assert!(input("  ", None).for_test().into_parts().is_ok());
        assert_eq!(input("local", None).for_test().name, "local");
        assert_eq!(input("  ", None).into_parts().unwrap_err().to_string(), "Name is required");
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
