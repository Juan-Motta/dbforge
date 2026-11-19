//! Where updates come from and the key they are signed with: the same as
//! the former Tauri app's (its `plugins.updater` settings), so installed
//! copies and this one read the same manifest and trust the same signatures.

use std::path::PathBuf;

/// Each release publishes its own `latest.json`; this always points at the newest's.
pub const ENDPOINT: &str = "https://github.com/Juan-Motta/dbforge/releases/latest/download/latest.json";

/// The updater's public key (base64 of the minisign public key file); the
/// release signs with its private half (`dbforge-release sign`).
pub const PUBLIC_KEY: &str = include_str!("../../../../packaging/updater.pub");

/// A signature must name the version it was made for (`requireSignedVersion`).
pub const REQUIRE_SIGNED_VERSION: bool = true;

/// The updater's settings: the published ones, which development builds may
/// point elsewhere to try the updater against a local manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub endpoint: String,
    pub public_key: String,
    /// Development only: what to replace instead of the running copy.
    pub install_place: Option<PathBuf>,
}

impl Settings {
    /// The published settings; in development builds, `DBFORGE_UPDATE_ENDPOINT`,
    /// `DBFORGE_UPDATE_PUBKEY` and `DBFORGE_UPDATE_INSTALL_PATH` override them.
    /// A release build ignores those variables.
    pub fn current() -> Self {
        let published =
            Self { endpoint: ENDPOINT.to_owned(), public_key: PUBLIC_KEY.trim().to_owned(), install_place: None };
        #[cfg(debug_assertions)]
        {
            let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.trim().is_empty());
            Self {
                endpoint: variable("DBFORGE_UPDATE_ENDPOINT").unwrap_or(published.endpoint),
                public_key: variable("DBFORGE_UPDATE_PUBKEY").unwrap_or(published.public_key),
                install_place: variable("DBFORGE_UPDATE_INSTALL_PATH").map(PathBuf::from),
            }
        }
        #[cfg(not(debug_assertions))]
        published
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values every installed 0.3.x copy was built with (the Tauri
    /// app's `plugins.updater`): changing either strands those copies.
    #[test]
    fn keeps_the_tauri_app_s_endpoint_and_key() {
        assert_eq!(ENDPOINT, "https://github.com/Juan-Motta/dbforge/releases/latest/download/latest.json");
        assert_eq!(
            PUBLIC_KEY.trim(),
            "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDI3NEREMDU1NTRDODQ5RTEKUldUaFNjaFVWZEJOSnhjVTU2N2s0NUFza29CR25OelZ3WTlHTGNOYmZ6RTBpczZJOGNiK0JybnQK"
        );
        const { assert!(REQUIRE_SIGNED_VERSION) };
    }
}
