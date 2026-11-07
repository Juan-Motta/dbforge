//! Commands of the Redis key browser (see `drivers::redis`).

use tauri::State;

use crate::connections::model::Engine;
use crate::connections::secrets;
use crate::connections::store::ConnectionStore;
use crate::drivers::redis::{self, KeyEdit, KeyValue, NewKey, Overview, ScanPage};
use crate::error::{AppError, Result};

/// Keys per page of the browser, and items per page of a value.
const MAX_PAGE: usize = 1000;

fn redis_connection(
    store: &ConnectionStore,
    id: &str,
) -> Result<(crate::connections::model::Connection, Option<String>)> {
    let connection = store.get(id)?;
    if connection.engine != Engine::Redis {
        return Err(AppError::Invalid(format!("{} is not a Redis connection.", connection.name)));
    }
    let password = secrets::get_password(&connection.id)?;
    Ok((connection, password))
}

/// Server metrics and the number of keys in database `db`.
#[tauri::command]
pub async fn redis_overview(store: State<'_, ConnectionStore>, connection_id: String, db: i64) -> Result<Overview> {
    let (connection, password) = redis_connection(&store, &connection_id)?;
    redis::overview(&connection, password.as_deref(), db).await
}

/// One page of keys matching `pattern` (and `kind`), from `cursor` (`"0"` first).
#[tauri::command]
pub async fn redis_scan(
    store: State<'_, ConnectionStore>,
    connection_id: String,
    db: i64,
    pattern: String,
    kind: Option<String>,
    cursor: String,
    limit: usize,
) -> Result<ScanPage> {
    let (connection, password) = redis_connection(&store, &connection_id)?;
    let limit = limit.clamp(1, MAX_PAGE);
    redis::scan(&connection, password.as_deref(), db, &pattern, kind.as_deref(), &cursor, limit).await
}

/// One page of a key's value, from `cursor` (`None` first).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn redis_read_key(
    store: State<'_, ConnectionStore>,
    connection_id: String,
    db: i64,
    key: String,
    cursor: Option<String>,
    filter: String,
    limit: usize,
) -> Result<KeyValue> {
    let (connection, password) = redis_connection(&store, &connection_id)?;
    let limit = limit.clamp(1, MAX_PAGE);
    redis::read_key(&connection, password.as_deref(), db, &key, cursor.as_deref(), &filter, limit).await
}

/// Changes `key`, which the browser showed as a `kind` (see `redis::KeyEdit`).
#[tauri::command]
pub async fn redis_edit_key(
    store: State<'_, ConnectionStore>,
    connection_id: String,
    db: i64,
    key: String,
    kind: String,
    edit: KeyEdit,
) -> Result<()> {
    let (connection, password) = redis_connection(&store, &connection_id)?;
    redis::edit_key(&connection, password.as_deref(), db, &key, &kind, &edit).await
}

/// Creates a key with its first element; refused when the name is taken.
#[tauri::command]
pub async fn redis_create_key(
    store: State<'_, ConnectionStore>,
    connection_id: String,
    db: i64,
    key: NewKey,
) -> Result<()> {
    let (connection, password) = redis_connection(&store, &connection_id)?;
    redis::create_key(&connection, password.as_deref(), db, &key).await
}
