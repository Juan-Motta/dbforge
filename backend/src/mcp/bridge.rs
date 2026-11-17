//! DBForge as an MCP server over stdio: `DBForge --mcp-stdio`. An agent
//! starts this process itself and keeps it for its whole session, so the
//! server is there even while the app is closed or restarting. It lists the
//! tools on its own and hands each call to the app's HTTP server on
//! 127.0.0.1; with the app closed, or its agent server off, the call answers
//! what to do instead of the agent losing DBForge until it restarts.
//!
//! It reads what it needs where the app keeps it (the port in `mcp.json`, the
//! token in the credential store), so the agent's configuration holds no
//! secret, and it writes nothing. The same on macOS, Windows and Linux: the
//! paths come from the OS's config directory, as the app's own do.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ClientCapabilities, ClientConfig, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerConfig,
};
use rmcp::service::{RequestContext, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ErrorData, Peer, RoleClient, RoleServer, ServerHandler, ServiceError, ServiceExt};
use tokio::sync::Mutex;

use super::server::PATH;
use super::settings::McpSettings;
use super::token;
use super::tools::{self, Tools};
use crate::credentials::Credentials;

/// The argument that starts the bridge instead of the app.
pub const FLAG: &str = "--mcp-stdio";

/// The app's bundle identifier (`tauri.conf.json`), which names its config directory.
const IDENTIFIER: &str = "dev.dbforge.app";

const CLOSED: &str = "DBForge isn't open. Ask the user to open DBForge, then try again.";
const OFF: &str = "DBForge's agent server is off. Ask the user to turn on Agents (MCP) in DBForge's settings, \
                   then try again.";

/// Serves MCP on stdin and stdout until the agent closes them; the process's exit code.
pub fn run() -> i32 {
    crate::drivers::install_crypto();
    let Some(config) = dirs::config_dir().map(|dir| dir.join(IDENTIFIER)) else {
        eprintln!("DBForge: no config directory on this system");
        return 1;
    };
    tauri::async_runtime::block_on(async move {
        let served = match Bridge::new(config).serve(rmcp::transport::stdio()).await {
            Ok(running) => running.waiting().await.map(drop).map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        match served {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("DBForge: {error}");
                1
            }
        }
    })
}

/// The handler on the agent's side: DBForge's tools, run by the app.
pub struct Bridge {
    /// The app's config directory: `mcp.json` and the credential store.
    config: PathBuf,
    /// The session with the app, opened on the first call and again after it was lost.
    app: Mutex<Option<Arc<RunningService<RoleClient, ClientConfig>>>>,
}

impl Bridge {
    pub fn new(config: PathBuf) -> Self {
        Self { config, app: Mutex::new(None) }
    }

    /// The session with the app, opened as `agent` (so the app's approval
    /// dialog names the agent, not the bridge) unless one is open already.
    async fn app(&self, agent: Implementation) -> Result<Peer<RoleClient>, String> {
        let mut app = self.app.lock().await;
        if let Some(running) = app.as_ref().filter(|running| !running.is_closed()) {
            return Ok(running.peer().clone());
        }
        let running = Arc::new(self.connect(agent).await?);
        let peer = running.peer().clone();
        *app = Some(running);
        Ok(peer)
    }

