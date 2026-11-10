//! sqlx connections whose every statement is recorded in the statement log.
//!
//! A driver opens a [`Logged`] connection in place of sqlx's own, and runs
//! `sqlx::query(…).fetch_all(&mut conn)` on it as before: [`Logged`] is an
//! sqlx executor that hands each statement to the real connection and
//! records it on the way. A transaction begun on it is a
//! [`LoggedTransaction`], which records `BEGIN`, its statements, and `COMMIT`
//! or `ROLLBACK`. There is no way to reach the connection underneath, so no
//! statement goes unrecorded.

use std::fmt::{self, Debug};
use std::marker::PhantomData;

use either::Either;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use sqlx::error::BoxDynError;
use sqlx::{Connection, Database, Error, Execute, Executor, SqlStr, Transaction};

use crate::error::database_message;
use crate::statement_log::Recording;

/// How many rows a statement changed, for the engines sqlx speaks to.
pub trait Affected {
    fn rows_affected(&self) -> u64;
}

impl Affected for sqlx::postgres::PgQueryResult {
    fn rows_affected(&self) -> u64 {
        self.rows_affected()
    }
}

impl Affected for sqlx::mysql::MySqlQueryResult {
    fn rows_affected(&self) -> u64 {
        self.rows_affected()
    }
}

impl Affected for sqlx::sqlite::SqliteQueryResult {
    fn rows_affected(&self) -> u64 {
        self.rows_affected()
    }
}

/// An sqlx connection whose statements are recorded, with the database it is on.
pub struct Logged<C> {
    inner: C,
    database: Option<String>,
}

impl<C: Connection> Logged<C> {
    /// Wraps a connection opened on `database` (empty or `None`: the server's default).
    pub fn new(inner: C, database: Option<&str>) -> Self {
        Self { inner, database: database.filter(|name| !name.is_empty()).map(str::to_owned) }
    }

    /// The database statements are recorded as sent to.
    pub fn database(&self) -> Option<&str> {
        self.database.as_deref()
    }

    /// Whether the connection still answers. Not a statement, so not recorded.
    pub async fn ping(&mut self) -> Result<(), Error> {
        self.inner.ping().await
    }

    pub async fn close(self) -> Result<(), Error> {
        self.inner.close().await
    }

    /// Begins a transaction; its statements are recorded like the connection's.
    pub async fn begin(&mut self) -> Result<LoggedTransaction<'_, C::Database>, Error> {
        let mut recording = Recording::start(self.database.as_deref(), || "BEGIN".into());
        match self.inner.begin().await {
            Ok(inner) => Ok(LoggedTransaction { inner: Some(inner), database: self.database.clone() }),
            Err(error) => {
                recording.fail(database_message(&error));
                Err(error)
            }
        }
    }
}

impl<C> Debug for Logged<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Logged").field("database", &self.database).finish_non_exhaustive()
    }
}

/// A transaction on a [`Logged`] connection. Dropped without [`commit`](Self::commit),
/// it rolls back, as sqlx's does, and records the `ROLLBACK`.
pub struct LoggedTransaction<'c, DB: Database> {
    /// `None` once committed or rolled back.
    inner: Option<Transaction<'c, DB>>,
    database: Option<String>,
}

impl<DB: Database> LoggedTransaction<'_, DB> {
    pub async fn commit(mut self) -> Result<(), Error> {
        let transaction = self.inner.take().expect("a transaction is open until committed or rolled back");
        let mut recording = Recording::start(self.database.as_deref(), || "COMMIT".into());
        transaction.commit().await.inspect_err(|error| recording.fail(database_message(error)))
    }

    pub async fn rollback(mut self) -> Result<(), Error> {
        let transaction = self.inner.take().expect("a transaction is open until committed or rolled back");
        let mut recording = Recording::start(self.database.as_deref(), || "ROLLBACK".into());
        transaction.rollback().await.inspect_err(|error| recording.fail(database_message(error)))
    }

    fn connection(&mut self) -> &mut DB::Connection {
        self.inner.as_deref_mut().expect("a transaction is open until committed or rolled back")
    }
}

impl<DB: Database> Drop for LoggedTransaction<'_, DB> {
    fn drop(&mut self) {
        // sqlx rolls an unfinished transaction back when it is next used; say so now.
        if self.inner.is_some() {
            drop(Recording::start(self.database.as_deref(), || "ROLLBACK".into()));
        }
    }
}

impl<DB: Database> Debug for LoggedTransaction<'_, DB> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoggedTransaction").field("database", &self.database).finish_non_exhaustive()
    }
}

/// What `fetch_many` streams: each statement's result, and the rows it returns.
type Steps<'e, DB> = BoxStream<'e, Result<Either<<DB as Database>::QueryResult, <DB as Database>::Row>, Error>>;

/// A statement taken apart to read its text, then handed on unchanged.
struct Forwarded<DB: Database> {
    sql: SqlStr,
    arguments: Option<DB::Arguments>,
    persistent: bool,
    _database: PhantomData<DB>,
}

