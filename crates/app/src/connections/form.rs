//! The connection form's values and rules, apart from drawing them: what a
//! new or saved connection fills in, what switching engine changes, when it
//! can be tested or saved, and the input sent to the core. Passwords are only
//! what the user typed: empty keeps the saved one, which is never read back.

use dbforge_core::connections::model::{AgentAccess, Connection, ConnectionInput, Engine};
use dbforge_core::connections::transport::{SshAuth, SshTunnel, Ssl, SslMode};
use url::Url;

use super::engine::{self, default_port, default_ssl_mode, is_account, is_file, ssl_mode_works_through_ssh};

/// Form values as typed by the user.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionForm {
    pub name: String,
    pub engine: Engine,
    /// A tag id, or none.
    pub environment: Option<String>,
    /// The server's name, or for an account engine its whole URI (`https://….documents.azure.com:443/`).
    pub host: String,
    pub port: String,
    pub database: String,
    pub username: String,
    /// Typed here only; empty keeps the saved one.
    pub password: String,
    /// How SSL/TLS is used. For an account engine the URI's scheme decides
    /// whether it encrypts, and `require` stands for "accept self-signed".
    pub ssl: Ssl,
    pub ssh: SshForm,
    /// The SSH password or key passphrase; empty keeps the saved one.
    pub ssh_secret: String,
    pub srv: bool,
    /// `None` until the user chooses: a new connection never gets agent access by default.
    pub agent_access: Option<AgentAccess>,
}

/// The SSH tunnel as typed: its port is text, like the server's.
#[derive(Debug, Clone, PartialEq)]
pub struct SshForm {
    pub enabled: bool,
    pub host: String,
    pub port: String,
    pub username: String,
    pub auth: SshAuth,
    pub key_file: String,
}

impl Default for SshForm {
    fn default() -> Self {
        Self::from(&SshTunnel::default())
    }
}

impl From<&SshTunnel> for SshForm {
    fn from(ssh: &SshTunnel) -> Self {
        Self {
            enabled: ssh.enabled,
            host: ssh.host.clone(),
            port: ssh.port.to_string(),
            username: ssh.username.clone(),
            auth: ssh.auth,
            key_file: ssh.key_file.clone(),
        }
    }
}

/// What a pasted URL or connection string sets; `None` leaves a field as it is.
#[derive(Debug, Clone, PartialEq)]
pub struct FormChanges {
    pub engine: Engine,
    pub host: String,
    pub port: Option<String>,
    pub username: String,
    pub password: String,
    pub database: Option<String>,
    pub ssl: Option<Ssl>,
    pub srv: bool,
}

/// SSL settings with `mode` and no files.
pub fn ssl_with(mode: SslMode) -> Ssl {
    Ssl { mode, ..Ssl::default() }
}

pub fn empty_form() -> ConnectionForm {
    ConnectionForm {
        name: String::new(),
        engine: Engine::Postgres,
        environment: None,
        host: "localhost".into(),
        port: default_port(Engine::Postgres).to_string(),
        database: String::new(),
        username: String::new(),
        password: String::new(),
        ssl: ssl_with(default_ssl_mode(Engine::Postgres)),
        ssh: SshForm::default(),
        ssh_secret: String::new(),
        srv: false,
        agent_access: None,
    }
}

/// A saved connection as form values, without its secrets. A host saved as a
/// whole URL is split into fields; an account's host becomes its URI again.
pub fn form_from_connection(connection: &Connection, extra_hosts: fn(&str) -> String) -> ConnectionForm {
    let form = ConnectionForm {
        name: connection.name.clone(),
        engine: connection.engine,
        environment: connection.environment.clone(),
        host: connection.host.clone(),
        port: connection.port.to_string(),
        database: connection.database.clone(),
        username: connection.username.clone(),
        password: String::new(),
        ssl: connection.ssl.clone(),
        ssh: SshForm::from(&connection.ssh),
        ssh_secret: String::new(),
        srv: connection.srv,
        agent_access: Some(connection.agent_access),
    };
    if is_account(connection.engine) {
        let scheme = if connection.ssl.mode.encrypts() { "https" } else { "http" };
        return ConnectionForm { host: format!("{scheme}://{}:{}/", connection.host, connection.port), ..form };
    }
    match super::url::parse_connection_url(&connection.host, extra_hosts) {
        Some(parsed) => overwrite(form, parsed.changes),
        None => form,
    }
}

