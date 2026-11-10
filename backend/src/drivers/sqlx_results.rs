//! What a console script returns on the engines sqlx speaks to (PostgreSQL,
//! MySQL and MariaDB, SQLite): one result per statement. Each engine says how
//! its values read as text; the rest is the same for all.

use futures_util::TryStreamExt;
use futures_util::stream::BoxStream;
use sqlx::{Column as _, Either, Row as _, TypeInfo as _};

use super::logged::Affected;
use super::{Column, StatementResult};
use crate::error::Result;

/// The result of every statement `stream` returns. All rows are read so every
/// statement completes, but only the first `row_limit` of each result are
/// kept. A statement that returns rows counts them as its rows affected.
pub async fn read_results<DB: sqlx::Database>(
    mut stream: BoxStream<'_, sqlx::Result<Either<DB::QueryResult, DB::Row>>>,
    row_limit: usize,
    text_values: impl Fn(&DB::Row) -> Result<Vec<Option<String>>>,
) -> Result<Vec<StatementResult>>
where
    DB::QueryResult: Affected,
{
    let mut results = Vec::new();
    let mut current = StatementResult::default();
    // Rows the current statement returned, kept or not.
    let mut returned = 0u64;
    while let Some(item) = stream.try_next().await? {
        match item {
            Either::Right(row) => {
                returned += 1;
                if current.columns.is_empty() {
                    current.columns = columns_of::<DB>(&row);
                }
                if current.rows.len() < row_limit {
                    current.rows.push(text_values(&row)?);
                } else {
                    current.truncated = true;
                }
            }
            // Marks the end of one statement.
            Either::Left(done) => {
                // MySQL and SQLite report no affected rows for a SELECT; what it returned is the count.
                current.rows_affected = if current.columns.is_empty() { done.rows_affected() } else { returned };
                results.push(std::mem::take(&mut current));
                returned = 0;
            }
        }
    }
    Ok(results)
}

/// A result's columns, as the row describes them.
pub fn columns_of<DB: sqlx::Database>(row: &DB::Row) -> Vec<Column> {
    row.columns()
        .iter()
        .map(|c| Column {
            name: c.name().to_string(),
            data_type: c.type_info().name().to_lowercase(),
            primary_key: false,
            generated: false,
            default: None,
        })
        .collect()
}
