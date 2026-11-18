use serde::{Deserialize, Serialize};

use super::transport::{SshAuth, SshTunnel, Ssl, SslMode};
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

/// A saved connection. Its password, and its SSH password or passphrase, are
/// never part of it: they are kept where the user chose (see `credentials`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "StoredConnection")]
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
    /// How SSL/TLS is used and verified.
    pub ssl: Ssl,
    /// The SSH tunnel the server is reached through, when enabled.
    pub ssh: SshTunnel,
    /// MongoDB: `host` names a DNS seed list (`mongodb+srv://`), which lists
    /// the servers and their port.
    pub srv: bool,
    /// What agents may do with it over MCP; none for connections saved before agents could.
    pub agent_access: AgentAccess,
}

impl Connection {
    /// The id its SSH password or passphrase is saved under, beside its password.
    pub fn ssh_credential_id(id: &str) -> String {
        format!("{id}.ssh")
    }
}

/// A connection as saved, by this version or an older one: connections saved
/// before SSL modes have `tls` and `allowSelfSigned` flags instead, and load
/// with the mode that behaves as they did.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredConnection {
    id: String,
    name: String,
    engine: Engine,
    environment: Option<String>,
    host: String,
    port: u16,
    database: String,
    username: String,
    #[serde(default)]
    ssl: Option<Ssl>,
    #[serde(default)]
    ssh: SshTunnel,
    #[serde(default)]
    tls: bool,
    #[serde(default)]
    allow_self_signed: bool,
    #[serde(default)]
    srv: bool,
    #[serde(default)]
    agent_access: AgentAccess,
}

impl From<StoredConnection> for Connection {
    fn from(stored: StoredConnection) -> Self {
        let ssl = stored.ssl.unwrap_or_else(|| Ssl {
            mode: stored.engine.legacy_ssl_mode(stored.tls || stored.srv, stored.allow_self_signed),
            ..Ssl::default()
        });
        Connection {
            id: stored.id,
            name: stored.name,
            engine: stored.engine,
            environment: stored.environment,
            host: stored.host,
            port: stored.port,
            database: stored.database,
            username: stored.username,
            ssl,
            ssh: stored.ssh,
            srv: stored.srv,
            agent_access: stored.agent_access,
        }
    }
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
    pub ssl: Ssl,
    #[serde(default)]
    pub ssh: SshTunnel,
    /// The SSH password or key passphrase, as `password` is kept or cleared.
    #[serde(default)]
    pub ssh_secret: Option<String>,
    #[serde(default)]
    pub srv: bool,
    #[serde(default)]
    pub agent_access: AgentAccess,
}

/// A validated input: the connection to save and its secrets, each `None` to
/// keep the saved one and empty to clear it.
#[derive(Debug)]
pub struct ConnectionParts {
    pub connection: Connection,
    pub password: Option<String>,
    pub ssh_secret: Option<String>,
}

impl ConnectionInput {
    /// The input for a connection test, which saves nothing: a name isn't needed yet.
    pub fn for_test(self) -> Self {
        if self.name.trim().is_empty() { Self { name: "Untitled".into(), ..self } } else { self }
    }

    /// Validates the input and splits it into the connection to save and its secrets.
    pub fn into_parts(self) -> Result<ConnectionParts> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Invalid("Name is required".into()));
        }
        // A SQLite connection is its file: without it there is nothing to open.
        if self.engine == Engine::Sqlite && self.database.trim().is_empty() {
            return Err(AppError::Invalid("Choose the SQLite database file".into()));
        }
        let srv = self.engine == Engine::Mongodb && self.srv;
        let host = self.host.trim().to_string();
        let ssh = tunnel(self.engine, &host, srv, self.ssh)?;
        let ssl = ssl(self.engine, srv, self.ssl, ssh.enabled)?;
        let connection = Connection {
            id: self.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            engine: self.engine,
            environment: self.environment.filter(|tag| !tag.is_empty()),
            host,
            port: self.port,
            database: self.database.trim().to_string(),
            username: self.username.trim().to_string(),
            ssl,
            ssh,
            srv,
            agent_access: self.agent_access,
        };
        Ok(ConnectionParts { connection, password: self.password, ssh_secret: self.ssh_secret })
    }
}

