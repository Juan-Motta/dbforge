use serde::Deserialize;
use tauri::State;

use crate::connections::store::ConnectionStore;
use crate::drivers::{self, NewObject};
use crate::error::Result;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    connection_id: String,
    object: NewObject,
}

/// Creates a database, role or user on a saved connection's server (see `drivers::create_object`).
#[tauri::command]
pub async fn create_object(store: State<'_, ConnectionStore>, request: CreateRequest) -> Result<()> {
    let connection = store.get(&request.connection_id)?;
    let password = store.password(&connection.id)?;
    drivers::create_object(&connection, password.as_deref(), &request.object).await
}
