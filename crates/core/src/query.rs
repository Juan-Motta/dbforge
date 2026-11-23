//! Query consoles. Each console (a tab in the UI) owns a session: a connection
//! kept open between runs, so transactions and `SET` carry over like in psql.
//!
//! A SQL console runs in Auto-commit, where each statement commits unless the
//! script opens a transaction itself, or in Manual, where its first run opens
//! a transaction that every later run stays in until Commit or Rollback.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::connections::model::{Connection, Engine};
use crate::connections::store::ConnectionStore;
use crate::drivers::{self, CancelTarget, QueryOutcome};
use crate::error::{AppError, Result};
use crate::history::{HistoryEntry, HistoryStore};
use crate::statement_log::{self, Origin, StatementLog};

/// Rows kept per result; the rest are read and dropped.
const ROW_LIMIT: usize = 500;

/// How long closing a console waits for its cancelled query to stop before
/// giving up on rolling back itself; the server rolls back when the connection closes.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// Refused while a transaction is open: the new session would leave it behind.
const CHANGE_REFUSED: &str =
    "A transaction is open on this console: commit or roll back before changing the database or the connection.";

/// Where a console's queries run: a database (the connection's own when
/// `None`) and, for Cosmos DB, the container picked in the console.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct QueryScope {
    pub database: Option<String>,
    pub container: Option<String>,
}

/// Whether a console commits each statement or keeps a transaction open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransactionMode {
    /// Each statement commits, unless the script opens a transaction itself.
    #[default]
    Auto,
    /// The first run opens a transaction; it stays open until Commit or Rollback.
    Manual,
}

/// The transaction a Manual console has open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTransaction {
    /// When it opened, in milliseconds since the Unix epoch.
    pub started_at_ms: i64,
    /// When a run in it last ended, in milliseconds since the Unix epoch.
    pub last_activity_ms: i64,
    /// Statements that returned no result set: those that changed something.
    pub changes: u32,
    /// Rows those statements changed.
    pub rows: u64,
    /// A statement failed in it on PostgreSQL, which then refuses everything
    /// until the transaction ends: only Rollback works now.
    pub aborted: bool,
}

impl OpenTransaction {
    fn new(now: i64) -> Self {
        Self { started_at_ms: now, last_activity_ms: now, changes: 0, rows: 0, aborted: false }
    }
}

/// How a transaction ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransactionEnd {
    Committed,
    RolledBack,
}

/// What Commit or Rollback did, with the transaction as it was when it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitOutcome {
    pub ended: TransactionEnd,
    pub transaction: OpenTransaction,
}

/// A console with a transaction open, e.g. to list before quitting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleTransaction {
    pub session_id: String,
    pub connection_id: String,
    pub connection_name: String,
    pub transaction: OpenTransaction,
}

/// The statements that open and end a transaction in an engine's SQL.
struct Dialect {
    begin: &'static str,
    commit: &'static str,
    rollback: &'static str,
    /// Sent after the transaction ends, to undo what `begin` changed on the connection.
    restore: Option<&'static str>,
}

impl Dialect {
    /// `None` for engines without transactions a console can hold open.
    ///
    /// MySQL and MariaDB turn autocommit off rather than `START TRANSACTION`:
    /// a statement that commits implicitly (DDL, `LOCK TABLES`…) then leaves
    /// the next statements in a new transaction instead of autocommitting them.
    fn of(engine: Engine) -> Option<Self> {
        let (begin, commit, rollback, restore) = match engine {
            Engine::Postgres | Engine::Sqlite => ("BEGIN", "COMMIT", "ROLLBACK", None),
            Engine::Mysql | Engine::Mariadb => ("SET autocommit = 0", "COMMIT", "ROLLBACK", Some("SET autocommit = 1")),
            Engine::Sqlserver => ("BEGIN TRANSACTION", "COMMIT TRANSACTION", "ROLLBACK TRANSACTION", None),
            Engine::Mongodb | Engine::Redis | Engine::Cosmosdb => return None,
        };
        Some(Self { begin, commit, rollback, restore })
    }
}

/// Said when a console closed while its run was opening the connection.
const CLOSED: &str = "This console was closed.";

/// Said when SQL Server ended the transaction itself: a trigger's ROLLBACK, `XACT_ABORT`.
const SERVER_ENDED: &str = "The server ended the transaction (it was rolled back).";

/// Whether a console on `engine` can run in [`TransactionMode::Manual`]:
/// the SQL engines can; MongoDB, Redis and Cosmos DB can't.
pub fn supports_manual_transactions(engine: Engine) -> bool {
    Dialect::of(engine).is_some()
}

/// Open sessions by console id, and each console's transaction mode.
#[derive(Default)]
pub struct QuerySessions {
    sessions: Mutex<HashMap<String, Arc<QuerySession>>>,
    /// Set when the console is created, before its session opens; Auto when absent.
    modes: Mutex<HashMap<String, TransactionMode>>,
}

struct QuerySession {
    /// The connection's settings when the session opened.
    connection: Connection,
    /// Its password then, to cancel with.
    password: Option<String>,
    scope: QueryScope,
    cancel_target: CancelTarget,
    /// Locked while a query runs; one query at a time per session.
    driver: tokio::sync::Mutex<drivers::Session>,
    /// The transaction a Manual console opened on this connection. It lives
    /// and dies with the connection: a new session never inherits it.
    transaction: Mutex<Option<OpenTransaction>>,
}

impl QuerySessions {
    fn get(&self, id: &str) -> Option<Arc<QuerySession>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    fn insert(&self, id: String, session: Arc<QuerySession>) {
        self.sessions.lock().unwrap().insert(id, session);
    }

    fn take(&self, id: &str) -> Option<Arc<QuerySession>> {
        self.sessions.lock().unwrap().remove(id)
    }