/// The SSL settings to save: a mode the engine supports, and only the files it can use.
fn ssl(engine: Engine, srv: bool, ssl: Ssl, through_ssh: bool) -> Result<Ssl> {
    let support = engine.ssl_support();
    // A seed list is always reached over TLS, as mongodb+srv:// defines.
    let mode = if srv && !ssl.mode.encrypts() { SslMode::VerifyFull } else { ssl.mode };
    if !support.modes.contains(&mode) {
        return Err(AppError::Invalid(format!("{} can't use the SSL mode {}", engine.label(), mode.label())));
    }
    if through_ssh && mode == SslMode::VerifyFull && !support.verifies_name_through_ssh {
        return Err(AppError::Invalid(format!(
            "Through an SSH tunnel, {} can't check the server's name: choose another SSL mode",
            engine.label()
        )));
    }
    let file = |path: String, used: bool| if used { path.trim().to_string() } else { String::new() };
    // Without encryption there is no certificate to check or present.
    let ca_file = file(ssl.ca_file, support.ca_file && mode.encrypts());
    let cert_file = file(ssl.cert_file, support.client_cert && mode.encrypts());
    let key_file = file(ssl.key_file, support.client_cert && support.separate_key && mode.encrypts());
    if support.separate_key && cert_file.is_empty() != key_file.is_empty() {
        return Err(AppError::Invalid("Choose both the client certificate and its key, or neither".into()));
    }
    Ok(Ssl { mode, ca_file, cert_file, key_file })
}

