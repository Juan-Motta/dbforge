//! What the Settings section reads and changes: on or off, the port, the token.

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::server::{self, McpServer, Status};
use super::settings::{McpSettings, SettingsFile};
use super::token;
use crate::connections::store::ConnectionStore;
use crate::error::Result;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpState {
    pub settings: McpSettings,
    pub status: Status,
}

/// Starts or stops the server to match `settings`.
pub async fn apply(app: &AppHandle, settings: McpSettings) -> Status {
    let server = app.state::<McpServer>();
    if !settings.enabled {
        server.stop();
        return server.status();
    }
    match server::token_of(app) {
        Ok(token) => server.start(app.clone(), settings.port, token).await,
        Err(error) => Status::Failed { error: error.to_string() },
    }
}

#[tauri::command]
pub fn mcp_state(settings: State<'_, SettingsFile>, server: State<'_, McpServer>) -> McpState {
    McpState { settings: settings.get(), status: server.status() }
}

/// Saves `settings` and starts, restarts or stops the server to match.
#[tauri::command]
pub async fn set_mcp_settings(app: AppHandle, settings: McpSettings) -> Result<McpState> {
    app.state::<SettingsFile>().set(settings)?;
    let status = apply(&app, settings).await;
    Ok(McpState { settings, status })
}

/// The token agents send, for the user to copy into an agent's settings.
#[tauri::command]
pub fn mcp_token(app: AppHandle) -> Result<String> {
    server::token_of(&app)
}

/// A new token; agents using the old one are refused from now on.
#[tauri::command]
pub async fn regenerate_mcp_token(app: AppHandle) -> Result<String> {
    let token = token::regenerate(app.state::<ConnectionStore>().credentials())?;
    let settings = app.state::<SettingsFile>().get();
    apply(&app, settings).await;
    Ok(token)
}

/// The user's answer to an agent's change; false when it had already timed out.
#[tauri::command]
pub fn answer_mcp_approval(approvals: State<'_, super::approvals::Approvals>, id: String, approved: bool) -> bool {
    approvals.answer(&id, approved)
}
