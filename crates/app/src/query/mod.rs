//! The query console (SQL, mongosh for MongoDB, SQL over a container for
//! Cosmos DB): an editor, the results of its runs or the plan of an
//! explained statement, and the Library (history and saved queries).

mod console;
pub mod plan;
pub mod plan_view;
pub mod results;

pub use console::{CommitTransaction, ConsoleEvent, QueryConsole, Run, SaveQuery, ToggleLibrary, key_bindings};

/// The console's shortcuts as each OS writes them.
pub mod shortcuts {
    const MAC: bool = cfg!(target_os = "macos");
    /// Runs the selection, or the whole script.
    pub const RUN: &str = if MAC { "⌘↵" } else { "Ctrl+Enter" };
    /// Shows or hides the Library: the history and the saved queries.
    pub const LIBRARY: &str = if MAC { "⌘Y" } else { "Ctrl+Y" };
    /// Commits the console's open transaction.
    pub const COMMIT: &str = if MAC { "⇧⌘↵" } else { "Ctrl+Shift+Enter" };
}
