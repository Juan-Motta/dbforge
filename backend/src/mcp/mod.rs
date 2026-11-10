//! An MCP server inside DBForge, so AI agents (Claude Code, Codex, …) can
//! query and change the saved connections without ever seeing their
//! credentials. It listens on 127.0.0.1 while the app is open and the user
//! has turned it on, behind a bearer token kept in the credential store. Each
//! connection says what agents may do with it (`AgentAccess`); every statement
//! an agent causes lands in the statement log as the agent's.

mod approvals;
pub mod commands;
mod server;
mod settings;
pub mod token;
mod tools;

pub use approvals::Approvals;
pub use server::McpServer;
pub use settings::SettingsFile;
