//! The command palette (⌘K / Ctrl+K), as the former web UI's `CommandPalette.tsx`:
//! jump to a table or view of the selected connection, or run a command.
//! [`search`] ranks what is typed; [`commands`] lists what can be run.

pub mod commands;
pub mod search;
mod view;

pub use commands::{Command, CommandAction};
pub use view::{CommandPalette, PaletteEvent, PaletteSource, key_bindings};
