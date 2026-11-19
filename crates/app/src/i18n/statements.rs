//! The texts of the statement console: the statements DBForge runs.

/// The statement console's texts in one language.
pub struct StatementMessages {
    pub title: &'static str,
    pub count: fn(usize) -> String,
    pub all: &'static str,
    pub console: &'static str,
    pub agents: &'static str,
    pub changes: &'static str,
    pub reads: &'static str,
    pub metadata: &'static str,
    pub copy: &'static str,
    pub copy_all: &'static str,
    pub clear: &'static str,
    /// Its close button, with the shortcut that toggles it.
    pub close: fn(&str) -> String,
    pub resize: &'static str,
    pub empty: &'static str,
    pub empty_filtered: &'static str,
    pub rows: fn(u64) -> String,
    pub unknown_connection: &'static str,
    pub origin_console: &'static str,
    pub origin_browse: &'static str,
    pub origin_edit: &'static str,
    pub origin_metadata: &'static str,
    pub origin_admin: &'static str,
    pub origin_export: &'static str,
    pub origin_import: &'static str,
    pub origin_dump: &'static str,
    pub origin_connect: &'static str,
    pub origin_agent: &'static str,
}
