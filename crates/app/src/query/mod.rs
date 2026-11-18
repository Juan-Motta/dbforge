//! The SQL console: an editor, the results of its runs and the history.
//! MongoDB's and Cosmos DB's consoles come with their own features.

mod console;
pub mod results;

pub use console::{ConsoleEvent, QueryConsole, Run, ToggleHistory, key_bindings};

/// The console's shortcuts as each OS writes them.
pub mod shortcuts {
    const MAC: bool = cfg!(target_os = "macos");
    /// Runs the selection, or the whole script.
    pub const RUN: &str = if MAC { "⌘↵" } else { "Ctrl+Enter" };
    pub const HISTORY: &str = if MAC { "⌘Y" } else { "Ctrl+Y" };
}
