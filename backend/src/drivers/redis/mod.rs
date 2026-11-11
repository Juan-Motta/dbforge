//! Redis, through the `redis` crate. A server holds numbered databases of
//! keys; the explorer lists the databases, and a database opens in a key
//! browser rather than a table grid (see `keys`). The connection's `database`
//! field is the database number, `0` when empty.

mod admin;
mod cli;
mod edit;
mod keys;
mod logged;
mod read_only;

use std::collections::BTreeMap;
use std::time::Duration;

use redis::{AsyncConnectionConfig, Client, ConnectionAddr, ConnectionInfo, IntoConnectionInfo, RedisConnectionInfo};
use serde::Serialize;

pub use admin::create_user;
pub use cli::{Session, cancel, open_session};
pub use edit::{KeyEdit, NewKey, create_key, edit_key};
pub use keys::{KeyValue, ScanPage, Text, read_key, scan};
use logged::LoggedConnection;
pub use read_only::run_read_only;

use super::{DbObject, ObjectKind};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// How long browsing waits for a reply; consoles wait as long as a command takes.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// A connection to database `db`, or the connection's own when `None`, over TLS
/// when the connection asks for it, recording every command sent on it. Hosts saved as a whole `redis://` or
/// `rediss://` URL, before the form split URLs into fields, still work.
pub(crate) async fn connect(
    connection: &Connection,
    password: Option<&str>,
    db: Option<i64>,
    response_timeout: Option<Duration>,
) -> Result<LoggedConnection> {
    let url = is_url(&connection.host);
    if connection.ssl.mode.encrypts() || connection.host.starts_with("rediss://") {
        super::install_crypto();
    }
    let mut info: ConnectionInfo = if url {
        connection.host.as_str().into_connection_info()?
    } else if connection.ssl.mode.encrypts() {
        let (host, port) = (connection.host.clone(), connection.port);
        ConnectionAddr::TcpTls { host, port, insecure: !connection.ssl.mode.verifies(), tls_params: None }
            .into_connection_info()?
    } else {
        ConnectionAddr::Tcp(connection.host.clone(), connection.port).into_connection_info()?
    };
    let mut settings: RedisConnectionInfo = info.redis_settings().clone();
    if !connection.username.is_empty() {
        settings = settings.set_username(&connection.username);
    }
    if let Some(password) = password.filter(|p| !p.is_empty()) {
        settings = settings.set_password(password);
    }
    let db = match db {
        Some(db) => db,
        None if url && connection.database.is_empty() => settings.db(),
        None => database_number(&connection.database)?,
    };
    info = info.set_redis_settings(settings.set_db(db));

    let config = AsyncConnectionConfig::new()
        .set_connection_timeout(Some(CONNECT_TIMEOUT))
        .set_response_timeout(response_timeout);
    let inner = Client::open(info)?.get_multiplexed_async_connection_with_config(&config).await?;
    Ok(LoggedConnection::new(inner, db))
}

fn is_url(host: &str) -> bool {
    host.starts_with("redis://") || host.starts_with("rediss://")
}

/// `"3"` or `"db3"` as 3; empty means 0.
pub fn database_number(text: &str) -> Result<i64> {
    let digits = text.trim().trim_start_matches("db");
    if digits.is_empty() {
        return Ok(0);
    }
    digits
        .parse()
        .ok()
        .filter(|n| *n >= 0)
        .ok_or_else(|| AppError::Invalid(format!("\"{text}\" isn't a Redis database number. Use 0, 1, 2…")))
}

async fn browse(connection: &Connection, password: Option<&str>, db: Option<i64>) -> Result<LoggedConnection> {
    connect(connection, password, db, Some(RESPONSE_TIMEOUT)).await
}

/// `INFO` as `section field → value`, flattened to `field → value`.
async fn info(conn: &mut LoggedConnection, section: &str) -> Result<BTreeMap<String, String>> {
    let text: String = redis::cmd("INFO").arg(section).query_async(conn).await?;
    Ok(text
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_string(), value.trim().to_string()))
        .collect())
}