/// Switches engine, moving the port to the new default unless the user
/// changed it. An account engine's host is a URI, which no other engine's
/// host field takes. The SSL mode stays when the new engine has it, else
/// becomes its default.
pub fn with_engine(form: ConnectionForm, engine: Engine) -> ConnectionForm {
    let uses_default_port = form.port == default_port(form.engine).to_string();
    let host = if is_account(engine) == is_account(form.engine) {
        form.host.clone()
    } else if is_account(engine) {
        String::new()
    } else {
        "localhost".into()
    };
    let mode =
        if engine.ssl_support().modes.contains(&form.ssl.mode) { form.ssl.mode } else { default_ssl_mode(engine) };
    let port = if uses_default_port { default_port(engine).to_string() } else { form.port.clone() };
    ConnectionForm { engine, host, port, ssl: Ssl { mode, ..form.ssl.clone() }, ..form }
}

/// Fills the form from a pasted URL: its engine first (as switching engine
/// does), then what the URL holds.
pub fn with_changes(form: ConnectionForm, changes: FormChanges) -> ConnectionForm {
    let switched = with_engine(form, changes.engine);
    overwrite(switched, changes)
}

fn overwrite(form: ConnectionForm, changes: FormChanges) -> ConnectionForm {
    ConnectionForm {
        engine: changes.engine,
        host: changes.host,
        port: changes.port.unwrap_or(form.port),
        username: changes.username,
        password: changes.password,
        database: changes.database.unwrap_or(form.database),
        ssl: changes.ssl.unwrap_or(form.ssl),
        srv: changes.srv,
        ..form
    }
}

/// The SSL modes the form offers: the engine's, without `disable` for a seed
/// list, which is always encrypted.
pub fn ssl_modes_of(form: &ConnectionForm) -> Vec<SslMode> {
    let modes = form.engine.ssl_support().modes.iter().copied();
    if uses_srv(form) { modes.filter(|mode| mode.encrypts()).collect() } else { modes.collect() }
}

/// The tunnel is on, where the engine can use one.
pub fn tunnel_on(form: &ConnectionForm) -> bool {
    form.engine.tunnels() && form.ssh.enabled
}

/// The SSL mode checks the server's name, which this engine can't do through the tunnel.
pub fn ssl_blocked_by_tunnel(form: &ConnectionForm) -> bool {
    tunnel_on(form) && !ssl_mode_works_through_ssh(form.engine, effective_ssl_mode(form))
}

/// A tunnel reaches one host and port, not a DNS seed list or a whole URL.
pub fn tunnel_unreachable(form: &ConnectionForm) -> bool {
    tunnel_on(form) && (uses_srv(form) || form.host.contains("://"))
}

/// A client certificate without its key, or a key without its certificate,
/// where they are separate files.
pub fn client_cert_incomplete(form: &ConnectionForm) -> bool {
    let support = form.engine.ssl_support();
    if !support.client_cert || !support.separate_key || !form.ssl.mode.encrypts() {
        return false;
    }
    form.ssl.cert_file.trim().is_empty() != form.ssl.key_file.trim().is_empty()
}

/// Enough to open the tunnel: its host, user and port, and the key file for key auth.
fn tunnel_complete(ssh: &SshForm) -> bool {
    !ssh.host.trim().is_empty()
        && !ssh.username.trim().is_empty()
        && is_valid_port(&ssh.port)
        && (ssh.auth != SshAuth::Key || !ssh.key_file.trim().is_empty())
}

/// The mode sent for a server: a seed list is always encrypted, as mongodb+srv:// defines.
fn effective_ssl_mode(form: &ConnectionForm) -> SslMode {
    if uses_srv(form) && !form.ssl.mode.encrypts() { SslMode::VerifyFull } else { form.ssl.mode }
}

/// An account URI: `https://<account>.documents.azure.com:443/`, or `http://` for the local emulator.
pub fn account_url(text: &str) -> Option<Url> {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    if rest.is_empty() || rest.starts_with('/') {
        return None;
    }
    Url::parse(trimmed).ok().filter(|url| url.host_str().is_some_and(|host| !host.is_empty()))
}

