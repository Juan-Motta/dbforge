//! Saved queries in the window: the Library beside a console (its History
//! and its Saved queries), saving a console (⌘S), running a saved query
//! with `:parameters`, and the rules they follow. The queries themselves
//! live in the core ([`dbforge_core::saved_queries`]); [`store`] keeps the
//! window's copy of the list.

mod dialogs;
mod library;
mod list;
pub mod rules;
mod run_dialog;
mod save_form;
pub mod store;

use gpui::KeyBinding;

pub use library::{LibraryEvent, LibraryPanel, LibraryView, WIDTH};
pub use list::{SavedEvent, SavedPanel};
pub use run_dialog::{RunDialog, RunDialogEvent, RunRequest};
pub use save_form::{Draft, SaveForm, SaveFormEvent};
pub use store::SavedStore;

/// The shortcuts of the saved queries' dialogs and form.
pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = dialogs::key_bindings();
    bindings.extend(run_dialog::key_bindings());
    bindings.extend(save_form::key_bindings());
    bindings
}

#[cfg(test)]
mod tests;
