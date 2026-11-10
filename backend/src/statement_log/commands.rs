use tauri::{AppHandle, Emitter, State};

use super::{Entry, Sink, StatementLog};

/// The event each new entry is sent as, to the app's window.
const LOGGED: &str = "statement-logged";

/// Sends each new entry to the app's window as it is recorded.
pub struct EventSink(pub AppHandle);

impl Sink for EventSink {
    fn logged(&self, entry: &Entry) {
        // A window that isn't listening yet reads the entries with `list_statements`.
        let _ = self.0.emit(LOGGED, entry);
    }
}

/// The statements recorded so far, oldest first.
#[tauri::command]
pub fn list_statements(log: State<'_, StatementLog>) -> Vec<Entry> {
    log.entries()
}

/// Forgets the statements recorded so far.
#[tauri::command]
pub fn clear_statements(log: State<'_, StatementLog>) {
    log.clear();
}
