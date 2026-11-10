//! Where connection passwords are kept: the OS keychain (the default) or an
//! encrypted local file, as the user chooses in the settings.

mod file;
mod keychain;

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use self::file::{EncryptedFile, MachineId};
use self::keychain::Keychain;
use crate::error::{AppError, Result};

/// Reads, saves and removes one connection's password.
trait SecretStore: Send + Sync {
    fn get(&self, connection_id: &str) -> Result<Option<String>>;
    fn set(&self, connection_id: &str, password: &str) -> Result<()>;
    fn delete(&self, connection_id: &str) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Storage {
    #[default]
    Keychain,
    File,
}

/// The settings DBForge keeps for itself (the interface's own preferences
/// live in the web view).
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    #[serde(default)]
    credential_storage: Storage,
}

/// What the settings show about it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    storage: Storage,
    /// Where the encrypted file is, or would be.
    file_path: String,
}

pub struct Credentials {
    settings_path: PathBuf,
    storage: Mutex<Storage>,
    keychain: Box<dyn SecretStore>,
    file: EncryptedFile,
}

impl Credentials {
    /// Reads the chosen storage from `settings.json` in `dir`; the keychain
    /// when nothing was chosen.
    pub fn load(dir: PathBuf) -> Result<Self> {
        Self::with(dir, Box::new(Keychain), file::machine_id)
    }

    fn with(dir: PathBuf, keychain: Box<dyn SecretStore>, machine_id: MachineId) -> Result<Self> {
        let settings_path = dir.join("settings.json");
        let settings: Settings = match fs::read_to_string(&settings_path) {
            Ok(json) => serde_json::from_str(&json)?,
            Err(error) if error.kind() == ErrorKind::NotFound => Settings::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            settings_path,
            storage: Mutex::new(settings.credential_storage),
            keychain,
            file: EncryptedFile::new(dir, machine_id),
        })
    }

    pub fn info(&self) -> StorageInfo {
        StorageInfo { storage: *self.lock(), file_path: self.file.path().display().to_string() }
    }

    pub fn get(&self, connection_id: &str) -> Result<Option<String>> {
        let storage = self.lock();
        self.store(*storage).get(connection_id)
    }

    /// Saves the password, or removes it when empty.
    pub fn set(&self, connection_id: &str, password: &str) -> Result<()> {
        let storage = self.lock();
        if password.is_empty() {
            return self.store(*storage).delete(connection_id);
        }
        self.store(*storage).set(connection_id, password)
    }

    pub fn delete(&self, connection_id: &str) -> Result<()> {
        let storage = self.lock();
        self.store(*storage).delete(connection_id)
    }

    /// Moves the passwords of `connection_ids` to `to`, then saves the choice.
    /// If one can't be moved, or the choice can't be saved, nothing switches
    /// and the copies already made are removed again.
    pub fn switch(&self, to: Storage, connection_ids: &[String]) -> Result<()> {
        let mut storage = self.lock();
        if *storage == to {
            return Ok(());
        }
        let (from, into) = (self.store(*storage), self.store(to));
        let moved = copy_all(from, into, connection_ids)?;
        if let Err(error) = self.save(to) {
            undo(into, &moved);
            return Err(error);
        }
        *storage = to;
        // The passwords now live in `to`; copies left in the old place would
        // keep them where the user chose not to.
        let left: Vec<&str> = moved.iter().filter(|id| from.delete(id).is_err()).map(String::as_str).collect();
        if !left.is_empty() {
            return Err(AppError::Invalid(format!(
                "Passwords are now saved in the new place, but {} couldn't be removed from the old one.",
                left.len()
            )));
        }
        Ok(())
    }

    fn store(&self, storage: Storage) -> &dyn SecretStore {
        match storage {
            Storage::Keychain => self.keychain.as_ref(),
            Storage::File => &self.file,
        }
    }

    fn save(&self, storage: Storage) -> Result<()> {
        if let Some(dir) = self.settings_path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.settings_path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(&Settings { credential_storage: storage })?)?;
        fs::rename(&tmp, &self.settings_path)?;
        Ok(())
    }

    /// Reads and writes wait while the storage switches.
    fn lock(&self) -> MutexGuard<'_, Storage> {
        self.storage.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Copies every password `from` holds into `into`; the ids copied. On the
