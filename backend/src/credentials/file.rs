//! Passwords in a local file that works the same on every platform and never
//! asks for anything.
//!
//! `credentials.bin` holds the passwords as JSON, encrypted with
//! XChaCha20-Poly1305. Its key is derived (HKDF-SHA256) from a random key in
//! `credentials.key` and this machine's id, so a copy of both files on another
//! machine can't be read. Both files are owner-only on macOS and Linux. Someone
//! using the same account on this machine can still read them: the protection
//! is against copies (backups, synced folders), not against that.

use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;

use super::SecretStore;
use crate::error::{AppError, Result};

const DATA_FILE: &str = "credentials.bin";
const KEY_FILE: &str = "credentials.key";
/// The file's first bytes: what it is and its format version. Also
/// authenticated, so a file of another version is never misread.
const HEADER: &[u8] = b"DBFORGE-CREDENTIALS\x01";
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;

const UNREADABLE: &str = "The saved passwords can't be read on this computer: the file was made on another computer, \
                          or it was changed. Enter the passwords again.";

/// This machine's id, read from the OS without asking anything.
pub type MachineId = fn() -> Result<String>;

pub fn machine_id() -> Result<String> {
    machine_uid::get().map_err(|error| AppError::Invalid(format!("This computer's id can't be read: {error}")))
}

/// What the file holds, as this machine sees it.
enum Contents {
    Passwords(BTreeMap<String, String>),
    /// Made on another machine, changed, or missing its key.
    Unreadable,
}

pub struct EncryptedFile {
    dir: PathBuf,
    machine_id: MachineId,
}

impl EncryptedFile {
    pub fn new(dir: PathBuf, machine_id: MachineId) -> Self {
        Self { dir, machine_id }
    }

    /// Where the passwords are, as the settings show it.
    pub fn path(&self) -> PathBuf {
        self.dir.join(DATA_FILE)
    }

    fn read(&self) -> Result<Contents> {
        let bytes = match fs::read(self.path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Contents::Passwords(BTreeMap::new())),
            Err(error) => return Err(error.into()),
        };
        let key = match fs::read(self.dir.join(KEY_FILE)) {
            Ok(key) => key,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Contents::Unreadable),
            Err(error) => return Err(error.into()),
        };
        let Some(rest) = bytes.strip_prefix(HEADER) else { return Ok(Contents::Unreadable) };
        if rest.len() < NONCE_LEN || key.len() != KEY_LEN {
            return Ok(Contents::Unreadable);
        }
        let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
        let Ok(nonce) = XNonce::try_from(nonce) else { return Ok(Contents::Unreadable) };
        let cipher = self.cipher(&key)?;
        let Ok(plaintext) = cipher.decrypt(&nonce, Payload { msg: ciphertext, aad: HEADER }) else {
            return Ok(Contents::Unreadable);
        };
        Ok(serde_json::from_slice(&plaintext).map_or(Contents::Unreadable, Contents::Passwords))
    }

    fn write(&self, passwords: &BTreeMap<String, String>) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        let key = self.key()?;
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(random_error)?;
        let plaintext = serde_json::to_vec(passwords)?;
        let ciphertext = self
            .cipher(&key)?
            .encrypt(&XNonce::from(nonce), Payload { msg: &plaintext, aad: HEADER })
            .map_err(|_| AppError::Invalid("The passwords couldn't be encrypted.".into()))?;
        let mut bytes = HEADER.to_vec();
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&ciphertext);
        write_private(&self.path(), &bytes)
    }

    /// The random half of the key, created on first use.
    fn key(&self) -> Result<Vec<u8>> {
        let path = self.dir.join(KEY_FILE);
        match fs::read(&path) {
            Ok(key) if key.len() == KEY_LEN => return Ok(key),
            Ok(_) => return Err(AppError::Invalid(UNREADABLE.into())),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut key = vec![0u8; KEY_LEN];
        getrandom::fill(&mut key).map_err(random_error)?;
        write_private(&path, &key)?;
        Ok(key)
    }

    fn cipher(&self, key: &[u8]) -> Result<XChaCha20Poly1305> {
        let machine = (self.machine_id)()?;
        let mut derived = [0u8; KEY_LEN];
        Hkdf::<Sha256>::new(Some(machine.as_bytes()), key)
            .expand(b"dbforge credentials v1", &mut derived)
            .map_err(|_| AppError::Invalid("The passwords' key couldn't be derived.".into()))?;
        XChaCha20Poly1305::new_from_slice(&derived)
            .map_err(|_| AppError::Invalid("The passwords' key couldn't be derived.".into()))
    }
}