impl<DB: Database> Forwarded<DB> {
    fn from<'q>(mut query: impl Execute<'q, DB>) -> Result<Self, BoxDynError> {
        let arguments = query.take_arguments()?;
        let persistent = query.persistent();
        Ok(Self { sql: query.sql(), arguments, persistent, _database: PhantomData })
    }
}

impl<'q, DB: Database> Execute<'q, DB> for Forwarded<DB> {
    fn sql(self) -> SqlStr {
        self.sql
    }

    fn statement(&self) -> Option<&DB::Statement> {
        None
    }

    fn take_arguments(&mut self) -> Result<Option<DB::Arguments>, BoxDynError> {
        Ok(self.arguments.take())
    }

    fn persistent(&self) -> bool {
        self.persistent
    }
}

/// Runs `query` on `executor` as a stream, recording it with its rows, the
/// rows it changed, or its error once the stream ends or is dropped.
fn fetch_many_recorded<'e, 'q: 'e, DB, X, E>(executor: X, database: Option<&str>, query: E) -> Steps<'e, DB>
where
    DB: Database,
    DB::QueryResult: Affected,
    X: Executor<'e, Database = DB> + 'e,
    E: 'q + Execute<'q, DB>,
{
    let query = match Forwarded::from(query) {
        Ok(query) => query,
        Err(error) => return futures_util::stream::once(async { Err(Error::Encode(error)) }).boxed(),
    };
    let mut recording = Recording::start(database, || query.sql.as_str().to_owned());
    executor
        .fetch_many(query)
        .inspect(move |step| match step {
            Ok(Either::Left(done)) => recording.affected(done.rows_affected()),
            Ok(Either::Right(_)) => recording.row(),
            Err(error) => recording.fail(database_message(error)),
        })
        .boxed()
}

/// Runs `query` on `executor` for at most one row, recording it.
fn fetch_optional_recorded<'e, 'q: 'e, DB, X, E>(
    executor: X,
    database: Option<&str>,
    query: E,
) -> BoxFuture<'e, Result<Option<DB::Row>, Error>>
where
    DB: Database,
    X: Executor<'e, Database = DB> + 'e,
    E: 'q + Execute<'q, DB>,
{
    let query = match Forwarded::from(query) {
        Ok(query) => query,
        Err(error) => return async { Err(Error::Encode(error)) }.boxed(),
    };
    let mut recording = Recording::start(database, || query.sql.as_str().to_owned());
    executor
        .fetch_optional(query)
        .map(move |result| {
            match &result {
                Ok(row) => recording.rows(u64::from(row.is_some())),
                Err(error) => recording.fail(database_message(error)),
            }
            result
        })
        .boxed()
}

impl<'c, C> Executor<'c> for &'c mut Logged<C>
where
    C: Connection,
    <C::Database as Database>::QueryResult: Affected,
    for<'a> &'a mut C: Executor<'a, Database = C::Database>,
{
    type Database = C::Database;

    fn fetch_many<'e, 'q: 'e, E>(self, query: E) -> Steps<'e, C::Database>
    where
        'c: 'e,
        E: 'q + Execute<'q, C::Database>,
    {
        fetch_many_recorded(&mut self.inner, self.database.as_deref(), query)
    }

    fn fetch_optional<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxFuture<'e, Result<Option<<C::Database as Database>::Row>, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, C::Database>,
    {
        fetch_optional_recorded(&mut self.inner, self.database.as_deref(), query)
    }

    fn prepare_with<'e>(
        self,
        sql: SqlStr,
        parameters: &'e [<C::Database as Database>::TypeInfo],
    ) -> BoxFuture<'e, Result<<C::Database as Database>::Statement, Error>>
    where
        'c: 'e,
    {
        self.inner.prepare_with(sql, parameters)
    }
}

impl<'c, 't, DB> Executor<'c> for &'c mut LoggedTransaction<'t, DB>
where
    DB: Database,
    DB::QueryResult: Affected,
    for<'a> &'a mut DB::Connection: Executor<'a, Database = DB>,
{
    type Database = DB;

    fn fetch_many<'e, 'q: 'e, E>(self, query: E) -> Steps<'e, DB>
    where
        'c: 'e,
        E: 'q + Execute<'q, DB>,
    {
        let database = self.database.clone();
        fetch_many_recorded(self.connection(), database.as_deref(), query)
    }

    fn fetch_optional<'e, 'q: 'e, E>(self, query: E) -> BoxFuture<'e, Result<Option<DB::Row>, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, DB>,
    {
        let database = self.database.clone();
        fetch_optional_recorded(self.connection(), database.as_deref(), query)
    }

    fn prepare_with<'e>(
        self,
        sql: SqlStr,
        parameters: &'e [DB::TypeInfo],
    ) -> BoxFuture<'e, Result<DB::Statement, Error>>
    where
        'c: 'e,
    {
        self.connection().prepare_with(sql, parameters)
    }
}
