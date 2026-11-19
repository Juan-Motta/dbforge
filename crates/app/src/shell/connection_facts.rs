//! What the toolbar's connection details list, as label/value pairs: the
//! server (still being asked, or its version), where it is (a file's path,
//! or the user, host and port), its database, its SSL mode where the engine
//! has SSL, and the SSH tunnel it goes through.

use dbforge_core::connections::model::Connection;
use dbforge_core::connections::transport::SslMode;

use crate::connections::engine::{is_account, is_file};
use crate::i18n::ShellMessages;

/// The facts of `connection`, the server's version once `server` says it
/// (`None` while it is being asked).
pub fn connection_facts(
    connection: &Connection,
    server: Option<&str>,
    m: &ShellMessages,
) -> Vec<(&'static str, String)> {
    let mut facts = vec![(m.server, server.unwrap_or(m.checking).to_owned())];
    if is_file(connection.engine) {
        facts.push((m.file, connection.database.clone()));
        return facts;
    }
    facts.push((m.address, connection_address(connection, m)));
    if !connection.database.is_empty() {
        facts.push((m.database, connection.database.clone()));
    }
    if connection.engine.ssl_support().modes.len() > 1 {
        facts.push((m.ssl, ssl_mode_label(connection.ssl.mode, m).to_owned()));
    }
    let ssh = &connection.ssh;
    if connection.engine.tunnels() && ssh.enabled {
        facts.push((m.ssh, format!("{}@{}:{}", ssh.username, ssh.host, ssh.port)));
    }
    facts
}

/// Where `connection` is: a file's path, an account's host and port, or
/// the user (the server's default when none is set) at the host and port.
pub fn connection_address(connection: &Connection, m: &ShellMessages) -> String {
    if is_file(connection.engine) {
        return connection.database.clone();
    }
    if is_account(connection.engine) {
        return format!("{}:{}", connection.host, connection.port);
    }
    let user = if connection.username.is_empty() { m.default_user } else { &connection.username };
    format!("{user}@{}:{}", connection.host, connection.port)
}

fn ssl_mode_label(mode: SslMode, m: &ShellMessages) -> &'static str {
    match mode {
        SslMode::Disable => m.ssl_disable,
        SslMode::Prefer => m.ssl_prefer,
        SslMode::Require => m.ssl_require,
        SslMode::VerifyCa => m.ssl_verify_ca,
        SslMode::VerifyFull => m.ssl_verify_full,
    }
}

#[cfg(test)]
mod tests {
    use dbforge_core::connections::model::{AgentAccess, Engine};
    use dbforge_core::connections::transport::{SshTunnel, Ssl};

    use super::*;
    use crate::i18n::Language;

    fn base() -> Connection {
        Connection {
            id: "1".into(),
            name: "prod".into(),
            engine: Engine::Postgres,
            environment: Some("production".into()),
            host: "db.internal".into(),
            port: 5432,
            database: "shop".into(),
            username: "app".into(),
            ssl: Ssl { mode: SslMode::Prefer, ..Ssl::default() },
            ssh: SshTunnel::default(),
            srv: false,
            agent_access: AgentAccess::None,
        }
    }

    fn facts(connection: &Connection, server: Option<&str>, language: Language) -> Vec<(String, String)> {
        connection_facts(connection, server, &language.messages().shell)
            .into_iter()
            .map(|(label, value)| (label.to_owned(), value))
            .collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(label, value)| (label.to_string(), value.to_string())).collect()
    }

    #[test]
    fn lists_the_server_address_and_database_the_status_bar_used_to_show() {
        assert_eq!(
            facts(&base(), Some("PostgreSQL 16.2"), Language::En),
            pairs(&[
                ("Server", "PostgreSQL 16.2"),
                ("Address", "app@db.internal:5432"),
                ("Database", "shop"),
                ("SSL", "Prefer (when the server offers it)"),
            ])
        );
    }

    #[test]
    fn adds_the_ssh_tunnel_when_it_is_on() {
        let ssh = SshTunnel {
            enabled: true,
            host: "bastion".into(),
            port: 2222,
            username: "ops".into(),
            ..SshTunnel::default()
        };
        let tunnelled = Connection { ssh: ssh.clone(), ..base() };
        assert!(facts(&tunnelled, Some("x"), Language::En).contains(&("SSH".into(), "ops@bastion:2222".into())));
        let off = Connection { ssh: SshTunnel { enabled: false, ..ssh }, ..base() };
        assert!(!facts(&off, Some("x"), Language::En).iter().any(|(label, _)| label == "SSH"));
    }

    #[test]
    fn adds_the_ssl_mode_where_the_connection_can_use_it_and_says_when_the_server_is_still_being_asked() {
        let redis = Connection {
            engine: Engine::Redis,
            database: String::new(),
            username: String::new(),
            ssl: Ssl { mode: SslMode::Require, ..Ssl::default() },
            ..base()
        };
        assert_eq!(
            facts(&redis, None, Language::En),
            pairs(&[
                ("Server", "Checking…"),
                ("Address", "default user@db.internal:5432"),
                ("SSL", "Require (any certificate)"),
            ])
        );
        let mongo = Connection {
            engine: Engine::Mongodb,
            database: String::new(),
            ssl: Ssl { mode: SslMode::Disable, ..Ssl::default() },
            ..base()
        };
        assert!(facts(&mongo, Some("x"), Language::En).contains(&("SSL".into(), "Off".into())));
    }

    #[test]
    fn shows_a_sqlite_file_as_its_path_and_an_account_as_its_host() {
        let sqlite = Connection {
            engine: Engine::Sqlite,
            host: String::new(),
            port: 0,
            database: "/data/app.db".into(),
            ..base()
        };
        assert_eq!(
            facts(&sqlite, Some("SQLite 3.46"), Language::En),
            pairs(&[("Server", "SQLite 3.46"), ("File", "/data/app.db")])
        );
        let cosmos = Connection {
            engine: Engine::Cosmosdb,
            host: "https://acct.documents.azure.com".into(),
            port: 443,
            ..base()
        };
        assert_eq!(connection_address(&cosmos, &Language::En.messages().shell), "https://acct.documents.azure.com:443");
    }

    #[test]
    fn writes_the_labels_in_spanish() {
        let mongo = Connection {
            engine: Engine::Mongodb,
            database: String::new(),
            ssl: Ssl { mode: SslMode::Disable, ..Ssl::default() },
            ..base()
        };
        assert!(facts(&mongo, None, Language::Es).contains(&("SSL".into(), "Desactivado".into())));
        assert_eq!(facts(&base(), None, Language::Es)[0], ("Servidor".into(), "Comprobando…".into()));
        let no_user = Connection { username: String::new(), ..base() };
        assert_eq!(
            facts(&no_user, None, Language::Es)[1],
            ("Dirección".into(), "usuario por defecto@db.internal:5432".into())
        );
    }
}
