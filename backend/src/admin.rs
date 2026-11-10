use serde::Deserialize;
use tauri::State;

use crate::connections::store::ConnectionStore;
use crate::drivers::{self, NewObject};
use crate::error::Result;
use crate::statement_log::{Origin, StatementLog};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    connection_id: String,
    object: NewObject,
}

/// Creates a database, role or user on a saved connection's server (see `drivers::create_object`).
#[tauri::command]
pub async fn create_object(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    request: CreateRequest,
) -> Result<()> {
    store
        .open(&request.connection_id)?
        .run(&log, Origin::Admin, async |connection, password| {
            drivers::create_object(connection, password, &request.object).await
        })
        .await
}
