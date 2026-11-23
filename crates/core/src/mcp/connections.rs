//! The saved connections, for agents: what a connection is, without its
//! secrets, and creating, changing or deleting one once the user approves.
//!
//! Agents never send nor see a secret. A connection's password and its SSH
//! password or key passphrase are typed by the user in the approval dialog,
//! and reach the store with the answer ([`ApprovalSecrets`]); on an update,
//! nothing typed keeps the saved ones. Agents can't choose what agents may do
//! with a connection, nor its tag: the user chooses a new connection's agent
//! access in the dialog (Read only unless changed), and an update keeps both.
//!
//! As everywhere agents go, a connection they may not use reads as unknown.
//! A connection with a console's transaction open is neither changed nor
//! deleted, as in the app: the transaction would be left behind.

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::approvals::{ApprovalRequest, ApprovalSecrets, ApprovalSubject, ConnectionChange, SecretFields};
use super::tools::Tools;
use crate::connections::model::{AgentAccess, Connection, ConnectionInput, ConnectionParts, Engine};
use crate::connections::transport::{SshAuth, SshTunnel, Ssl, SslMode};
use crate::events::Event;
use crate::query::open_transactions;

/// What the agent hears when the user says no, or nothing in time.
const DECLINED: &str = "The user declined this change in DBForge, or didn't answer in time. Nothing was changed.";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDetailsRequest {
    /// From `list_connections`.
    pub connection_id: String,
}

