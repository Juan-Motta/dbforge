//! SSH tunnels. A connection with an SSH tunnel reaches its server through a
//! local port that forwards every socket over one SSH session to the server.
//! Sessions are shared by every operation on the same settings, opened on first
//! use and closed after they sit idle, so no query waits for an SSH handshake.

mod host_keys;
mod session;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::connections::model::Connection;
use crate::connections::transport::SshTunnel;
use crate::error::{AppError, Result};

/// How long a tunnel with no open socket stays up.
const IDLE: Duration = Duration::from_secs(300);

/// Where a connection's driver should connect: the server itself, or the
/// local end of its tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub host: String,
    pub port: u16,
}

/// Reads a saved secret by its credential id.
pub type SecretSource = Box<dyn Fn(&str) -> Result<Option<String>> + Send + Sync>;

/// What one tunnel goes to, through which SSH server, for which connection's secret.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    connection_id: String,
    ssh: SshTunnel,
    host: String,
    port: u16,
}

/// A tunnel's place: locked while it opens, so operations that start together
/// share one tunnel, without holding up other connections' tunnels.
type Slot = Arc<tokio::sync::Mutex<Option<Tunnel>>>;

/// The process's tunnels.
pub struct Tunnels {
    open: Mutex<HashMap<Key, Slot>>,
    /// Secrets for connections being tested before they are saved, by connection id.
    lent: Mutex<HashMap<String, String>>,
    secrets: SecretSource,
    known_hosts: PathBuf,
}

static TUNNELS: OnceLock<Tunnels> = OnceLock::new();

/// Sets up the tunnels once: saved SSH secrets come from `secrets`, and the
/// host keys DBForge has accepted are kept in `known_hosts`.
pub fn init(known_hosts: PathBuf, secrets: SecretSource) {
    let tunnels = Tunnels { open: Default::default(), lent: Default::default(), secrets, known_hosts };
    if TUNNELS.set(tunnels).is_ok() {
        tauri::async_runtime::spawn(close_idle());
    }
}

fn tunnels() -> Result<&'static Tunnels> {
    TUNNELS.get().ok_or_else(|| AppError::Invalid("SSH tunnels are not available yet".into()))
}

/// Where `connection`'s driver should connect, opening its tunnel when it has one.
pub async fn route(connection: &Connection) -> Result<Route> {
    if !connection.ssh.enabled {
        return Ok(Route { host: connection.host.clone(), port: connection.port });
    }
    let tunnels = tunnels()?;
    let key = Key {
        connection_id: connection.id.clone(),
        ssh: connection.ssh.clone(),
        host: connection.host.clone(),
        port: connection.port,
    };
    let slot = tunnels.open.lock().unwrap().entry(key.clone()).or_default().clone();
    let mut slot = slot.lock().await;
    if let Some(tunnel) = slot.as_ref().filter(|tunnel| tunnel.is_up()) {
        tunnel.touch();
        return Ok(tunnel.route());
    }
    let secret = tunnels.secret(&connection.id)?;
    let tunnel = Tunnel::open(&key, secret.as_deref(), &tunnels.known_hosts).await?;
    let route = tunnel.route();
    if let Some(old) = slot.replace(tunnel) {
        old.close();
    }
    Ok(route)
}

/// Lends `secret` as connection `id`'s SSH secret until the guard drops, for a
/// test of settings that are not saved yet.
pub fn lend_secret(id: &str, secret: Option<String>) -> Result<LentSecret> {
    let tunnels = tunnels()?;
    if let Some(secret) = secret {
        tunnels.lent.lock().unwrap().insert(id.to_string(), secret);
    }
    Ok(LentSecret { id: id.to_string() })
}

/// Takes a lent secret back when dropped.
pub struct LentSecret {
    id: String,
}

impl Drop for LentSecret {
    fn drop(&mut self) {
        if let Some(tunnels) = TUNNELS.get() {
            tunnels.lent.lock().unwrap().remove(&self.id);
        }
    }
}

