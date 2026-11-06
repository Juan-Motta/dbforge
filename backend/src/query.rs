//! Query consoles. Each console (a tab in the UI) owns a session: a connection
//! kept open between runs, so transactions and `SET` carry over like in psql.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::State;

use crate::connections::model::Connection;
use crate::connections::secrets;
use crate::connections::store::ConnectionStore;
use crate::drivers::{self, CancelTarget, QueryOutcome};
use crate::error::{AppError, Result};

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

/// Runs a SQL script in the console's session, opening it on first use.
/// `database` defaults to the connection's own.
#[tauri::command]
pub async fn run_query(
    store: State<'_, ConnectionStore>,
    sessions: State<'_, QuerySessions>,
    session_id: String,
    connection_id: String,
    database: Option<String>,
    sql: String,
) -> Result<QueryOutcome> {
    let connection = store.get(&connection_id)?;
    let session = match sessions.get(&session_id) {
        // Edited connection settings take effect on the next run.
        Some(session) if session.connection == connection && session.database == database => session,
        _ => {
            let session = Arc::new(QuerySession::open(connection, database).await?);
            sessions.insert(session_id.clone(), session.clone());
            session
        }
    };

    let mut driver = session
        .driver
        .try_lock()
        .map_err(|_| AppError::Invalid("A query is already running in this console".into()))?;
    let outcome = driver.run(&sql, ROW_LIMIT).await;
    // A dropped connection can't be reused; the next run opens a new one.
    if outcome.is_err() && !driver.is_alive().await {
        sessions.remove(&session_id, &session);
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
