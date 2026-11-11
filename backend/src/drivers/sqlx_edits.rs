//! Runs the grid's edit statements on the engines sqlx speaks to.

use sqlx::{AssertSqlSafe, Database, Encode, Executor, IntoArguments, Type};

use super::edit_sql::{self, Dialect, Statement};
use super::logged::{Affected, LoggedTransaction};
use super::{Column, ImportBatches, TableRef};
use crate::error::Result;
use crate::statement_log;

/// Runs `statements` in `transaction`, in order, binding their parameters.
/// Each is recorded with its values written in, so it can be copied and run
/// as is. A statement that should change one row (it has a key) and changes
/// none means the row changed elsewhere: the error leaves the transaction to
/// be dropped, which rolls everything back.
pub async fn run<DB>(
    transaction: &mut LoggedTransaction<'_, DB>,
    dialect: &Dialect,
    statements: &[Statement],
) -> Result<()>
where
    DB: Database,
    DB::QueryResult: Affected,
    for<'a> &'a mut DB::Connection: Executor<'a, Database = DB>,
    for<'q> DB::Arguments: IntoArguments<DB>,
    for<'q> Option<String>: Encode<'q, DB> + Type<DB>,
    crate::error::AppError: From<sqlx::Error>,
{
    for statement in statements {
        let mut query = sqlx::query::<DB>(AssertSqlSafe(statement.sql.clone()));
        for param in &statement.params {
            query = query.bind(param.clone());
        }
        let shown = edit_sql::inline(dialect, statement);
        let affected = statement_log::shown_as(shown, query.execute(&mut *transaction)).await?.rows_affected();
        if let Some(key) = &statement.key
            && affected != 1
        {
            return Err(edit_sql::changed_elsewhere(key));
        }
    }
    Ok(())
}

/// Inserts the rows of an import in `transaction` as they come, one
/// multi-row INSERT at a time, sized to `max_params` parameters; `check`
/// refuses a value its column can't take as text before it is sent. Each is
/// recorded as its columns and row count, not its values, which would make
/// the statement log as large as the file. Returns how many rows went in.
#[allow(clippy::too_many_arguments)]
pub async fn insert_batches<DB>(
    transaction: &mut LoggedTransaction<'_, DB>,
    dialect: &Dialect,
    columns: &[Column],
    table: &TableRef,
    names: &[String],
    batches: &mut ImportBatches,
    max_params: usize,
    check: impl Fn(&Column, &str) -> Result<()>,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<u64>
where
    DB: Database,
    DB::QueryResult: Affected,
    for<'a> &'a mut DB::Connection: Executor<'a, Database = DB>,
    for<'q> DB::Arguments: IntoArguments<DB>,
    for<'q> Option<String>: Encode<'q, DB> + Type<DB>,
    crate::error::AppError: From<sqlx::Error>,
{
    let size = edit_sql::rows_per_insert(max_params, names.len());
    let targets: Vec<Option<&Column>> = names.iter().map(|name| columns.iter().find(|c| &c.name == name)).collect();
    let mut inserted = 0;
    while let Some(batch) = batches.recv().await {
        let batch = batch?;
        for row in &batch {
            for (column, value) in targets.iter().zip(row) {
                if let (Some(column), Some(text)) = (column, value) {
                    check(column, text)?;
                }
            }
        }
        for rows in batch.chunks(size) {
            let edit_sql::InsertBatch { statement, shown } =
                edit_sql::insert_rows(dialect, columns, table, names, rows)?;
            let mut query = sqlx::query::<DB>(AssertSqlSafe(statement.sql));
            for param in statement.params {
                query = query.bind(param);
            }
            statement_log::shown_as(shown, query.execute(&mut *transaction)).await?;
            inserted += rows.len() as u64;
            progress(inserted);
        }
    }
    Ok(inserted)
}
