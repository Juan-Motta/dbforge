//! Query consoles. Each console (a tab in the UI) owns a session: a connection
//! kept open between runs, so transactions and `SET` carry over like in psql.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

use crate::connections::model::Connection;
use crate::connections::store::ConnectionStore;
use crate::drivers::{self, CancelTarget, QueryOutcome};
use crate::error::{AppError, Result};
use crate::history::{HistoryEntry, HistoryStore};
use crate::statement_log::{self, Origin, StatementLog};

/// Rows kept per result; the rest are read and dropped.
const ROW_LIMIT: usize = 500;

/// Where a console's queries run: a database (the connection's own when
/// `None`) and, for Cosmos DB, the container picked in the console.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct QueryScope {
    pub database: Option<String>,
    pub container: Option<String>,
}

/// Open sessions by console id.
#[derive(Default)]
pub struct QuerySessions(Mutex<HashMap<String, Arc<QuerySession>>>);

struct QuerySession {
    /// The connection's settings when the session opened.
    connection: Connection,
    /// Its password then, to cancel with.
    password: Option<String>,
    scope: QueryScope,
    cancel_target: CancelTarget,
    /// Locked while a query runs; one query at a time per session.
    driver: tokio::sync::Mutex<drivers::Session>,
}

impl QuerySessions {
    fn get(&self, id: &str) -> Option<Arc<QuerySession>> {
        self.0.lock().unwrap().get(id).cloned()
    }

    fn insert(&self, id: String, session: Arc<QuerySession>) {
        self.0.lock().unwrap().insert(id, session);
    }

    fn take(&self, id: &str) -> Option<Arc<QuerySession>> {
        self.0.lock().unwrap().remove(id)
    }

    /// Removes `id` only while it still maps to `session`, so a newer one survives.
    fn remove(&self, id: &str, session: &Arc<QuerySession>) {
        let mut sessions = self.0.lock().unwrap();
        if sessions.get(id).is_some_and(|current| Arc::ptr_eq(current, session)) {
            sessions.remove(id);
        }
    }
}

impl QuerySession {
    async fn open(connection: Connection, password: Option<String>, scope: QueryScope) -> Result<Self> {
        let (database, container) = (scope.database.as_deref(), scope.container.as_deref());
        let driver = drivers::open_session(&connection, password.as_deref(), database, container).await?;
        Ok(Self { cancel_target: driver.cancel_target(), connection, password, scope, driver: driver.into() })
    }

    fn is_running(&self) -> bool {
        self.driver.try_lock().is_err()
    }

    async fn cancel(&self) -> Result<()> {
        let database = self.scope.database.as_deref();
        drivers::cancel(&self.connection, self.password.as_deref(), database, self.cancel_target.clone()).await
    }
}

/// Runs a SQL script in the console's session, opening it on first use, and
/// records the run in the history.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn run_query(
    store: State<'_, ConnectionStore>,
    sessions: State<'_, QuerySessions>,
    history: State<'_, HistoryStore>,
    log: State<'_, StatementLog>,
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
    let run = run_in_session(&sessions, &session_id, connection.clone(), password, scope.clone(), &sql);
    let outcome = statement_log::scoped(&log, &connection.id, Origin::Console, run).await;

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
        error: outcome.as_ref().err().map(ToString::to_string),
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
    let session = match sessions.get(session_id) {
        // Edited connection settings, or another database or container, take effect on the next run.
        Some(session) if session.connection == connection && session.scope == scope => session,
        _ => {
            let session = Arc::new(QuerySession::open(connection, password?, scope).await?);
            sessions.insert(session_id.to_string(), session.clone());
            session
        }
    };

    let mut driver = session
        .driver
        .try_lock()
        .map_err(|_| AppError::Invalid("A query is already running in this console".into()))?;
    let outcome = driver.run(sql, ROW_LIMIT).await;
    // A dropped connection can't be reused; the next run opens a new one.
    if outcome.is_err() && !driver.is_alive().await {
        sessions.remove(session_id, &session);
    }
    outcome
}

/// The plan of the console's statement. See `drivers::explain` for why ANALYZE is safe.
#[tauri::command]
pub async fn explain_query(
    store: State<'_, ConnectionStore>,
    log: State<'_, StatementLog>,
    connection_id: String,
    scope: QueryScope,
    sql: String,
    analyze: bool,
) -> Result<serde_json::Value> {
    let (database, container) = (scope.database.as_deref(), scope.container.as_deref());
    store
        .open(&connection_id)?
        .run(&log, Origin::Console, async |connection, password| {
            drivers::explain(connection, password, database, container, &sql, analyze).await
        })
        .await
}

/// Stops the query running in the console, if any.
#[tauri::command]
pub async fn cancel_query(
    sessions: State<'_, QuerySessions>,
    log: State<'_, StatementLog>,
    session_id: String,
) -> Result<()> {
    match sessions.get(&session_id) {
        Some(session) if session.is_running() => {
            statement_log::scoped(&log, &session.connection.id, Origin::Console, session.cancel()).await
        }
        _ => Ok(()),
    }
}

/// Closes the console's session, cancelling its query first if one is running.
/// Uncommitted work is rolled back by the server.
#[tauri::command]
pub async fn close_session(
    sessions: State<'_, QuerySessions>,
    log: State<'_, StatementLog>,
    session_id: String,
) -> Result<()> {
    match sessions.take(&session_id) {
        Some(session) if session.is_running() => {
            statement_log::scoped(&log, &session.connection.id, Origin::Console, session.cancel()).await
        }
        // Dropping the last reference closes the connection.
        _ => Ok(()),
    }
}
