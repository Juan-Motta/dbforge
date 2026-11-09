//! Entry points that dispatch to the right database driver by engine.

mod cosmos;
mod edit_sql;
mod mongodb;
mod row_filter;

pub use row_filter::RowFilter;
mod mysql;
mod postgres;
pub mod redis;
mod sqlite;
mod sqlserver;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::connections::model::{Connection, Engine};
use crate::error::{AppError, Result};

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectKind {
    Database,
    Schema,
    Table,
    View,
}

/// One object in the explorer tree.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbObject {
    pub name: String,
    pub kind: ObjectKind,
    /// Approximate row count for tables, when the server knows it.
    pub row_estimate: Option<i64>,
}

impl DbObject {
    fn new(name: String, kind: ObjectKind) -> Self {
        Self { name, kind, row_estimate: None }
    }
}

/// A table or view found by searching a whole database.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Relation {
    pub schema: String,
    pub name: String,
    pub kind: ObjectKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelationList {
    /// The database searched, resolved even when the connection leaves it unset.
    pub database: String,
    pub relations: Vec<Relation>,
}

/// A value written to a cell: the server's text form, NULL, or the column's DEFAULT.
/// On the wire it is a string, `null`, or `{"kind": "default"}`, so a cell holding
/// the text `DEFAULT` stays text.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "WireValue")]
pub enum CellValue {
    Text(String),
    Null,
    Default,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WireValue {
    Value(Option<String>),
    Keyword { kind: Keyword },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Keyword {
    Default,
}

impl From<WireValue> for CellValue {
    fn from(wire: WireValue) -> Self {
        match wire {
            WireValue::Value(value) => value.into(),
            WireValue::Keyword { kind: Keyword::Default } => CellValue::Default,
        }
    }
}

impl From<Option<String>> for CellValue {
    fn from(value: Option<String>) -> Self {
        value.map_or(CellValue::Null, CellValue::Text)
    }
}

/// One edit to a table row. Keys are the server's text form; `None` is NULL.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RowChange {
    /// `key` holds every primary key column with the value the row was loaded with.
    Update {
        key: BTreeMap<String, Option<String>>,
        values: BTreeMap<String, CellValue>,
    },
    /// Columns left out take their DEFAULT.
    Insert {
        values: BTreeMap<String, CellValue>,
    },
    Delete {
        key: BTreeMap<String, Option<String>>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApplySummary {
    pub updated: u64,
    pub inserted: u64,
    pub deleted: u64,
}

/// Identifies a table or view to read rows from.
#[derive(Debug, Clone, Deserialize)]
pub struct TableRef {
    pub database: String,
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    /// The type as the server writes it, e.g. `character varying(80)`.
    pub data_type: String,
    #[serde(default)]
    pub primary_key: bool,
    /// Computed by the database (a generated column, or an identity that is
    /// GENERATED ALWAYS), so it can't be given a value.
    #[serde(default)]
    pub generated: bool,
    /// The column's DEFAULT expression, where an engine needs it written out to
    /// put a value back to its default. Not sent to the UI.
    #[serde(skip)]
    pub default: Option<String>,
}

/// Receives a result one row at a time, so it never has to fit in memory.
pub trait RowSink {
    fn columns(&mut self, columns: &[Column]) -> Result<()>;
    fn row(&mut self, values: &[Option<String>]) -> Result<()>;
}

/// One page of rows. Values are the server's text representation; `None` is NULL.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowsPage {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Option<String>>>,
    pub has_more: bool,
}

/// The outcome of one statement in a script.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementResult {
    /// Empty for statements that return no rows (INSERT, CREATE, …).
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Option<String>>>,
    /// True when more rows came back than the row limit; the rest were dropped.
    pub truncated: bool,
    /// Rows changed, or rows returned for a SELECT.
    pub rows_affected: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryOutcome {
    pub results: Vec<StatementResult>,
    pub elapsed_ms: u64,
}

/// Everything the structure view shows about a table or view.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableStructure {
    /// `Table` or `View`.
    pub kind: ObjectKind,
    pub row_estimate: Option<i64>,
    pub data_bytes: i64,
    pub index_bytes: i64,
    pub comment: Option<String>,
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKey>,
    pub triggers: Vec<TriggerInfo>,
    /// A `CREATE` script that rebuilds the object, comments and triggers included.
    pub ddl: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    /// The default expression, or how the value is generated (identity, computed).
    pub default: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    /// Access method, e.g. `btree` or `gin`.
    pub method: String,
    pub unique: bool,
    pub primary: bool,
    /// Key columns or expressions, in order.
    pub columns: Vec<String>,
    /// The `WHERE` of a partial index, which covers only the rows that match it.
    pub condition: Option<String>,
    pub bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    /// Schema-qualified when outside the search path, e.g. `billing.invoices`.
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    /// SQL action names, e.g. `NO ACTION` or `CASCADE`.
    pub on_update: String,
    pub on_delete: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerInfo {
    pub name: String,
    pub enabled: bool,
    /// The full `CREATE TRIGGER` statement.
    pub definition: String,
}

/// The tables of one schema and the foreign keys between them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiagram {
    pub tables: Vec<DiagramTable>,
    /// Foreign keys declared in the schema; some may point to tables elsewhere.
    pub relationships: Vec<Relationship>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramTable {
    pub name: String,
    pub row_estimate: Option<i64>,
    pub columns: Vec<DiagramColumn>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relationship {
    pub name: String,
    pub table: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
}

pub async fn server_version(connection: &Connection, password: Option<&str>) -> Result<String> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => postgres::server_version(connection, password).await,
            Engine::Sqlite => sqlite::server_version(connection).await,
            Engine::Mysql | Engine::Mariadb => mysql::server_version(connection, password).await,
            Engine::Sqlserver => sqlserver::server_version(connection, password).await,
            Engine::Mongodb => mongodb::server_version(connection, password).await,
            Engine::Redis => redis::server_version(connection, password).await,
            Engine::Cosmosdb => cosmos::server_version(connection, password).await,
        }
    })
    .await
}

