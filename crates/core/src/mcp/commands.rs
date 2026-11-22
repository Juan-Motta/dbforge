//! What the Settings section reads and changes: on or off, the port, the token.

use std::sync::Arc;

use serde::Serialize;

use super::agents::{self, AgentRegistry, AgentSeen};
use super::server::{self, McpServer, Status};
use super::sessions::SessionFile;
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

/// The agents that have used DBForge: the ones that acted in this run, most
/// recently active first, then the ones only known from saved sessions, by name.
pub fn mcp_agents(registry: &AgentRegistry, sessions: &SessionFile) -> Vec<AgentSeen> {
    agents::merge(registry.seen(), sessions.agents())
}

/// The user's answer to an agent's change; false when it had already timed out.
pub fn answer_mcp_approval(approvals: &super::approvals::Approvals, id: String, approved: bool) -> bool {
    approvals.answer(&id, approved)
}

#[cfg(test)]
mod tests {
    use rmcp::model::InitializeRequestParams;
    use rmcp::transport::streamable_http_server::session::{SessionState, SessionStore};

    use super::*;
    use crate::events::NoEvents;
    use crate::mcp::{AgentRegistry, AgentSeen, SessionFile};

    fn session(agent: &str, version: &str) -> SessionState {
        let params: InitializeRequestParams = serde_json::from_value(serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": agent, "version": version }
        }))
        .unwrap();
        SessionState::new(params)
    }

    #[tokio::test]
    async fn lists_this_run_s_agents_then_the_ones_known_from_saved_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = SessionFile::load(dir.path().join("mcp-sessions.json")).unwrap();
        sessions.store("1", &session("opencode", "1")).await.unwrap();
        sessions.store("2", &session("claude-code", "1.0")).await.unwrap();
        sessions.store("3", &session("aider", "")).await.unwrap();
        let registry = AgentRegistry::default();
        registry.record(&NoEvents, "claude-code", Some("2.0"));

        let agents = mcp_agents(&registry, &sessions);
        let names: Vec<&str> = agents.iter().map(|agent| agent.name.as_str()).collect();
        assert_eq!(names, ["claude-code", "aider", "opencode"]);
        assert_eq!(agents[0].version.as_deref(), Some("2.0"));
        assert!(agents[0].last_active_ms.is_some());
        assert_eq!(agents[1], AgentSeen { name: "aider".into(), version: None, last_active_ms: None });
        assert_eq!(agents[2], AgentSeen { name: "opencode".into(), version: Some("1".into()), last_active_ms: None });
    }
}
