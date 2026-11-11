//! One SSH session: connecting, checking the server's host key, signing in,
//! and opening a channel to the database server for each forwarded socket.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, AuthResult, Handle};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::{PrivateKeyWithHashAlg, PublicKey};
use russh::{ChannelStream, Disconnect};

use super::host_keys::{self, HostKey};
use crate::connections::transport::{SshAuth, SshTunnel};
use crate::error::{AppError, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

pub struct Session {
    handle: Handle<Checker>,
}

impl Session {
    /// Connects to the SSH server in `ssh` and signs in, with `secret` as the
    /// password or the key's passphrase.
    pub async fn open(ssh: &SshTunnel, secret: Option<&str>, known_hosts: &Path) -> Result<Self> {
        let config = Arc::new(client::Config {
            // Keeps a quiet tunnel from being dropped by firewalls between queries.
            keepalive_interval: Some(Duration::from_secs(30)),
            nodelay: true,
            ..Default::default()
        });
        let checker = Checker { host: ssh.host.clone(), port: ssh.port, known_hosts: known_hosts.to_path_buf() };
        let address = (ssh.host.as_str(), ssh.port);
        let mut handle = match tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config, address, checker)).await {
            Ok(Ok(handle)) => handle,
            Ok(Err(CheckError::HostKey(message))) => return Err(AppError::Invalid(message)),
            Ok(Err(CheckError::Ssh(error))) => return Err(unreachable(ssh, &error.to_string())),
            Err(_) => return Err(unreachable(ssh, "it did not answer in time")),
        };
        let signed_in = match ssh.auth {
            SshAuth::Password => {
                handle.authenticate_password(&ssh.username, secret.unwrap_or_default()).await.map_err(ssh_error)?
            }
            SshAuth::Key => {
                let key = russh::keys::load_secret_key(&ssh.key_file, secret.filter(|s| !s.is_empty()))
                    .map_err(|error| key_error(&ssh.key_file, &error))?;
                let hash = handle.best_supported_rsa_hash().await.map_err(ssh_error)?.flatten();
                let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
                handle.authenticate_publickey(&ssh.username, key).await.map_err(ssh_error)?
            }
            SshAuth::Agent => sign_in_with_agent(&mut handle, &ssh.username).await?,
        };
        if !signed_in.success() {
            return Err(AppError::Invalid(format!(
                "The SSH server {}:{} refused the user {} with {}",
                ssh.host,
                ssh.port,
                ssh.username,
                match ssh.auth {
                    SshAuth::Password => "this password",
                    SshAuth::Key => "this key",
                    SshAuth::Agent => "the keys in the SSH agent",
                }
            )));
        }
        Ok(Self { handle })
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// A stream to `host:port`, as the SSH server reaches it, for a socket from `peer`.
    pub async fn open_channel(&self, host: &str, port: u16, peer: SocketAddr) -> Result<ChannelStream<client::Msg>> {
        let channel = self
            .handle
            .channel_open_direct_tcpip(host, port.into(), peer.ip().to_string(), peer.port().into())
            .await
            .map_err(ssh_error)?;
        Ok(channel.into_stream())
    }

    pub async fn close(&self) {
        let _ = self.handle.disconnect(Disconnect::ByApplication, "", "en").await;
    }
}

/// Tries each key the system's SSH agent holds until the server takes one.
async fn sign_in_with_agent(handle: &mut Handle<Checker>, user: &str) -> Result<AuthResult> {
    let mut agent = connect_agent().await?;
    let identities = agent.request_identities().await.map_err(|error| AppError::Invalid(error.to_string()))?;
    let keys: Vec<PublicKey> = identities
        .into_iter()
        .filter_map(|identity| match identity {
            AgentIdentity::PublicKey { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    if keys.is_empty() {
        return Err(AppError::Invalid("The SSH agent holds no keys: add one with ssh-add".into()));
    }
    let hash = handle.best_supported_rsa_hash().await.map_err(ssh_error)?.flatten();
    let mut refused = AuthResult::Failure { remaining_methods: russh::MethodSet::empty(), partial_success: false };
    for key in keys {
        let result = handle
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
            .map_err(|error| AppError::Invalid(format!("The SSH agent could not sign: {error}")))?;
        if result.success() {
            return Ok(result);
        }
        refused = result;
    }
    Ok(refused)
}

/// The system's SSH agent, wherever it listens.
type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;

/// The agent at `SSH_AUTH_SOCK`, a Unix socket.
#[cfg(unix)]
async fn connect_agent() -> Result<Agent> {
    let agent = AgentClient::connect_env()
        .await
        .map_err(|error| AppError::Invalid(format!("Could not reach the SSH agent (SSH_AUTH_SOCK): {error}")))?;
    Ok(agent.dynamic())
}

/// Windows' OpenSSH agent, on the named pipe `SSH_AUTH_SOCK` names or its
/// default one, or else Pageant (PuTTY's agent).
#[cfg(windows)]
async fn connect_agent() -> Result<Agent> {
    let pipe = std::env::var("SSH_AUTH_SOCK").unwrap_or_else(|_| r"\\.\pipe\openssh-ssh-agent".into());
    if let Ok(agent) = AgentClient::connect_named_pipe(&pipe).await {
        return Ok(agent.dynamic());
    }
    let agent = AgentClient::connect_pageant().await.map_err(|error| {
        AppError::Invalid(format!(
            "Could not reach an SSH agent: start the OpenSSH Authentication Agent service, or Pageant ({error})"
        ))
    })?;
    Ok(agent.dynamic())
}

fn unreachable(ssh: &SshTunnel, cause: &str) -> AppError {
    AppError::Invalid(format!("Could not reach the SSH server {}:{}: {cause}", ssh.host, ssh.port))
}

fn ssh_error(error: russh::Error) -> AppError {
    AppError::Invalid(format!("SSH: {error}"))
}

/// Why the key at `path` couldn't be loaded: an encrypted key without its
/// passphrase or with a wrong one, or a file that isn't a key.
fn key_error(path: &str, error: &russh::keys::Error) -> AppError {
    let encrypted = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| russh::keys::PrivateKey::from_openssh(text).ok())
        .is_some_and(|key| key.is_encrypted());
    if encrypted {
        AppError::Invalid(format!("The SSH key {path} needs its passphrase, or the passphrase is wrong"))
    } else {
        AppError::Invalid(format!("Could not read the SSH key {path}: {error}"))
    }
}

/// Checks the server's host key against the ones known for its address.
struct Checker {
    host: String,
    port: u16,
    known_hosts: PathBuf,
}

/// Why a connection stopped before signing in.
#[derive(Debug)]
enum CheckError {
    Ssh(russh::Error),
    /// The host key changed, or couldn't be checked; the message says which.
    HostKey(String),
}

impl From<russh::Error> for CheckError {
    fn from(error: russh::Error) -> Self {
        CheckError::Ssh(error)
    }
}

impl client::Handler for Checker {
    type Error = CheckError;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Err(CheckError::HostKey(format!(
                "The SSH server {}:{} presented a host certificate, which DBForge doesn't check yet",
                self.host, self.port
            )));
        };
        match host_keys::check(&self.host, self.port, key, &self.known_hosts) {
            HostKey::Known | HostKey::Learned => Ok(true),
            HostKey::Changed(message) | HostKey::Unreadable(message) => Err(CheckError::HostKey(message)),
        }
    }
}