impl SecretStore for EncryptedFile {
    fn get(&self, connection_id: &str) -> Result<Option<String>> {
        match self.read()? {
            Contents::Passwords(mut passwords) => Ok(passwords.remove(connection_id)),
            Contents::Unreadable => Err(AppError::Invalid(UNREADABLE.into())),
        }
    }

    /// A file that can't be read here holds nothing this machine can use, so
    /// saving a password starts a new one.
    fn set(&self, connection_id: &str, password: &str) -> Result<()> {
        let mut passwords = match self.read()? {
            Contents::Passwords(passwords) => passwords,
            Contents::Unreadable => BTreeMap::new(),
        };
        passwords.insert(connection_id.to_string(), password.to_string());
        self.write(&passwords)
    }

    fn delete(&self, connection_id: &str) -> Result<()> {
        // An unreadable file has nothing this machine could remove.
        if let Contents::Passwords(mut passwords) = self.read()?
            && passwords.remove(connection_id).is_some()
        {
            self.write(&passwords)?;
        }
        Ok(())
    }
}

fn random_error(error: getrandom::Error) -> AppError {
    AppError::Invalid(format!("No random bytes for the passwords' key: {error}"))
}

/// Writes through a temporary file, owner-only from the moment it exists, so a
/// crash never leaves half a file and no other user can read it.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    // A leftover temporary file from a crash keeps its old mode; tighten it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub fn this_machine() -> Result<String> {
        Ok("machine-a".into())
    }

    fn other_machine() -> Result<String> {
        Ok("machine-b".into())
    }

    /// A temporary folder, removed when dropped (also when a test fails).
    pub fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn keeps_passwords_encrypted_and_owner_only() {
        let scratch = scratch();
        let dir = scratch.path().to_path_buf();
        let file = EncryptedFile::new(dir.clone(), this_machine);
        assert_eq!(file.get("a").unwrap(), None);
        file.set("a", "it's \\ sécret").unwrap();
        file.set("b", "two").unwrap();
        assert_eq!(file.get("a").unwrap().as_deref(), Some("it's \\ sécret"));
        assert_eq!(file.get("b").unwrap().as_deref(), Some("two"));

        let bytes = fs::read(file.path()).unwrap();
        assert!(bytes.starts_with(HEADER));
        assert!(!bytes.windows(3).any(|w| w == b"two"), "the password is in the file as plain text");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in [DATA_FILE, KEY_FILE] {
                let mode = fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "{name}");
            }
        }

        file.delete("a").unwrap();
        assert_eq!(file.get("a").unwrap(), None);
        assert_eq!(file.get("b").unwrap().as_deref(), Some("two"));
    }

    #[test]
    fn a_copy_on_another_machine_or_a_changed_file_cant_be_read() {
        let scratch = scratch();
        let dir = scratch.path().to_path_buf();
        EncryptedFile::new(dir.clone(), this_machine).set("a", "secret").unwrap();

        let elsewhere = EncryptedFile::new(dir.clone(), other_machine);
        assert_eq!(elsewhere.get("a").unwrap_err().to_string(), UNREADABLE);
        // Saving there starts a new file, readable there.
        elsewhere.set("b", "new").unwrap();
        assert_eq!(elsewhere.get("b").unwrap().as_deref(), Some("new"));
        assert_eq!(elsewhere.get("a").unwrap(), None);

        let here = EncryptedFile::new(dir.clone(), this_machine);
        here.set("a", "secret").unwrap();
        let mut bytes = fs::read(here.path()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(here.path(), &bytes).unwrap();
        assert_eq!(here.get("a").unwrap_err().to_string(), UNREADABLE);
        // Deleting from an unreadable file has nothing to remove.
        here.delete("a").unwrap();

        fs::remove_file(dir.join(KEY_FILE)).unwrap();
        assert_eq!(here.get("a").unwrap_err().to_string(), UNREADABLE);
    }
}
