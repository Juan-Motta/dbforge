//! Which SSH servers are trusted: the keys in the user's `~/.ssh/known_hosts`,
//! then the ones DBForge accepted on first connection, kept in its own file.

use std::path::Path;

use russh::keys::PublicKey;
use russh::keys::known_hosts::{check_known_hosts_path, learn_known_hosts_path};

/// What checking a server's host key found.
#[derive(Debug, PartialEq, Eq)]
pub enum HostKey {
    /// A file already has this key for the server.
    Known,
    /// No key was known for the server: this one is now recorded (trust on first use).
    Learned,
    /// The server's key is not the one recorded: it may be another machine.
    Changed(String),
    /// A file couldn't be read or written.
    Unreadable(String),
}

/// Checks `key` for `host:port` against `~/.ssh/known_hosts` and then `own`,
/// recording it in `own` when neither knows the server.
pub fn check(host: &str, port: u16, key: &PublicKey, own: &Path) -> HostKey {
    let users = std::env::home_dir().map(|home| home.join(".ssh").join("known_hosts"));
    for file in users.iter().map(|path| path.as_path()).chain([own]) {
        match check_known_hosts_path(host, port, key, file) {
            Ok(true) => return HostKey::Known,
            Ok(false) => {}
            Err(russh::keys::Error::KeyChanged { line }) => {
                return HostKey::Changed(format!(
                    "The SSH server {host}:{port} has a different host key than the one in {} (line {line}). \
                     If the server was reinstalled, remove that line; otherwise another machine may be answering in its place.",
                    file.display()
                ));
            }
            Err(error) => return HostKey::Unreadable(format!("Could not read {}: {error}", file.display())),
        }
    }
    match learn_known_hosts_path(host, port, key, own) {
        Ok(()) => HostKey::Learned,
        Err(error) => HostKey::Unreadable(format!("Could not save the SSH server's key in {}: {error}", own.display())),
    }
}

#[cfg(test)]
mod tests {
    use russh::keys::parse_public_key_base64;

    use super::*;

    const KEYS: [&str; 3] = [
        "AAAAC3NzaC1lZDI1NTE5AAAAIAJFaMvFUbLMaCHGu+86CKs8ZJ4ut5hJVrQK8A4rs9nC",
        "AAAAC3NzaC1lZDI1NTE5AAAAIKSx3ACTdvSyEMHGbm5jbe2BDAgvcqfkmzQIteld9k2N",
        "AAAAC3NzaC1lZDI1NTE5AAAAIJKG16Z5MduFWsAXdMquG576AddFhAJ0esKCGVE+5f0A",
    ];

    fn key(n: usize) -> PublicKey {
        parse_public_key_base64(KEYS[n]).unwrap()
    }

    #[test]
    fn a_new_server_is_learned_then_known_and_a_changed_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("known_hosts");
        assert_eq!(check("bastion.test", 2222, &key(0), &own), HostKey::Learned);
        assert_eq!(check("bastion.test", 2222, &key(0), &own), HostKey::Known);
        let HostKey::Changed(message) = check("bastion.test", 2222, &key(1), &own) else { panic!("not refused") };
        assert!(message.contains("bastion.test:2222") && message.contains(&own.display().to_string()), "{message}");
        assert_eq!(check("other.test", 22, &key(2), &own), HostKey::Learned, "per server");
    }
}
