use std::path::Path;

use super::*;
use crate::connections::model::{AgentAccess, Engine};
use crate::connections::tags::TagColor;
use crate::credentials::Credentials;

/// An installation's stores, in `dir`.
struct Install {
    connections: ConnectionStore,
    tags: TagStore,
    mcp: SettingsFile,
    tools: ToolPaths,
}

impl Install {
    fn at(dir: &Path) -> Self {
        let credentials = Credentials::load(dir.to_path_buf()).unwrap();
        Self {
            connections: ConnectionStore::load(dir.join("connections.json"), credentials).unwrap(),
            tags: TagStore::load(dir.join("tags.json")).unwrap(),
            mcp: SettingsFile::load(dir.join("mcp.json")).unwrap(),
            tools: ToolPaths::load(dir.join("tools.json")).unwrap(),
        }
    }

    fn stores(&self) -> Stores<'_> {
        Stores { connections: &self.connections, tags: &self.tags, mcp: &self.mcp, tools: &self.tools }
    }
}

fn connection(id: &str, name: &str, tag: Option<&str>) -> Connection {
    Connection {
        id: id.into(),
        name: name.into(),
        engine: Engine::Postgres,
        environment: tag.map(String::from),
        host: "db".into(),
        port: 5432,
        database: "shop".into(),
        username: "app".into(),
        ssl: crate::connections::transport::Ssl {
            mode: crate::connections::transport::SslMode::Disable,
            ..Default::default()
        },
        ssh: Default::default(),
        srv: false,
        agent_access: AgentAccess::Read,
    }
}

/// `connection` reached through a bastion with a key, verifying the server's CA.
fn tunnelled(connection: Connection) -> Connection {
    use crate::connections::transport::{SshAuth, SshTunnel, Ssl, SslMode};
    Connection {
        ssl: Ssl { mode: SslMode::VerifyCa, ca_file: "/ca.pem".into(), ..Ssl::default() },
        ssh: SshTunnel {
            enabled: true,
            host: "bastion".into(),
            username: "ops".into(),
            auth: SshAuth::Key,
            key_file: "/id_ed25519".into(),
            ..SshTunnel::default()
        },
        ..connection
    }
}

const ALL: Parts = Parts { connections: true, mcp: true, tool_paths: true };

/// An installation with two connections (one with a password), a custom tag,
/// MCP on port 9000 with a token, and a pg_dump path that exists only there.
fn source(dir: &Path) -> Install {
    let install = Install::at(dir);
    let mut tags = install.tags.list();
    tags.push(Tag { id: "qa".into(), name: "QA".into(), color: TagColor::Teal, confirm: false });
    install.tags.replace(tags).unwrap();
    install.connections.upsert(connection("a", "Shop", Some("qa"))).unwrap();
    install.connections.upsert(tunnelled(connection("b", "Reports", None))).unwrap();
    install.connections.credentials().set("a", "s3cret").unwrap();
    install.connections.credentials().set("b.ssh", "passphr4se").unwrap();
    install.mcp.set(McpSettings { enabled: true, port: 9000 }).unwrap();
    install.connections.credentials().set(crate::mcp::token::CREDENTIAL_ID, "tok").unwrap();
    install.tools.set("pg_dump", Some(dir.join("pg_dump-only-here"))).unwrap();
    install
}

