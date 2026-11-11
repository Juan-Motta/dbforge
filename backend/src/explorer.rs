use tauri::State;

use crate::connections::store::ConnectionStore;
use crate::drivers::{
    self, CompletionSchema, DbObject, RelationList, RowFilter, RowsPage, SchemaDiagram, TableRef, TableStructure,
};
use crate::error::Result;
use crate::statement_log::{Origin, StatementLog};

/// Lists the objects under `path` in a saved connection (see `drivers::list_objects`).
#[tauri::command]
pub async fn list_objects(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    path: Vec<String>,
) -> Result<Vec<DbObject>> {
    store
        .open(&connection_id)?
        .run(&log, Origin::Metadata, async |connection, password| {
            drivers::list_objects(connection, password, &path).await
        })
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
    store
        .open(&connection_id)?
        .run(&log, Origin::Metadata, async |connection, password| {
            drivers::list_relations(connection, password, database.as_deref()).await
        })
        .await
}

/// The tables, views and columns of a database, for completing names in the
/// query console. `database` defaults to the connection's own.
#[tauri::command]
pub async fn completion_schema(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    database: Option<String>,
) -> Result<CompletionSchema> {
    store
        .open(&connection_id)?
        .run(&log, Origin::Metadata, async |connection, password| {
            drivers::completion_schema(connection, password, database.as_deref()).await
        })
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
    let limit = limit.clamp(1, MAX_PAGE_SIZE);
    let filter = filter.unwrap_or_default();
    store
        .open(&connection_id)?
        .run(&log, Origin::Browse, async |connection, password| {
            drivers::fetch_rows(connection, password, &table, &filter, offset, limit).await
        })
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
    store
        .open(&connection_id)?
        .run(&log, Origin::Metadata, async |connection, password| {
            drivers::table_structure(connection, password, &table).await
        })
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
    store
        .open(&connection_id)?
        .run(&log, Origin::Metadata, async |connection, password| {
            drivers::schema_diagram(connection, password, &database, &schema).await
        })
        .await
}