/// A whole number from 1 to 65535.
pub fn is_valid_port(port: &str) -> bool {
    port.trim().parse::<u32>().is_ok_and(|value| (1..=65535).contains(&value))
}

/// Enough to try connecting: a file for SQLite, a valid port for servers, and
/// SSL and tunnel settings that work together.
pub fn can_test(form: &ConnectionForm) -> bool {
    if is_file(form.engine) {
        return !form.database.trim().is_empty();
    }
    if is_account(form.engine) {
        return account_url(&form.host).is_some();
    }
    if client_cert_incomplete(form) {
        return false;
    }
    if tunnel_on(form) && (!tunnel_complete(&form.ssh) || tunnel_unreachable(form) || ssl_blocked_by_tunnel(form)) {
        return false;
    }
    // A seed list's DNS records name the port.
    uses_srv(form) || is_valid_port(&form.port)
}

pub fn uses_srv(form: &ConnectionForm) -> bool {
    form.engine == Engine::Mongodb && form.srv
}

/// The user chose what agents may do; saving asks for it, as the field can be out of view.
pub fn agent_access_chosen(form: &ConnectionForm) -> bool {
    form.agent_access.is_some()
}

pub fn can_submit(form: &ConnectionForm) -> bool {
    !form.name.trim().is_empty() && can_test(form)
}

/// The input the core saves or tests. An empty password or SSH secret means
/// "no change", so it is left out. A file needs no server details, SSL or
/// tunnel; an account has no tunnel.
pub fn to_input(form: &ConnectionForm, id: Option<&str>) -> ConnectionInput {
    let secret = |value: &str| (!value.is_empty()).then(|| value.to_owned());
    // A test runs before access is chosen; it gives agents nothing, and saving requires a choice.
    let base = ConnectionInput {
        id: id.map(str::to_owned),
        name: form.name.clone(),
        engine: form.engine,
        environment: form.environment.clone(),
        host: form.host.clone(),
        port: 0,
        database: form.database.clone(),
        username: form.username.clone(),
        password: None,
        ssl: ssl_with(SslMode::Disable),
        ssh: SshTunnel::default(),
        ssh_secret: None,
        srv: false,
        agent_access: form.agent_access.unwrap_or(AgentAccess::None),
    };
    if is_file(form.engine) {
        return ConnectionInput { host: String::new(), username: String::new(), ..base };
    }
    if is_account(form.engine) {
        // The databases are all listed in the explorer, and the key stands alone.
        if let Some(account) = account_url(&form.host) {
            let https = account.scheme() == "https";
            let mode = match (https, form.ssl.mode) {
                (false, _) => SslMode::Disable,
                (true, SslMode::Require) => SslMode::Require,
                (true, _) => SslMode::VerifyFull,
            };
            return ConnectionInput {
                host: account.host_str().unwrap_or_default().to_owned(),
                port: account.port_or_known_default().unwrap_or(if https { 443 } else { 80 }),
                database: String::new(),
                username: String::new(),
                password: secret(&form.password),
                ssl: ssl_with(mode),
                ..base
            };
        }
    }
    let srv = uses_srv(form);
    let port = match form.port.trim().parse::<u16>() {
        Ok(port) if is_valid_port(&form.port) => port,
        _ if srv => default_port(Engine::Mongodb),
        _ => 0,
    };
    let ssh = if form.engine.tunnels() {
        SshTunnel {
            enabled: form.ssh.enabled,
            host: form.ssh.host.clone(),
            port: if is_valid_port(&form.ssh.port) {
                form.ssh.port.trim().parse().unwrap_or(22)
            } else {
                SshTunnel::default().port
            },
            username: form.ssh.username.clone(),
            auth: form.ssh.auth,
            key_file: form.ssh.key_file.clone(),
        }
    } else {
        SshTunnel::default()
    };
    ConnectionInput {
        port,
        // Redis databases are numbered and all listed in the explorer; there is none to pick here.
        database: if form.engine == Engine::Redis { String::new() } else { form.database.clone() },
        password: secret(&form.password),
        ssl: Ssl { mode: effective_ssl_mode(form), ..form.ssl.clone() },
        ssh,
        ssh_secret: secret(&form.ssh_secret),
        srv,
        ..base
    }
}

