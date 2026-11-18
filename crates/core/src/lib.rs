//! DBForge's logic without a UI: the saved connections and their
//! credentials, the database drivers and SSH tunnels, queries, edits,
//! exports, imports, dumps and backups, the history, the statement log and
//! the MCP server agents use. The app holds one [`Core`] and calls the
//! modules' functions; what happens meanwhile reaches it through [`Events`].

pub mod admin;
pub mod backup;
pub mod connections;
mod core;
pub mod credentials;
pub mod drivers;
pub mod dump;
pub mod edit;
pub mod error;
pub mod events;
pub mod explorer;
pub mod export;
pub mod files;
pub mod history;
pub mod import;
pub mod jobs;
pub mod keyspace;
pub mod mcp;
pub mod query;
pub mod statement_log;
pub mod tunnel;

pub use crate::core::{Core, Dirs, IDENTIFIER};
pub use crate::error::{AppError, Result};
pub use crate::events::{Event, Events};

/// DBForge's version: what a backup records and the MCP server reports.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The argument that starts DBForge as an agent's MCP server over stdio.
pub const MCP_STDIO_FLAG: &str = mcp::bridge::FLAG;

/// Serves MCP over stdio for an agent, handing calls to the running app; the exit code.
pub fn run_mcp_stdio() -> i32 {
    mcp::bridge::run()
}
