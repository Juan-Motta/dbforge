//! An MCP server inside DBForge, so AI agents (Claude Code, Codex, …) can
//! query and change the saved connections without ever seeing their
//! credentials. It listens on 127.0.0.1 while the app is open and the user
//! has turned it on, behind a bearer token kept in the credential store. Each
//! connection says what agents may do with it (`AgentAccess`); every statement
//! an agent causes lands in the statement log as the agent's.

mod agents;
mod approvals;
pub mod bridge;
pub mod commands;
mod connections;
mod dumps;
mod guide;
mod saved;
mod server;
mod sessions;
mod settings;
pub mod token;
mod tools;

pub use agents::{AgentRegistry, AgentSeen};
pub use approvals::{
    Answer, ApprovalRequest, ApprovalSecrets, ApprovalSubject, Approvals, ConnectionChange, ConnectionField,
    DraftScope, DumpAction, DumpApproval, SavedQueryChange, SavedQueryDraft, SavedQueryField, SecretFields,
};
pub use server::{McpServer, Status};
pub use sessions::SessionFile;
pub use settings::{McpSettings, SettingsFile};
