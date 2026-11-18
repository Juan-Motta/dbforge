//! What the Settings section reads and changes: on or off, the port, the token.

use std::sync::Arc;

use serde::Serialize;

use super::server::{self, McpServer, Status};
use super::settings::{McpSettings, SettingsFile};
use super::token;
use crate::Core;
use crate::connections::store::ConnectionStore;
use crate::error::Result;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpState {
    pub settings: McpSettings,
    pub status: Status,
    /// The executable agents start to reach DBForge over stdio; `None` when it can't be found.
    pub executable: Option<String>,
}

impl McpState {
    fn new(settings: McpSettings, status: Status) -> Self {
        Self { settings, status, executable: super::bridge::executable() }
    }
}

/// Starts or stops the server to match `settings`.
pub async fn apply(core: &Arc<Core>, settings: McpSettings) -> Status {
    let server = core.mcp_server();
    if !settings.enabled {
        server.stop().await;
        return server.status();
    }
    match server::token_of(core.connections()) {
        Ok(token) => server.start(core.clone(), settings.port, token).await,
        Err(error) => Status::Failed { error: error.to_string() },
    }
}

pub fn mcp_state(settings: &SettingsFile, server: &McpServer) -> McpState {
    McpState::new(settings.get(), server.status())
}

/// Saves `settings` and starts, restarts or stops the server to match.
pub async fn set_mcp_settings(core: &Arc<Core>, settings: McpSettings) -> Result<McpState> {
    core.mcp_settings().set(settings)?;
    let status = apply(core, settings).await;
    Ok(McpState::new(settings, status))
}

/// The token agents send, for the user to copy into an agent's settings.
pub fn mcp_token(store: &ConnectionStore) -> Result<String> {
    server::token_of(store)
}

/// A new token; agents using the old one are refused from now on.
pub async fn regenerate_mcp_token(store: &ConnectionStore, server: &McpServer) -> Result<String> {
    let token = token::regenerate(store.credentials())?;
    // The running server takes the new token at once; the old one stops working.
    server.use_token(token.clone());
    Ok(token)
}

/// The user's answer to an agent's change; false when it had already timed out.
pub fn answer_mcp_approval(approvals: &super::approvals::Approvals, id: String, approved: bool) -> bool {
    approvals.answer(&id, approved)
}
