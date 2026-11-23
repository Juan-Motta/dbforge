//! Everything DBForge keeps while it runs, loaded once at start: the saved
//! connections and settings, the running queries and jobs, the statement log,
//! the history and the MCP server. The UI holds one [`Core`] and calls the
//! modules' functions with what it hands out.

use std::path::PathBuf;
use std::sync::Arc;

use crate::connections::store::ConnectionStore;
use crate::connections::tags::TagStore;
use crate::credentials::Credentials;
use crate::dump::{ExportFolder, ToolPaths};
use crate::error::{AppError, Result};
use crate::events::Events;
use crate::files::SetAside;
use crate::history::HistoryStore;
use crate::jobs::Jobs;
use crate::mcp::{self, AgentRegistry, Approvals, McpServer, SessionFile, SettingsFile};
use crate::query::QuerySessions;
use crate::saved_queries::SavedQueries;
use crate::statement_log::{self, StatementLog};
use crate::tunnel;

/// The app's identifier, which names its config and data directories (and,
/// before, its Tauri bundle). Installed users' files are found under it.
pub const IDENTIFIER: &str = "dev.dbforge.app";

/// The files DBForge keeps in its config directory.
pub const CONNECTIONS_FILE: &str = "connections.json";
pub const TAGS_FILE: &str = "tags.json";
pub const TOOLS_FILE: &str = "tools.json";
pub const MCP_SETTINGS_FILE: &str = "mcp.json";
pub const MCP_SESSIONS_FILE: &str = "mcp-sessions.json";
pub const SAVED_QUERIES_FILE: &str = "saved_queries.json";
pub const KNOWN_HOSTS_FILE: &str = "known_hosts";
/// The window's own preferences (theme, language, panel sizes), which the app keeps.
pub const PREFERENCES_FILE: &str = "preferences.json";
/// The query history, in the data directory.
pub const HISTORY_FILE: &str = "history.jsonl";

/// How many statements the statement log keeps.
const STATEMENT_LOG_CAPACITY: usize = 2000;

/// Where DBForge keeps its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// Connections, tags, settings and the credential store.
    pub config: PathBuf,
    /// The query history.
    pub data: PathBuf,
}

impl Dirs {
    /// The directories the Tauri app used, so an installed user keeps
    /// everything: the OS's config and data directories, under [`IDENTIFIER`].
    /// `None` on a system without them.
    pub fn default_for_app() -> Option<Self> {
        Some(Self { config: dirs::config_dir()?.join(IDENTIFIER), data: dirs::data_dir()?.join(IDENTIFIER) })
    }

    /// Where the app keeps the window's preferences ([`PREFERENCES_FILE`]).
    pub fn preferences_file(&self) -> PathBuf {
        self.config.join(PREFERENCES_FILE)
    }
}

/// DBForge's state while it runs.
pub struct Core {
    dirs: Dirs,
    events: Arc<dyn Events>,
    connections: Arc<ConnectionStore>,
    tags: TagStore,
    tool_paths: ToolPaths,
    export_folder: ExportFolder,
    saved_queries: SavedQueries,
    query_sessions: QuerySessions,
    jobs: Jobs,
    mcp_settings: SettingsFile,
    mcp_server: McpServer,
    mcp_approvals: Approvals,
    mcp_sessions: Arc<SessionFile>,
    mcp_agents: AgentRegistry,
    statement_log: StatementLog,
    history: HistoryStore,
    set_aside: SetAside,
}

impl Core {
    /// Loads everything from `dirs`, sets up the SSH tunnels and starts the
    /// MCP server if its saved settings say so. Runs inside a tokio runtime,
    /// which the tunnels and the MCP server use from then on. A settings file
    /// that can't be read is set aside ([`Core::set_aside`]) rather than failing.
    pub fn load(dirs: Dirs, events: Arc<dyn Events>) -> Result<Arc<Core>> {
        tokio::runtime::Handle::try_current()
            .map_err(|_| AppError::Invalid("DBForge's core needs a tokio runtime to start in".into()))?;
        let config = &dirs.config;
        let credentials = Credentials::load(config.clone())?;
        let connections = Arc::new(ConnectionStore::load(config.join(CONNECTIONS_FILE), credentials)?);
        let secrets = connections.clone();
        tunnel::init(config.join(KNOWN_HOSTS_FILE), Box::new(move |id| secrets.credentials().get(id)));
        let tags = TagStore::load(config.join(TAGS_FILE))?;
        let tool_paths = ToolPaths::load(config.join(TOOLS_FILE))?;
        let saved_queries = SavedQueries::load(config.join(SAVED_QUERIES_FILE))?;
        let mcp_settings = SettingsFile::load(config.join(MCP_SETTINGS_FILE))?;
        let mcp_sessions = Arc::new(SessionFile::load(config.join(MCP_SESSIONS_FILE))?);
        let statement_log = StatementLog::new(STATEMENT_LOG_CAPACITY, statement_log::EventSink(events.clone()));
        // Files that couldn't be read and were set aside, for the window to tell the user.
        let set_aside = [
            connections.set_aside(),
            tags.set_aside(),
            tool_paths.set_aside(),
            saved_queries.set_aside(),
            mcp_settings.set_aside(),
            mcp_sessions.set_aside(),
        ]
        .into_iter()
        .flatten()
        .map(|path| path.display().to_string())
        .collect();
        let history = HistoryStore::load(dirs.data.join(HISTORY_FILE))?;
        let settings = mcp_settings.get();
        let core = Arc::new(Core {
            events,
            connections,
            tags,
            tool_paths,
            export_folder: ExportFolder::default(),
            saved_queries,
            query_sessions: QuerySessions::default(),
            jobs: Jobs::default(),
            mcp_settings,
            mcp_server: McpServer::new(mcp_sessions.clone()),
            mcp_approvals: Approvals::default(),
            mcp_sessions,
            mcp_agents: AgentRegistry::default(),
            statement_log,
            history,
            set_aside: SetAside(set_aside),
            dirs,
        });
        let starting = core.clone();
        tokio::spawn(async move {
            mcp::commands::apply(&starting, settings).await;
        });
        Ok(core)
    }