/// first failure the copies are removed again.
fn copy_all(from: &dyn SecretStore, into: &dyn SecretStore, connection_ids: &[String]) -> Result<Vec<String>> {
    let mut moved = Vec::new();
    for id in connection_ids {
        let copied = from.get(id).and_then(|password| match password {
            Some(password) => into.set(id, &password).map(|()| true),
            None => Ok(false),
        });
        match copied {
            Ok(true) => moved.push(id.clone()),
            Ok(false) => {}
            Err(error) => {
                undo(into, &moved);
                return Err(error);
            }
        }
    }
    Ok(moved)
}

fn undo(into: &dyn SecretStore, moved: &[String]) {
    for id in moved {
        let _ = into.delete(id);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::file::tests::{scratch, this_machine};
    use super::*;

    /// The keychain, in memory; `fail` makes reading or saving that id fail.
    #[derive(Default)]
    struct FakeKeychain {
        passwords: Mutex<BTreeMap<String, String>>,
        fail: Option<String>,
    }

    impl SecretStore for std::sync::Arc<FakeKeychain> {
        fn get(&self, id: &str) -> Result<Option<String>> {
            if self.fail.as_deref() == Some(id) {
                return Err(AppError::Invalid("keychain refused".into()));
            }
            Ok(self.passwords.lock().unwrap().get(id).cloned())
        }
        fn set(&self, id: &str, password: &str) -> Result<()> {
            self.passwords.lock().unwrap().insert(id.into(), password.into());
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<()> {
            self.passwords.lock().unwrap().remove(id);
            Ok(())
        }
    }

    fn keychain(passwords: &[(&str, &str)], fail: Option<&str>) -> std::sync::Arc<FakeKeychain> {
        std::sync::Arc::new(FakeKeychain {
            passwords: Mutex::new(passwords.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
            fail: fail.map(String::from),
        })
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn starts_in_the_keychain_and_switches_with_every_password() {
        let scratch = scratch();
        let dir = scratch.path().to_path_buf();
        let fake = keychain(&[("a", "one"), ("b", "two")], None);
        let credentials = Credentials::with(dir.clone(), Box::new(fake.clone()), this_machine).unwrap();
        assert_eq!(credentials.info().storage, Storage::Keychain);
        assert_eq!(credentials.get("a").unwrap().as_deref(), Some("one"));

        credentials.switch(Storage::File, &ids(&["a", "b", "c"])).unwrap();
        assert_eq!(credentials.info().storage, Storage::File);
        assert_eq!(credentials.get("a").unwrap().as_deref(), Some("one"));
        assert_eq!(credentials.get("b").unwrap().as_deref(), Some("two"));
        assert!(fake.passwords.lock().unwrap().is_empty(), "passwords left in the keychain");

        // The choice survives a restart, and new passwords go to the file.
        let reopened = Credentials::with(dir.clone(), Box::new(fake.clone()), this_machine).unwrap();
        assert_eq!(reopened.info().storage, Storage::File);
        reopened.set("c", "three").unwrap();
        assert!(fake.passwords.lock().unwrap().is_empty());
        reopened.set("c", "").unwrap();
        assert_eq!(reopened.get("c").unwrap(), None);

        reopened.switch(Storage::Keychain, &ids(&["a", "b"])).unwrap();
        assert_eq!(fake.passwords.lock().unwrap().get("b").map(String::as_str), Some("two"));
        assert_eq!(reopened.file.get("a").unwrap(), None, "passwords left in the file");
    }

    #[test]
    fn a_password_that_cant_be_moved_switches_nothing() {
        let scratch = scratch();
        let dir = scratch.path().to_path_buf();
        let fake = keychain(&[("a", "one"), ("b", "two")], Some("b"));
        let credentials = Credentials::with(dir.clone(), Box::new(fake.clone()), this_machine).unwrap();
        let error = credentials.switch(Storage::File, &ids(&["a", "b"])).unwrap_err();
        assert_eq!(error.to_string(), "keychain refused");
        assert_eq!(credentials.info().storage, Storage::Keychain);
        assert!(!dir.join("settings.json").exists());
        assert_eq!(credentials.file.get("a").unwrap(), None, "a copy was left in the file");
        assert_eq!(fake.passwords.lock().unwrap().get("a").map(String::as_str), Some("one"));
    }
}
