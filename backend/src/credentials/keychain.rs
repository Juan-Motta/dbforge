//! Passwords in the OS keychain (Keychain on macOS, Credential Manager on
//! Windows, Secret Service on Linux), keyed by connection id.

use keyring::Entry;

use super::SecretStore;
use crate::error::Result;

const SERVICE: &str = "dev.dbforge.app";

pub struct Keychain;

impl SecretStore for Keychain {
    fn get(&self, connection_id: &str) -> Result<Option<String>> {
        match Entry::new(SERVICE, connection_id)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn set(&self, connection_id: &str, password: &str) -> Result<()> {
        Ok(Entry::new(SERVICE, connection_id)?.set_password(password)?)
    }

    fn delete(&self, connection_id: &str) -> Result<()> {
        match Entry::new(SERVICE, connection_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
