use std::sync::Arc;

use super::{Entry, Sink, StatementLog};
use crate::events::{Event, Events};

/// Sends each new entry to the UI as it is recorded.
pub struct EventSink(pub Arc<dyn Events>);

impl Sink for EventSink {
    fn logged(&self, entry: &Entry) {
        // A window that isn't listening yet reads the entries with `list_statements`.
        self.0.emit(Event::StatementLogged(entry.clone()));
    }
}

/// The statements recorded so far, oldest first.
pub fn list_statements(log: &StatementLog) -> Vec<Entry> {
    log.entries()
}

/// Forgets the statements recorded so far.
pub fn clear_statements(log: &StatementLog) {
    log.clear();
}
