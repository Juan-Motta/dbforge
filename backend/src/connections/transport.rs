//! How a connection reaches its server: whether and how SSL/TLS is used and
//! verified, and whether it goes through an SSH tunnel. What each engine can do
//! with them is listed here, so validation and the drivers agree.

use serde::{Deserialize, Serialize};

use super::model::Engine;

/// How a connection uses SSL/TLS, from not at all to verifying the server's
/// certificate and its name. The names follow PostgreSQL's `sslmode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    /// Never encrypts.
    #[default]
    Disable,
    /// Encrypts when the server offers it, without checking its certificate.
    Prefer,
    /// Always encrypts, without checking the server's certificate.
    Require,
    /// Always encrypts, and checks the certificate was signed by a trusted authority.
    VerifyCa,
    /// As `VerifyCa`, and checks the certificate names the server.
    VerifyFull,
}

impl SslMode {
    /// The connection is encrypted, at least when the server allows it.
    pub fn encrypts(self) -> bool {
        self != SslMode::Disable
    }

    /// The server's certificate is checked against the trusted authorities.
    pub fn verifies(self) -> bool {
        matches!(self, SslMode::VerifyCa | SslMode::VerifyFull)
    }

    pub fn label(self) -> &'static str {
        match self {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify CA",
            SslMode::VerifyFull => "verify CA and host",
        }
    }
}

/// A connection's SSL/TLS settings. Files are paths on this machine, in PEM;
/// empty when not used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ssl {
    pub mode: SslMode,
    /// Authorities to trust besides the system's, to verify the server.
    #[serde(default)]
    pub ca_file: String,
    /// The client's certificate, for servers that ask for one. For MongoDB it
    /// holds the key too, as the driver reads both from one file.
    #[serde(default)]
    pub cert_file: String,
    /// The client certificate's private key.
    #[serde(default)]
    pub key_file: String,
}

/// How the SSH tunnel signs in to its server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SshAuth {
    /// The SSH secret is the account's password.
    #[default]
    Password,
    /// A private key file; the SSH secret, if any, is its passphrase.
    Key,
    /// The keys the system's SSH agent holds (`SSH_AUTH_SOCK`).
    Agent,
}

/// A tunnel through an SSH server that can reach the database, as bastions
/// give. The password or passphrase is kept with the saved passwords.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnel {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
    /// The private key, with `SshAuth::Key`.
    #[serde(default)]
    pub key_file: String,
}

impl Default for SshTunnel {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: 22,
            username: String::new(),
            auth: SshAuth::default(),
            key_file: String::new(),
        }
    }
}

/// The SSL/TLS settings an engine's driver can apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SslSupport {
    pub modes: &'static [SslMode],
    /// It can trust an authority from a file.
    pub ca_file: bool,
    /// It can present a client certificate.
    pub client_cert: bool,
    /// The certificate and its key are separate files (MongoDB reads one file with both).
    pub separate_key: bool,
    /// It can check the server's name when it is reached through an SSH tunnel,
    /// whose local end is all the driver sees.
    pub verifies_name_through_ssh: bool,
}

use SslMode::*;

const NONE: SslSupport = SslSupport {
    modes: &[Disable],
    ca_file: false,
    client_cert: false,
    separate_key: false,
    verifies_name_through_ssh: false,
};

impl Engine {
    /// What the engine's driver can do with SSL/TLS.
    pub fn ssl_support(self) -> SslSupport {
        match self {
            Engine::Postgres | Engine::Mysql | Engine::Mariadb => SslSupport {
                modes: &[Disable, Prefer, Require, VerifyCa, VerifyFull],
                ca_file: true,
                client_cert: true,
                separate_key: true,
                verifies_name_through_ssh: false,
            },
            // The driver can check the certificate's name apart from the address it connects to.
            Engine::Sqlserver => SslSupport {
                modes: &[Disable, Prefer, Require, VerifyFull],
                ca_file: true,
                client_cert: true,
                separate_key: true,
                verifies_name_through_ssh: true,
            },
            Engine::Mongodb => SslSupport {
                modes: &[Disable, Require, VerifyFull],
                ca_file: true,
                client_cert: true,
                separate_key: false,
                verifies_name_through_ssh: false,
            },
            Engine::Redis => SslSupport {
                modes: &[Disable, Require, VerifyFull],
                ca_file: true,
                client_cert: true,
                separate_key: true,
                verifies_name_through_ssh: false,
            },
            // HTTPS, or HTTP for the local emulator; its form has no files.
            Engine::Cosmosdb => SslSupport { modes: &[Disable, Require, VerifyFull], ..NONE },
            Engine::Sqlite => NONE,
        }
    }

    /// Its server can be reached through an SSH tunnel: not a file, nor an account URL.
    pub fn tunnels(self) -> bool {
        !matches!(self, Engine::Sqlite | Engine::Cosmosdb)
    }

    /// The SSL mode of a connection saved before SSL modes existed, from its
    /// old `tls` and `allowSelfSigned` flags, so it behaves as it did.
    pub fn legacy_ssl_mode(self, tls: bool, allow_self_signed: bool) -> SslMode {
        match self {
            // The drivers' defaults: encrypt when offered, checking nothing.
            Engine::Postgres | Engine::Mysql | Engine::Mariadb => Prefer,
            // Always encrypted, trusting any certificate.
            Engine::Sqlserver => Require,
            Engine::Mongodb | Engine::Redis | Engine::Cosmosdb => match (tls, allow_self_signed) {
                (false, _) => Disable,
                (true, true) => Require,
                (true, false) => VerifyFull,
            },
            Engine::Sqlite => Disable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_serialize_as_postgres_names_them() {
        assert_eq!(serde_json::to_string(&SslMode::VerifyFull).unwrap(), "\"verify-full\"");
        assert_eq!(serde_json::to_string(&SslMode::VerifyCa).unwrap(), "\"verify-ca\"");
    }

    #[test]
    fn every_engine_supports_its_legacy_modes() {
        for engine in [
            Engine::Postgres,
            Engine::Mysql,
            Engine::Mariadb,
            Engine::Sqlite,
            Engine::Sqlserver,
            Engine::Mongodb,
            Engine::Redis,
            Engine::Cosmosdb,
        ] {
            let modes = engine.ssl_support().modes;
            for (tls, self_signed) in [(false, false), (true, false), (true, true)] {
                assert!(modes.contains(&engine.legacy_ssl_mode(tls, self_signed)), "{engine:?}");
            }
        }
    }
}
