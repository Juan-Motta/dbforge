//! A tiberius client whose every statement is recorded in the statement log.
//!
//! The driver holds a [`LoggedClient`] in place of tiberius' own client, which
//! it can't reach: batches go through [`LoggedClient::simple_query`],
//! statements with parameters through [`LoggedClient::query`] or
//! [`LoggedClient::execute`] as a [`Statement`], whose parameters are known
//! here and so recorded written in as literals.

use std::borrow::Cow;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use tiberius::{ExecuteResult, Query, QueryItem, QueryStream, Row};

use crate::drivers::edit_sql;
use crate::error::sqlserver_message;
use crate::statement_log::Recording;
use tokio::net::TcpStream;
use tokio_util::compat::Compat;

/// tiberius' own client, reached only from here.
type Tds = tiberius::Client<Compat<TcpStream>>;

/// A value bound to a statement's `@P1`, `@P2`… markers.
#[derive(Debug, Clone, PartialEq)]
pub enum Param<'a> {
    Text(Cow<'a, str>),
    Int(i64),
    Null,
}

impl<'a> From<&'a str> for Param<'a> {
    fn from(text: &'a str) -> Self {
        Param::Text(Cow::Borrowed(text))
    }
}

impl<'a> From<&'a String> for Param<'a> {
    fn from(text: &'a String) -> Self {
        Param::Text(Cow::Borrowed(text))
    }
}

impl From<i64> for Param<'_> {
    fn from(number: i64) -> Self {
        Param::Int(number)
    }
}

impl<'a> From<Option<&'a str>> for Param<'a> {
    fn from(value: Option<&'a str>) -> Self {
        value.map_or(Param::Null, Param::from)
    }
}

impl Param<'_> {
    /// As written in T-SQL: text as a Unicode literal.
    fn literal(&self) -> String {
        match self {
            Param::Text(text) => format!("N'{}'", text.replace('\'', "''")),
            Param::Int(number) => number.to_string(),
            Param::Null => "NULL".into(),
        }
    }
}

/// SQL with values for its `@P1`, `@P2`… markers.
#[derive(Debug, Clone)]
pub struct Statement<'a> {
    sql: Cow<'a, str>,
    params: Vec<Param<'a>>,
}

impl<'a> Statement<'a> {
    pub fn new(sql: impl Into<Cow<'a, str>>) -> Self {
        Self { sql: sql.into(), params: Vec::new() }
    }

    pub fn bind(mut self, param: impl Into<Param<'a>>) -> Self {
        self.params.push(param.into());
        self
    }

    /// The statement with its values written in, for the log.
    fn shown(&self) -> String {
        let literals: Vec<String> = self.params.iter().map(Param::literal).collect();
        edit_sql::substitute(['[', ']'], Some("@P"), &self.sql, &literals)
    }

    fn into_query(self) -> Query<'a> {
        let mut query = Query::new(self.sql);
        for param in self.params {
            match param {
                Param::Text(text) => query.bind(text),
                Param::Int(number) => query.bind(number),
                Param::Null => query.bind(Option::<&str>::None),
            }
        }
        query
    }
}

/// A SQL Server client whose statements are recorded, with the database it is on.
pub struct LoggedClient {
    inner: Tds,
    database: Option<String>,
}

impl LoggedClient {
    pub fn new(inner: Tds, database: Option<&str>) -> Self {
        Self { inner, database: database.filter(|name| !name.is_empty()).map(str::to_owned) }
    }

    fn start(&self, text: impl FnOnce() -> String) -> Recording {
        Recording::start(self.database.as_deref(), text)
    }

    /// Runs a batch of T-SQL as written, without parameters.
    pub async fn simple_query(&mut self, sql: impl Into<String>) -> tiberius::Result<Recorded<'_>> {
        let sql = sql.into();
        let mut recording = self.start(|| sql.clone());
        match self.inner.simple_query(sql).await {
            Ok(stream) => Ok(Recorded { stream, recording }),
            Err(error) => {
                recording.fail(sqlserver_message(&error));
                Err(error)
            }
        }
    }

    /// Runs a statement with parameters and reads what it returns.
    pub async fn query<'a>(&'a mut self, statement: Statement<'_>) -> tiberius::Result<Recorded<'a>> {
        let mut recording = self.start(|| statement.shown());
        match statement.into_query().query(&mut self.inner).await {
            Ok(stream) => Ok(Recorded { stream, recording }),
            Err(error) => {
                recording.fail(sqlserver_message(&error));
                Err(error)
            }
        }
    }

    /// Runs a statement with parameters that returns no rows; says how many it changed.
    pub async fn execute(&mut self, statement: Statement<'_>) -> tiberius::Result<ExecuteResult> {
        let mut recording = self.start(|| statement.shown());
        let result = statement.into_query().execute(&mut self.inner).await;
        match &result {
            Ok(done) => recording.affected(done.rows_affected().iter().sum()),
            Err(error) => recording.fail(sqlserver_message(error)),
        }
        result
    }

    /// Asks the server to stop the running query. Not a statement, so not recorded.
    pub async fn cancel_query(&mut self) -> tiberius::Result<()> {
        self.inner.cancel_query().await
    }

    pub async fn close(self) -> tiberius::Result<()> {
        self.inner.close().await
    }
}

/// What a statement returns, read as a stream or all at once; recorded with
/// the rows read, or its error, when read or dropped.
pub struct Recorded<'a> {
    stream: QueryStream<'a>,
    recording: Recording,
}

impl Recorded<'_> {
    pub async fn into_results(self) -> tiberius::Result<Vec<Vec<Row>>> {
        let Self { stream, mut recording } = self;
        let results = stream.into_results().await;
        match &results {
            Ok(sets) => recording.rows(sets.iter().map(|set| set.len() as u64).sum()),
            Err(error) => recording.fail(sqlserver_message(error)),
        }
        results
    }

    #[cfg(test)]
    pub async fn into_row(self) -> tiberius::Result<Option<Row>> {
        let Self { stream, mut recording } = self;
        let row = stream.into_row().await;
        match &row {
            Ok(row) => recording.rows(u64::from(row.is_some())),
            Err(error) => recording.fail(sqlserver_message(error)),
        }
        row
    }

    pub async fn into_first_result(self) -> tiberius::Result<Vec<Row>> {
        let Self { stream, mut recording } = self;
        let rows = stream.into_first_result().await;
        match &rows {
            Ok(rows) => recording.rows(rows.len() as u64),
            Err(error) => recording.fail(sqlserver_message(error)),
        }
        rows
    }
}

impl Stream for Recorded<'_> {
    type Item = tiberius::Result<QueryItem>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let next = Pin::new(&mut this.stream).poll_next(cx);
        match &next {
            Poll::Ready(Some(Ok(QueryItem::Row(_)))) => this.recording.row(),
            Poll::Ready(Some(Err(error))) => this.recording.fail(sqlserver_message(error)),
            _ => {}
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_parameters_written_in_as_t_sql() {
        let statement = Statement::new("SELECT * FROM [t@P1] WHERE a = @P1 AND b = @P2 AND c = @P3 AND d = @P10")
            .bind("it's")
            .bind(Some("x"))
            .bind(Option::<&str>::None)
            .bind(4_i64);
        assert_eq!(statement.shown(), "SELECT * FROM [t@P1] WHERE a = N'it''s' AND b = N'x' AND c = NULL AND d = @P10");
    }
}
