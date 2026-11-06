//! Passwords live in the OS keychain (Keychain on macOS, Credential Manager on
//! Windows, Secret Service on Linux), keyed by connection id.

use keyring::Entry;

use crate::error::Result;

const SERVICE: &str = "dev.dbforge.app";

pub fn get_password(connection_id: &str) -> Result<Option<String>> {
    match Entry::new(SERVICE, connection_id)?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Stores the password, or removes it when empty.
pub fn set_password(connection_id: &str, password: &str) -> Result<()> {
    if password.is_empty() {
        return delete_password(connection_id);
    }
    Ok(Entry::new(SERVICE, connection_id)?.set_password(password)?)
}

pub fn delete_password(connection_id: &str) -> Result<()> {
    match Entry::new(SERVICE, connection_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.into()),
    }
}
