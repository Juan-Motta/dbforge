//! DBForge's configuration as one JSON file, to move it to another
//! installation or restore it: preferences, connections (with their passwords
//! when the user includes them), their tags, the MCP server's settings and
//! token, and the chosen pg_dump and psql. Secrets are in plain text: the user
//! chose that, and the file is written readable by its owner only.
//!
//! Importing merges, never deletes: connections and tags with an id the
//! installation already has are updated, others are added; a tag named like
//! one already here becomes that one.

pub mod commands;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::connections::model::Connection;
use crate::connections::store::ConnectionStore;
use crate::connections::tags::{Tag, TagStore};
use crate::dump::ToolPaths;
use crate::error::{AppError, Result};
use crate::mcp::{McpSettings, SettingsFile};

/// What the file says it is, so another JSON file is refused clearly.
const FORMAT: &str = "dbforge-backup";
/// Files from a newer DBForge may hold what this one can't read. Version 2
/// added SSL modes, SSH tunnels and their secrets; version 1 files still load.
const VERSION: u32 = 2;

/// The file's contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub format: String,
    pub version: u32,
    /// Milliseconds since the Unix epoch.
    pub exported_at: u64,
    pub app_version: String,
    /// The window's own preferences (theme, language, export folder), by key; the app reads them.
    #[serde(default)]
    pub preferences: BTreeMap<String, String>,
    #[serde(default)]
    pub connections: Vec<BackedUpConnection>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    pub mcp: Option<BackedUpMcp>,
    /// Chosen in place of the tools DBForge finds, by tool name.
    #[serde(default)]
    pub tool_paths: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackedUpConnection {
    #[serde(flatten)]
    pub connection: Connection,
    /// Left out when the export left secrets out, or the connection has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// The SSH password or key passphrase, left out as `password` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackedUpMcp {
    pub settings: McpSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// The stores a backup is made from and restored into.
pub struct Stores<'a> {
    pub connections: &'a ConnectionStore,
    pub tags: &'a TagStore,
    pub mcp: &'a SettingsFile,
    pub tools: &'a ToolPaths,
}

/// Everything there is to back up, secrets included only when `secrets`.
pub fn collect(
    stores: &Stores<'_>,
    preferences: BTreeMap<String, String>,
    secrets: bool,
    now: u64,
    app_version: &str,
) -> Result<Backup> {
    let secret = |id: &str| if secrets { stores.connections.credentials().get(id) } else { Ok(None) };
    let connections = stores
        .connections
        .list()
        .into_iter()
        .map(|connection| {
            Ok(BackedUpConnection {
                password: secret(&connection.id)?,
                ssh_secret: secret(&Connection::ssh_credential_id(&connection.id))?,
                connection,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Backup {
        format: FORMAT.into(),
        version: VERSION,
        exported_at: now,
        app_version: app_version.into(),
        preferences,
        connections,
        tags: stores.tags.list(),
        mcp: Some(BackedUpMcp { settings: stores.mcp.get(), token: secret(crate::mcp::token::CREDENTIAL_ID)? }),
        tool_paths: stores.tools.all(),
    })
}

/// Reads a backup file, refusing what isn't one or comes from a newer DBForge.
pub fn read(json: &str) -> Result<Backup> {
    let backup: Backup = serde_json::from_str(json)
        .map_err(|error| AppError::Invalid(format!("This isn't a DBForge backup: {error}")))?;
    if backup.format != FORMAT {
        return Err(AppError::Invalid("This isn't a DBForge backup.".into()));
    }
    if backup.version > VERSION {
        return Err(AppError::Invalid("This backup comes from a newer DBForge. Update DBForge to import it.".into()));
    }
    Ok(backup)
}

/// What a backup holds, and what importing it would do here.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub exported_at: u64,
    pub app_version: String,
    pub preferences: BTreeMap<String, String>,
    /// Connections it would add, by name.
    pub new_connections: Vec<String>,
    /// Connections here it would update, by name.
    pub updated_connections: Vec<String>,
    pub passwords: usize,
    pub tags: usize,
    pub mcp: Option<McpSettings>,
    pub mcp_token: bool,
    pub tool_paths: usize,
}

pub fn preview(stores: &Stores<'_>, backup: &Backup) -> Preview {
    let here: Vec<String> = stores.connections.list().into_iter().map(|connection| connection.id).collect();
    let (updated, new): (Vec<_>, Vec<_>) =
        backup.connections.iter().partition(|backed_up| here.contains(&backed_up.connection.id));
    let names = |list: Vec<&BackedUpConnection>| list.into_iter().map(|c| c.connection.name.clone()).collect();
    Preview {
        exported_at: backup.exported_at,
        app_version: backup.app_version.clone(),
        preferences: backup.preferences.clone(),
        new_connections: names(new),
        updated_connections: names(updated),
        passwords: backup.connections.iter().filter(|c| c.password.is_some()).count(),
        tags: backup.tags.len(),
        mcp: backup.mcp.as_ref().map(|mcp| mcp.settings),
        mcp_token: backup.mcp.as_ref().is_some_and(|mcp| mcp.token.is_some()),
        tool_paths: backup.tool_paths.len(),
    }
}

/// The parts of a backup to apply.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Parts {
    /// Connections, their passwords, and the tags they use.
    pub connections: bool,
    pub mcp: bool,
    pub tool_paths: bool,
}

/// What importing did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub added: usize,
    pub updated: usize,
    /// Connections that need a password typed in: they had none in the backup and have none here.
    pub without_password: Vec<String>,
    /// Tool paths left out because nothing is at them on this computer.
    pub missing_tools: Vec<String>,
    /// The MCP settings applied, for the caller to start or stop the server.
    pub mcp: Option<BackedUpMcp>,
}

