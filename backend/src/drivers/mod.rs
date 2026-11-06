//! Entry points that dispatch to the right database driver by engine.

mod postgres;

use std::future::Future;
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

pub async fn server_version(connection: &Connection, password: Option<&str>) -> Result<String> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => postgres::server_version(connection, password).await,
            other => Err(AppError::Unsupported(other.label())),
        }
    })
    .await
}

pub async fn list_objects(connection: &Connection, password: Option<&str>, path: &[String]) -> Result<Vec<DbObject>> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => postgres::list_objects(connection, password, path).await,
            other => Err(AppError::Unsupported(other.label())),
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
    sink: &mut impl RowSink,
) -> Result<()> {
    match connection.engine {
        Engine::Postgres => postgres::stream_table(connection, password, table, sink).await,
        other => Err(AppError::Unsupported(other.label())),
    }
}

pub async fn fetch_rows(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => postgres::fetch_rows(connection, password, table, offset, limit).await,
            other => Err(AppError::Unsupported(other.label())),
        }
    })
    .await
}

pub async fn table_structure(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
) -> Result<TableStructure> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => postgres::table_structure(connection, password, table).await,
            other => Err(AppError::Unsupported(other.label())),
        }
    })
    .await
}

/// A connection that stays open across query runs. Runs have no time limit;
/// long ones are stopped with [`cancel`].
pub enum Session {
    Postgres(postgres::Session),
}

/// What a cancel request needs to find a session's running query on the server.
#[derive(Debug, Clone, Copy)]
pub enum CancelTarget {
    Postgres { pid: i32 },
}

pub async fn open_session(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<Session> {
    with_timeout(async {
        match connection.engine {
            Engine::Postgres => Ok(Session::Postgres(postgres::open_session(connection, password, database).await?)),
            other => Err(AppError::Unsupported(other.label())),
        }
    })
    .await
}

impl Session {
    pub fn cancel_target(&self) -> CancelTarget {
        match self {
            Session::Postgres(session) => CancelTarget::Postgres { pid: session.pid() },
        }
    }

    /// Runs a script of one or more statements and returns every statement's result.
    pub async fn run(&mut self, sql: &str, row_limit: usize) -> Result<QueryOutcome> {
        match self {
            Session::Postgres(session) => session.run(sql, row_limit).await,
        }
    }

    /// False when the connection dropped and the session can't be used again.
    pub async fn is_alive(&mut self) -> bool {
        let check = async {
            Ok(match self {
                Session::Postgres(session) => session.is_alive().await,
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
        }
    })
    .await
}

async fn with_timeout<T>(request: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(TIMEOUT, request).await.map_err(|_| AppError::Timeout(TIMEOUT.as_secs()))?
}
