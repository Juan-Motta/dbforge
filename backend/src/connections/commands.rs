use tauri::{AppHandle, Emitter, Manager, State};

use super::model::{Connection, ConnectionInput};
use super::store::ConnectionStore;
use super::tags::{Tag, TagStore};
use crate::credentials::{Storage, StorageInfo, SwitchProgress};
use crate::drivers;
use crate::error::{AppError, Result};

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

/// Saves a copy of a connection, password included, under `name`.
#[tauri::command]
pub fn duplicate_connection(store: State<'_, ConnectionStore>, id: String, name: String) -> Result<Connection> {
    store.duplicate(&id, &name)
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

/// Moves every saved password to `storage` and keeps it as the choice,
/// emitting `credential-progress` before each one. It runs off the main
/// thread: the keychain may stop to ask the user, and the window must keep
/// drawing meanwhile.
#[tauri::command]
pub async fn set_credential_storage(app: AppHandle, storage: Storage) -> Result<StorageInfo> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<ConnectionStore>();
        let ids: Vec<String> = store.list().into_iter().map(|connection| connection.id).collect();
        let report = |progress: SwitchProgress| {
            let _ = app.emit("credential-progress", progress);
        };
        store.credentials().switch(storage, &ids, &report)?;
        Ok(store.credentials().info())
    })
    .await
    .map_err(|error| AppError::Invalid(format!("Moving the passwords stopped: {error}")))?
}

#[tauri::command]
pub fn list_tags(tags: State<'_, TagStore>) -> Vec<Tag> {
    tags.list()
}

/// Saves the whole list of tags; connections lose the tags it no longer has.
#[tauri::command]
pub fn save_tags(store: State<'_, ConnectionStore>, tags: State<'_, TagStore>, list: Vec<Tag>) -> Result<Vec<Tag>> {
    tags.replace(list)?;
    let kept: Vec<String> = tags.list().into_iter().map(|tag| tag.id).collect();
    store.keep_only_tags(&kept)?;
    Ok(tags.list())
}