/// Where the form's fields point, as a connection with no id and no secrets,
/// for the connection URL shown under them.
pub fn preview_connection(form: &ConnectionForm) -> Connection {
    let input = to_input(form, None);
    Connection {
        id: String::new(),
        name: input.name,
        engine: input.engine,
        environment: input.environment,
        host: input.host,
        port: input.port,
        database: input.database,
        username: input.username,
        ssl: input.ssl,
        ssh: input.ssh,
        srv: input.srv,
        agent_access: input.agent_access,
    }
}

/// A field of the form typed as text, a file path included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextField {
    Name,
    Host,
    Port,
    Database,
    Username,
    Password,
    CaFile,
    CertFile,
    KeyFile,
    SshHost,
    SshPort,
    SshUser,
    SshKeyFile,
    SshSecret,
}

impl TextField {
    pub const ALL: [TextField; 14] = [
        TextField::Name,
        TextField::Host,
        TextField::Port,
        TextField::Database,
        TextField::Username,
        TextField::Password,
        TextField::CaFile,
        TextField::CertFile,
        TextField::KeyFile,
        TextField::SshHost,
        TextField::SshPort,
        TextField::SshUser,
        TextField::SshKeyFile,
        TextField::SshSecret,
    ];

    /// Its value in `form`. A seed list shows no port: its DNS records name it.
    pub fn value(self, form: &ConnectionForm) -> &str {
        match self {
            TextField::Name => &form.name,
            TextField::Host => &form.host,
            TextField::Port if uses_srv(form) => "",
            TextField::Port => &form.port,
            TextField::Database => &form.database,
            TextField::Username => &form.username,
            TextField::Password => &form.password,
            TextField::CaFile => &form.ssl.ca_file,
            TextField::CertFile => &form.ssl.cert_file,
            TextField::KeyFile => &form.ssl.key_file,
            TextField::SshHost => &form.ssh.host,
            TextField::SshPort => &form.ssh.port,
            TextField::SshUser => &form.ssh.username,
            TextField::SshKeyFile => &form.ssh.key_file,
            // The agent needs none, whatever was typed before switching to it.
            TextField::SshSecret if form.ssh.auth == SshAuth::Agent => "",
            TextField::SshSecret => &form.ssh_secret,
        }
    }

    /// Sets its value in `form`.
    pub fn set(self, form: &mut ConnectionForm, value: &str) {
        let slot = match self {
            TextField::Name => &mut form.name,
            TextField::Host => &mut form.host,
            TextField::Port => &mut form.port,
            TextField::Database => &mut form.database,
            TextField::Username => &mut form.username,
            TextField::Password => &mut form.password,
            TextField::CaFile => &mut form.ssl.ca_file,
            TextField::CertFile => &mut form.ssl.cert_file,
            TextField::KeyFile => &mut form.ssl.key_file,
            TextField::SshHost => &mut form.ssh.host,
            TextField::SshPort => &mut form.ssh.port,
            TextField::SshUser => &mut form.ssh.username,
            TextField::SshKeyFile => &mut form.ssh.key_file,
            TextField::SshSecret => &mut form.ssh_secret,
        };
        value.clone_into(slot);
    }

    /// The field takes a secret: it is drawn masked and never read back.
    pub fn is_secret(self) -> bool {
        matches!(self, TextField::Password | TextField::SshSecret)
    }
}

/// The dialog's sections, as its tabs list them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    General,
    Ssl,
    Ssh,
}

/// The sections `engine` has: SSL where it has more than one mode (an
/// account's URI says it on General), SSH where its server can be tunneled to.
pub fn sections_of(engine: Engine) -> Vec<Section> {
    let mut sections = vec![Section::General];
    if has_ssl_section(engine) {
        sections.push(Section::Ssl);
    }
    if engine.tunnels() {
        sections.push(Section::Ssh);
    }
    sections
}

/// What pressing Save (or Enter) does with the form as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submission {
    /// Something needed is missing or can't work together: nothing happens.
    Blocked,
    /// Everything else is ready, but what agents may do wasn't chosen: the
    /// dialog shows that field and asks for it.
    AskAgentAccess,
    Save,
}

pub fn submission(form: &ConnectionForm) -> Submission {
    if !can_submit(form) {
        Submission::Blocked
    } else if !agent_access_chosen(form) {
        Submission::AskAgentAccess
    } else {
        Submission::Save
    }
}

