//! Whether the MCP server runs and on which port, kept in `mcp.json`. The
//! token is a secret, so it lives in the credential store instead (see
//! [`super::token`]).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::files::JsonFile;

/// The port agents are told to use unless the user picks another.
pub const DEFAULT_PORT: u16 = 7419;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
}

impl Default for McpSettings {
    /// Off until the user turns it on: nothing listens unasked.
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_PORT }
    }
}

pub struct SettingsFile {
    settings: JsonFile<McpSettings>,
}

impl SettingsFile {
    /// The settings saved at `file`; the defaults until the user changes them.
    pub fn load(file: PathBuf) -> Result<Self> {
        Ok(Self { settings: JsonFile::load(file, McpSettings::default)? })
    }

    pub fn get(&self) -> McpSettings {
        self.settings.get()
    }

    /// Where the settings file was moved when it couldn't be read at start.
    pub fn set_aside(&self) -> Option<&std::path::Path> {
        self.settings.set_aside()
    }

    /// Keeps `settings`, after checking the port.
    pub fn set(&self, settings: McpSettings) -> Result<()> {
        if settings.port < 1024 {
            return Err(AppError::Invalid("Choose a port from 1024 to 65535.".into()));
        }
        self.settings.update(|current| {
            *current = settings;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_until_turned_on_and_kept_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("mcp.json");
        let settings = SettingsFile::load(file.clone()).unwrap();
        assert_eq!(settings.get(), McpSettings { enabled: false, port: DEFAULT_PORT });

        settings.set(McpSettings { enabled: true, port: 9000 }).unwrap();
        assert_eq!(SettingsFile::load(file).unwrap().get(), McpSettings { enabled: true, port: 9000 });
    }

    #[test]
    fn refuses_privileged_ports() {
        let dir = tempfile::tempdir().unwrap();
        let settings = SettingsFile::load(dir.path().join("mcp.json")).unwrap();
        assert!(settings.set(McpSettings { enabled: true, port: 80 }).is_err());
        assert_eq!(settings.get().port, DEFAULT_PORT);
    }
}