    /// Removes `id` only while it still maps to `session`, so a newer one survives.
    fn remove(&self, id: &str, session: &Arc<QuerySession>) {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.get(id).is_some_and(|current| Arc::ptr_eq(current, session)) {
            sessions.remove(id);
        }
    }

    fn mode(&self, id: &str) -> TransactionMode {
        self.modes.lock().unwrap().get(id).copied().unwrap_or_default()
    }

    fn has_mode(&self, id: &str) -> bool {
        self.modes.lock().unwrap().contains_key(id)
    }

    /// Inserts `session` unless the console was closed since `had_mode` was
    /// read. Checked with the modes locked, as `close_session` removes the
    /// mode before taking the session: it either finds this one or it was first.
    fn insert_unless_closed(&self, id: &str, had_mode: bool, session: Arc<QuerySession>) -> Result<()> {
        let modes = self.modes.lock().unwrap();
        if had_mode && !modes.contains_key(id) {
            return Err(AppError::Invalid(CLOSED.into()));
        }
        self.insert(id.to_string(), session);
        Ok(())
    }
}

impl QuerySession {
    async fn open(connection: Connection, password: Option<String>, scope: QueryScope) -> Result<Self> {
        let (database, container) = (scope.database.as_deref(), scope.container.as_deref());
        let driver = drivers::open_session(&connection, password.as_deref(), database, container).await?;
        Ok(Self {
            cancel_target: driver.cancel_target(),
            connection,
            password,
            scope,
            driver: driver.into(),
            transaction: Mutex::default(),
        })
    }

    fn is_running(&self) -> bool {
        self.driver.try_lock().is_err()
    }

    async fn cancel(&self) -> Result<()> {
        let database = self.scope.database.as_deref();
        drivers::cancel(&self.connection, self.password.as_deref(), database, self.cancel_target.clone()).await
    }

    fn transaction(&self) -> Option<OpenTransaction> {
        self.transaction.lock().unwrap().clone()
    }

    fn set_transaction(&self, transaction: OpenTransaction) {
        *self.transaction.lock().unwrap() = Some(transaction);
    }

    fn take_transaction(&self) -> Option<OpenTransaction> {
        self.transaction.lock().unwrap().take()
    }

    /// Counts what a run in the open transaction did. A failure there leaves a
    /// PostgreSQL transaction aborted; a deadlock on MySQL, MariaDB or SQL
    /// Server rolls the whole transaction back, which the run's error then says.
    /// `recovered`: the run failed but was undone back to its savepoint (PostgreSQL), so the
    /// transaction is still usable.
    fn record_run(&self, outcome: Result<QueryOutcome>, recovered: bool) -> Result<QueryOutcome> {
        let mut guard = self.transaction.lock().unwrap();
        let Some(transaction) = guard.as_mut() else { return outcome };
        transaction.last_activity_ms = now_ms();
        if let Ok(outcome) = &outcome {
            for result in outcome.results.iter().filter(|result| result.columns.is_empty()) {
                transaction.changes += 1;
                transaction.rows += result.rows_affected;
            }
        }
        let error = match &outcome {
            Ok(outcome) => outcome.error.clone(),
            Err(error) => Some(error.to_string()),
        };
        let Some(error) = error else { return outcome };
        let engine = self.connection.engine;
        if engine == Engine::Postgres {
            transaction.aborted = !recovered;
            return outcome;
        }
        if !rolled_back_by(engine, &error) {
            return outcome;
        }
        *guard = None;
        const ROLLED_BACK: &str = "The server rolled back the transaction.";
        match outcome {
            Ok(mut outcome) => {
                outcome.error = Some(format!("{error} {ROLLED_BACK}"));
                Ok(outcome)
            }
            Err(_) => Err(AppError::Invalid(format!("{error} {ROLLED_BACK}"))),
        }
    }
}