pub async fn list_objects(connection: &Connection, password: Option<&str>, path: &[String]) -> Result<Vec<DbObject>> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => postgres::list_objects(connection, password, path).await,
            Engine::Sqlite => sqlite::list_objects(connection, path).await,
            Engine::Mysql | Engine::Mariadb => mysql::list_objects(connection, password, path).await,
            Engine::Sqlserver => sqlserver::list_objects(connection, password, path).await,
            Engine::Mongodb => mongodb::list_objects(connection, password, path).await,
            Engine::Redis => redis::list_objects(connection, password, path).await,
            Engine::Cosmosdb => cosmos::list_objects(connection, password, path).await,
        }
    })
    .await
}

pub async fn list_relations(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
) -> Result<RelationList> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => postgres::list_relations(connection, password, database).await,
            Engine::Sqlite => sqlite::list_relations(connection).await,
            Engine::Mysql | Engine::Mariadb => mysql::list_relations(connection, password, database).await,
            Engine::Sqlserver => sqlserver::list_relations(connection, password, database).await,
            Engine::Mongodb => mongodb::list_relations(connection, password, database).await,
            Engine::Cosmosdb => cosmos::list_relations(connection, password, database).await,
            other => Err(AppError::Unsupported(other.label())),
        }
    })
    .await
}

/// Sends every row of a table or view to `sink`, ordered by primary key when there is one.
/// Only connecting has a time limit: a big table can take as long as it needs.
pub async fn stream_table(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    filter: &RowFilter,
    sink: &mut impl RowSink,
) -> Result<()> {
    match connection.engine {
        Engine::Postgres => postgres::stream_table(connection, password, table, filter, sink).await,
        Engine::Sqlite => sqlite::stream_table(connection, table, filter, sink).await,
        Engine::Mysql | Engine::Mariadb => mysql::stream_table(connection, password, table, filter, sink).await,
        Engine::Sqlserver => sqlserver::stream_table(connection, password, table, filter, sink).await,
        Engine::Mongodb => mongodb::stream_table(connection, password, table, filter, sink).await,
        Engine::Cosmosdb => cosmos::stream_table(connection, password, table, filter, sink).await,
        other => Err(AppError::Unsupported(other.label())),
    }
}

pub async fn fetch_rows(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    filter: &RowFilter,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => postgres::fetch_rows(connection, password, table, filter, offset, limit).await,
            Engine::Sqlite => sqlite::fetch_rows(connection, table, filter, offset, limit).await,
            Engine::Mysql | Engine::Mariadb => {
                mysql::fetch_rows(connection, password, table, filter, offset, limit).await
            }
            Engine::Sqlserver => sqlserver::fetch_rows(connection, password, table, filter, offset, limit).await,
            Engine::Mongodb => mongodb::fetch_rows(connection, password, table, filter, offset, limit).await,
            Engine::Cosmosdb => cosmos::fetch_rows(connection, password, table, filter, offset, limit).await,
            other => Err(AppError::Unsupported(other.label())),
        }
    })
    .await
}

