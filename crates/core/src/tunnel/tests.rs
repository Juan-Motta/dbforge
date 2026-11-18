//! Runs against the SSH bastion `scripts/test-tls-certs.sh` documents, on port
//! 52222, which reaches the database containers at `host.docker.internal`.

use std::sync::Once;

use super::*;
use crate::connections::transport::{SshAuth, Ssl, SslMode};
use crate::drivers::postgres::tests::{PASSWORD, local};
use crate::drivers::test_certs::{path, trusting};

/// Sets up the process's tunnels once, with the secrets of the connections below.
pub(crate) fn setup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        init(
            dir.join("known_hosts"),
            Box::new(|id| {
                Ok(match id {
                    "password.ssh" | "passphrase.ssh" => Some("dbforge".into()),
                    "wrong.ssh" => Some("nope".into()),
                    _ => None,
                })
            }),
        );
    });
}

pub(crate) fn bastion(auth: SshAuth, key_file: &str) -> SshTunnel {
    SshTunnel {
        enabled: true,
        host: "127.0.0.1".into(),
        port: 52222,
        username: "ops".into(),
        auth,
        key_file: key_file.into(),
    }
}

/// The test PostgreSQL, as the bastion sees it, through `ssh`, under `id`.
fn through(id: &str, ssh: SshTunnel) -> Connection {
    Connection { id: id.into(), host: "host.docker.internal".into(), ssh, ..local() }
}

#[tokio::test]
#[ignore = "needs the SSH bastion on port 52222 and PostgreSQL on 55432"]
async fn signs_in_with_a_password_or_a_key_and_shares_the_tunnel() {
    setup();
    let password = through("password", bastion(SshAuth::Password, ""));
    let version = crate::drivers::server_version(&password, PASSWORD).await.unwrap();
    assert!(version.starts_with("PostgreSQL"), "{version}");
    let first = route(&password).await.unwrap();
    assert_eq!(first.host, "127.0.0.1");
    assert_eq!(route(&password).await.unwrap(), first, "one tunnel for every operation");

    let key = through("key", bastion(SshAuth::Key, &path("id_ed25519")));
    assert!(crate::drivers::server_version(&key, PASSWORD).await.is_ok());
    let passphrase = through("passphrase", bastion(SshAuth::Key, &path("id_ed25519_passphrase")));
    assert!(crate::drivers::server_version(&passphrase, PASSWORD).await.is_ok());

    close("password").await;
    assert_ne!(route(&password).await.unwrap(), first, "closed, then opened again");
}

#[tokio::test]
#[ignore = "needs the SSH bastion on port 52222"]
async fn says_why_the_ssh_server_refused() {
    setup();
    let wrong = through("wrong", bastion(SshAuth::Password, ""));
    let error = route(&wrong).await.unwrap_err().to_string();
    assert_eq!(error, "The SSH server 127.0.0.1:52222 refused the user ops with this password");

    let no_passphrase = through("none", bastion(SshAuth::Key, &path("id_ed25519_passphrase")));
    let error = route(&no_passphrase).await.unwrap_err().to_string();
    assert!(error.contains("needs its passphrase, or the passphrase is wrong"), "{error}");
    let wrong_passphrase = through("wrong", bastion(SshAuth::Key, &path("id_ed25519_passphrase")));
    let error = route(&wrong_passphrase).await.unwrap_err().to_string();
    assert!(error.contains("needs its passphrase, or the passphrase is wrong"), "{error}");

    let closed = through("closed", SshTunnel { port: 52299, ..bastion(SshAuth::Password, "") });
    let error = route(&closed).await.unwrap_err().to_string();
    assert!(error.starts_with("Could not reach the SSH server 127.0.0.1:52299"), "{error}");
}

#[tokio::test]
#[ignore = "needs the SSH bastion on port 52222 and the TLS PostgreSQL on 55433"]
async fn ssl_goes_through_the_tunnel() {
    setup();
    let tls =
        Connection { port: 55433, database: "postgres".into(), ..through("password", bastion(SshAuth::Password, "")) };
    let verified = Connection { ssl: trusting(SslMode::VerifyCa), ..tls.clone() };
    assert!(crate::drivers::server_version(&verified, PASSWORD).await.is_ok());
    let unknown = Connection { ssl: Ssl { mode: SslMode::VerifyCa, ..Ssl::default() }, ..tls };
    assert!(crate::drivers::server_version(&unknown, PASSWORD).await.is_err(), "the CA is still checked");
}

#[tokio::test]
#[ignore = "needs the SSH bastion on port 52222, PostgreSQL on 55432 and pg_dump"]
async fn pg_dump_goes_through_the_tunnel() {
    setup();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shop.sql");
    let connection = through("key", bastion(SshAuth::Key, &path("id_ed25519")));
    crate::dump::tests::export_to(&connection, PASSWORD, "shop", &file).await.unwrap();
    assert!(std::fs::read_to_string(&file).unwrap().contains("PostgreSQL database dump"));
}

/// Run with the test key in an agent:
/// `eval $(ssh-agent) && ssh-add ~/.cache/dbforge-test-certs/id_ed25519 && cargo test -- --ignored agent`
#[tokio::test]
#[ignore = "needs the SSH bastion on port 52222, PostgreSQL on 55432 and an agent holding the test key"]
async fn signs_in_with_the_agents_keys() {
    setup();
    let agent = through("agent", bastion(SshAuth::Agent, ""));
    assert!(crate::drivers::server_version(&agent, PASSWORD).await.is_ok());
}