pub async fn server_version(connection: &Connection, password: Option<&str>) -> Result<String> {
    let mut conn = browse(connection, password, None).await?;
    let server = info(&mut conn, "server").await?;
    let version = server.get("redis_version").map_or("?", String::as_str);
    let mode = server.get("redis_mode").map_or("standalone", String::as_str);
    Ok(format!("Redis {version} · {mode}"))
}

/// The databases: every one the server is configured with, or, where `CONFIG`
/// is not allowed (managed services), those holding keys plus the connection's own.
pub async fn list_objects(connection: &Connection, password: Option<&str>, path: &[String]) -> Result<Vec<DbObject>> {
    if !path.is_empty() {
        // A database opens in the key browser; it has no children in the tree.
        return Ok(Vec::new());
    }
    let mut conn = browse(connection, password, None).await?;
    let keyspace = keyspace(&mut conn).await?;
    let configured: Option<i64> = redis::cmd("CONFIG")
        .arg("GET")
        .arg("databases")
        .query_async::<Vec<String>>(&mut conn)
        .await
        .ok()
        .and_then(|reply| reply.get(1)?.parse().ok());
    let mut numbers: Vec<i64> = match configured {
        Some(count) => (0..count).collect(),
        None => keyspace.keys().copied().chain([database_number(&connection.database)?]).collect(),
    };
    numbers.sort_unstable();
    numbers.dedup();
    Ok(numbers
        .into_iter()
        .map(|n| DbObject {
            name: format!("db{n}"),
            kind: ObjectKind::Database,
            row_estimate: Some(keyspace.get(&n).copied().unwrap_or(0)),
        })
        .collect())
}

/// Keys per database, from `INFO keyspace` (`db0:keys=12,expires=1,…`).
async fn keyspace(conn: &mut LoggedConnection) -> Result<BTreeMap<i64, i64>> {
    Ok(info(conn, "keyspace")
        .await?
        .into_iter()
        .filter_map(|(db, stats)| {
            let number = db.strip_prefix("db")?.parse().ok()?;
            let keys = stats.split(',').find_map(|pair| pair.strip_prefix("keys="))?.parse().ok()?;
            Some((number, keys))
        })
        .collect())
}

/// What the metrics bar and status bar show.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub version: String,
    /// `standalone`, `sentinel` or `cluster`.
    pub mode: String,
    /// Keys in the selected database.
    pub keys: i64,
    pub used_memory: i64,
    /// `0` when no limit is set.
    pub max_memory: i64,
    pub ops_per_sec: i64,
    pub keyspace_hits: i64,
    pub keyspace_misses: i64,
    pub clients: i64,
    pub evicted_keys: i64,
    pub expired_keys: i64,
}