/// Whether `error` means the server rolled back the whole transaction, not
/// just the statement: a deadlock victim on MySQL, MariaDB and SQL Server.
fn rolled_back_by(engine: Engine, error: &str) -> bool {
    match engine {
        Engine::Mysql | Engine::Mariadb => error.contains("Deadlock found"),
        Engine::Sqlserver => error.contains("deadlock victim"),
        _ => false,
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

/// Ends the transaction with `statement`, then restores what the dialect's
/// BEGIN changed on the connection. A connection whose restore failed is
/// dropped from `sessions`: an Auto run on it would never commit.
async fn send_end(
    sessions: &QuerySessions,
    session_id: &str,
    session: &Arc<QuerySession>,
    driver: &mut drivers::Session,
    statement: &str,
    restore: Option<&str>,
) -> Result<()> {
    run_statement(driver, statement).await?;
    if let Some(restore) = restore
        && run_statement(driver, restore).await.is_err()
    {
        sessions.remove(session_id, session);
    }
    Ok(())
}

/// Whether SQL Server still has a transaction open on `driver`, read with
/// `@@TRANCOUNT`; `None` when it couldn't be read.
async fn sqlserver_in_transaction(driver: &mut drivers::Session) -> Option<bool> {
    let outcome = Box::pin(driver.run("SELECT @@TRANCOUNT", ROW_LIMIT)).await.ok()?;
    let count = outcome.results.first()?.rows.first()?.first()?.clone()?;
    Some(count.trim() != "0")
}

/// Runs one statement the console sends itself (BEGIN, COMMIT, ROLLBACK).
/// The savepoint each PostgreSQL run in an open transaction starts from.
const RUN_SAVEPOINT: &str = "SAVEPOINT dbforge_run";

/// Ends a run's savepoint: rolled back to it when the run `failed` (true when that worked,
/// the transaction then usable again), released otherwise.
async fn finish_savepoint(driver: &mut drivers::Session, failed: bool) -> bool {
    if failed {
        run_statement(driver, "ROLLBACK TO SAVEPOINT dbforge_run").await.is_ok()
    } else {
        let _ = run_statement(driver, "RELEASE SAVEPOINT dbforge_run").await;
        false
    }
}

async fn run_statement(driver: &mut drivers::Session, sql: &str) -> Result<()> {
    // Boxed, like every run here: a session's run future is large for the stack.
    match Box::pin(driver.run(sql, ROW_LIMIT)).await?.error {
        Some(error) => Err(AppError::Invalid(error)),
        None => Ok(()),
    }
}

/// Sets the console's mode; the app sets it when it creates the console, so
/// it can come before the console's session opens. Back to Auto is refused
/// while a transaction is open, or while a run could be opening one.
pub fn set_transaction_mode(sessions: &QuerySessions, session_id: &str, mode: TransactionMode) -> Result<()> {
    // Held while checking, so a run reads the mode either before or after the change.
    let mut modes = sessions.modes.lock().unwrap();
    let current = modes.get(session_id).copied().unwrap_or_default();
    if mode == TransactionMode::Auto
        && current == TransactionMode::Manual
        && let Some(session) = sessions.get(session_id)
    {
        if session.transaction().is_some() {
            return Err(AppError::Invalid(
                "A transaction is open on this console: commit or roll back before switching to Auto-commit.".into(),
            ));
        }
        if session.is_running() {
            return Err(AppError::Invalid(
                "A query is running in this console: wait for it before switching to Auto-commit.".into(),
            ));
        }
    }
    modes.insert(session_id.to_string(), mode);
    Ok(())
}

/// The console's mode: Auto unless set otherwise.
pub fn transaction_mode(sessions: &QuerySessions, session_id: &str) -> TransactionMode {
    sessions.mode(session_id)
}

/// The transaction the console has open, if any.
pub fn transaction_state(sessions: &QuerySessions, session_id: &str) -> Option<OpenTransaction> {
    sessions.get(session_id).and_then(|session| session.transaction())
}

/// Every console with a transaction open, e.g. to ask about before quitting.
pub fn open_transactions(sessions: &QuerySessions) -> Vec<ConsoleTransaction> {
    let open: Vec<(String, Arc<QuerySession>)> =
        sessions.sessions.lock().unwrap().iter().map(|(id, session)| (id.clone(), session.clone())).collect();
    let mut open: Vec<ConsoleTransaction> = open
        .into_iter()
        .filter_map(|(session_id, session)| {
            Some(ConsoleTransaction {
                transaction: session.transaction()?,
                session_id,
                connection_id: session.connection.id.clone(),
                connection_name: session.connection.name.clone(),
            })
        })
        .collect();
    open.sort_by_key(|console| console.transaction.started_at_ms);
    open
}

/// Commits the console's open transaction.
pub async fn commit_transaction(
    sessions: &QuerySessions,
    log: &StatementLog,
    session_id: String,
) -> Result<CommitOutcome> {
    end_transaction(sessions, log, &session_id, TransactionEnd::Committed).await
}

/// Rolls back the console's open transaction.
pub async fn rollback_transaction(
    sessions: &QuerySessions,
    log: &StatementLog,
    session_id: String,
) -> Result<CommitOutcome> {
    end_transaction(sessions, log, &session_id, TransactionEnd::RolledBack).await
}

async fn end_transaction(
    sessions: &QuerySessions,
    log: &StatementLog,
    session_id: &str,
    end: TransactionEnd,
) -> Result<CommitOutcome> {
    let no_transaction = || AppError::Invalid("No transaction is open on this console.".into());
    let session = sessions.get(session_id).ok_or_else(no_transaction)?;
    let mut driver = session
        .driver
        .try_lock()
        .map_err(|_| AppError::Invalid("A query is running in this console: wait for it or cancel it first.".into()))?;
    let transaction = session.transaction().ok_or_else(no_transaction)?;
    let engine = session.connection.engine;
    if end == TransactionEnd::Committed && transaction.aborted {
        return Err(AppError::Invalid("A statement failed in this transaction, so it can only be rolled back.".into()));
    }
    let dialect = Dialect::of(engine).ok_or_else(no_transaction)?;
    let statement = match end {
        TransactionEnd::Committed => dialect.commit,
        TransactionEnd::RolledBack => dialect.rollback,
    };
    let end_it = send_end(sessions, session_id, &session, &mut driver, statement, dialect.restore);
    let sent = statement_log::scoped(log, &session.connection.id, Origin::Console, end_it).await;
    let ended = |end| CommitOutcome { ended: end, transaction: transaction.clone() };
    let Err(error) = sent else {
        session.take_transaction();
        return Ok(ended(end));
    };
    if !driver.is_alive().await {
        // The server rolls back the transaction of a connection it lost.
        sessions.remove(session_id, &session);
        session.take_transaction();
        return match end {
            TransactionEnd::RolledBack => Ok(ended(end)),
            TransactionEnd::Committed => Err(AppError::Invalid(format!(
                "The connection was lost, so the server rolled back the transaction instead of committing it: {error}"
            ))),
        };
    }
    if end == TransactionEnd::Committed && engine == Engine::Postgres {
        // A COMMIT PostgreSQL refuses (a deferred constraint, a serialization failure) ends the transaction.
        session.take_transaction();
        return Err(AppError::Invalid(format!(
            "PostgreSQL rolled back the transaction instead of committing it: {error}"
        )));
    }
    // Elsewhere a refused COMMIT or ROLLBACK (SQLite's database is locked) leaves it open.
    Err(error)
}

/// Runs a SQL script in the console's session, opening it on first use, and
/// records the run in the history. In Manual, a run with no transaction open
/// opens one first.
#[allow(clippy::too_many_arguments)]
pub async fn run_query(
    store: &ConnectionStore,
    sessions: &QuerySessions,
    history: &HistoryStore,
    log: &StatementLog,
    session_id: String,
    connection_id: String,
    scope: QueryScope,
    sql: String,
) -> Result<QueryOutcome> {
    let connection = store.get(&connection_id)?;
    let executed_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let started = Instant::now();
    // Read now, used only if a session opens; a failure is recorded like any other.
    let password = store.password(&connection.id);
    let run = run_in_session(sessions, &session_id, connection.clone(), password, scope.clone(), &sql);
    let outcome = statement_log::scoped(log, &connection.id, Origin::Console, run).await;

    let entry = HistoryEntry {
        id: Uuid::new_v4().to_string(),
        executed_at,
        connection_id: connection.id,
        connection_name: connection.name,
        database: scope.database,
        container: scope.container,
        elapsed_ms: match &outcome {
            Ok(outcome) => outcome.elapsed_ms,
            Err(_) => started.elapsed().as_millis() as u64,
        },
        statements: outcome.as_ref().map_or(0, |o| o.results.len()),
        rows: outcome.as_ref().ok().and_then(|o| o.results.last()).map_or(0, |r| r.rows_affected),
        // A script that stopped partway failed too, after its first statements ran.
        error: match &outcome {
            Ok(outcome) => outcome.error.clone(),
            Err(error) => Some(error.to_string()),
        },
        sql,
    };
    // Losing a history line must never fail the query itself.
    if let Err(error) = history.record(entry) {
        eprintln!("Could not save query history: {error}");
    }
    outcome
}

async fn run_in_session(
    sessions: &QuerySessions,
    session_id: &str,
    connection: Connection,
    password: Result<Option<String>>,
    scope: QueryScope,
    sql: &str,
) -> Result<QueryOutcome> {
    let engine = connection.engine;
    if sessions.mode(session_id) == TransactionMode::Manual && !supports_manual_transactions(engine) {
        return Err(AppError::Invalid(format!("{} consoles can only run in Auto-commit.", engine.label())));
    }
    // A console the app created has a mode; once it's gone, the console was closed.
    let had_mode = sessions.has_mode(session_id);
    let session = match sessions.get(session_id) {
        // Edited connection settings, or another database or container, take effect on the next run.
        Some(session) if session.connection == connection && session.scope == scope => session,
        // A new session would leave the open transaction behind, uncommitted.
        Some(session) if session.transaction().is_some() => return Err(AppError::Invalid(CHANGE_REFUSED.into())),
        _ => {
            let session = Arc::new(Box::pin(QuerySession::open(connection, password?, scope)).await?);
            // Closed while it connected: dropping the new session closes its connection.
            sessions.insert_unless_closed(session_id, had_mode, session.clone())?;
            session
        }
    };

    let mut driver = session
        .driver
        .try_lock()
        .map_err(|_| AppError::Invalid("A query is already running in this console".into()))?;
    // Closed between the insert and the lock: `close_session` found nothing running to wait for.
    if had_mode && !sessions.has_mode(session_id) {
        sessions.remove(session_id, &session);
        return Err(AppError::Invalid(CLOSED.into()));
    }
    // Read with the session locked: `set_transaction_mode` refuses Auto while it runs.
    let begin = match sessions.mode(session_id) {
        TransactionMode::Manual if session.transaction().is_none() => Dialect::of(engine).map(|d| d.begin),
        _ => None,
    };
    if let Some(begin) = begin {
        if let Err(error) = run_statement(&mut driver, begin).await {
            if !driver.is_alive().await {
                sessions.remove(session_id, &session);
            }
            return Err(error);
        }
        session.set_transaction(OpenTransaction::new(now_ms()));
    }
    // PostgreSQL fails a whole transaction on one failed statement: each run in an open one
    // gets a savepoint, so a failed run undoes only itself and the transaction carries on.
    let savepoint = engine == Engine::Postgres
        && session.transaction().is_some_and(|transaction| !transaction.aborted)
        && run_statement(&mut driver, RUN_SAVEPOINT).await.is_ok();
    let outcome = Box::pin(driver.run(sql, ROW_LIMIT)).await;
    // A dropped connection can't be reused; the next run opens a new one.
    let failed = outcome.as_ref().map_or(true, |outcome| outcome.error.is_some());
    if failed && !driver.is_alive().await {
        sessions.remove(session_id, &session);
        // Never carry on as if it were still open: the next run would autocommit.
        if session.take_transaction().is_some() {
            let error = match &outcome {
                Ok(outcome) => outcome.error.clone().unwrap_or_default(),
                Err(error) => error.to_string(),
            };
            return Err(AppError::Invalid(format!(
                "The connection was lost, so the server rolled back the open transaction: {error}"
            )));
        }
        return outcome;
    }
    let recovered = savepoint && finish_savepoint(&mut driver, failed).await;
    let was_open = session.transaction().is_some();
    let outcome = session.record_run(outcome, recovered);
    if was_open
        && session.transaction().is_none()
        && let Some(restore) = Dialect::of(engine).and_then(|dialect| dialect.restore)
        && run_statement(&mut driver, restore).await.is_err()
    {
        // The server rolled back (a deadlock), but autocommit is still off: never reuse it.
        sessions.remove(session_id, &session);
    }
    if engine == Engine::Sqlserver
        && session.transaction().is_some()
        && sqlserver_in_transaction(&mut driver).await == Some(false)
    {
        // A trigger's ROLLBACK or XACT_ABORT ended it: the next run must open a new one.
        session.take_transaction();
        return match outcome {
            Ok(mut outcome) => {
                outcome.error = Some(match outcome.error {
                    Some(error) => format!("{error} {SERVER_ENDED}"),
                    None => SERVER_ENDED.to_owned(),
                });
                Ok(outcome)
            }
            Err(error) => Err(AppError::Invalid(format!("{error} {SERVER_ENDED}"))),
        };
    }
    outcome
}

/// The plan of the console's statement. See `drivers::explain` for why ANALYZE is safe.
pub async fn explain_query(
    store: &ConnectionStore,
    log: &StatementLog,
    connection_id: String,
    scope: QueryScope,
    sql: String,
    analyze: bool,
) -> Result<serde_json::Value> {
    let (database, container) = (scope.database.as_deref(), scope.container.as_deref());
    store
        .open(&connection_id)?
        .run(log, Origin::Console, async |connection, password| {
            drivers::explain(connection, password, database, container, &sql, analyze).await
        })
        .await
}

/// Stops the query running in the console, if any. On SQL Server this resets
/// the console's connection, which rolls back an open transaction; the run
/// that was cancelled says so.
pub async fn cancel_query(sessions: &QuerySessions, log: &StatementLog, session_id: String) -> Result<()> {
    match sessions.get(&session_id) {
        Some(session) if session.is_running() => {
            statement_log::scoped(log, &session.connection.id, Origin::Console, session.cancel()).await
        }
        _ => Ok(()),
    }
}

/// Closes the console's session, cancelling its query first if one is
/// running, and forgets its mode. An open transaction is rolled back here
/// rather than left to the server, so it shows in the statement log.
pub async fn close_session(sessions: &QuerySessions, log: &StatementLog, session_id: String) -> Result<()> {
    sessions.modes.lock().unwrap().remove(&session_id);
    // Dropping the last reference closes the connection.
    let Some(session) = sessions.take(&session_id) else { return Ok(()) };
    let id = session.connection.id.clone();
    let cancelled = match session.is_running() {
        true => statement_log::scoped(log, &id, Origin::Console, session.cancel()).await,
        false => Ok(()),
    };
    if session.transaction().is_some() {
        // Waits for a cancelled run to stop; past that, closing the connection rolls back.
        if let Ok(mut driver) = tokio::time::timeout(CLOSE_WAIT, session.driver.lock()).await
            && let (Some(_), Some(dialect)) = (session.take_transaction(), Dialect::of(session.connection.engine))
        {
            // Best effort: if it fails, the connection closes next and the server rolls back.
            let end_it = send_end(sessions, &session_id, &session, &mut driver, dialect.rollback, dialect.restore);
            let _ = statement_log::scoped(log, &id, Origin::Console, end_it).await;
        }
    }
    cancelled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::transport::{Ssl, SslMode};
    use crate::credentials::Credentials;
    use crate::statement_log::Entry;
    use crate::statement_log::store::tests::Collected;

    /// A SQLite file holding table `t`, saved as a connection, with the stores a console run needs.
    struct Fixture {
        _dir: tempfile::TempDir,
        store: ConnectionStore,
        sessions: QuerySessions,
        history: HistoryStore,
        log: StatementLog,
        connection_id: String,
    }

    impl Fixture {
        async fn sqlite() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.db");
            std::fs::File::create(&path).unwrap();
            let connection = Connection {
                id: "sqlite".into(),
                name: "Local file".into(),
                engine: Engine::Sqlite,
                environment: None,
                host: String::new(),
                port: 0,
                database: path.to_string_lossy().into_owned(),
                username: String::new(),
                ssl: Ssl { mode: SslMode::Disable, ..Default::default() },
                ssh: Default::default(),
                srv: false,
                agent_access: Default::default(),
            };
            let fixture = Self::with(dir, connection, None);
            fixture.run("setup", "CREATE TABLE t (id INTEGER)").await.unwrap();
            close_session(&fixture.sessions, &fixture.log, "setup".into()).await.unwrap();
            fixture.log.clear();
            fixture
        }

        /// `connection` saved in a fresh store under `dir`, with its password.
        fn with(dir: tempfile::TempDir, connection: Connection, password: Option<&str>) -> Self {
            let config = dir.path().join("config");
            std::fs::create_dir_all(&config).unwrap();
            // A new config folder keeps passwords in the encrypted file, never the keychain.
            let store =
                ConnectionStore::load(config.join("connections.json"), Credentials::load(config).unwrap()).unwrap();
            if let Some(password) = password {
                store.credentials().set(&connection.id, password).unwrap();
            }
            let connection_id = connection.id.clone();
            store.upsert(connection).unwrap();
            Self {
                history: HistoryStore::load(dir.path().join("history.jsonl")).unwrap(),
                _dir: dir,
                store,
                sessions: QuerySessions::default(),
                log: StatementLog::new(100, Collected::default()),
                connection_id,
            }
        }

        async fn run_in(&self, session: &str, scope: QueryScope, sql: &str) -> Result<QueryOutcome> {
            let (id, connection) = (session.to_string(), self.connection_id.clone());
            run_query(&self.store, &self.sessions, &self.history, &self.log, id, connection, scope, sql.into()).await
        }

        async fn run(&self, session: &str, sql: &str) -> Result<QueryOutcome> {
            self.run_in(session, QueryScope::default(), sql).await
        }

        /// Rows in `t` as another connection sees them: only what was committed.
        async fn committed_rows(&self) -> String {
            let outcome = self.run("observer", "SELECT count(*) FROM t").await.unwrap();
            close_session(&self.sessions, &self.log, "observer".into()).await.unwrap();
            outcome.results[0].rows[0][0].clone().unwrap()
        }

        fn manual(&self, session: &str) {
            set_transaction_mode(&self.sessions, session, TransactionMode::Manual).unwrap();
        }

        fn state(&self, session: &str) -> Option<OpenTransaction> {
            transaction_state(&self.sessions, session)
        }

        /// The statements logged for `session`'s connection, as (origin, text).
        fn logged(&self) -> Vec<(Origin, String)> {
            let entries: Vec<Entry> = self.log.entries();
            // SQL Server logs each batch with the line break that ended it.
            entries.into_iter().map(|entry| (entry.origin, entry.text.trim_end().to_owned())).collect()
        }

        fn console(&self, text: &str) -> (Origin, String) {
            (Origin::Console, text.to_string())
        }
    }

    #[tokio::test]
    async fn manual_opens_a_transaction_on_the_first_run_and_keeps_it_from_other_connections() {
        let fx = Fixture::sqlite().await;
        fx.manual("c1");
        assert_eq!(transaction_mode(&fx.sessions, "c1"), TransactionMode::Manual);
        assert_eq!(fx.state("c1"), None, "nothing opens before a run");

        fx.run("c1", "INSERT INTO t VALUES (1)").await.unwrap();
        let open = fx.state("c1").expect("the first run opened a transaction");
        assert!(!open.aborted);
        // The console sees its own work; another connection doesn't yet.
        let mine = fx.run("c1", "SELECT count(*) FROM t").await.unwrap();
        assert_eq!(mine.results[0].rows[0][0].as_deref(), Some("1"));
        assert_eq!(fx.committed_rows().await, "0");
        // Later runs stay in the same transaction: BEGIN went out once.
        let begins = fx.logged().iter().filter(|(_, text)| text == "BEGIN").count();
        assert_eq!(begins, 1);
        assert_eq!(fx.logged()[0], fx.console("BEGIN"));
    }

    #[tokio::test]
    async fn commit_makes_the_work_visible_and_is_logged_as_a_console_statement() {
        let fx = Fixture::sqlite().await;
        fx.manual("c1");
        fx.run("c1", "INSERT INTO t VALUES (1), (2)").await.unwrap();
        let open = fx.state("c1").unwrap();

        let outcome = commit_transaction(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        assert_eq!(outcome, CommitOutcome { ended: TransactionEnd::Committed, transaction: open });
        assert_eq!(fx.state("c1"), None);
        assert!(fx.logged().contains(&fx.console("COMMIT")), "{:?}", fx.logged());
        assert_eq!(fx.committed_rows().await, "2");

        // The next run opens a new transaction.
        fx.run("c1", "INSERT INTO t VALUES (3)").await.unwrap();
        assert!(fx.state("c1").is_some());
        assert_eq!(fx.logged().iter().filter(|(_, text)| text == "BEGIN").count(), 2);
    }

    #[tokio::test]
    async fn rollback_discards_the_work() {
        let fx = Fixture::sqlite().await;
        fx.manual("c1");
        fx.run("c1", "INSERT INTO t VALUES (1)").await.unwrap();

        let outcome = rollback_transaction(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        assert_eq!(outcome.ended, TransactionEnd::RolledBack);
        assert_eq!(fx.state("c1"), None);
        assert!(fx.logged().contains(&fx.console("ROLLBACK")));
        let mine = fx.run("c1", "SELECT count(*) FROM t").await.unwrap();
        assert_eq!(mine.results[0].rows[0][0].as_deref(), Some("0"));
        assert_eq!(fx.committed_rows().await, "0");
    }

    #[tokio::test]
    async fn commit_or_rollback_without_a_transaction_is_refused() {
        let fx = Fixture::sqlite().await;
        fx.manual("c1");
        let error = commit_transaction(&fx.sessions, &fx.log, "c1".into()).await.unwrap_err();
        assert_eq!(error.to_string(), "No transaction is open on this console.");
        fx.run("c1", "SELECT 1").await.unwrap();
        rollback_transaction(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        let error = rollback_transaction(&fx.sessions, &fx.log, "c1".into()).await.unwrap_err();
        assert_eq!(error.to_string(), "No transaction is open on this console.");
    }

    #[tokio::test]
    async fn switching_to_auto_is_refused_while_a_transaction_is_open() {
        let fx = Fixture::sqlite().await;
        fx.manual("c1");
        fx.run("c1", "INSERT INTO t VALUES (1)").await.unwrap();

        let error = set_transaction_mode(&fx.sessions, "c1", TransactionMode::Auto).unwrap_err();
        assert_eq!(
            error.to_string(),
            "A transaction is open on this console: commit or roll back before switching to Auto-commit."
        );
        assert_eq!(transaction_mode(&fx.sessions, "c1"), TransactionMode::Manual);

        commit_transaction(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        set_transaction_mode(&fx.sessions, "c1", TransactionMode::Auto).unwrap();
        fx.run("c1", "INSERT INTO t VALUES (2)").await.unwrap();
        assert_eq!(fx.state("c1"), None, "Auto opens nothing");
        assert_eq!(fx.committed_rows().await, "2");
    }

    #[tokio::test]
    async fn changing_the_scope_or_the_connection_is_refused_while_a_transaction_is_open() {
        let fx = Fixture::sqlite().await;
        fx.manual("c1");
        fx.run("c1", "INSERT INTO t VALUES (1)").await.unwrap();
        let before = fx.state("c1").unwrap();
        const REFUSED: &str = "A transaction is open on this console: commit or roll back before changing the \
                               database or the connection.";

        let other = QueryScope { database: Some("other".into()), container: None };
        assert_eq!(fx.run_in("c1", other.clone(), "SELECT 1").await.unwrap_err().to_string(), REFUSED);
        // An edited connection would open a new session too.
        let mut edited = fx.store.get(&fx.connection_id).unwrap();
        edited.name = "Renamed".into();
        fx.store.upsert(edited).unwrap();
        assert_eq!(fx.run("c1", "SELECT 1").await.unwrap_err().to_string(), REFUSED);

        // The transaction is untouched, and still ends as asked.
        assert_eq!(fx.state("c1"), Some(before));
        commit_transaction(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        assert_eq!(fx.committed_rows().await, "1");
        // With none open, the change takes effect as before.
        fx.run_in("c1", other, "SELECT 1").await.unwrap();
    }

    #[tokio::test]
    async fn closing_the_console_rolls_back_its_transaction() {
        let fx = Fixture::sqlite().await;
        fx.manual("c1");
        fx.run("c1", "INSERT INTO t VALUES (1)").await.unwrap();

        close_session(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        assert_eq!(fx.state("c1"), None);
        assert_eq!(fx.logged().last(), Some(&fx.console("ROLLBACK")));
        assert_eq!(fx.committed_rows().await, "0");
        // The mode goes with the console.
        assert_eq!(transaction_mode(&fx.sessions, "c1"), TransactionMode::Auto);
    }

    #[tokio::test]
    async fn closing_the_console_while_its_first_manual_run_connects_runs_nothing() {
        let fx = Arc::new(Fixture::sqlite().await);
        fx.manual("c1");
        let running = tokio::spawn({
            let fx = fx.clone();
            async move { fx.run("c1", "INSERT INTO t VALUES (1)").await }
        });
        // The run is now opening the console's connection.
        tokio::task::yield_now().await;
        close_session(&fx.sessions, &fx.log, "c1".into()).await.unwrap();

        let error = running.await.unwrap().unwrap_err();
        assert_eq!(error.to_string(), "This console was closed.");
        assert!(fx.sessions.get("c1").is_none(), "no session is left behind");
        assert_eq!(fx.state("c1"), None);
        assert!(fx.logged().is_empty(), "nothing was sent: {:?}", fx.logged());
        assert_eq!(fx.committed_rows().await, "0");
    }

    #[tokio::test]
    async fn counts_changes_and_rows_and_the_last_activity() {
        let fx = Fixture::sqlite().await;
        fx.manual("c1");
        fx.run("c1", "INSERT INTO t VALUES (1); INSERT INTO t VALUES (2), (3); SELECT * FROM t").await.unwrap();
        let first = fx.state("c1").unwrap();
        assert_eq!((first.changes, first.rows), (2, 3), "the SELECT changed nothing");
        assert!(first.last_activity_ms >= first.started_at_ms);

        tokio::time::sleep(Duration::from_millis(5)).await;
        fx.run("c1", "UPDATE t SET id = id + 10").await.unwrap();
        let second = fx.state("c1").unwrap();
        assert_eq!((second.changes, second.rows), (3, 6));
        assert_eq!(second.started_at_ms, first.started_at_ms);
        assert!(second.last_activity_ms > first.last_activity_ms);

        let listed = open_transactions(&fx.sessions);
        assert_eq!(
            listed,
            vec![ConsoleTransaction {
                session_id: "c1".into(),
                connection_id: "sqlite".into(),
                connection_name: "Local file".into(),
                transaction: second,
            }]
        );
    }

    #[tokio::test]
    async fn a_failed_run_keeps_a_sqlite_transaction_open_and_not_aborted() {
        let fx = Fixture::sqlite().await;
        fx.manual("c1");
        fx.run("c1", "INSERT INTO t VALUES (1)").await.unwrap();
        assert!(fx.run("c1", "INSERT INTO missing VALUES (1)").await.is_err());
        let open = fx.state("c1").expect("SQLite keeps the transaction");
        assert!(!open.aborted);
        commit_transaction(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        assert_eq!(fx.committed_rows().await, "1");
    }

    #[tokio::test]
    async fn manual_is_refused_on_engines_without_console_transactions() {
        for engine in [Engine::Mongodb, Engine::Redis, Engine::Cosmosdb] {
            assert!(!supports_manual_transactions(engine), "{engine:?}");
        }
        for engine in [Engine::Postgres, Engine::Mysql, Engine::Mariadb, Engine::Sqlite, Engine::Sqlserver] {
            assert!(supports_manual_transactions(engine), "{engine:?}");
        }
        // Refused before any connection is made: nothing listens on this port.
        let redis = Connection {
            id: "redis".into(),
            engine: Engine::Redis,
            host: "127.0.0.1".into(),
            port: 1,
            database: String::new(),
            ..crate::drivers::postgres::tests::local()
        };
        let fx = Fixture::with(tempfile::tempdir().unwrap(), redis, None);
        fx.manual("c1");
        let error = fx.run("c1", "PING").await.unwrap_err();
        assert_eq!(error.to_string(), "Redis consoles can only run in Auto-commit.");
        assert!(fx.logged().is_empty());
    }

    #[tokio::test]
    async fn auto_is_unchanged_and_a_typed_transaction_still_works() {
        let fx = Fixture::sqlite().await;
        fx.run("c1", "INSERT INTO t VALUES (1)").await.unwrap();
        assert_eq!(fx.state("c1"), None);
        assert_eq!(fx.committed_rows().await, "1", "each statement commits");

        fx.run("c1", "BEGIN; INSERT INTO t VALUES (2)").await.unwrap();
        assert_eq!(fx.state("c1"), None, "a typed transaction isn't tracked");
        assert_eq!(fx.committed_rows().await, "1");
        fx.run("c1", "COMMIT").await.unwrap();
        assert_eq!(fx.committed_rows().await, "2");
        assert!(!fx.logged().iter().any(|(_, text)| text == "BEGIN"), "Auto sends nothing of its own");
    }

    /// Needs the PostgreSQL container on port 55432 (`drivers::postgres::tests::local`).
    fn postgres() -> Fixture {
        let connection = crate::drivers::postgres::tests::local();
        let password = crate::drivers::postgres::tests::PASSWORD;
        Fixture::with(tempfile::tempdir().unwrap(), connection, password)
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn a_failed_run_undoes_only_itself_and_the_postgres_transaction_carries_on() {
        let fx = postgres();
        fx.manual("c1");
        fx.run("c1", "CREATE TEMP TABLE kept (n int)").await.unwrap();
        fx.run("c1", "INSERT INTO kept VALUES (1)").await.unwrap();
        assert!(fx.run("c1", "INSERT INTO kept VALUES (2); SELECT 1 / 0").await.is_err());
        let open = fx.state("c1").expect("still open");
        assert!(!open.aborted, "the failed run was rolled back to its savepoint");
        // The transaction carries on with what came before the failed run, and nothing of it.
        let outcome = fx.run("c1", "SELECT count(*)::text FROM kept").await.unwrap();
        assert_eq!(outcome.results[0].rows[0][0].as_deref(), Some("1"));
        assert!(fx.logged().iter().any(|(_, text)| text == "ROLLBACK TO SAVEPOINT dbforge_run"));
        commit_transaction(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        assert_eq!(fx.state("c1"), None);
        close_session(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn a_lost_connection_says_the_server_rolled_back_and_never_reconnects_into_it() {
        let fx = postgres();
        fx.manual("c1");
        let pid = fx.run("c1", "SELECT pg_backend_pid()").await.unwrap().results[0].rows[0][0].clone().unwrap();
        fx.run("killer", &format!("SELECT pg_terminate_backend({pid})")).await.unwrap();

        let error = fx.run("c1", "SELECT 1").await.unwrap_err().to_string();
        assert!(
            error.starts_with("The connection was lost, so the server rolled back the open transaction"),
            "{error}"
        );
        assert_eq!(fx.state("c1"), None);
        // The next run opens a new connection and, in Manual, a new transaction.
        fx.run("c1", "SELECT 1").await.unwrap();
        assert!(fx.state("c1").is_some());
        close_session(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        close_session(&fx.sessions, &fx.log, "killer".into()).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs the SQL Server container on port 51433"]
    async fn cancelling_on_sql_server_resets_the_connection_and_ends_the_transaction() {
        use crate::drivers::sqlserver::tests::{PASSWORD, database};
        let connection = database("dbforge_txn", "CREATE TABLE t (id INT)").await;
        let fx = Arc::new(Fixture::with(tempfile::tempdir().unwrap(), connection, PASSWORD));
        fx.manual("c1");
        fx.run("c1", "INSERT INTO t VALUES (1)").await.unwrap();
        assert_eq!(fx.logged()[0], fx.console("BEGIN TRANSACTION"));

        let running = tokio::spawn({
            let fx = fx.clone();
            async move { fx.run("c1", "WAITFOR DELAY '00:00:30'").await }
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        cancel_query(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        let error = running.await.unwrap().unwrap_err().to_string();
        assert!(error.contains("rolled back"), "{error}");
        assert_eq!(fx.state("c1"), None);
        assert_eq!(fx.committed_rows().await, "0");
    }

    #[tokio::test]
    #[ignore = "needs the SQL Server container on port 51433"]
    async fn sql_server_ending_the_transaction_itself_is_noticed_after_the_run() {
        use crate::drivers::sqlserver::tests::{PASSWORD, database};
        let connection = database(
            "dbforge_txn_ended",
            "CREATE TABLE t (id INT)
             GO
             CREATE TRIGGER no_99 ON t AFTER INSERT AS
             IF EXISTS (SELECT 1 FROM inserted WHERE id = 99) ROLLBACK TRANSACTION",
        )
        .await;
        let fx = Fixture::with(tempfile::tempdir().unwrap(), connection, PASSWORD);
        fx.manual("c1");
        fx.run("c1", "INSERT INTO t VALUES (1)").await.unwrap();
        assert!(fx.state("c1").is_some());

        // The trigger's ROLLBACK ends the console's transaction too.
        let error = fx.run("c1", "INSERT INTO t VALUES (99)").await.unwrap_err().to_string();
        assert!(error.ends_with("The server ended the transaction (it was rolled back)."), "{error}");
        assert_eq!(fx.state("c1"), None);
        assert!(fx.logged().contains(&fx.console("SELECT @@TRANCOUNT")), "{:?}", fx.logged());
        assert_eq!(fx.committed_rows().await, "0");

        // XACT_ABORT rolls back on any error.
        fx.run("c1", "SET XACT_ABORT ON; INSERT INTO t VALUES (2)").await.unwrap();
        let error = fx.run("c1", "INSERT INTO t VALUES ('x')").await.unwrap_err().to_string();
        assert!(error.ends_with("The server ended the transaction (it was rolled back)."), "{error}");
        assert_eq!(fx.state("c1"), None);
        assert_eq!(fx.committed_rows().await, "0");
        // The next run opens a new transaction.
        fx.run("c1", "INSERT INTO t VALUES (3)").await.unwrap();
        assert!(fx.state("c1").is_some());
        close_session(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs the MySQL container on port 53306"]
    async fn mysql_turns_autocommit_off_so_an_implicit_commit_never_leaves_later_runs_autocommitting() {
        use crate::drivers::mysql::tests::{PASSWORD, local};
        let fx = Fixture::with(tempfile::tempdir().unwrap(), local(), PASSWORD);
        fx.run("setup", "DROP TABLE IF EXISTS t; CREATE TABLE t (id INT)").await.unwrap();
        close_session(&fx.sessions, &fx.log, "setup".into()).await.unwrap();
        fx.log.clear();
        fx.manual("c1");
        fx.run("c1", "INSERT INTO t VALUES (1)").await.unwrap();
        // After the session's own SELECT CONNECTION_ID(), which cancelling needs.
        let logged = fx.logged();
        let at = |text: &str| logged.iter().position(|entry| *entry == fx.console(text));
        assert!(at("SET autocommit = 0").is_some_and(|set| Some(set) < at("INSERT INTO t VALUES (1)")), "{logged:?}");

        // DDL commits implicitly, past the app's check; what follows stays uncommitted.
        fx.run("c1", "CREATE TABLE IF NOT EXISTS other (id INT)").await.unwrap();
        fx.run("c1", "INSERT INTO t VALUES (2)").await.unwrap();
        rollback_transaction(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
        let logged = fx.logged();
        assert_eq!(&logged[logged.len() - 2..], [fx.console("ROLLBACK"), fx.console("SET autocommit = 1")]);
        assert_eq!(fx.committed_rows().await, "1", "only the row the CREATE committed");

        // Back in Auto, each statement commits again.
        set_transaction_mode(&fx.sessions, "c1", TransactionMode::Auto).unwrap();
        fx.run("c1", "INSERT INTO t VALUES (3)").await.unwrap();
        assert_eq!(fx.committed_rows().await, "2");
        fx.run("c1", "DROP TABLE t; DROP TABLE other").await.unwrap();
        close_session(&fx.sessions, &fx.log, "c1".into()).await.unwrap();
    }
}
