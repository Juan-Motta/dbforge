//! The query console (SQL, mongosh for MongoDB, SQL over a container for
//! Cosmos DB): an editor, the results of its runs or the plan of an
//! explained statement, and the history.

mod console;
pub mod plan;
pub mod plan_view;
pub mod results;

pub use console::{ConsoleEvent, QueryConsole, Run, ToggleHistory, key_bindings};

/// The console's shortcuts as each OS writes them.
pub mod shortcuts {
    const MAC: bool = cfg!(target_os = "macos");
    /// Runs the selection, or the whole script.
    pub const RUN: &str = if MAC { "⌘↵" } else { "Ctrl+Enter" };
    pub const HISTORY: &str = if MAC { "⌘Y" } else { "Ctrl+Y" };
}