/// The engine has an SSL section: more than one mode, and not an account
/// (whose URI says it on General).
pub fn has_ssl_section(engine: Engine) -> bool {
    engine.ssl_support().modes.len() > 1 && engine::reach(engine) != engine::Reach::Account
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssl(mode: SslMode) -> Ssl {
        ssl_with(mode)
    }

    fn named(engine: Engine) -> ConnectionForm {
        with_engine(ConnectionForm { name: "n".into(), ..empty_form() }, engine)
    }

    /// A form with the tunnel on and complete, for `engine`.
    fn tunneled(engine: Engine) -> ConnectionForm {
        ConnectionForm {
            ssh: SshForm { enabled: true, host: "bastion".into(), username: "ops".into(), ..SshForm::default() },
            ..named(engine)
        }
    }

    fn hosts(hosts: &str) -> String {
        format!("hosts {hosts}")
    }

    #[test]
    fn switching_engine_moves_an_untouched_default_port_and_keeps_a_changed_one() {
        assert_eq!(with_engine(empty_form(), Engine::Mysql).port, "3306");
        let changed = ConnectionForm { port: "6543".into(), ..empty_form() };
        assert_eq!(with_engine(changed, Engine::Mysql).port, "6543");
    }

    #[test]
    fn switching_to_and_from_an_account_resets_the_host() {
        let cosmos = with_engine(empty_form(), Engine::Cosmosdb);
        assert_eq!((cosmos.host.as_str(), cosmos.port.as_str()), ("", "443"));
        let typed = ConnectionForm { host: "https://a.documents.azure.com/".into(), ..cosmos };
        assert_eq!(with_engine(typed, Engine::Mongodb).host, "localhost");
    }

    #[test]
    fn ports_are_whole_numbers_from_1_to_65535() {
        for port in ["1", "5432", "65535"] {
            assert!(is_valid_port(port), "{port}");
        }
        for port in ["", "0", "65536", "54.3", "abc", "-1"] {
            assert!(!is_valid_port(port), "{port:?}");
        }
    }

    #[test]
    fn saving_needs_a_name_and_a_valid_port() {
        assert!(!can_submit(&empty_form()));
        assert!(can_submit(&ConnectionForm { name: "local".into(), ..empty_form() }));
        assert!(!can_submit(&ConnectionForm { name: "local".into(), port: "x".into(), ..empty_form() }));
    }

    #[test]
    fn sqlite_needs_a_file_rather_than_a_port() {
        let sqlite = ConnectionForm { name: "local".into(), ..with_engine(empty_form(), Engine::Sqlite) };
        assert!(!can_submit(&sqlite));
        assert!(can_submit(&ConnectionForm { database: "/data/app.db".into(), ..sqlite }));
    }

    #[test]
    fn a_new_connection_has_no_agent_access_until_chosen() {
        assert_eq!(empty_form().agent_access, None);
        assert!(!agent_access_chosen(&empty_form()));
        assert!(agent_access_chosen(&ConnectionForm { agent_access: Some(AgentAccess::None), ..empty_form() }));
        // A test before choosing gives agents nothing.
        assert_eq!(to_input(&named(Engine::Postgres), None).agent_access, AgentAccess::None);
    }

    fn saved() -> Connection {
        Connection {
            id: "abc".into(),
            name: "prod".into(),
            engine: Engine::Postgres,
            environment: Some("production".into()),
            host: "db".into(),
            port: 5433,
            database: "shop".into(),
            username: "app".into(),
            ssl: Ssl {
                mode: SslMode::VerifyCa,
                ca_file: "/ca.pem".into(),
                cert_file: "/c.pem".into(),
                key_file: "/c.key".into(),
            },
            ssh: SshTunnel {
                enabled: true,
                host: "bastion".into(),
                port: 2222,
                username: "ops".into(),
                auth: SshAuth::Key,
                key_file: "/id_ed25519".into(),
            },
            srv: false,
            agent_access: AgentAccess::Read,
        }
    }

    #[test]
    fn a_saved_connection_round_trips_without_its_secrets() {
        let connection = saved();
        let form = form_from_connection(&connection, hosts);
        assert_eq!(form.agent_access, Some(AgentAccess::Read));
        assert_eq!((form.password.as_str(), form.ssh_secret.as_str()), ("", ""));
        let input = to_input(&form, Some("abc"));
        assert_eq!(input.id.as_deref(), Some("abc"));
        assert_eq!(input.password, None, "an empty password keeps the saved one");
        assert_eq!(input.ssh_secret, None);
        assert_eq!(
            (input.name, input.environment, input.host, input.port, input.database, input.username),
            ("prod".into(), Some("production".into()), "db".into(), 5433, "shop".into(), "app".into())
        );
        assert_eq!(input.ssl, connection.ssl);
        assert_eq!(input.ssh, connection.ssh);
        assert_eq!(input.agent_access, AgentAccess::Read);
    }

    #[test]
    fn typed_secrets_are_sent_and_empty_ones_kept() {
        let typed = ConnectionForm { password: "s3cret".into(), ..empty_form() };
        assert_eq!(to_input(&typed, None).password.as_deref(), Some("s3cret"));
        assert_eq!(to_input(&tunneled(Engine::Postgres), None).ssh_secret, None);
        let secret = ConnectionForm { ssh_secret: "pass".into(), ..tunneled(Engine::Postgres) };
        assert_eq!(to_input(&secret, None).ssh_secret.as_deref(), Some("pass"));
    }

    #[test]
    fn a_sqlite_file_sends_no_server_details_ssl_or_tunnel() {
        let form = ConnectionForm {
            database: "/a.db".into(),
            password: "x".into(),
            ssl: ssl(SslMode::Require),
            ..tunneled(Engine::Sqlite)
        };
        let input = to_input(&form, None);
        assert_eq!((input.host.as_str(), input.port, input.username.as_str()), ("", 0, ""));
        assert_eq!(input.database, "/a.db");
        assert_eq!(input.password, None);
        assert_eq!(input.ssl, ssl(SslMode::Disable));
        assert_eq!(input.ssh, SshTunnel::default());
    }

    #[test]
    fn new_connections_start_on_prefer_where_the_engine_has_it() {
        assert_eq!(empty_form().ssl.mode, SslMode::Prefer);
        let mode = |engine| with_engine(empty_form(), engine).ssl.mode;
        for engine in [Engine::Mysql, Engine::Mariadb, Engine::Sqlserver] {
            assert_eq!(mode(engine), SslMode::Prefer, "{engine:?}");
        }
        for engine in [Engine::Mongodb, Engine::Redis, Engine::Sqlite, Engine::Cosmosdb] {
            assert_eq!(mode(engine), SslMode::Disable, "{engine:?}");
        }
    }

    #[test]
    fn the_ssl_mode_stays_when_the_new_engine_has_it() {
        let verified = ConnectionForm { ssl: ssl(SslMode::VerifyFull), ..empty_form() };
        assert_eq!(with_engine(verified, Engine::Redis).ssl.mode, SslMode::VerifyFull);
        let ca = ConnectionForm { ssl: ssl(SslMode::VerifyCa), ..empty_form() };
        assert_eq!(with_engine(ca, Engine::Sqlserver).ssl.mode, SslMode::Prefer);
    }

    #[test]
    fn a_mongodb_seed_list_is_always_encrypted_and_needs_no_port() {
        let atlas = ConnectionForm { srv: true, port: String::new(), ..named(Engine::Mongodb) };
        assert!(can_submit(&atlas));
        assert_eq!(to_input(&atlas, None).ssl.mode, SslMode::VerifyFull);
        assert_eq!(to_input(&atlas, None).port, 27017);
        let require = ConnectionForm { ssl: ssl(SslMode::Require), ..atlas.clone() };
        assert_eq!(to_input(&require, None).ssl.mode, SslMode::Require);
        assert_eq!(ssl_modes_of(&atlas), [SslMode::Require, SslMode::VerifyFull]);
    }

    #[test]
    fn redis_sends_no_database() {
        let redis = ConnectionForm { database: "3".into(), ..named(Engine::Redis) };
        assert_eq!(to_input(&redis, None).database, "");
    }

    #[test]
    fn an_account_uri_splits_into_host_port_and_tls() {
        let form = ConnectionForm { username: "x".into(), database: "y".into(), ..named(Engine::Cosmosdb) };
        let https = ConnectionForm { host: " https://shop.documents.azure.com:443/ ".into(), ..form.clone() };
        let input = to_input(&ConnectionForm { ssl: ssl(SslMode::Require), ..https.clone() }, None);
        assert_eq!((input.host.as_str(), input.port), ("shop.documents.azure.com", 443));
        assert_eq!(input.ssl, ssl(SslMode::Require));
        assert_eq!((input.username.as_str(), input.database.as_str()), ("", ""));
        assert_eq!(input.ssh, SshTunnel::default());
        assert_eq!(to_input(&https, None).ssl.mode, SslMode::VerifyFull);
        let emulator = ConnectionForm { host: "http://127.0.0.1:8081/".into(), ssl: ssl(SslMode::Require), ..form };
        let input = to_input(&emulator, None);
        assert_eq!((input.host.as_str(), input.port, input.ssl.mode), ("127.0.0.1", 8081, SslMode::Disable));
        let bare = ConnectionForm { host: "shop.documents.azure.com".into(), ..emulator.clone() };
        assert!(!can_test(&bare));
        assert!(can_test(&ConnectionForm { host: "https://shop.documents.azure.com".into(), ..emulator }));
    }

    #[test]
    fn a_saved_account_opens_as_its_uri() {
        let emulator = Connection {
            engine: Engine::Cosmosdb,
            host: "127.0.0.1".into(),
            port: 8081,
            ssl: ssl(SslMode::Disable),
            ssh: SshTunnel::default(),
            ..saved()
        };
        assert_eq!(form_from_connection(&emulator, hosts).host, "http://127.0.0.1:8081/");
        let https = Connection { ssl: ssl(SslMode::Require), port: 443, ..emulator };
        let form = form_from_connection(&https, hosts);
        assert_eq!((form.host.as_str(), form.ssl.mode), ("https://127.0.0.1:443/", SslMode::Require));
    }

    #[test]
    fn a_saved_url_host_opens_as_fields() {
        let cache = Connection {
            engine: Engine::Redis,
            host: "rediss://127.0.0.1:56380/#insecure".into(),
            port: 0,
            ssl: ssl(SslMode::Disable),
            ssh: SshTunnel::default(),
            ..saved()
        };
        let form = form_from_connection(&cache, hosts);
        assert_eq!((form.host.as_str(), form.port.as_str()), ("127.0.0.1", "56380"));
        assert_eq!(form.ssl, ssl(SslMode::Require));
    }

    #[test]
    fn the_tunnel_needs_its_host_user_port_and_key_file() {
        let base = tunneled(Engine::Postgres);
        let with = |ssh: SshForm| ConnectionForm { ssh, ..base.clone() };
        assert!(can_test(&base));
        assert!(!can_test(&with(SshForm { host: " ".into(), ..base.ssh.clone() })));
        assert!(!can_test(&with(SshForm { username: String::new(), ..base.ssh.clone() })));
        assert!(!can_test(&with(SshForm { port: "0".into(), ..base.ssh.clone() })));
        assert!(!can_test(&with(SshForm { auth: SshAuth::Key, ..base.ssh.clone() })));
        assert!(can_test(&with(SshForm { auth: SshAuth::Key, key_file: "/k".into(), ..base.ssh.clone() })));
        assert!(can_test(&with(SshForm { auth: SshAuth::Agent, ..base.ssh.clone() })));
        // Off, what it holds doesn't matter.
        assert!(can_test(&with(SshForm { enabled: false, host: String::new(), ..base.ssh.clone() })));
    }

    #[test]
    fn the_server_name_cant_be_checked_through_a_tunnel_except_on_sql_server() {
        let mode = |engine, mode| ConnectionForm { ssl: ssl(mode), ..tunneled(engine) };
        assert!(!can_test(&mode(Engine::Postgres, SslMode::VerifyFull)));
        assert!(ssl_blocked_by_tunnel(&mode(Engine::Postgres, SslMode::VerifyFull)));
        assert!(can_test(&mode(Engine::Postgres, SslMode::VerifyCa)));
        assert!(can_test(&mode(Engine::Sqlserver, SslMode::VerifyFull)));
    }

    #[test]
    fn a_tunnel_doesnt_reach_a_seed_list_or_a_url() {
        assert!(!can_test(&ConnectionForm { srv: true, ..tunneled(Engine::Mongodb) }));
        assert!(tunnel_unreachable(&ConnectionForm { host: "redis://h".into(), ..tunneled(Engine::Redis) }));
    }

    #[test]
    fn the_tunnel_port_is_sent_as_a_number_and_an_off_tunnel_as_typed() {
        let ssh = to_input(&tunneled(Engine::Postgres), None).ssh;
        assert_eq!(
            ssh,
            SshTunnel { enabled: true, host: "bastion".into(), username: "ops".into(), ..SshTunnel::default() }
        );
        let mut off = tunneled(Engine::Postgres);
        off.ssh.enabled = false;
        let ssh = to_input(&off, None).ssh;
        assert_eq!((ssh.enabled, ssh.host.as_str()), (false, "bastion"));
    }

    #[test]
    fn a_client_certificate_needs_its_key_where_they_are_separate_files() {
        let cert = Ssl { cert_file: "/c.pem".into(), ..ssl(SslMode::Require) };
        let pg = ConnectionForm { ssl: cert.clone(), ..named(Engine::Postgres) };
        assert!(!can_test(&pg));
        assert!(can_test(&ConnectionForm { ssl: Ssl { key_file: "/c.key".into(), ..cert.clone() }, ..pg.clone() }));
        let mongo = ConnectionForm { ssl: cert, ..with_engine(pg, Engine::Mongodb) };
        assert!(can_test(&mongo));
    }

    #[test]
    fn a_pasted_url_switches_engine_then_fills_the_fields() {
        let changes = FormChanges {
            engine: Engine::Redis,
            host: "cache".into(),
            port: None,
            username: "app".into(),
            password: "pw".into(),
            database: None,
            ssl: None,
            srv: false,
        };
        let form = with_changes(ConnectionForm { name: "n".into(), database: "d".into(), ..empty_form() }, changes);
        assert_eq!(form.engine, Engine::Redis);
        assert_eq!(form.port, "6379", "the engine's default port, as switching engine gives it");
        assert_eq!((form.host.as_str(), form.username.as_str(), form.database.as_str()), ("cache", "app", "d"));
        assert_eq!(form.ssl.mode, SslMode::Disable);
        assert_eq!(form.name, "n");
    }

    #[test]
    fn each_text_field_reads_and_writes_its_own_value() {
        let mut form = empty_form();
        for (index, field) in TextField::ALL.into_iter().enumerate() {
            field.set(&mut form, &format!("v{index}"));
        }
        for (index, field) in TextField::ALL.into_iter().enumerate() {
            assert_eq!(field.value(&form), format!("v{index}"), "{field:?}");
        }
        assert!(TextField::Password.is_secret() && TextField::SshSecret.is_secret());
        assert!(!TextField::Host.is_secret());
    }

    #[test]
    fn a_seed_list_shows_no_port_and_the_agent_no_secret() {
        let mut atlas = ConnectionForm { srv: true, ..named(Engine::Mongodb) };
        assert_eq!(TextField::Port.value(&atlas), "");
        atlas.ssh_secret = "typed".into();
        atlas.ssh.auth = SshAuth::Agent;
        assert_eq!(TextField::SshSecret.value(&atlas), "");
    }

    #[test]
    fn each_engine_shows_the_sections_it_can_use() {
        use Section::*;
        assert_eq!(sections_of(Engine::Postgres), [General, Ssl, Ssh]);
        assert_eq!(sections_of(Engine::Redis), [General, Ssl, Ssh]);
        assert_eq!(sections_of(Engine::Sqlite), [General]);
        assert_eq!(sections_of(Engine::Cosmosdb), [General]);
    }

    #[test]
    fn saving_asks_for_agent_access_once_the_rest_is_ready() {
        assert_eq!(submission(&empty_form()), Submission::Blocked);
        let ready = named(Engine::Postgres);
        assert_eq!(submission(&ready), Submission::AskAgentAccess);
        let chosen = ConnectionForm { agent_access: Some(AgentAccess::Read), ..ready };
        assert_eq!(submission(&chosen), Submission::Save);
        let blocked = ConnectionForm { port: "0".into(), ..chosen };
        assert_eq!(submission(&blocked), Submission::Blocked);
    }

    #[test]
    fn only_engines_with_several_modes_and_no_account_have_an_ssl_section() {
        assert!(has_ssl_section(Engine::Postgres));
        assert!(has_ssl_section(Engine::Redis));
        assert!(!has_ssl_section(Engine::Sqlite));
        assert!(!has_ssl_section(Engine::Cosmosdb));
    }
}
