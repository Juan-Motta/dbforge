//! A table's or view's tab: its rows a page at a time, narrowed and ordered
//! by the filter bar, edited in place with the changes pending until they
//! are applied in one transaction (or discarded), a cell inspector beside
//! them, and foreign keys that open the row they point to. MongoDB's and
//! Cosmos DB's rows also show as whole documents, edited one at a time.

pub mod cell_actions;
mod cell_menu;
pub mod cell_text;
pub mod choices;
#[cfg(debug_assertions)]
mod development;
pub mod dialogs;
pub mod documents;
pub mod editors;
pub mod edits;
pub mod filter;
pub mod filter_bar;
pub mod inspector;
pub mod links;
pub mod markdown;
pub mod page;
pub mod temporal;
#[cfg(test)]
mod tests;
pub mod view;

use gpui::{KeyBinding, actions};

pub use page::{TablePage, TablePageEvent, TableTabView};
pub use view::{TableEvent, TableView};

actions!(table, [Apply, Inspect, Paste, Deselect]);

/// ⌘S (Ctrl+S) applies the pending changes anywhere in the table's tab;
/// in the grid ⇧↵ shows the picked cell in the inspector, ⌘V (Ctrl+V)
/// pastes into it and Escape lets go of the selection.
pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("secondary-s", Apply, Some("TableView")),
        KeyBinding::new("shift-enter", Inspect, Some("DataGrid")),
        KeyBinding::new("secondary-v", Paste, Some("DataGrid")),
        KeyBinding::new("escape", Deselect, Some("DataGrid")),
    ];
    bindings.extend(dialogs::key_bindings());
    bindings.extend(documents::key_bindings());
    bindings
}

/// The table's shortcuts as each OS writes them.
pub mod shortcuts {
    const MAC: bool = cfg!(target_os = "macos");
    /// Applies the pending changes.
    pub const APPLY: &str = if MAC { "⌘S" } else { "Ctrl+S" };
    /// Keeps the document being edited.
    pub const KEEP_DOCUMENT: &str = if MAC { "⌘↵" } else { "Ctrl+Enter" };
}