/// The SSH tunnel to save: off where the engine has no server to tunnel to,
/// and complete when on.
fn tunnel(engine: Engine, host: &str, srv: bool, ssh: SshTunnel) -> Result<SshTunnel> {
    if !engine.tunnels() || !ssh.enabled {
        // Kept as typed, so turning the tunnel on again brings its settings back.
        return Ok(SshTunnel { enabled: false, ..ssh });
    }
    if srv || host.contains("://") {
        return Err(AppError::Invalid(
            "Through an SSH tunnel, enter the server's host and port rather than a URL or a seed list".into(),
        ));
    }
    let ssh = SshTunnel {
        host: ssh.host.trim().to_string(),
        username: ssh.username.trim().to_string(),
        key_file: if ssh.auth == SshAuth::Key { ssh.key_file.trim().to_string() } else { String::new() },
        ..ssh
    };
    if ssh.host.is_empty() {
        return Err(AppError::Invalid("Enter the SSH server's host".into()));
    }
    if ssh.username.is_empty() {
        return Err(AppError::Invalid("Enter the SSH user".into()));
    }
    if ssh.auth == SshAuth::Key && ssh.key_file.is_empty() {
        return Err(AppError::Invalid("Choose the SSH private key file".into()));
    }
    Ok(ssh)
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
            ssl: Ssl::default(),
            ssh: SshTunnel::default(),
            ssh_secret: None,
            srv: false,
            agent_access: Default::default(),
        }
    }

    #[test]
    fn agents_get_no_access_until_given_some() {
        let saved = r#"{"id":"a","name":"n","engine":"postgres","environment":null,"host":"h","port":5432,"database":"","username":""}"#;
        let connection: Connection = serde_json::from_str(saved).unwrap();
        assert_eq!(connection.agent_access, AgentAccess::None);

        let connection =
            ConnectionInput { agent_access: AgentAccess::Write, ..input("n", None) }.into_parts().unwrap().connection;
        assert_eq!(connection.agent_access, AgentAccess::Write);
        assert_eq!(serde_json::to_value(connection).unwrap()["agentAccess"], "write");
    }

    #[test]
    fn environments_saved_before_tags_are_the_default_tags_ids() {
        let json = r#"{"id":"a","name":"n","engine":"postgres","environment":"production","host":"h","port":5432,"database":"","username":""}"#;
        let connection: Connection = serde_json::from_str(json).unwrap();
        assert_eq!(connection.environment.as_deref(), Some("production"));
        let untagged = ConnectionInput { environment: Some(String::new()), ..input("r", None) };
        assert_eq!(untagged.into_parts().unwrap().connection.environment, None);
    }

    fn load(engine: &str, extra: &str) -> Connection {
        let json = format!(
            r#"{{"id":"a","name":"n","engine":"{engine}","environment":null,"host":"h","port":1,"database":"","username":""{extra}}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn connections_saved_before_ssl_modes_keep_behaving_as_they_did() {
        assert_eq!(load("postgres", "").ssl.mode, SslMode::Prefer);
        assert_eq!(load("mysql", r#","tls":false"#).ssl.mode, SslMode::Prefer);
        assert_eq!(load("sqlserver", "").ssl.mode, SslMode::Require);
        assert_eq!(load("redis", "").ssl.mode, SslMode::Disable);
        assert_eq!(load("redis", r#","tls":true,"allowSelfSigned":true"#).ssl.mode, SslMode::Require);
        assert_eq!(load("mongodb", r#","tls":true"#).ssl.mode, SslMode::VerifyFull);
        assert_eq!(load("mongodb", r#","srv":true"#).ssl.mode, SslMode::VerifyFull);
        assert_eq!(load("cosmosdb", r#","tls":true,"allowSelfSigned":true"#).ssl.mode, SslMode::Require);
        let old = load("redis", "");
        assert_eq!((old.ssh.enabled, old.ssh.port, old.srv), (false, 22, false));
    }

    #[test]
    fn connections_save_their_ssl_and_ssh_settings_and_load_them_back() {
        let saved = load(
            "postgres",
            r#","ssl":{"mode":"verify-ca","caFile":"/ca.pem"},"ssh":{"enabled":true,"host":"b","port":2222,"username":"u","auth":"agent"}"#,
        );
        assert_eq!(saved.ssl, Ssl { mode: SslMode::VerifyCa, ca_file: "/ca.pem".into(), ..Ssl::default() });
        assert_eq!((saved.ssh.port, saved.ssh.auth), (2222, SshAuth::Agent));
        let json = serde_json::to_value(&saved).unwrap();
        assert!(json.get("tls").is_none(), "the old flags are not written again");
        assert_eq!(serde_json::from_value::<Connection>(json).unwrap(), saved);
    }

    fn with_ssl(engine: Engine, mode: SslMode) -> ConnectionInput {
        ConnectionInput { engine, ssl: Ssl { mode, ..Ssl::default() }, ..input("c", None) }
    }

    fn files(ca: &str, cert: &str, key: &str) -> Ssl {
        Ssl { mode: SslMode::VerifyCa, ca_file: ca.into(), cert_file: cert.into(), key_file: key.into() }
    }

    fn bastion(auth: SshAuth) -> SshTunnel {
        SshTunnel { enabled: true, host: " bastion ".into(), username: " ops ".into(), auth, ..SshTunnel::default() }
    }

    #[test]
    fn an_engine_only_takes_the_ssl_modes_its_driver_supports() {
        let error = with_ssl(Engine::Redis, SslMode::Prefer).into_parts().unwrap_err();
        assert_eq!(error.to_string(), "Redis can't use the SSL mode prefer");
        assert!(with_ssl(Engine::Sqlserver, SslMode::VerifyCa).into_parts().is_err());
        assert!(with_ssl(Engine::Sqlite, SslMode::Require).into_parts().is_err());
        assert!(with_ssl(Engine::Postgres, SslMode::VerifyFull).into_parts().is_ok());
    }

    #[test]
    fn certificate_files_are_kept_only_where_they_are_used() {
        let ssl = ConnectionInput { ssl: files(" /ca ", "/c", "/k"), ..input("c", None) }.into_parts().unwrap();
        assert_eq!(ssl.connection.ssl, files("/ca", "/c", "/k"));

        let off =
            ConnectionInput { ssl: Ssl { mode: SslMode::Disable, ..files("/ca", "/c", "/k") }, ..input("c", None) };
        assert_eq!(off.into_parts().unwrap().connection.ssl, Ssl::default(), "nothing to check without TLS");

        let mongo = ConnectionInput {
            engine: Engine::Mongodb,
            ssl: Ssl { mode: SslMode::Require, ..files("/ca", "/both.pem", "/k") },
            ..input("m", None)
        };
        let ssl = mongo.into_parts().unwrap().connection.ssl;
        assert_eq!((ssl.cert_file.as_str(), ssl.key_file.as_str()), ("/both.pem", ""), "one file holds both");

        let half = ConnectionInput { ssl: files("", "/c", ""), ..input("c", None) };
        assert_eq!(
            half.into_parts().unwrap_err().to_string(),
            "Choose both the client certificate and its key, or neither"
        );
    }

    #[test]
    fn a_seed_list_always_uses_tls() {
        let atlas = ConnectionInput { srv: true, ..with_ssl(Engine::Mongodb, SslMode::Disable) };
        let connection = atlas.into_parts().unwrap().connection;
        assert_eq!((connection.srv, connection.ssl.mode), (true, SslMode::VerifyFull));
        let redis = ConnectionInput { srv: true, ..with_ssl(Engine::Redis, SslMode::Disable) };
        let connection = redis.into_parts().unwrap().connection;
        assert_eq!((connection.srv, connection.ssl.mode), (false, SslMode::Disable), "SRV records are MongoDB's");
    }

    #[test]
    fn an_ssh_tunnel_needs_its_server_user_and_key() {
        let parts = ConnectionInput { ssh: bastion(SshAuth::Agent), ..input("c", None) }.into_parts().unwrap();
        assert_eq!((parts.connection.ssh.host.as_str(), parts.connection.ssh.username.as_str()), ("bastion", "ops"));

        let no_key = ConnectionInput { ssh: bastion(SshAuth::Key), ..input("c", None) };
        assert_eq!(no_key.into_parts().unwrap_err().to_string(), "Choose the SSH private key file");
        let no_host =
            ConnectionInput { ssh: SshTunnel { host: " ".into(), ..bastion(SshAuth::Password) }, ..input("c", None) };
        assert_eq!(no_host.into_parts().unwrap_err().to_string(), "Enter the SSH server's host");

        let key = SshTunnel { key_file: "/id".into(), ..bastion(SshAuth::Password) };
        let parts =
            ConnectionInput { ssh: key, ssh_secret: Some("pw".into()), ..input("c", None) }.into_parts().unwrap();
        assert_eq!((parts.connection.ssh.key_file.as_str(), parts.ssh_secret.as_deref()), ("", Some("pw")));
    }

    #[test]
    fn a_tunnel_reaches_a_host_not_a_url_nor_a_file() {
        let url = ConnectionInput { host: "redis://h:6379".into(), ssh: bastion(SshAuth::Agent), ..input("c", None) };
        assert!(url.into_parts().unwrap_err().to_string().contains("rather than a URL"));

        let sqlite = ConnectionInput { engine: Engine::Sqlite, ssh: bastion(SshAuth::Agent), ..input("c", None) };
        let ssh = sqlite.into_parts().unwrap().connection.ssh;
        assert!(!ssh.enabled, "a file has no server");
        assert_eq!(ssh.host, " bastion ", "kept as typed while off");
    }

    #[test]
    fn through_ssh_the_server_name_is_checked_only_where_the_driver_can() {
        let postgres =
            ConnectionInput { ssh: bastion(SshAuth::Agent), ..with_ssl(Engine::Postgres, SslMode::VerifyFull) };
        assert!(postgres.into_parts().unwrap_err().to_string().contains("can't check the server's name"));
        let ca = ConnectionInput { ssh: bastion(SshAuth::Agent), ..with_ssl(Engine::Postgres, SslMode::VerifyCa) };
        assert!(ca.into_parts().is_ok());
        let sqlserver =
            ConnectionInput { ssh: bastion(SshAuth::Agent), ..with_ssl(Engine::Sqlserver, SslMode::VerifyFull) };
        assert!(sqlserver.into_parts().is_ok());
    }

    #[test]
    fn new_connection_gets_an_id_and_trimmed_fields() {
        let ConnectionParts { connection, password, .. } = input(" local ", None).into_parts().unwrap();
        assert!(!connection.id.is_empty());
        assert_eq!(connection.name, "local");
        assert_eq!(connection.host, "localhost");
        assert_eq!(password.as_deref(), Some("secret"));
    }

    #[test]
    fn existing_connection_keeps_its_id() {
        let connection = input("local", Some("abc")).into_parts().unwrap().connection;
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
