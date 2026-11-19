//! Agents (MCP), as the former web UI's `features/mcp`: Settings › Agents turns
//! the core's MCP server on or off, sets its port, shows, copies and
//! regenerates the token agents send, and writes the steps that add DBForge
//! to an agent ([`McpSection`]); and the dialog asking the user to approve
//! a change an agent wants to make on a connection whose tag asks first
//! ([`Approvals`], [`ApprovalDialog`]). The core owns the server, the token
//! and the wait for an answer; nothing here answers for the user.

mod agent_setup;
mod approvals;
mod dialog;
mod queue;
mod section;
pub mod setup;

pub use approvals::Approvals;
pub use dialog::{ApprovalDialog, key_bindings};
pub use section::McpSection;
