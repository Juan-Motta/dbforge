use tauri::State;

use crate::connections::store::ConnectionStore;
use crate::drivers::{self, DbObject, RelationList, RowFilter, RowsPage, SchemaDiagram, TableRef, TableStructure};
use crate::error::Result;
use crate::statement_log::{self, Origin, StatementLog};

/// Lists the objects under `path` in a saved connection (see `drivers::list_objects`).
#[tauri::command]
pub async fn list_objects(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    path: Vec<String>,
) -> Result<Vec<DbObject>> {
    let connection = store.get(&connection_id)?;
    let password = store.password(&connection.id)?;
    statement_log::scoped(
        &log,
        &connection.id,
        Origin::Metadata,
        drivers::list_objects(&connection, password.as_deref(), &path),
    )
    .await
}

/// Lists every table and view in a database, for the command palette's search.
/// `database` defaults to the connection's own.
#[tauri::command]
pub async fn list_relations(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    database: Option<String>,
) -> Result<RelationList> {
    let connection = store.get(&connection_id)?;
    let password = store.password(&connection.id)?;
    statement_log::scoped(
        &log,
        &connection.id,
        Origin::Metadata,
        drivers::list_relations(&connection, password.as_deref(), database.as_deref()),
    )
    .await
}

const MAX_PAGE_SIZE: u32 = 1000;

/// Reads one page of rows from a table or view in a saved connection,
/// narrowed and ordered by `filter` when the table view has one.
#[tauri::command]
pub async fn fetch_rows(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    table: TableRef,
    filter: Option<RowFilter>,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    let connection = store.get(&connection_id)?;
    let password = store.password(&connection.id)?;
    let limit = limit.clamp(1, MAX_PAGE_SIZE);
    statement_log::scoped(
        &log,
        &connection.id,
        Origin::Browse,
        drivers::fetch_rows(&connection, password.as_deref(), &table, &filter.unwrap_or_default(), offset, limit),
    )
    .await
}

/// Reads the columns, indexes, keys, triggers and DDL of a table or view.
#[tauri::command]
pub async fn table_structure(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    table: TableRef,
) -> Result<TableStructure> {
    let connection = store.get(&connection_id)?;
    let password = store.password(&connection.id)?;
    statement_log::scoped(
        &log,
        &connection.id,
        Origin::Metadata,
        drivers::table_structure(&connection, password.as_deref(), &table),
    )
    .await
}

/// Every table of a schema with its columns, and the foreign keys between them.
#[tauri::command]
pub async fn schema_diagram(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    database: String,
    schema: String,
) -> Result<SchemaDiagram> {
    let connection = store.get(&connection_id)?;
    let password = store.password(&connection.id)?;
    statement_log::scoped(
        &log,
        &connection.id,
        Origin::Metadata,
        drivers::schema_diagram(&connection, password.as_deref(), &database, &schema),
    )
    .await
}
