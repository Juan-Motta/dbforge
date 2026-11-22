//! The agents' MCP sessions, kept in `mcp-sessions.json` so they outlive the
//! app: an agent connected before DBForge restarts (an update, a quit and
//! reopen) carries on with the same session instead of being refused. Only
//! what the agent sent when it connected is kept (its name, version and
//! capabilities); the token that let it in is never written here.

use std::path::PathBuf;

use rmcp::transport::streamable_http_server::session::{SessionState, SessionStore, SessionStoreError};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::files::JsonFile;

/// Sessions kept at most. Agents rarely say goodbye, so the oldest go first.
const MAX_SESSIONS: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
struct Saved {
    id: String,
    state: SessionState,
}

/// The saved sessions, oldest first.
pub struct SessionFile {
    sessions: JsonFile<Vec<Saved>>,
}

impl SessionFile {
    /// The sessions saved at `file`; none until an agent connects.
    pub fn load(file: PathBuf) -> Result<Self> {
        Ok(Self { sessions: JsonFile::load(file, Vec::new)? })
    }

    /// The agent of each saved session (its own name and version, as it sent
    /// them when it connected), oldest session first.
    pub fn agents(&self) -> Vec<(String, Option<String>)> {
        self.sessions.read(|sessions| {
            sessions
                .iter()
                .map(|saved| {
                    let client = &saved.state.initialize_params.client_info;
                    let version = Some(client.version.clone()).filter(|version| !version.is_empty());
                    (client.name.clone(), version)
                })
                .collect()
        })
    }

    /// Where the file was moved when it couldn't be read at start.
    pub fn set_aside(&self) -> Option<&std::path::Path> {
        self.sessions.set_aside()
    }
}

#[async_trait::async_trait]
impl SessionStore for SessionFile {
    async fn load(&self, session_id: &str) -> std::result::Result<Option<SessionState>, SessionStoreError> {
        Ok(self.sessions.read(|sessions| sessions.iter().find(|s| s.id == session_id).map(|s| s.state.clone())))
    }

    async fn store(&self, session_id: &str, state: &SessionState) -> std::result::Result<(), SessionStoreError> {
        self.sessions.update(|sessions| {
            sessions.retain(|s| s.id != session_id);
            sessions.push(Saved { id: session_id.to_owned(), state: state.clone() });
            let excess = sessions.len().saturating_sub(MAX_SESSIONS);
            sessions.drain(..excess);
            Ok(())
        })?;
        Ok(())
    }

    async fn delete(&self, session_id: &str) -> std::result::Result<(), SessionStoreError> {
        if self.sessions.read(|sessions| sessions.iter().all(|s| s.id != session_id)) {
            return Ok(());
        }
        self.sessions.update(|sessions| {
            sessions.retain(|s| s.id != session_id);
            Ok(())
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rmcp::model::InitializeRequestParams;

    use super::*;

    fn state(agent: &str) -> SessionState {
        let params: InitializeRequestParams = serde_json::from_value(serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": agent, "version": "1" }
        }))
        .unwrap();
        SessionState::new(params)
    }

    fn agent_of(state: Option<SessionState>) -> Option<String> {
        state.map(|s| s.initialize_params.client_info.name)
    }

    #[tokio::test]
    async fn sessions_outlive_the_file_being_reopened_until_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp-sessions.json");
        let file = SessionFile::load(path.clone()).unwrap();
        file.store("a", &state("claude-code")).await.unwrap();
        file.store("b", &state("codex")).await.unwrap();
        file.delete("b").await.unwrap();

        let reopened = SessionFile::load(path).unwrap();
        assert_eq!(agent_of(reopened.load("a").await.unwrap()).as_deref(), Some("claude-code"));
        assert_eq!(agent_of(reopened.load("b").await.unwrap()), None);
        assert!(!std::fs::read_to_string(dir.path().join("mcp-sessions.json")).unwrap().contains("Bearer"));
    }

    #[tokio::test]
    async fn lists_the_agents_of_the_saved_sessions_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let file = SessionFile::load(dir.path().join("mcp-sessions.json")).unwrap();
        assert!(file.agents().is_empty());
        file.store("a", &state("claude-code")).await.unwrap();
        file.store("b", &state("codex")).await.unwrap();
        file.store("a", &state("claude-code")).await.unwrap();
        assert_eq!(
            file.agents(),
            [("codex".to_owned(), Some("1".to_owned())), ("claude-code".to_owned(), Some("1".to_owned()))]
        );
    }

    #[tokio::test]
    async fn keeps_only_the_newest_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let file = SessionFile::load(dir.path().join("mcp-sessions.json")).unwrap();
        for n in 0..=MAX_SESSIONS {
            file.store(&n.to_string(), &state("agent")).await.unwrap();
        }
        assert!(file.load("0").await.unwrap().is_none(), "the oldest goes");
        assert!(file.load("1").await.unwrap().is_some());
        assert!(file.load(&MAX_SESSIONS.to_string()).await.unwrap().is_some());
    }
}