/// A connection's settings as agents see them: everything but its secrets.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDetails {
    /// What every other tool takes as `connectionId`.
    pub id: String,
    pub name: String,
    /// What `save_connection` takes as `engine`: `postgres`, `mysql`, `mariadb`,
    /// `sqlite`, `sqlserver`, `mongodb`, `redis` or `cosmosdb`.
    pub engine: String,
    /// The engine's name, e.g. PostgreSQL.
    pub engine_name: String,
    /// The server's host; a Cosmos DB account's; empty for SQLite.
    pub host: String,
    pub port: u16,
    /// The database it opens; for SQLite, the database file's path.
    pub database: String,
    pub username: String,
    pub ssl: SslView,
    pub ssh: SshView,
    /// MongoDB: `host` names a DNS seed list (`mongodb+srv://`).
    pub srv: bool,
    /// The tag the user gave it (e.g. PROD), if any.
    pub tag: Option<String>,
    /// What agents may do with it: `none`, `read` or `write`.
    pub access: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SslView {
    /// `disable`, `prefer`, `require`, `verify-ca` or `verify-full`.
    pub mode: String,
    /// Paths on the user's computer; empty when not used.
    pub ca_file: String,
    pub cert_file: String,
    pub key_file: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SshView {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// `password`, `key` or `agent` (the system's SSH agent).
    pub auth: String,
    /// The private key file's path, with `key`; empty otherwise.
    pub key_file: String,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EngineInput {
    Postgres,
    Mysql,
    Mariadb,
    Sqlite,
    Sqlserver,
    Mongodb,
    Redis,
    Cosmosdb,
}

impl From<EngineInput> for Engine {
    fn from(engine: EngineInput) -> Self {
        match engine {
            EngineInput::Postgres => Engine::Postgres,
            EngineInput::Mysql => Engine::Mysql,
            EngineInput::Mariadb => Engine::Mariadb,
            EngineInput::Sqlite => Engine::Sqlite,
            EngineInput::Sqlserver => Engine::Sqlserver,
            EngineInput::Mongodb => Engine::Mongodb,
            EngineInput::Redis => Engine::Redis,
            EngineInput::Cosmosdb => Engine::Cosmosdb,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SslModeInput {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl From<SslModeInput> for SslMode {
    fn from(mode: SslModeInput) -> Self {
        match mode {
            SslModeInput::Disable => SslMode::Disable,
            SslModeInput::Prefer => SslMode::Prefer,
            SslModeInput::Require => SslMode::Require,
            SslModeInput::VerifyCa => SslMode::VerifyCa,
            SslModeInput::VerifyFull => SslMode::VerifyFull,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SshAuthInput {
    /// The SSH account's password, which the user types when approving.
    Password,
    /// A private key file; its passphrase, if any, the user types when approving.
    Key,
    /// The keys the system's SSH agent holds.
    Agent,
}

impl From<SshAuthInput> for SshAuth {
    fn from(auth: SshAuthInput) -> Self {
        match auth {
            SshAuthInput::Password => SshAuth::Password,
            SshAuthInput::Key => SshAuth::Key,
            SshAuthInput::Agent => SshAuth::Agent,
        }
    }
}

/// SSL/TLS settings; what is left out keeps its current value (its default, for a new connection).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SslInput {
    pub mode: Option<SslModeInput>,
    /// Paths on the user's computer, in PEM.
    pub ca_file: Option<String>,
    pub cert_file: Option<String>,
    pub key_file: Option<String>,
}

/// An SSH tunnel; what is left out keeps its current value (its default, for a new connection).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SshInput {
    pub enabled: Option<bool>,
    pub host: Option<String>,
    /// 22 unless given.
    pub port: Option<u16>,
    pub username: Option<String>,
    pub auth: Option<SshAuthInput>,
    /// The private key file's path, with `key`.
    pub key_file: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveConnectionRequest {
    /// The connection to change (from `list_connections`); left out to create one.
    pub id: Option<String>,
    /// Needed to create one.
    pub name: Option<String>,
    /// Needed to create one.
    pub engine: Option<EngineInput>,
    /// The server's host (a Cosmos DB account's); not for SQLite.
    pub host: Option<String>,
    /// The engine's usual port unless given.
    pub port: Option<u16>,
    /// The database to open; for SQLite, the database file's path, needed.
    pub database: Option<String>,
    pub username: Option<String>,
    pub ssl: Option<SslInput>,
    pub ssh: Option<SshInput>,
    /// MongoDB: `host` names a DNS seed list.
    pub srv: Option<bool>,
    // Never taken: named so a refusal can say why rather than "unknown field".
    #[serde(default)]
    #[schemars(skip)]
    pub password: Option<Value>,
    #[serde(default)]
    #[schemars(skip)]
    pub ssh_secret: Option<Value>,
    #[serde(default)]
    #[schemars(skip)]
    pub ssh_password: Option<Value>,
    #[serde(default)]
    #[schemars(skip)]
    pub passphrase: Option<Value>,
    #[serde(default)]
    #[schemars(skip)]
    pub agent_access: Option<Value>,
    #[serde(default)]
    #[schemars(skip)]
    pub tag: Option<Value>,
    #[serde(default)]
    #[schemars(skip)]
    pub environment: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteConnectionRequest {
    /// From `list_connections`.
    pub id: String,
}

fn not_found(id: &str) -> String {
    format!("No connection with id {id}. Call list_connections to see the ones you can use.")
}

fn changed_meanwhile(name: &str) -> String {
    format!(
        "The connection {name} changed while waiting for approval, so nothing was changed. Call connection_details and try again."
    )
}

fn deleted_meanwhile(name: &str) -> String {
    format!("The connection {name} was deleted while waiting for approval, so nothing was changed.")
}

/// The app's own refusal, for a connection with a console's transaction open.
fn busy(name: &str) -> String {
    format!("A transaction is open on {name}: commit or roll back before editing or deleting the connection.")
}

fn access_name(access: AgentAccess) -> &'static str {
    match access {
        AgentAccess::None => "none",
        AgentAccess::Read => "read",
        AgentAccess::Write => "write",
    }
}

fn engine_id(engine: Engine) -> &'static str {
    match engine {
        Engine::Postgres => "postgres",
        Engine::Mysql => "mysql",
        Engine::Mariadb => "mariadb",
        Engine::Sqlite => "sqlite",
        Engine::Sqlserver => "sqlserver",
        Engine::Mongodb => "mongodb",
        Engine::Redis => "redis",
        Engine::Cosmosdb => "cosmosdb",
    }
}

fn ssl_mode_name(mode: SslMode) -> &'static str {
    match mode {
        SslMode::Disable => "disable",
        SslMode::Prefer => "prefer",
        SslMode::Require => "require",
        SslMode::VerifyCa => "verify-ca",
        SslMode::VerifyFull => "verify-full",
    }
}

fn ssh_auth_name(auth: SshAuth) -> &'static str {
    match auth {
        SshAuth::Password => "password",
        SshAuth::Key => "key",
        SshAuth::Agent => "agent",
    }
}

/// `connection`'s settings, without anything secret.
fn details(connection: Connection, tag: Option<String>) -> ConnectionDetails {
    let Connection { id, name, engine, host, port, database, username, ssl, ssh, srv, agent_access, .. } = connection;
    ConnectionDetails {
        id,
        name,
        engine: engine_id(engine).into(),
        engine_name: engine.label().into(),
        host,
        port,
        database,
        username,
        ssl: SslView {
            mode: ssl_mode_name(ssl.mode).into(),
            ca_file: ssl.ca_file,
            cert_file: ssl.cert_file,
            key_file: ssl.key_file,
        },
        ssh: SshView {
            enabled: ssh.enabled,
            host: ssh.host,
            port: ssh.port,
            username: ssh.username,
            auth: ssh_auth_name(ssh.auth).into(),
            key_file: ssh.key_file,
        },
        srv,
        tag,
        access: access_name(agent_access).into(),
    }
}

/// What a new connection of `engine` starts with: `prefer` where the engine has it, as in the app.
fn default_ssl(engine: Engine) -> Ssl {
    let mode = if engine.ssl_support().modes.contains(&SslMode::Prefer) { SslMode::Prefer } else { SslMode::Disable };
    Ssl { mode, ..Ssl::default() }
}

/// `input` over `ssl`.
fn merge_ssl(ssl: Ssl, input: Option<SslInput>) -> Ssl {
    let Some(input) = input else { return ssl };
    Ssl {
        mode: input.mode.map_or(ssl.mode, Into::into),
        ca_file: input.ca_file.unwrap_or(ssl.ca_file),
        cert_file: input.cert_file.unwrap_or(ssl.cert_file),
        key_file: input.key_file.unwrap_or(ssl.key_file),
    }
}

/// `input` over `ssh`.
fn merge_ssh(ssh: SshTunnel, input: Option<SshInput>) -> SshTunnel {
    let Some(input) = input else { return ssh };
    SshTunnel {
        enabled: input.enabled.unwrap_or(ssh.enabled),
        host: input.host.unwrap_or(ssh.host),
        port: input.port.unwrap_or(ssh.port),
        username: input.username.unwrap_or(ssh.username),
        auth: input.auth.map_or(ssh.auth, Into::into),
        key_file: input.key_file.unwrap_or(ssh.key_file),
    }
}

/// Refuses a request naming what agents may never give: secrets, agent access, a tag.
fn refuse_forbidden(request: &SaveConnectionRequest) -> Result<(), String> {
    let named = |fields: &[(&'static str, &Option<Value>)]| -> Vec<&'static str> {
        fields.iter().filter(|(_, value)| value.is_some()).map(|(name, _)| *name).collect()
    };
    let secrets = named(&[
        ("password", &request.password),
        ("sshSecret", &request.ssh_secret),
        ("sshPassword", &request.ssh_password),
        ("passphrase", &request.passphrase),
    ]);
    if !secrets.is_empty() {
        return Err(format!(
            "DBForge never takes passwords or other secrets from agents: the user types them in DBForge when approving. Leave {} out.",
            secrets.join(", ")
        ));
    }
    let settings =
        named(&[("agentAccess", &request.agent_access), ("tag", &request.tag), ("environment", &request.environment)]);
    if !settings.is_empty() {
        return Err(format!(
            "Agents can't set what agents may do with a connection, nor its tag: the user chooses them in DBForge. Leave {} out.",
            settings.join(", ")
        ));
    }
    Ok(())
}

impl Tools {
    /// Connection `id` when agents may use it; otherwise it reads as unknown.
    fn reachable_connection(&self, id: &str) -> Result<Connection, String> {
        self.core
            .connections()
            .get(id)
            .ok()
            .filter(|connection| connection.agent_access != AgentAccess::None)
            .ok_or_else(|| not_found(id))
    }

    /// Refuses while a console has a transaction open on `connection`.
    fn refuse_while_open(&self, connection: &Connection) -> Result<(), String> {
        let open = open_transactions(self.core.query_sessions());
        match open.iter().any(|open| open.connection_id == connection.id) {
            true => Err(busy(&connection.name)),
            false => Ok(()),
        }
    }

    /// Waits for the user to approve `change`; what they typed, or why nothing happens.
    async fn approve_connection(
        &self,
        agent: Option<String>,
        change: ConnectionChange,
    ) -> Result<ApprovalSecrets, String> {
        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            agent,
            subject: ApprovalSubject::Connection(Box::new(change)),
        };
        let answer = self.core.mcp_approvals().ask_for_answer(self.core.events().as_ref(), request).await;
        match answer.approved {
            true => Ok(answer.secrets.unwrap_or_default()),
            false => Err(DECLINED.into()),
        }
    }

    /// Re-reads connection `before` once approved: still as the dialog showed it, and free of transactions.
    fn unchanged_since(&self, before: &Connection) -> Result<(), String> {
        let now = self.core.connections().get(&before.id).map_err(|_| deleted_meanwhile(&before.name))?;
        if &now != before {
            return Err(changed_meanwhile(&before.name));
        }
        self.refuse_while_open(before)
    }

    pub(super) fn details_of(&self, request: ConnectionDetailsRequest) -> Result<ConnectionDetails, String> {
        let connection = self.reachable_connection(&request.connection_id)?;
        let tag = self.tag_name(&connection);
        Ok(details(connection, tag))
    }

    pub(super) async fn save_conn(
        &self,
        agent: Option<String>,
        request: SaveConnectionRequest,
    ) -> Result<ConnectionDetails, String> {
        refuse_forbidden(&request)?;
        match request.id.clone() {
            Some(id) => self.update_connection(agent, &id, request).await,
            None => self.create_connection(agent, request).await,
        }
    }

    async fn create_connection(
        &self,
        agent: Option<String>,
        request: SaveConnectionRequest,
    ) -> Result<ConnectionDetails, String> {
        let engine: Engine = request.engine.ok_or("Give engine: the new connection's engine.")?.into();
        let name = request.name.ok_or("Give name: the new connection's name.")?;
        let input = ConnectionInput {
            id: None,
            name,
            engine,
            environment: None,
            host: request.host.unwrap_or_default(),
            port: request.port.unwrap_or(engine.default_port()),
            database: request.database.unwrap_or_default(),
            username: request.username.unwrap_or_default(),
            password: None,
            ssl: merge_ssl(default_ssl(engine), request.ssl),
            ssh: merge_ssh(SshTunnel::default(), request.ssh),
            ssh_secret: None,
            srv: request.srv.unwrap_or(false),
            agent_access: AgentAccess::Read,
        };
        let connection = input.into_parts().map_err(|error| error.to_string())?.connection;
        let mut secrets =
            self.approve_connection(agent, ConnectionChange::Create { connection: connection.clone() }).await?;
        let connection = Connection { agent_access: secrets.agent_access.unwrap_or(AgentAccess::Read), ..connection };
        let asked = SecretFields::of(&connection);
        let parts = ConnectionParts {
            password: secrets.take_password().filter(|_| asked.password),
            ssh_secret: secrets.take_ssh_secret().filter(|_| asked.ssh.is_some()),
            connection,
        };
        drop(secrets);
        let saved = self.core.connections().save(parts).map_err(|error| error.to_string())?;
        self.core.events().emit(Event::ConnectionsChanged { id: saved.id.clone(), deleted: false });
        Ok(details(saved, None))
    }

    async fn update_connection(
        &self,
        agent: Option<String>,
        id: &str,
        request: SaveConnectionRequest,
    ) -> Result<ConnectionDetails, String> {
        let before = self.reachable_connection(id)?;
        let tag = self.tag_name(&before);
        let input = ConnectionInput {
            id: Some(before.id.clone()),
            name: request.name.unwrap_or_else(|| before.name.clone()),
            engine: request.engine.map_or(before.engine, Into::into),
            environment: before.environment.clone(),
            host: request.host.unwrap_or_else(|| before.host.clone()),
            port: request.port.unwrap_or(before.port),
            database: request.database.unwrap_or_else(|| before.database.clone()),
            username: request.username.unwrap_or_else(|| before.username.clone()),
            password: None,
            ssl: merge_ssl(before.ssl.clone(), request.ssl),
            ssh: merge_ssh(before.ssh.clone(), request.ssh),
            ssh_secret: None,
            srv: request.srv.unwrap_or(before.srv),
            agent_access: before.agent_access,
        };
        let after = input.into_parts().map_err(|error| error.to_string())?.connection;
        if after == before {
            // Nothing to approve: the secrets stay as they are too.
            return Ok(details(before, tag));
        }
        self.refuse_while_open(&before)?;
        let change = ConnectionChange::Update { before: before.clone(), after: after.clone(), tag: tag.clone() };
        let mut secrets = self.approve_connection(agent, change).await?;
        self.unchanged_since(&before)?;
        let asked = SecretFields::of(&after);
        let parts = ConnectionParts {
            password: secrets.take_password().filter(|_| asked.password),
            ssh_secret: secrets.take_ssh_secret().filter(|_| asked.ssh.is_some()),
            connection: after,
        };
        drop(secrets);
        let saved = self.core.connections().save(parts).map_err(|error| error.to_string())?;
        // A tunnel opened with the old settings or secret is not reused.
        crate::tunnel::close(&saved.id).await;
        self.core.events().emit(Event::ConnectionsChanged { id: saved.id.clone(), deleted: false });
        Ok(details(saved, tag))
    }

    pub(super) async fn delete_conn(
        &self,
        agent: Option<String>,
        request: DeleteConnectionRequest,
    ) -> Result<String, String> {
        let connection = self.reachable_connection(&request.id)?;
        self.refuse_while_open(&connection)?;
        let tag = self.tag_name(&connection);
        let change = ConnectionChange::Delete { connection: connection.clone(), tag };
        // A delete takes no secrets: whatever came is dropped unread.
        drop(self.approve_connection(agent, change).await?);
        self.unchanged_since(&connection)?;
        crate::connections::commands::delete_connection(self.core.connections(), connection.id.clone())
            .await
            .map_err(|error| error.to_string())?;
        self.core.events().emit(Event::ConnectionsChanged { id: connection.id, deleted: true });
        Ok(format!("Deleted the connection {}.", connection.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::transport::SshAuth;
    use crate::mcp::saved::tests::{Setup, setup};
    use crate::query::{QueryScope, TransactionMode, run_query, set_transaction_mode};

    /// A request saving nothing yet: fields are set by each test.
    fn request(id: Option<&str>) -> SaveConnectionRequest {
        serde_json::from_value(match id {
            Some(id) => serde_json::json!({ "id": id }),
            None => serde_json::json!({}),
        })
        .unwrap()
    }

    fn create(name: &str) -> SaveConnectionRequest {
        SaveConnectionRequest {
            name: Some(name.into()),
            engine: Some(EngineInput::Postgres),
            host: Some("db.example.com".into()),
            database: Some("shop".into()),
            username: Some("app".into()),
            ..request(None)
        }
    }

    /// A Postgres connection named `name` with a password and an SSH passphrase saved.
    fn with_secrets(s: &Setup, name: &str, access: AgentAccess) -> Connection {
        let connection = Connection {
            id: name.into(),
            name: name.into(),
            engine: Engine::Postgres,
            environment: None,
            host: "db.example.com".into(),
            port: 5432,
            database: "shop".into(),
            username: "app".into(),
            ssl: Ssl { mode: SslMode::Require, ca_file: "/certs/ca.pem".into(), ..Ssl::default() },
            ssh: SshTunnel {
                enabled: true,
                host: "bastion".into(),
                port: 22,
                username: "jump".into(),
                auth: SshAuth::Key,
                key_file: "/keys/id_ed25519".into(),
            },
            srv: false,
            agent_access: access,
        };
        let parts = ConnectionParts {
            connection: connection.clone(),
            password: Some("pg-s3cret".into()),
            ssh_secret: Some("key-passphrase".into()),
        };
        s.core().connections().save(parts).unwrap()
    }

    fn typed(password: &str, ssh: &str, access: Option<AgentAccess>) -> ApprovalSecrets {
        ApprovalSecrets { password: Some(password.into()), ssh_secret: Some(ssh.into()), agent_access: access }
    }

    /// Runs `work`, answering each approval as `answer` says (approve, and what the user typed).
    async fn answering<T>(
        s: &mut Setup,
        answer: impl Fn(&ApprovalRequest) -> (bool, Option<ApprovalSecrets>),
        work: impl Future<Output = T>,
    ) -> (T, Vec<ApprovalRequest>) {
        s.answering_with(answer, work).await
    }

    fn changed(s: &Setup) -> Vec<(String, bool)> {
        s.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::ConnectionsChanged { id, deleted } => Some((id, deleted)),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn details_never_hold_a_secret() {
        let s = setup();
        let shop = with_secrets(&s, "shop", AgentAccess::Read);
        let shown = s.tools().details_of(ConnectionDetailsRequest { connection_id: shop.id }).unwrap();
        let json = serde_json::to_string(&shown).unwrap();
        for secret in ["pg-s3cret", "key-passphrase"] {
            assert!(!json.contains(secret), "{json}");
        }
        assert_eq!(shown.engine, "postgres");
        assert_eq!((shown.host.as_str(), shown.port, shown.username.as_str()), ("db.example.com", 5432, "app"));
        assert_eq!((shown.ssl.mode.as_str(), shown.ssl.ca_file.as_str()), ("require", "/certs/ca.pem"));
        assert_eq!((shown.ssh.auth.as_str(), shown.ssh.key_file.as_str()), ("key", "/keys/id_ed25519"));
        assert_eq!(shown.access, "read");
    }

    #[tokio::test]
    async fn a_connection_out_of_reach_reads_as_unknown() {
        let mut s = setup();
        let hidden = with_secrets(&s, "hidden", AgentAccess::None);
        let tools = s.tools();
        let unknown = not_found("hidden");
        let details = tools.details_of(ConnectionDetailsRequest { connection_id: hidden.id.clone() });
        assert_eq!(details.unwrap_err(), unknown);
        let rename = SaveConnectionRequest { name: Some("Mine".into()), ..request(Some("hidden")) };
        let (updated, asked) = answering(&mut s, |_| (true, None), tools.save_conn(None, rename)).await;
        assert_eq!(updated.unwrap_err(), unknown);
        let delete = tools.delete_conn(None, DeleteConnectionRequest { id: "hidden".into() });
        let (deleted, asked_delete) = answering(&mut s, |_| (true, None), delete).await;
        assert_eq!(deleted.unwrap_err(), unknown);
        assert!(asked.is_empty() && asked_delete.is_empty());
        assert_eq!(s.core().connections().get("hidden").unwrap(), hidden);
    }

    #[tokio::test]
    async fn creating_asks_and_saves_what_the_user_typed_never_echoing_it() {
        let mut s = setup();
        let tools = s.tools();
        let (created, asked) = answering(
            &mut s,
            |_| (true, Some(typed("typed-pw", "ignored", Some(AgentAccess::Write)))),
            tools.save_conn(Some("codex".into()), create("Shop")),
        )
        .await;
        let created = created.unwrap();
        let [request] = asked.as_slice() else { panic!("asked once: {asked:?}") };
        let ApprovalSubject::Connection(change) = &request.subject else { panic!("{request:?}") };
        let ConnectionChange::Create { connection } = change.as_ref() else { panic!("{change:?}") };
        assert_eq!(connection.agent_access, AgentAccess::Read, "the dialog starts on Read only");
        assert_eq!(change.secrets(), SecretFields { password: true, ssh: None });

        let saved = s.core().connections().get(&created.id).unwrap();
        assert_eq!(saved.agent_access, AgentAccess::Write, "as the user chose");
        assert_eq!((saved.name.as_str(), saved.port, saved.ssl.mode), ("Shop", 5432, SslMode::Prefer));
        assert_eq!(saved.environment, None);
        assert_eq!(s.core().connections().password(&saved.id).unwrap().as_deref(), Some("typed-pw"));
        assert_eq!(s.core().connections().ssh_secret(&saved.id).unwrap(), None, "no tunnel, no SSH secret");
        let json = serde_json::to_string(&created).unwrap();
        assert!(!json.contains("typed-pw"), "{json}");
        assert_eq!(changed(&s), [(saved.id.clone(), false)]);
    }

    #[tokio::test]
    async fn a_declined_or_unanswered_create_saves_nothing() {
        let mut s = setup();
        let tools = s.tools();
        let (declined, asked) = answering(
            &mut s,
            |_| (false, Some(typed("pw", "x", Some(AgentAccess::Write)))),
            tools.save_conn(None, create("Shop")),
        )
        .await;
        assert_eq!(declined.unwrap_err(), DECLINED);
        assert_eq!(asked.len(), 1);
        // The agent goes away before the user answers.
        let gone =
            tokio::time::timeout(std::time::Duration::from_millis(20), tools.save_conn(None, create("Other"))).await;
        assert!(gone.is_err());
        assert!(s.core().connections().list().is_empty());
        assert!(changed(&s).is_empty());
    }

    #[tokio::test]
    async fn agents_can_t_send_secrets_access_or_a_tag() {
        let mut s = setup();
        let shop = with_secrets(&s, "shop", AgentAccess::Write);
        let tools = s.tools();
        let given = [
            serde_json::json!({ "name": "x", "engine": "postgres", "password": "p" }),
            serde_json::json!({ "id": "shop", "sshPassword": "p" }),
            serde_json::json!({ "id": "shop", "agentAccess": "write" }),
            serde_json::json!({ "name": "x", "engine": "postgres", "agentAccess": "read" }),
            serde_json::json!({ "id": "shop", "tag": "PROD" }),
            serde_json::json!({ "id": "shop", "environment": "production" }),
        ];
        for value in given {
            let request: SaveConnectionRequest = serde_json::from_value(value.clone()).unwrap();
            let (refused, asked) = answering(&mut s, |_| (true, None), tools.save_conn(None, request)).await;
            assert!(refused.is_err(), "{value}");
            assert!(asked.is_empty(), "refused before asking: {value}");
        }
        for unknown in [
            serde_json::json!({ "id": "shop", "token": "t" }),
            serde_json::json!({ "id": "shop", "ssh": { "password": "p" } }),
        ] {
            assert!(serde_json::from_value::<SaveConnectionRequest>(unknown.clone()).is_err(), "{unknown}");
        }
        assert_eq!(s.core().connections().get("shop").unwrap(), shop);
    }

    #[tokio::test]
    async fn an_update_shows_before_and_after_and_keeps_the_secrets_unless_typed() {
        let mut s = setup();
        let shop = with_secrets(&s, "shop", AgentAccess::Read);
        let tools = s.tools();
        let rename = || SaveConnectionRequest { name: Some("Shop EU".into()), ..request(Some("shop")) };
        let (renamed, asked) = answering(&mut s, |_| (true, None), tools.save_conn(None, rename())).await;
        assert_eq!(renamed.unwrap().name, "Shop EU");
        let [asked] = asked.as_slice() else { panic!("{asked:?}") };
        let ApprovalSubject::Connection(change) = &asked.subject else { panic!() };
        assert_eq!(change.changed_fields(), [crate::mcp::ConnectionField::Name]);
        assert_eq!(change.secrets(), SecretFields { password: true, ssh: Some(SshAuth::Key) });
        let store = s.core().connections();
        assert_eq!(store.get("shop").unwrap(), Connection { name: "Shop EU".into(), ..shop.clone() });
        assert_eq!(store.password("shop").unwrap().as_deref(), Some("pg-s3cret"), "kept");
        assert_eq!(store.ssh_secret("shop").unwrap().as_deref(), Some("key-passphrase"), "kept");

        let port = SaveConnectionRequest { port: Some(6543), ..request(Some("shop")) };
        let (moved, _) =
            answering(&mut s, |_| (true, Some(typed("new-pw", "", None))), tools.save_conn(None, port)).await;
        assert_eq!(moved.unwrap().port, 6543);
        let store = s.core().connections();
        assert_eq!(store.password("shop").unwrap().as_deref(), Some("new-pw"), "typed");
        assert_eq!(store.ssh_secret("shop").unwrap().as_deref(), Some("key-passphrase"), "empty keeps it");
        assert_eq!(store.get("shop").unwrap().agent_access, AgentAccess::Read, "an update keeps the access");
        assert_eq!(changed(&s), [("shop".into(), false), ("shop".into(), false)]);
    }

    #[tokio::test]
    async fn an_update_that_changes_nothing_asks_nothing() {
        let mut s = setup();
        let shop = with_secrets(&s, "shop", AgentAccess::Read);
        let tools = s.tools();
        let same = SaveConnectionRequest { name: Some(" shop ".into()), ..request(Some("shop")) };
        let (kept, asked) = answering(&mut s, |_| (true, None), tools.save_conn(None, same)).await;
        assert_eq!(kept.unwrap().id, "shop");
        assert!(asked.is_empty());
        assert_eq!(s.core().connections().get("shop").unwrap(), shop);
    }

    #[tokio::test]
    async fn a_declined_update_or_delete_changes_nothing() {
        let mut s = setup();
        let shop = with_secrets(&s, "shop", AgentAccess::Write);
        let tools = s.tools();
        let rename = SaveConnectionRequest { name: Some("Other".into()), ..request(Some("shop")) };
        let (declined, _) =
            answering(&mut s, |_| (false, Some(typed("x", "y", None))), tools.save_conn(None, rename)).await;
        assert_eq!(declined.unwrap_err(), DECLINED);
        let delete = tools.delete_conn(None, DeleteConnectionRequest { id: "shop".into() });
        let (declined, asked) = answering(&mut s, |_| (false, None), delete).await;
        assert_eq!(declined.unwrap_err(), DECLINED);
        assert!(matches!(
            &asked[0].subject,
            ApprovalSubject::Connection(change) if matches!(change.as_ref(), ConnectionChange::Delete { .. })
        ));
        let store = s.core().connections();
        assert_eq!(store.get("shop").unwrap(), shop);
        assert_eq!(store.password("shop").unwrap().as_deref(), Some("pg-s3cret"));
        assert!(changed(&s).is_empty());
    }

    #[tokio::test]
    async fn deleting_asks_and_removes_the_connection_and_its_secrets() {
        let mut s = setup();
        with_secrets(&s, "shop", AgentAccess::Read);
        let tools = s.tools();
        let delete = tools.delete_conn(None, DeleteConnectionRequest { id: "shop".into() });
        let (deleted, asked) = answering(&mut s, |_| (true, None), delete).await;
        assert_eq!(deleted.unwrap(), "Deleted the connection shop.");
        assert_eq!(asked.len(), 1);
        let store = s.core().connections();
        assert!(store.get("shop").is_err());
        assert_eq!(store.password("shop").unwrap(), None);
        assert_eq!(store.ssh_secret("shop").unwrap(), None);
        assert_eq!(changed(&s), [("shop".into(), true)]);
    }

    #[tokio::test]
    async fn a_change_made_while_the_approval_waits_wins() {
        let mut s = setup();
        let shop = with_secrets(&s, "shop", AgentAccess::Read);
        let tools = s.tools();
        let core = s.tools();
        let edited = Connection { host: "elsewhere".into(), ..shop.clone() };
        let rename = SaveConnectionRequest { name: Some("Agent's".into()), ..request(Some("shop")) };
        let (refused, _) = answering(
            &mut s,
            |_| {
                core.core.connections().upsert(edited.clone()).unwrap();
                (true, Some(typed("agent-time", "", None)))
            },
            tools.save_conn(None, rename),
        )
        .await;
        assert_eq!(refused.unwrap_err(), changed_meanwhile("shop"));
        let store = s.core().connections();
        assert_eq!(store.get("shop").unwrap(), edited, "the user's edit is kept");
        assert_eq!(store.password("shop").unwrap().as_deref(), Some("pg-s3cret"));

        let delete = tools.delete_conn(None, DeleteConnectionRequest { id: "shop".into() });
        let moved = Connection { port: 1, ..edited.clone() };
        let (refused, _) = answering(
            &mut s,
            |_| {
                core.core.connections().upsert(moved.clone()).unwrap();
                (true, None)
            },
            delete,
        )
        .await;
        assert_eq!(refused.unwrap_err(), changed_meanwhile("shop"));
        assert_eq!(s.core().connections().get("shop").unwrap(), moved, "not deleted");
    }

    #[tokio::test]
    async fn a_connection_with_an_open_transaction_is_neither_changed_nor_deleted() {
        let mut s = setup();
        let local = s.connection("local", Engine::Sqlite, AgentAccess::Write, None);
        let core = s.core();
        set_transaction_mode(core.query_sessions(), "c1", TransactionMode::Manual).unwrap();
        run_query(
            core.connections(),
            core.query_sessions(),
            core.history(),
            core.statement_log(),
            "c1".into(),
            local.id.clone(),
            QueryScope::default(),
            "CREATE TABLE t (id INTEGER)".into(),
        )
        .await
        .unwrap();
        assert!(!open_transactions(core.query_sessions()).is_empty());
        let tools = s.tools();
        let busy_message = busy("local");
        let delete = tools.delete_conn(None, DeleteConnectionRequest { id: "local".into() });
        let (refused, asked) = answering(&mut s, |_| (true, None), delete).await;
        assert_eq!(refused.unwrap_err(), busy_message);
        let rename = SaveConnectionRequest { name: Some("Renamed".into()), ..request(Some("local")) };
        let (refused_update, asked_update) = answering(&mut s, |_| (true, None), tools.save_conn(None, rename)).await;
        assert_eq!(refused_update.unwrap_err(), busy_message);
        assert!(asked.is_empty() && asked_update.is_empty(), "refused before asking");
        assert_eq!(s.core().connections().get("local").unwrap(), local);
        crate::query::close_session(s.core().query_sessions(), s.core().statement_log(), "c1".into()).await.unwrap();
    }
}
