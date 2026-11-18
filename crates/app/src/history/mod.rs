//! The query history: every run from a console, newest first, beside the
//! console that asks for it.

pub mod format;
mod panel;

pub use panel::{HistoryEvent, HistoryPanel, WIDTH};
