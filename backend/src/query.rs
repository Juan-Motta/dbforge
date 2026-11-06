//! Query consoles. Each console (a tab in the UI) owns a session: a connection
//! kept open between runs, so transactions and `SET` carry over like in psql.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tauri::State;
use uuid::Uuid;

use crate::connections::model::Connection;
use crate::connections::secrets;
use crate::connections::store::ConnectionStore;
use crate::drivers::{self, CancelTarget, QueryOutcome};
use crate::error::{AppError, Result};
use crate::history::{HistoryEntry, HistoryStore};

/// Rows kept per result; the rest are read and dropped.
const ROW_LIMIT: usize = 500;

/// Open sessions by console id.
#[derive(Default)]
pub struct QuerySessions(Mutex<HashMap<String, Arc<QuerySession>>>);

struct QuerySession {
    /// The connection's settings when the session opened.
    connection: Connection,
    database: Option<String>,
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
    async fn open(connection: Connection, database: Option<String>) -> Result<Self> {
        let password = secrets::get_password(&connection.id)?;
        let driver = drivers::open_session(&connection, password.as_deref(), database.as_deref()).await?;
        Ok(Self { cancel_target: driver.cancel_target(), connection, database, driver: driver.into() })
    }

    fn is_running(&self) -> bool {
        self.driver.try_lock().is_err()
    }

    async fn cancel(&self) -> Result<()> {
        let password = secrets::get_password(&self.connection.id)?;
        drivers::cancel(&self.connection, password.as_deref(), self.database.as_deref(), self.cancel_target).await
    }
}

/// Runs a SQL script in the console's session, opening it on first use, and
/// records the run in the history. `database` defaults to the connection's own.
#[tauri::command]
pub async fn run_query(
    store: State<'_, ConnectionStore>,
    sessions: State<'_, QuerySessions>,
    history: State<'_, HistoryStore>,
    session_id: String,
    connection_id: String,
    database: Option<String>,
    sql: String,
) -> Result<QueryOutcome> {
    let connection = store.get(&connection_id)?;
    let executed_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let started = Instant::now();
    let outcome = run_in_session(&sessions, &session_id, connection.clone(), database.clone(), &sql).await;

    let entry = HistoryEntry {
        id: Uuid::new_v4().to_string(),
        executed_at,
        connection_id: connection.id,
        connection_name: connection.name,
        database,
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
    database: Option<String>,
    sql: &str,
) -> Result<QueryOutcome> {
    let session = match sessions.get(session_id) {
        // Edited connection settings take effect on the next run.
        Some(session) if session.connection == connection && session.database == database => session,
        _ => {
            let session = Arc::new(QuerySession::open(connection, database).await?);
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

/// Stops the query running in the console, if any.
#[tauri::command]
pub async fn cancel_query(sessions: State<'_, QuerySessions>, session_id: String) -> Result<()> {
    match sessions.get(&session_id) {
        Some(session) if session.is_running() => session.cancel().await,
        _ => Ok(()),
    }
}

/// Closes the console's session, cancelling its query first if one is running.
/// Uncommitted work is rolled back by the server.
#[tauri::command]
pub async fn close_session(sessions: State<'_, QuerySessions>, session_id: String) -> Result<()> {
    match sessions.take(&session_id) {
        Some(session) if session.is_running() => session.cancel().await,
        // Dropping the last reference closes the connection.
        _ => Ok(()),
    }
}