    pub fn dirs(&self) -> &Dirs {
        &self.dirs
    }

    pub fn events(&self) -> &Arc<dyn Events> {
        &self.events
    }

    pub fn connections(&self) -> &Arc<ConnectionStore> {
        &self.connections
    }

    pub fn tags(&self) -> &TagStore {
        &self.tags
    }

    pub fn tool_paths(&self) -> &ToolPaths {
        &self.tool_paths
    }

    /// Where exports go: the app keeps it as its preference says, for the MCP tools.
    pub fn export_folder(&self) -> &ExportFolder {
        &self.export_folder
    }

    /// The queries the user saved with a name.
    pub fn saved_queries(&self) -> &SavedQueries {
        &self.saved_queries
    }

    pub fn query_sessions(&self) -> &QuerySessions {
        &self.query_sessions
    }

    pub fn jobs(&self) -> &Jobs {
        &self.jobs
    }

    pub fn mcp_settings(&self) -> &SettingsFile {
        &self.mcp_settings
    }

    pub fn mcp_server(&self) -> &McpServer {
        &self.mcp_server
    }

    pub fn mcp_approvals(&self) -> &Approvals {
        &self.mcp_approvals
    }

    /// The agents' saved MCP sessions.
    pub fn mcp_sessions(&self) -> &SessionFile {
        &self.mcp_sessions
    }

    /// The agents that called a tool in this run.
    pub fn mcp_agents(&self) -> &AgentRegistry {
        &self.mcp_agents
    }

    pub fn statement_log(&self) -> &StatementLog {
        &self.statement_log
    }

    pub fn history(&self) -> &HistoryStore {
        &self.history
    }

    /// The settings files set aside at start, for the window to tell the user once.
    pub fn set_aside(&self) -> &SetAside {
        &self.set_aside
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NoEvents;

    /// Installed users' files are where the Tauri app kept them, under the same names.
    #[test]
    fn keeps_the_tauri_app_s_directories_and_file_names() {
        assert_eq!(IDENTIFIER, "dev.dbforge.app");
        let dirs = Dirs::default_for_app().unwrap();
        assert_eq!(dirs.config, dirs::config_dir().unwrap().join("dev.dbforge.app"));
        assert_eq!(dirs.data, dirs::data_dir().unwrap().join("dev.dbforge.app"));
        assert_eq!(
            [CONNECTIONS_FILE, TAGS_FILE, TOOLS_FILE, MCP_SETTINGS_FILE, MCP_SESSIONS_FILE, KNOWN_HOSTS_FILE],
            ["connections.json", "tags.json", "tools.json", "mcp.json", "mcp-sessions.json", "known_hosts"]
        );
        assert_eq!(SAVED_QUERIES_FILE, "saved_queries.json");
        assert_eq!(dirs.preferences_file(), dirs.config.join("preferences.json"));
        assert_eq!(HISTORY_FILE, "history.jsonl");
    }

    #[tokio::test]
    async fn loads_an_empty_install_and_reads_its_files_back() {
        let scratch = tempfile::tempdir().unwrap();
        let dirs = Dirs { config: scratch.path().join("config"), data: scratch.path().join("data") };
        // The process's tunnels as the tunnel tests set them up, so this test leaves them alone.
        crate::tunnel::tests::setup();
        let core = Core::load(dirs.clone(), Arc::new(NoEvents)).unwrap();
        assert!(core.connections().list().is_empty());
        assert!(core.set_aside().0.is_empty());
        assert!(core.saved_queries().list().is_empty());
        assert_eq!(core.dirs(), &dirs);
    }

    #[test]
    fn refuses_to_start_outside_a_tokio_runtime() {
        let scratch = tempfile::tempdir().unwrap();
        let dirs = Dirs { config: scratch.path().join("config"), data: scratch.path().join("data") };
        assert!(Core::load(dirs, Arc::new(NoEvents)).is_err());
    }
}
