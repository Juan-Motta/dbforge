use tauri::State;

use super::model::{Connection, ConnectionInput};
use super::store::ConnectionStore;
use crate::credentials::{Storage, StorageInfo};
use crate::drivers;
use crate::error::Result;

#[tauri::command]
pub fn list_connections(store: State<'_, ConnectionStore>) -> Vec<Connection> {
    store.list()
}

#[tauri::command]
pub fn save_connection(store: State<'_, ConnectionStore>, input: ConnectionInput) -> Result<Connection> {
    let (connection, password) = input.into_parts()?;
    if let Some(password) = password {
        store.credentials().set(&connection.id, &password)?;
    }
    store.upsert(connection.clone())?;
    Ok(connection)
}

#[tauri::command]
pub fn delete_connection(store: State<'_, ConnectionStore>, id: String) -> Result<()> {
    store.remove(&id)?;
    store.credentials().delete(&id)
}

/// Tests the values currently in the form, which may not be saved yet.
/// Returns the server version.
#[tauri::command]
pub async fn test_connection(store: State<'_, ConnectionStore>, input: ConnectionInput) -> Result<String> {
    let existing_id = input.id.clone();
    let (connection, password) = input.for_test().into_parts()?;
    let password = match (password, existing_id) {
        (Some(password), _) => Some(password),
        (None, Some(id)) => store.password(&id)?,
        (None, None) => None,
    };
    drivers::server_version(&connection, password.as_deref()).await
}

/// Where passwords are saved, as the settings show it.
#[tauri::command]
pub fn credential_storage(store: State<'_, ConnectionStore>) -> StorageInfo {
    store.credentials().info()
}

/// Moves every saved password to `storage` and keeps it as the choice.
#[tauri::command]
pub fn set_credential_storage(store: State<'_, ConnectionStore>, storage: Storage) -> Result<StorageInfo> {
    let ids: Vec<String> = store.list().into_iter().map(|connection| connection.id).collect();
    store.credentials().switch(storage, &ids)?;
    Ok(store.credentials().info())
}