pub async fn schema_diagram(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    schema: &str,
) -> Result<SchemaDiagram> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => postgres::schema_diagram(connection, password, database, schema).await,
            Engine::Sqlite => sqlite::schema_diagram(connection).await,
            Engine::Mysql | Engine::Mariadb => mysql::schema_diagram(connection, password, database, schema).await,
            Engine::Sqlserver => sqlserver::schema_diagram(connection, password, database, schema).await,
            other => Err(AppError::Unsupported(other.label())),
        }
    })
    .await
}

/// Applies row edits in one transaction; see `postgres::edit` for the guarantees.
/// Only connecting has a time limit: a large batch can take as long as it needs.
pub async fn apply_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<ApplySummary> {
    match connection.engine {
        Engine::Postgres => postgres::apply_changes(connection, password, table, changes).await,
        Engine::Sqlite => sqlite::apply_changes(connection, table, changes).await,
        Engine::Mysql | Engine::Mariadb => mysql::apply_changes(connection, password, table, changes).await,
        Engine::Sqlserver => sqlserver::apply_changes(connection, password, table, changes).await,
        Engine::Mongodb => mongodb::apply_changes(connection, password, table, changes).await,
        Engine::Cosmosdb => cosmos::apply_changes(connection, password, table, changes).await,
        other => Err(AppError::Unsupported(other.label())),
    }
}

/// The statements `apply_changes` would run, with values written in.
pub async fn preview_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<Vec<String>> {
    match connection.engine {
        Engine::Postgres => postgres::preview_changes(connection, password, table, changes).await,
        Engine::Sqlite => sqlite::preview_changes(connection, table, changes).await,
        Engine::Mysql | Engine::Mariadb => mysql::preview_changes(connection, password, table, changes).await,
        Engine::Sqlserver => sqlserver::preview_changes(connection, password, table, changes).await,
        Engine::Mongodb => mongodb::preview_changes(connection, password, table, changes).await,
        Engine::Cosmosdb => cosmos::preview_changes(connection, password, table, changes).await,
        other => Err(AppError::Unsupported(other.label())),
    }
}

pub async fn table_structure(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
) -> Result<TableStructure> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => postgres::table_structure(connection, password, table).await,
            Engine::Sqlite => sqlite::table_structure(connection, table).await,
            Engine::Mysql | Engine::Mariadb => mysql::table_structure(connection, password, table).await,
            Engine::Sqlserver => sqlserver::table_structure(connection, password, table).await,
            Engine::Mongodb => mongodb::table_structure(connection, password, table).await,
            Engine::Cosmosdb => cosmos::table_structure(connection, password, table).await,
            other => Err(AppError::Unsupported(other.label())),
        }
    })
    .await
}

/// A connection that stays open across query runs. Runs have no time limit;
/// long ones are stopped with [`cancel`].
pub enum Session {
    Postgres(postgres::Session),
    Mysql(mysql::Session),
    Sqlite(sqlite::Session),
    /// Boxed: a tiberius client is much larger than the other sessions.
    Sqlserver(Box<sqlserver::Session>),
    Mongodb(mongodb::Session),
    Redis(redis::Session),
    Cosmosdb(cosmos::Session),
}

/// What a cancel request needs to find a session's running query on the server.
#[derive(Debug, Clone)]
pub enum CancelTarget {
    Postgres {
        pid: i32,
    },
    Mysql {
        id: u64,
    },
    /// SQL Server cancels on the session's own connection, and Cosmos DB stops
    /// reading the query's pages; this wakes the running query to do it.
    Signal(Arc<tokio::sync::Notify>),
    /// MongoDB finds the operations tagged with the session's comment.
    Mongodb {
        tag: String,
    },
    /// Redis can end a blocking command of another connection.
    Redis {
        client_id: i64,
    },
    /// SQLite can't stop a query from another connection.
    Unavailable,
}

