//! Every statement DBForge sends to a database: its text as the user could
//! run it again, where it ran, what started it, how long it took, and what it
//! returned or why it failed.
//!
//! A command sets the context once with [`scoped`] (which connection, and
//! what the statement is for); the drivers record each statement they send
//! with a [`Recording`], without passing that context down. Outside a scope,
//! as in the drivers' own tests, nothing is recorded.

pub mod commands;
mod context;
pub(crate) mod store;

pub use commands::EventSink;
pub use context::{Recording, scoped, shown_as};
pub use store::{Sink, StatementLog};

use serde::Serialize;

/// What a statement was sent for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    /// Typed by the user in a query console.
    Console,
    /// Reading rows or keys to show them.
    Browse,
    /// Saving changes made in a table or key.
    Edit,
    /// Reading what the database holds: schemas, tables, columns, versions.
    Metadata,
    /// Creating a database, role or user.
    Admin,
    /// Writing rows to a file.
    Export,
    /// Reading rows from a file into a table.
    Import,
    /// Preparing or cleaning up after pg_dump and psql.
    Dump,
    /// Testing a connection.
    Connect,
    /// Sent by an AI agent through the MCP server.
    Agent,
}

/// One statement as the console shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Increases with every statement, so entries sort and key by it.
    pub id: u64,
    /// When it started, in milliseconds since the Unix epoch.
    pub at: u64,
    pub connection_id: String,
    pub database: Option<String>,
    pub origin: Origin,
    /// The statement in the engine's own language: SQL, a mongosh call, a redis-cli line.
    pub text: String,
    pub elapsed_ms: u64,
    /// Rows returned, or affected when it returned none; `None` when the engine doesn't say.
    pub rows: Option<u64>,
    pub error: Option<String>,
}
