//! What Settings › Backup calls.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use super::{Imported, Parts, Preview, Stores};
use crate::connections::store::ConnectionStore;
use crate::connections::tags::TagStore;
use crate::dump::ToolPaths;
use crate::error::Result;
use crate::files::{Access, write_atomic};
use crate::mcp::{McpServer, SettingsFile};

fn with_stores<T>(app: &AppHandle, work: impl FnOnce(&Stores<'_>) -> Result<T>) -> Result<T> {
    let (connections, tags) = (app.state::<ConnectionStore>(), app.state::<TagStore>());
    let (mcp, tools) = (app.state::<SettingsFile>(), app.state::<ToolPaths>());
    work(&Stores { connections: &connections, tags: &tags, mcp: &mcp, tools: &tools })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub connections: usize,
    pub passwords: usize,
}

/// Writes the configuration to `path`; `secrets` puts passwords and the MCP
/// token in, as plain text. The file is readable by its owner only.
#[tauri::command]
pub fn export_backup(
    app: AppHandle,
    path: PathBuf,
    preferences: BTreeMap<String, String>,
    secrets: bool,
) -> Result<Exported> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let version = app.package_info().version.to_string();
    let backup = with_stores(&app, |stores| super::collect(stores, preferences, secrets, now, &version))?;
    write_atomic(&path, serde_json::to_string_pretty(&backup)?.as_bytes(), Access::Private)?;
    Ok(Exported {
        connections: backup.connections.len(),
        passwords: backup.connections.iter().filter(|c| c.password.is_some()).count(),
    })
}

/// What the backup at `path` holds, and what importing it would do here.
#[tauri::command]
pub fn read_backup(app: AppHandle, path: PathBuf) -> Result<Preview> {
    let backup = super::read(&fs::read_to_string(path)?)?;
    with_stores(&app, |stores| Ok(super::preview(stores, &backup)))
}

/// Applies `parts` of the backup at `path`, then starts or stops the MCP
/// server to match its imported settings. Preferences are the window's to apply.
#[tauri::command]
pub async fn import_backup(app: AppHandle, path: PathBuf, parts: Parts) -> Result<Imported> {
    let backup = super::read(&fs::read_to_string(path)?)?;
    let imported = with_stores(&app, |stores| super::apply(stores, backup, parts))?;
    if let Some(mcp) = &imported.mcp {
        if let Some(token) = &mcp.token {
            app.state::<McpServer>().use_token(token.clone());
        }
        crate::mcp::commands::apply(&app, mcp.settings).await;
    }
    Ok(imported)
}