pub async fn overview(connection: &Connection, password: Option<&str>, db: i64) -> Result<Overview> {
    let mut conn = browse(connection, password, Some(db)).await?;
    let info = info(&mut conn, "all").await?;
    let keys: i64 = redis::cmd("DBSIZE").query_async(&mut conn).await?;
    let number = |field: &str| info.get(field).and_then(|v| v.parse().ok()).unwrap_or(0);
    Ok(Overview {
        version: info.get("redis_version").cloned().unwrap_or_default(),
        mode: info.get("redis_mode").cloned().unwrap_or_else(|| "standalone".into()),
        keys,
        used_memory: number("used_memory"),
        max_memory: number("maxmemory"),
        ops_per_sec: number("instantaneous_ops_per_sec"),
        keyspace_hits: number("keyspace_hits"),
        keyspace_misses: number("keyspace_misses"),
        clients: number("connected_clients"),
        evicted_keys: number("evicted_keys"),
        expired_keys: number("expired_keys"),
    })
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::connections::model::Engine;
    use crate::connections::transport::{Ssl, SslMode};

    pub const PASSWORD: Option<&str> = Some("dbforge");

    pub fn local() -> Connection {
        Connection {
            id: "test".into(),
            name: "test".into(),
            engine: Engine::Redis,
            environment: None,
            // Not `localhost`: Docker on macOS answers slowly on IPv6, which is tried first.
            host: "127.0.0.1".into(),
            port: 56379,
            database: String::new(),
            username: String::new(),
            ssl: crate::connections::transport::Ssl {
                mode: crate::connections::transport::SslMode::Disable,
                ..Default::default()
            },
            ssh: Default::default(),
            srv: false,
            agent_access: Default::default(),
        }
    }

    /// Database `db` emptied, with `setup` run in it. Each test uses its own database.
    pub async fn database(db: i64, setup: &[&[&str]]) -> (Connection, LoggedConnection) {
        let connection = Connection { database: db.to_string(), ..local() };
        let mut conn = connect(&connection, PASSWORD, None, None).await.unwrap();
        redis::cmd("FLUSHDB").query_async::<()>(&mut conn).await.unwrap();
        for command in setup {
            redis::cmd(command[0]).arg(&command[1..]).query_async::<()>(&mut conn).await.unwrap();
        }
        (connection, conn)
    }

    #[test]
    fn reads_database_numbers() {
        assert_eq!(database_number("").unwrap(), 0);
        assert_eq!(database_number("3").unwrap(), 3);
        assert_eq!(database_number("db12").unwrap(), 12);
        assert_eq!(database_number("x").unwrap_err().to_string(), "\"x\" isn't a Redis database number. Use 0, 1, 2…");
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn reports_version_databases_and_metrics() {
        let (connection, _) = database(1, &[&["SET", "a", "1"], &["SET", "b", "2"], &["GET", "a"]]).await;
        assert!(server_version(&connection, PASSWORD).await.unwrap().ends_with(" · standalone"));

        let databases = list_objects(&connection, PASSWORD, &[]).await.unwrap();
        assert_eq!(databases.len(), 16);
        assert_eq!(databases[1], DbObject { name: "db1".into(), kind: ObjectKind::Database, row_estimate: Some(2) });
        assert!(list_objects(&connection, PASSWORD, &["db1".into()]).await.unwrap().is_empty());

        let overview = overview(&connection, PASSWORD, 1).await.unwrap();
        assert_eq!(overview.keys, 2);
        assert!(overview.used_memory > 0 && overview.clients >= 1 && overview.keyspace_hits >= 1, "{overview:?}");
    }

    #[tokio::test]
    #[ignore = "needs a local Redis with TLS on port 56380 (self-signed certificate)"]
    async fn connects_over_tls() {
        let tls = Connection { port: 56380, ssl: Ssl { mode: SslMode::Require, ..Ssl::default() }, ..local() };
        assert!(server_version(&tls, PASSWORD).await.unwrap().starts_with("Redis 7."));
        // Hosts saved as URLs keep working.
        let url = Connection { host: "rediss://127.0.0.1:56380/#insecure".into(), port: 0, ..local() };
        assert!(server_version(&url, PASSWORD).await.is_ok());

        let verified = Connection { ssl: Ssl { mode: SslMode::VerifyFull, ..Ssl::default() }, ..tls };
        let error = server_version(&verified, PASSWORD).await.unwrap_err().to_string();
        assert!(error.starts_with("The server's TLS certificate isn't trusted"), "{error}");
        assert!(error.contains("Accept self-signed certificate"), "{error}");
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn a_missing_password_or_an_unknown_host_says_what_to_fix() {
        let error = server_version(&local(), None).await.unwrap_err();
        assert_eq!(error.to_string(), "This server requires a password. Enter it in Password.");
        let nowhere = Connection { host: "no-such-host.invalid".into(), ..local() };
        let error = server_version(&nowhere, PASSWORD).await.unwrap_err();
        assert!(error.to_string().starts_with("Could not find the host."), "{error}");
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn wrong_password_is_reported() {
        let error = server_version(&local(), Some("wrong")).await.unwrap_err();
        assert_eq!(error.to_string(), "Authentication failed: wrong user name or password.");
    }
}
