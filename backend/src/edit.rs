//! Commands behind editing rows in the table grid.

use tauri::State;

use crate::connections::store::ConnectionStore;
use crate::drivers::{self, ApplySummary, RowChange, TableRef};
use crate::error::Result;
use crate::statement_log::{self, Origin, StatementLog};

/// Applies the grid's pending edits in one transaction; all of them or none.
#[tauri::command]
pub async fn apply_changes(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    table: TableRef,
    changes: Vec<RowChange>,
) -> Result<ApplySummary> {
    let connection = store.get(&connection_id)?;
    let password = store.password(&connection.id)?;
    statement_log::scoped(
        &log,
        &connection.id,
        Origin::Edit,
        drivers::apply_changes(&connection, password.as_deref(), &table, &changes),
    )
    .await
}

/// The SQL the pending edits would run, for the user to review first.
#[tauri::command]
pub async fn preview_changes(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    table: TableRef,
    changes: Vec<RowChange>,
) -> Result<Vec<String>> {
    let connection = store.get(&connection_id)?;
    let password = store.password(&connection.id)?;
    statement_log::scoped(
        &log,
        &connection.id,
        Origin::Metadata,
        drivers::preview_changes(&connection, password.as_deref(), &table, &changes),
    )
    .await
}
