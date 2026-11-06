//! A SQL Server table's structure and a schema's ER diagram.

use super::super::{SchemaDiagram, TableRef, TableStructure};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

pub async fn table_structure(
    _connection: &Connection,
    _password: Option<&str>,
    _table: &TableRef,
) -> Result<TableStructure> {
    Err(AppError::Unsupported("SQL Server structure"))
}

pub async fn schema_diagram(
    _connection: &Connection,
    _password: Option<&str>,
    _database: &str,
    _schema: &str,
) -> Result<SchemaDiagram> {
    Err(AppError::Unsupported("SQL Server diagrams"))
}
