use tauri::State;

use crate::connections::secrets;
use crate::connections::store::ConnectionStore;
use crate::drivers::{self, DbObject, RelationList, RowsPage, TableRef, TableStructure};
use crate::error::Result;

/// Lists the objects under `path` in a saved connection (see `drivers::list_objects`).
#[tauri::command]
pub async fn list_objects(
    store: State<'_, ConnectionStore>,
    connection_id: String,
    path: Vec<String>,
) -> Result<Vec<DbObject>> {
    let connection = store.get(&connection_id)?;
    let password = secrets::get_password(&connection.id)?;
    drivers::list_objects(&connection, password.as_deref(), &path).await
}

/// Lists every table and view in a database, for the command palette's search.
/// `database` defaults to the connection's own.
#[tauri::command]
pub async fn list_relations(
    store: State<'_, ConnectionStore>,
    connection_id: String,
    database: Option<String>,
) -> Result<RelationList> {
    let connection = store.get(&connection_id)?;
    let password = secrets::get_password(&connection.id)?;
    drivers::list_relations(&connection, password.as_deref(), database.as_deref()).await
}

const MAX_PAGE_SIZE: u32 = 1000;

/// Reads one page of rows from a table or view in a saved connection.
#[tauri::command]
pub async fn fetch_rows(
    store: State<'_, ConnectionStore>,
    connection_id: String,
    table: TableRef,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    let connection = store.get(&connection_id)?;
    let password = secrets::get_password(&connection.id)?;
    let limit = limit.clamp(1, MAX_PAGE_SIZE);
    drivers::fetch_rows(&connection, password.as_deref(), &table, offset, limit).await
}

/// Reads the columns, indexes, keys, triggers and DDL of a table or view.
#[tauri::command]
pub async fn table_structure(
    store: State<'_, ConnectionStore>,
    connection_id: String,
    table: TableRef,
) -> Result<TableStructure> {
    let connection = store.get(&connection_id)?;
    let password = secrets::get_password(&connection.id)?;
    drivers::table_structure(&connection, password.as_deref(), &table).await
}
