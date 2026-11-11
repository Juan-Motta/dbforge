use tauri::{AppHandle, Emitter, Manager, State};

use super::model::{Connection, ConnectionInput, ConnectionParts};
use super::store::ConnectionStore;
use super::tags::{Tag, TagStore};
use crate::credentials::{Storage, StorageInfo, SwitchProgress};
use crate::drivers;
use crate::error::{AppError, Result};
use crate::statement_log::{self, Origin, StatementLog};

#[tauri::command]
pub fn list_connections(store: State<'_, ConnectionStore>) -> Vec<Connection> {
    store.list()
}

// Commands that reach the keychain or the disk run off the main thread: the
// keychain may stop to ask the user, and the window must keep drawing meanwhile.
#[tauri::command]
pub async fn save_connection(store: State<'_, ConnectionStore>, input: ConnectionInput) -> Result<Connection> {
    let connection = store.save(input.into_parts()?)?;
    // A tunnel opened with the old settings or secret is not reused.
    crate::tunnel::close(&connection.id).await;
    Ok(connection)
}

/// Saves a copy of a connection, its secrets included, under `name`.
#[tauri::command(async)]
pub fn duplicate_connection(store: State<'_, ConnectionStore>, id: String, name: String) -> Result<Connection> {
    store.duplicate(&id, &name)
}

#[tauri::command]
pub async fn delete_connection(store: State<'_, ConnectionStore>, id: String) -> Result<()> {
    store.delete(&id)?;
    crate::tunnel::close(&id).await;
    Ok(())
}

/// Tests the values currently in the form, which may not be saved yet.
/// Returns the server version.
#[tauri::command]
pub async fn test_connection(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    input: ConnectionInput,
) -> Result<String> {
    let existing_id = input.id.clone();
    let ConnectionParts { connection, password, ssh_secret } = input.for_test().into_parts()?;
    let saved = |read: fn(&ConnectionStore, &str) -> Result<Option<String>>| match &existing_id {
        Some(id) => read(&store, id),
        None => Ok(None),
    };
    let password = match password {
        Some(password) => Some(password),
        None => saved(ConnectionStore::password)?,
    };
    let ssh_secret = match ssh_secret {
        Some(secret) => Some(secret),
        None => saved(ConnectionStore::ssh_secret)?,
    };
    // Tested on a tunnel of its own, with the secret typed: the saved
    // connection's tunnel keeps its settings meanwhile.
    let log_id = connection.id.clone();
    let connection = Connection { id: format!("test-{}", uuid::Uuid::new_v4()), ..connection };
    let _lent = crate::tunnel::lend_secret(&connection.id, ssh_secret)?;
    let version = statement_log::scoped(
        &log,
        &log_id,
        Origin::Connect,
        drivers::server_version(&connection, password.as_deref()),
    )
    .await;
    crate::tunnel::close(&connection.id).await;
    version
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
        let mut ids = store.credential_ids();
        // The MCP server's token is a secret too, kept wherever the passwords are.
        ids.push(crate::mcp::token::CREDENTIAL_ID.into());
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
#[tauri::command(async)]
pub fn save_tags(store: State<'_, ConnectionStore>, tags: State<'_, TagStore>, list: Vec<Tag>) -> Result<Vec<Tag>> {
    tags.replace(list)?;
    let kept: Vec<String> = tags.list().into_iter().map(|tag| tag.id).collect();
    store.keep_only_tags(&kept)?;
    Ok(tags.list())
}
