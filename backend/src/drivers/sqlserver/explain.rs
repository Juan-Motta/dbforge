//! Estimated plans from SHOWPLAN_XML.

use crate::connections::model::Connection;
use crate::error::{AppError, Result};

pub async fn explain(
    _connection: &Connection,
    _password: Option<&str>,
    _database: Option<&str>,
    _sql: &str,
    _analyze: bool,
) -> Result<serde_json::Value> {
    Err(AppError::Unsupported("SQL Server explain"))
}