/// Applies `parts` of `backup`, merging into what is here.
pub fn apply(stores: &Stores<'_>, backup: Backup, parts: Parts) -> Result<Imported> {
    let mut imported = Imported { added: 0, updated: 0, without_password: vec![], missing_tools: vec![], mcp: None };
    if parts.connections {
        let renamed = merge_tags(stores.tags, backup.tags)?;
        let credentials = stores.connections.credentials();
        let here: Vec<String> = stores.connections.list().into_iter().map(|connection| connection.id).collect();
        for BackedUpConnection { mut connection, password, ssh_secret } in backup.connections {
            if let Some(tag) = &connection.environment {
                connection.environment = Some(renamed.get(tag).cloned().unwrap_or_else(|| tag.clone()));
            }
            if here.contains(&connection.id) {
                imported.updated += 1
            } else {
                imported.added += 1
            }
            match password {
                Some(password) => credentials.set(&connection.id, &password)?,
                None if credentials.get(&connection.id)?.is_none() && needs_password(&connection) => {
                    imported.without_password.push(connection.name.clone());
                }
                None => {}
            }
            if let Some(secret) = ssh_secret {
                credentials.set(&Connection::ssh_credential_id(&connection.id), &secret)?;
            }
            stores.connections.upsert(connection)?;
        }
    }
    if parts.mcp
        && let Some(mcp) = backup.mcp
    {
        stores.mcp.set(mcp.settings)?;
        if let Some(token) = &mcp.token {
            stores.connections.credentials().set(crate::mcp::token::CREDENTIAL_ID, token)?;
        }
        imported.mcp = Some(mcp);
    }
    if parts.tool_paths {
        for (name, path) in backup.tool_paths {
            if path.is_file() {
                stores.tools.set(&name, Some(path))?;
            } else {
                imported.missing_tools.push(name);
            }
        }
    }
    Ok(imported)
}

/// A file database opens without one.
fn needs_password(connection: &Connection) -> bool {
    connection.engine != crate::connections::model::Engine::Sqlite
}

/// Adds the backup's tags to the ones here: same id updates it, a name already
/// here under another id becomes that tag. Returns the backup's ids that now go
/// by another id here.
fn merge_tags(store: &TagStore, incoming: Vec<Tag>) -> Result<HashMap<String, String>> {
    let mut tags = store.list();
    let mut renamed = HashMap::new();
    for tag in incoming {
        if let Some(same) = tags.iter_mut().find(|here| here.id == tag.id) {
            *same = tag;
        } else if let Some(named) = tags.iter().find(|here| here.name.eq_ignore_ascii_case(tag.name.trim())) {
            renamed.insert(tag.id, named.id.clone());
        } else {
            tags.push(tag);
        }
    }
    store.replace(tags)?;
    Ok(renamed)
}

#[cfg(test)]
mod tests;
