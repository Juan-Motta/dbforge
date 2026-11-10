//! Commands of the Redis key browser (see `drivers::redis`).

use tauri::State;

use crate::connections::model::Engine;
use crate::connections::store::{ConnectionStore, Opened};
use crate::drivers::redis::{self, KeyEdit, KeyValue, NewKey, Overview, ScanPage, Text};
use crate::error::{AppError, Result};
use crate::statement_log::{Origin, StatementLog};

/// Keys per page of the browser, and items per page of a value.
const MAX_PAGE: usize = 1000;

/// Connection `id` with its password, when it is a Redis connection.
fn redis_connection(store: &ConnectionStore, id: &str) -> Result<Opened> {
    let opened = store.open(id)?;
    if opened.connection.engine != Engine::Redis {
        return Err(AppError::Invalid(format!("{} is not a Redis connection.", opened.connection.name)));
    }
    Ok(opened)
}

/// Server metrics and the number of keys in database `db`.
#[tauri::command]
pub async fn redis_overview(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    db: i64,
) -> Result<Overview> {
    redis_connection(&store, &connection_id)?
        .run(&log, Origin::Browse, async |connection, password| redis::overview(connection, password, db).await)
        .await
}

/// One page of keys matching `pattern` (and `kind`), from `cursor` (`"0"` first).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn redis_scan(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    db: i64,
    pattern: String,
    kind: Option<String>,
    cursor: String,
    limit: usize,
) -> Result<ScanPage> {
    let limit = limit.clamp(1, MAX_PAGE);
    redis_connection(&store, &connection_id)?
        .run(&log, Origin::Browse, async |connection, password| {
            redis::scan(connection, password, db, &pattern, kind.as_deref(), &cursor, limit).await
        })
        .await
}

/// One page of a key's value, from `cursor` (`None` first).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn redis_read_key(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    db: i64,
    key: Text,
    cursor: Option<String>,
    filter: String,
    limit: usize,
) -> Result<KeyValue> {
    let limit = limit.clamp(1, MAX_PAGE);
    redis_connection(&store, &connection_id)?
        .run(&log, Origin::Browse, async |connection, password| {
            redis::read_key(connection, password, db, &key, cursor.as_deref(), &filter, limit).await
        })
        .await
}

/// Changes `key`, which the browser showed as a `kind` (see `redis::KeyEdit`).
#[tauri::command]
pub async fn redis_edit_key(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    db: i64,
    key: Text,
    kind: String,
    edit: KeyEdit,
) -> Result<()> {
    redis_connection(&store, &connection_id)?
        .run(&log, Origin::Edit, async |connection, password| {
            redis::edit_key(connection, password, db, &key, &kind, &edit).await
        })
        .await
}

/// Creates a key with its first element; refused when the name is taken.
#[tauri::command]
pub async fn redis_create_key(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    db: i64,
    key: NewKey,
) -> Result<()> {
    redis_connection(&store, &connection_id)?
        .run(&log, Origin::Edit, async |connection, password| redis::create_key(connection, password, db, &key).await)
        .await
}