    async fn connect(&self, agent: Implementation) -> Result<RunningService<RoleClient, ClientConfig>, String> {
        let settings = self.settings()?;
        if !settings.enabled {
            return Err(OFF.into());
        }
        // A refused connection is the app being closed; tell it apart from a server that answers badly.
        if tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, settings.port)).await.is_err() {
            return Err(CLOSED.into());
        }
        let credentials = Credentials::load(self.config.clone())
            .map_err(|error| format!("Could not open DBForge's credential store: {error}"))?;
        let token = credentials
            .get(token::CREDENTIAL_ID)
            .map_err(|error| format!("Could not read DBForge's agent token: {error}"))?
            .ok_or_else(|| OFF.to_owned())?;
        let url = format!("http://127.0.0.1:{}{PATH}", settings.port);
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
        );
        ClientConfig::new(ClientCapabilities::default(), agent)
            .serve(transport)
            .await
            .map_err(|error| format!("DBForge didn't accept the connection: {error}"))
    }

    /// The agent server's settings, read as the app last saved them; off when it never was.
    fn settings(&self) -> Result<McpSettings, String> {
        match std::fs::read_to_string(self.config.join("mcp.json")) {
            Ok(json) => serde_json::from_str(&json).map_err(|error| format!("DBForge's mcp.json is invalid: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(McpSettings::default()),
            Err(error) => Err(format!("Could not read DBForge's mcp.json: {error}")),
        }
    }

    /// Forgets the session with the app, so the next call opens a new one.
    async fn lost(&self) {
        self.app.lock().await.take();
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        tools::server_info()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(Tools::tool_router().list_all()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let agent = context
            .peer
            .peer_info()
            .map(|info| info.client_info.clone())
            .unwrap_or_else(|| Implementation::new("agent", "unknown"));
        let app = match self.app(agent).await {
            Ok(app) => app,
            Err(message) => return Ok(failed(message)),
        };
        // Not retried: a call that reached the app may have changed data already.
        match app.call_tool_once(request).await {
            Ok(response) => Ok(response),
            Err(ServiceError::McpError(error)) => Err(error),
            Err(error) => {
                self.lost().await;
                Ok(failed(format!(
                    "Lost DBForge before it answered ({error}). If DBForge was closed, ask the user to open it; \
                     the call may or may not have run."
                )))
            }
        }
    }
}

/// A tool result the agent reads as failed, saying why.
fn failed(message: String) -> CallToolResponse {
    CallToolResponse::Complete(rmcp::model::CallToolResult::error(vec![ContentBlock::text(message)]))
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::RwLock;

    use rmcp::model::CallToolRequestParams;
    use rmcp::{ServiceExt, tool, tool_handler, tool_router};
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::mcp::SessionFile;

    #[derive(Clone)]
    struct Echo;

    #[tool_router]
    impl Echo {
        #[tool(description = "Answers pong.")]
        async fn ping(&self) -> String {
            "pong".into()
        }

        #[tool(description = "Says which agent called, as the app's approval dialog does.")]
        async fn caller(&self, context: rmcp::service::RequestContext<RoleServer>) -> String {
            context.peer.peer_info().map(|info| info.client_info.name.clone()).unwrap_or_default()
        }
    }

    #[tool_handler]
    impl rmcp::ServerHandler for Echo {}

    /// An agent talking to a bridge over `config`, as over stdio.
    async fn agent(config: PathBuf) -> RunningService<RoleClient, ClientConfig> {
        let (agent_side, bridge_side) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            let running = Bridge::new(config).serve(tokio::io::split(bridge_side)).await.unwrap();
            let _ = running.waiting().await;
        });
        ClientConfig::new(ClientCapabilities::default(), Implementation::new("test-agent", "1"))
            .serve(tokio::io::split(agent_side))
            .await
            .unwrap()
    }

    async fn call(agent: &RunningService<RoleClient, ClientConfig>, tool: &str) -> rmcp::model::CallToolResult {
        agent.call_tool(CallToolRequestParams::new(tool.to_owned())).await.unwrap()
    }

    fn text(result: &rmcp::model::CallToolResult) -> String {
        result.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect()
    }

    /// The app's settings in `config`: the agent server on `port`, or off.
    fn settings(config: &std::path::Path, enabled: bool, port: u16) {
        std::fs::write(config.join("mcp.json"), serde_json::to_string(&McpSettings { enabled, port }).unwrap())
            .unwrap();
    }

    #[tokio::test]
    async fn lists_dbforge_s_tools_while_the_app_is_closed() {
        let config = tempfile::tempdir().unwrap();
        let agent = agent(config.path().to_owned()).await;
        let names: Vec<String> = agent.list_all_tools().await.unwrap().into_iter().map(|t| t.name.into()).collect();
        assert_eq!(names.len(), Tools::tool_router().list_all().len());
        assert!(names.contains(&"list_connections".to_owned()) && names.contains(&"execute".to_owned()));
        assert_eq!(agent.peer_info().unwrap().server_info.as_ref().map(|s| s.name.as_str()), Some("DBForge"));
    }

    #[tokio::test]
    async fn a_call_says_to_turn_the_server_on_or_open_the_app() {
        let config = tempfile::tempdir().unwrap();
        let agent = agent(config.path().to_owned()).await;
        let off = call(&agent, "list_connections").await;
        assert_eq!((off.is_error, text(&off)), (Some(true), OFF.to_owned()));

        // On, but nothing listens: a port just taken and let go.
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap().local_addr().unwrap().port();
        settings(config.path(), true, port);
        let closed = call(&agent, "list_connections").await;
        assert_eq!((closed.is_error, text(&closed)), (Some(true), CLOSED.to_owned()));
    }

    #[tokio::test]
    async fn hands_calls_to_the_app_once_it_opens() {
        crate::drivers::install_crypto();
        let config = tempfile::tempdir().unwrap();
        let credentials = Credentials::load(config.path().to_owned()).unwrap();
        let token = token::regenerate(&credentials).unwrap();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        settings(config.path(), true, port);
        let agent = agent(config.path().to_owned()).await;

        // The app opens after the agent started.
        let cancel = CancellationToken::new();
        let sessions = Arc::new(SessionFile::load(config.path().join("mcp-sessions.json")).unwrap());
        let router =
            super::super::server::secured(|| Echo, Arc::new(RwLock::new(token)), sessions, port, cancel.child_token());
        tokio::spawn(axum::serve(listener, router).into_future());

        let answered = call(&agent, "ping").await;
        assert_eq!((answered.is_error, text(&answered)), (Some(false), "pong".to_owned()));
        assert_eq!(text(&call(&agent, "caller").await), "test-agent", "the app sees the agent, not the bridge");
        cancel.cancel();
    }
}

#[cfg(test)]
mod identifier {
    #[test]
    fn matches_the_app_s_bundle_identifier() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], super::IDENTIFIER, "the bridge would look for the app's files elsewhere");
    }
}
