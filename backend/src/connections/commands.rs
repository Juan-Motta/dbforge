use tauri::State;

use super::model::{Connection, ConnectionInput};
use super::secrets;
use super::store::ConnectionStore;
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
        secrets::set_password(&connection.id, &password)?;
    }
    store.upsert(connection.clone())?;
    Ok(connection)
}

#[tauri::command]
pub fn delete_connection(store: State<'_, ConnectionStore>, id: String) -> Result<()> {
    store.remove(&id)?;
    secrets::delete_password(&id)
}

/// Tests the values currently in the form, which may not be saved yet.
/// Returns the server version.
#[tauri::command]
pub async fn test_connection(input: ConnectionInput) -> Result<String> {
    let existing_id = input.id.clone();
    let (connection, password) = input.for_test().into_parts()?;
    let password = match (password, existing_id) {
        (Some(password), _) => Some(password),
        (None, Some(id)) => secrets::get_password(&id)?,
        (None, None) => None,
    };
    drivers::server_version(&connection, password.as_deref()).await
}
