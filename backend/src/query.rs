use tauri::State;

use crate::connections::secrets;
use crate::connections::store::ConnectionStore;
use crate::drivers::{self, QueryOutcome};
use crate::error::Result;

/// Rows kept per result; the rest are read and dropped.
const ROW_LIMIT: usize = 500;

/// Runs a SQL script against `database` (or the connection's default database).
#[tauri::command]
pub async fn run_query(
    store: State<'_, ConnectionStore>,
    connection_id: String,
    database: Option<String>,
    sql: String,
) -> Result<QueryOutcome> {
    let connection = store.get(&connection_id)?;
    let password = secrets::get_password(&connection.id)?;
    drivers::run_query(&connection, password.as_deref(), database.as_deref(), &sql, ROW_LIMIT).await
}
