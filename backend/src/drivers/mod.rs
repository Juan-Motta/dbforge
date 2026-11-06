//! Entry points that dispatch to the right database driver by engine.

mod postgres;

use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::connections::model::{Connection, Engine};
use crate::error::{AppError, Result};

const TIMEOUT: Duration = Duration::from_secs(10);
/// User queries can legitimately run long; this only guards against hanging forever.
const QUERY_TIMEOUT: Duration = Duration::from_secs(300);

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

/// Identifies a table or view to read rows from.
#[derive(Debug, Clone, Deserialize)]
pub struct TableRef {
    pub database: String,
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    /// The type as the server writes it, e.g. `character varying(80)`.
    pub data_type: String,
    pub primary_key: bool,
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

/// Runs a script of one or more statements and returns every statement's result.
pub async fn run_query(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    sql: &str,
    row_limit: usize,
) -> Result<QueryOutcome> {
    let request = async {
        match connection.engine {
            Engine::Postgres => postgres::run_query(connection, password, database, sql, row_limit).await,
            other => Err(AppError::Unsupported(other.label())),
        }
    };
    tokio::time::timeout(QUERY_TIMEOUT, request)
        .await
        .map_err(|_| AppError::Timeout(QUERY_TIMEOUT.as_secs()))?
}

async fn with_timeout<T>(request: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(TIMEOUT, request)
        .await
        .map_err(|_| AppError::Timeout(TIMEOUT.as_secs()))?
}