pub async fn open_session(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<Session> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => Ok(Session::Postgres(postgres::open_session(connection, password, database).await?)),
            Engine::Sqlite => Ok(Session::Sqlite(sqlite::open_session(connection).await?)),
            Engine::Mysql | Engine::Mariadb => {
                Ok(Session::Mysql(mysql::open_session(connection, password, database).await?))
            }
            Engine::Sqlserver => {
                Ok(Session::Sqlserver(Box::new(sqlserver::open_session(connection, password, database).await?)))
            }
            Engine::Mongodb => Ok(Session::Mongodb(mongodb::open_session(connection, password, database).await?)),
            Engine::Redis => Ok(Session::Redis(redis::open_session(connection, password, database).await?)),
            Engine::Cosmosdb => Ok(Session::Cosmosdb(cosmos::open_session(connection, password, database).await?)),
        }
    })
    .await
}

impl Session {
    pub fn cancel_target(&self) -> CancelTarget {
        match self {
            Session::Postgres(session) => CancelTarget::Postgres { pid: session.pid() },
            Session::Mysql(session) => CancelTarget::Mysql { id: session.id() },
            Session::Sqlite(_) => CancelTarget::Unavailable,
            Session::Sqlserver(session) => CancelTarget::Signal(session.canceller()),
            Session::Mongodb(session) => CancelTarget::Mongodb { tag: session.tag() },
            Session::Redis(session) => CancelTarget::Redis { client_id: session.client_id() },
            Session::Cosmosdb(session) => CancelTarget::Signal(session.canceller()),
        }
    }

    /// Runs a script of one or more statements and returns every statement's result.
    pub async fn run(&mut self, sql: &str, row_limit: usize) -> Result<QueryOutcome> {
        match self {
            Session::Postgres(session) => session.run(sql, row_limit).await,
            Session::Mysql(session) => session.run(sql, row_limit).await,
            Session::Sqlite(session) => session.run(sql, row_limit).await,
            Session::Sqlserver(session) => session.run(sql, row_limit).await,
            Session::Mongodb(session) => session.run(sql, row_limit).await,
            Session::Redis(session) => session.run(sql, row_limit).await,
            Session::Cosmosdb(session) => session.run(sql, row_limit).await,
        }
    }

    /// False when the connection dropped and the session can't be used again.
    pub async fn is_alive(&mut self) -> bool {
        let check = async {
            Ok(match self {
                Session::Postgres(session) => session.is_alive().await,
                Session::Mysql(session) => session.is_alive().await,
                Session::Sqlite(session) => session.is_alive().await,
                Session::Sqlserver(session) => session.is_alive().await,
                Session::Mongodb(session) => session.is_alive().await,
                Session::Redis(session) => session.is_alive().await,
                Session::Cosmosdb(session) => session.is_alive().await,
            })
        };
        with_timeout(check).await.unwrap_or(false)
    }
}

/// The plan of one statement. With `analyze` it runs, but its changes are rolled back.
pub async fn explain(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    sql: &str,
    analyze: bool,
) -> Result<serde_json::Value> {
    match connection.engine {
        Engine::Postgres => postgres::explain(connection, password, database, sql, analyze).await,
        Engine::Sqlite => sqlite::explain(connection, sql, analyze).await,
        Engine::Mysql | Engine::Mariadb => mysql::explain(connection, password, database, sql, analyze).await,
        Engine::Sqlserver => sqlserver::explain(connection, password, database, sql, analyze).await,
        Engine::Mongodb => mongodb::explain(connection, password, database, sql, analyze).await,
        Engine::Cosmosdb => cosmos::explain(connection, password, database, sql, analyze).await,
        other => Err(AppError::Unsupported(other.label())),
    }
}

/// Stops the query a session is running. Harmless when it is idle.
pub async fn cancel(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    target: CancelTarget,
) -> Result<()> {
    with_timeout(async {
        match target {
            CancelTarget::Postgres { pid } => postgres::cancel(connection, password, database, pid).await,
            CancelTarget::Mysql { id } => mysql::cancel(connection, password, id).await,
            CancelTarget::Signal(canceller) => {
                sqlserver::cancel(&canceller);
                Ok(())
            }
            CancelTarget::Mongodb { tag } => mongodb::cancel(connection, password, &tag).await,
            CancelTarget::Redis { client_id } => redis::cancel(connection, password, client_id).await,
            CancelTarget::Unavailable => Err(AppError::Invalid("This database can't cancel a running query.".into())),
        }
    })
    .await
}

/// rustls needs one process-wide crypto provider when several are compiled in,
/// as here (the drivers bring their own). Installing it twice is harmless.
pub(crate) fn install_crypto() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

async fn with_timeout<T>(request: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(TIMEOUT, request).await.map_err(|_| AppError::Timeout(TIMEOUT.as_secs()))?
}
