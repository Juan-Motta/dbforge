//! Saved connections: the dialog that creates and edits one, the one that
//! deletes it, and the rules behind them (the form's values, engines, URLs).

mod delete;
mod dialog;
pub mod engine;
pub mod form;
mod general;
mod ssh;
mod ssl;
pub mod url;

use gpui::KeyBinding;

pub use delete::{DeleteConnectionDialog, DeleteConnectionEvent};
pub use dialog::{ConnectionDialog, ConnectionDialogEvent};

/// The keys the connection dialogs answer.
pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = dialog::key_bindings();
    bindings.extend(delete::key_bindings());
    bindings
}
