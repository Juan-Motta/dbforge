//! Runs the grid's edit statements on the engines sqlx speaks to.

use sqlx::{AssertSqlSafe, Database, Encode, Executor, IntoArguments, Type};

use super::edit_sql::{self, Dialect, Statement};
use super::logged::{Affected, LoggedTransaction};
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