/// Closes the tunnels of connection `id`, e.g. after it is edited or deleted.
pub async fn close(id: &str) {
    let Some(tunnels) = TUNNELS.get() else { return };
    let slots: Vec<Slot> = {
        let mut open = tunnels.open.lock().unwrap();
        let keys: Vec<Key> = open.keys().filter(|key| key.connection_id == id).cloned().collect();
        keys.iter().filter_map(|key| open.remove(key)).collect()
    };
    for slot in slots {
        if let Some(tunnel) = slot.lock().await.take() {
            tunnel.close();
        }
    }
}

impl Tunnels {
    fn secret(&self, id: &str) -> Result<Option<String>> {
        if let Some(secret) = self.lent.lock().unwrap().get(id) {
            return Ok(Some(secret.clone()));
        }
        (self.secrets)(&Connection::ssh_credential_id(id))
    }
}

/// Every minute, closes the tunnels idle for longer than [`IDLE`] and forgets
/// the ones that went down. A tunnel being opened is left alone.
async fn close_idle() {
    let mut every = tokio::time::interval(Duration::from_secs(60));
    loop {
        every.tick().await;
        let Some(tunnels) = TUNNELS.get() else { continue };
        tunnels.open.lock().unwrap().retain(|_, slot| {
            let Ok(mut tunnel) = slot.try_lock() else { return true };
            match tunnel.as_ref() {
                // An empty slot may have just been handed to a tunnel about to open.
                None => true,
                Some(open) if open.is_up() && !open.idle_for(IDLE) => true,
                Some(_) => {
                    tunnel.take().expect("checked").close();
                    false
                }
            }
        });
    }
}

/// One open tunnel: an SSH session and the local port that forwards to the server.
struct Tunnel {
    session: Arc<session::Session>,
    port: u16,
    forwarding: JoinHandle<()>,
    /// Sockets being forwarded now.
    active: Arc<AtomicUsize>,
    last_used: Arc<Mutex<Instant>>,
}

impl Tunnel {
    async fn open(key: &Key, secret: Option<&str>, known_hosts: &std::path::Path) -> Result<Self> {
        let session = Arc::new(session::Session::open(&key.ssh, secret, known_hosts).await?);
        // Only this machine may use the tunnel.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let active = Arc::new(AtomicUsize::new(0));
        let last_used = Arc::new(Mutex::new(Instant::now()));
        let forwarding = tokio::spawn(forward(
            listener,
            session.clone(),
            (key.host.clone(), key.port),
            active.clone(),
            last_used.clone(),
        ));
        Ok(Self { session, port, forwarding, active, last_used })
    }

    fn route(&self) -> Route {
        Route { host: "127.0.0.1".into(), port: self.port }
    }

    fn is_up(&self) -> bool {
        !self.session.is_closed() && !self.forwarding.is_finished()
    }

    fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }

    fn idle_for(&self, period: Duration) -> bool {
        self.active.load(Ordering::SeqCst) == 0 && self.last_used.lock().unwrap().elapsed() > period
    }

    fn close(&self) {
        self.forwarding.abort();
        let session = self.session.clone();
        tokio::spawn(async move { session.close().await });
    }
}

/// A tunnel dropped without `close` (its slot was replaced or forgotten) still
/// stops forwarding, so no listener or session outlives it.
impl Drop for Tunnel {
    fn drop(&mut self) {
        self.forwarding.abort();
    }
}

/// Forwards every socket accepted on `listener` to `target` over `session`.
async fn forward(
    listener: TcpListener,
    session: Arc<session::Session>,
    target: (String, u16),
    active: Arc<AtomicUsize>,
    last_used: Arc<Mutex<Instant>>,
) {
    while let Ok((mut socket, peer)) = listener.accept().await {
        let (session, target, active, last_used) = (session.clone(), target.clone(), active.clone(), last_used.clone());
        tokio::spawn(async move {
            active.fetch_add(1, Ordering::SeqCst);
            // The driver sees the failure as a closed socket; the reason is logged.
            match session.open_channel(&target.0, target.1, peer).await {
                Ok(mut channel) => {
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut channel).await;
                }
                Err(error) => eprintln!("SSH tunnel to {}:{}: {error}", target.0, target.1),
            }
            *last_used.lock().unwrap() = Instant::now();
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
pub(crate) mod tests;