#[test]
fn moves_everything_to_a_new_installation_secrets_included() {
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let preferences = BTreeMap::from([("theme".to_owned(), "light".to_owned())]);
    let backup = collect(&source(from.path()).stores(), preferences.clone(), true, 7, "0.1.0").unwrap();

    // Through the file's text, as a user moves it.
    let backup = read(&serde_json::to_string(&backup).unwrap()).unwrap();
    let target = Install::at(to.path());
    let preview = preview(&target.stores(), &backup);
    assert_eq!((preview.new_connections.len(), preview.updated_connections.len()), (2, 0));
    assert_eq!((preview.passwords, preview.mcp_token, preview.preferences), (1, true, preferences));

    let imported = apply(&target.stores(), backup, ALL).unwrap();
    assert_eq!((imported.added, imported.updated), (2, 0));
    assert_eq!(imported.without_password, ["Reports"]);
    assert_eq!(imported.missing_tools, ["pg_dump"], "a path that isn't on this computer is left out");
    assert_eq!(target.connections.get("a").unwrap(), connection("a", "Shop", Some("qa")));
    assert_eq!(target.connections.password("a").unwrap().as_deref(), Some("s3cret"));
    assert_eq!(target.connections.get("b").unwrap(), tunnelled(connection("b", "Reports", None)));
    assert_eq!(target.connections.ssh_secret("b").unwrap().as_deref(), Some("passphr4se"));
    assert!(target.tags.list().iter().any(|tag| tag.id == "qa" && tag.name == "QA"));
    assert_eq!(target.mcp.get(), McpSettings { enabled: true, port: 9000 });
    let token = target.connections.credentials().get(crate::mcp::token::CREDENTIAL_ID).unwrap();
    assert_eq!(token.as_deref(), Some("tok"));
}

#[test]
fn without_secrets_the_file_holds_no_password_or_token() {
    let from = tempfile::tempdir().unwrap();
    let backup = collect(&source(from.path()).stores(), BTreeMap::new(), false, 7, "0.1.0").unwrap();
    let text = serde_json::to_string(&backup).unwrap();
    assert!(!text.contains("s3cret") && !text.contains("\"tok\"") && !text.contains("passphr4se"), "{text}");
    assert_eq!(read(&text).unwrap().connections.len(), 2);
}

#[test]
fn merges_into_an_existing_installation_without_deleting() {
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let backup = collect(&source(from.path()).stores(), BTreeMap::new(), true, 7, "0.1.0").unwrap();
    let target = Install::at(to.path());
    // Here: the same connection "a" under another name, one of its own, and a QA tag of its own.
    target.connections.upsert(connection("a", "Old name", None)).unwrap();
    target.connections.upsert(connection("mine", "Mine", None)).unwrap();
    let mut tags = target.tags.list();
    tags.push(Tag { id: "my-qa".into(), name: "qa".into(), color: TagColor::Blue, confirm: true });
    target.tags.replace(tags).unwrap();

    let only_connections = Parts { connections: true, mcp: false, tool_paths: false };
    let imported = apply(&target.stores(), backup, only_connections).unwrap();

    assert_eq!((imported.added, imported.updated), (1, 1));
    let names: Vec<_> = target.connections.list().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["Shop", "Mine", "Reports"]);
    // The backup's QA tag is the one already named so here.
    assert_eq!(target.connections.get("a").unwrap().environment.as_deref(), Some("my-qa"));
    assert_eq!(target.tags.list().iter().filter(|tag| tag.name.eq_ignore_ascii_case("qa")).count(), 1);
    assert_eq!(target.mcp.get(), McpSettings::default(), "parts left unticked stay as they were");
}

#[test]
fn refuses_other_files_and_newer_backups() {
    assert!(read("{\"hello\": 1}").is_err());
    let other = r#"{"format":"something","version":1,"exportedAt":0,"appVersion":"x","mcp":null}"#;
    assert!(read(other).unwrap_err().to_string().contains("isn't a DBForge backup"));
    let newer = r#"{"format":"dbforge-backup","version":99,"exportedAt":0,"appVersion":"x","mcp":null}"#;
    assert!(read(newer).unwrap_err().to_string().contains("newer DBForge"));
}

#[test]
fn reads_backups_made_before_ssl_modes() {
    let v1 = r#"{"format":"dbforge-backup","version":1,"exportedAt":0,"appVersion":"0.2.1","mcp":null,
        "connections":[{"id":"r","name":"Cache","engine":"redis","environment":null,"host":"h","port":6380,
        "database":"0","username":"","tls":true,"allowSelfSigned":true,"password":"pw"}]}"#;
    let backup = read(v1).unwrap();
    let cache = &backup.connections[0];
    assert_eq!(cache.connection.ssl.mode, crate::connections::transport::SslMode::Require);
    assert_eq!((cache.password.as_deref(), cache.ssh_secret.as_deref()), (Some("pw"), None));
}
