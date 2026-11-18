//! What Settings › Backup calls.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use std::sync::Arc;

use serde::Serialize;

use super::{Imported, Parts, Preview, Stores};
use crate::error::Result;
use crate::files::{Access, write_atomic};
use crate::{Core, VERSION};

fn with_stores<T>(core: &Core, work: impl FnOnce(&Stores<'_>) -> Result<T>) -> Result<T> {
    work(&Stores {
        connections: core.connections(),
        tags: core.tags(),
        mcp: core.mcp_settings(),
        tools: core.tool_paths(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub connections: usize,
    pub passwords: usize,
}

/// Writes the configuration to `path`; `secrets` puts passwords and the MCP
/// token in, as plain text. The file is readable by its owner only.
pub fn export_backup(
    core: &Core,
    path: PathBuf,
    preferences: BTreeMap<String, String>,
    secrets: bool,
) -> Result<Exported> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let backup = with_stores(core, |stores| super::collect(stores, preferences, secrets, now, VERSION))?;
    write_atomic(&path, serde_json::to_string_pretty(&backup)?.as_bytes(), Access::Private)?;
    Ok(Exported {
        connections: backup.connections.len(),
        passwords: backup.connections.iter().filter(|c| c.password.is_some()).count(),
    })
}

/// What the backup at `path` holds, and what importing it would do here.
pub fn read_backup(core: &Core, path: PathBuf) -> Result<Preview> {
    let backup = super::read(&fs::read_to_string(path)?)?;
    with_stores(core, |stores| Ok(super::preview(stores, &backup)))
}

/// Applies `parts` of the backup at `path`, then starts or stops the MCP
/// server to match its imported settings. Preferences are the window's to apply.
pub async fn import_backup(core: &Arc<Core>, path: PathBuf, parts: Parts) -> Result<Imported> {
    let backup = super::read(&fs::read_to_string(path)?)?;
    let imported = with_stores(core, |stores| super::apply(stores, backup, parts))?;
    if let Some(mcp) = &imported.mcp {
        if let Some(token) = &mcp.token {
            core.mcp_server().use_token(token.clone());
        }
        crate::mcp::commands::apply(core, mcp.settings).await;
    }
    Ok(imported)
}
