//! The HTTP server agents connect to: MCP's Streamable HTTP transport on
//! 127.0.0.1 only, behind the bearer token. rmcp checks the `Host` header
//! against loopback names (DNS rebinding) and, with an allow-list that admits
//! no browser page, refuses requests a web page sends.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::SessionStore;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde::Serialize;
use tauri::AppHandle;
use tauri::async_runtime::JoinHandle;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use super::token;
use super::tools::Tools;
use crate::error::{AppError, Result};

/// Where agents send their requests, under the server's address.
pub const PATH: &str = "/mcp";

/// Whether the server is listening, and where; or why it couldn't start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    Stopped,
    Running { url: String },
    Failed { error: String },
}

/// The token requests must carry. Shared with the running server, so a new
/// token applies at once without restarting it.
pub type SharedToken = Arc<RwLock<String>>;

/// A server task listening, and what stops it.
struct Running {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

/// Where agents' sessions are kept, so they outlive a restart of the server.
pub type Sessions = Arc<dyn SessionStore>;

/// The running server, if any: started and stopped as the settings say.
pub struct McpServer {
    running: tokio::sync::Mutex<Option<Running>>,
    status: Mutex<Option<Status>>,
    token: SharedToken,
    sessions: Sessions,
}

/// How long a stopping server may take to let its agents go before it is cut off.
const STOP_WAIT: Duration = Duration::from_secs(3);

impl McpServer {
    pub fn new(sessions: Sessions) -> Self {
        Self { running: Default::default(), status: Default::default(), token: Default::default(), sessions }
    }

    pub fn status(&self) -> Status {
        lock(&self.status).clone().unwrap_or(Status::Stopped)
    }

    /// Requests must carry `token` from now on; the server keeps running.
    pub fn use_token(&self, token: String) {
        *self.token.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = token;
    }

    /// Listens on `port`, stopping a server already running first and waiting
    /// until it has let the port go. A port already taken is reported in the
    /// status rather than as an error, so the settings still save and say what
    /// went wrong.
    pub async fn start(&self, app: AppHandle, port: u16, token: String) -> Status {
        let mut running = self.running.lock().await;
        stop(running.take()).await;
        self.use_token(token);
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let status = match TcpListener::bind(address).await {
            Ok(listener) => {
                let cancel = CancellationToken::new();
                let router = secured(
                    move || Tools::new(app.clone()),
                    self.token.clone(),
                    self.sessions.clone(),
                    port,
                    cancel.child_token(),
                );
                let shutdown = cancel.clone();
                let task = tauri::async_runtime::spawn(async move {
                    let served = axum::serve(listener, router).with_graceful_shutdown(shutdown.cancelled_owned());
                    if let Err(error) = served.await {
                        eprintln!("The MCP server stopped: {error}");
                    }
                });
                *running = Some(Running { cancel, task });
                Status::Running { url: format!("http://127.0.0.1:{port}{PATH}") }
            }
            Err(error) => Status::Failed { error: format!("Could not listen on port {port}: {error}") },
        };
        *lock(&self.status) = Some(status.clone());
        status
    }

    /// Stops listening and waits until the port is free; agents connected get no further answers.
    pub async fn stop(&self) {
        stop(self.running.lock().await.take()).await;
        *lock(&self.status) = Some(Status::Stopped);
    }
}

/// Stops `running`, giving its agents a moment to be told, then cutting it off.
async fn stop(running: Option<Running>) {
    let Some(Running { cancel, mut task }) = running else { return };
    cancel.cancel();
    if tokio::time::timeout(STOP_WAIT, &mut task).await.is_err() {
        task.abort();
        let _ = task.await;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `handler` served over Streamable HTTP at [`PATH`], only to requests with
/// the token, a loopback `Host` and no web page's `Origin`. A session unknown
/// to this server (it restarted) is brought back from `sessions`.
pub(super) fn secured<H>(
    handler: impl Fn() -> H + Send + Sync + 'static,
    token: SharedToken,
    sessions: Sessions,
    port: u16,
    cancel: CancellationToken,
) -> Router
where
    H: rmcp::ServerHandler,
{
    let mut config = StreamableHttpServerConfig::default();
    config.cancellation_token = cancel;
    config.allowed_hosts =
        vec![format!("127.0.0.1:{port}"), format!("localhost:{port}"), "127.0.0.1".into(), "localhost".into()];
    // Agents don't send an Origin; a web page always does, and none is let in.
    config.allowed_origins = Vec::new();
    config = config.enforce_origin_validation();
    config.session_store = Some(sessions);
    let service = StreamableHttpService::new(move || Ok(handler()), Arc::new(LocalSessionManager::default()), config);
    Router::new().nest_service(PATH, service).layer(middleware::from_fn_with_state(token, authorize))
}

/// Lets through only requests carrying the token as `Authorization: Bearer …`
/// (the scheme in any case, as HTTP defines it).
async fn authorize(State(expected): State<SharedToken>, request: Request, next: Next) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map_or("", |(_, token)| token.trim());
    let allowed = token::matches(&expected.read().unwrap_or_else(|poisoned| poisoned.into_inner()), given);
    if allowed {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Bearer")], "Missing or wrong DBForge MCP token.")
            .into_response()
    }
}

/// The token, made on first use, from the app's credential store.
pub fn token_of(app: &AppHandle) -> Result<String> {
    use tauri::Manager;
    let store = app.state::<crate::connections::store::ConnectionStore>();
    token::get_or_create(store.credentials())
        .map_err(|error| AppError::Invalid(format!("Could not read the MCP token: {error}")))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: rmcp's macros need `Result` to be the standard one.
    use std::net::Ipv4Addr;
    use std::sync::{Arc, RwLock};

    use rmcp::{tool, tool_handler, tool_router};
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    use super::{PATH, Sessions, secured};
    use crate::drivers::install_crypto;
    use crate::mcp::SessionFile;

    /// Sessions in a file of their own, for one test.
    fn sessions() -> (Sessions, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Arc::new(SessionFile::load(dir.path().join("mcp-sessions.json")).unwrap()), dir)
    }

    #[derive(Clone)]
    struct Echo;

    #[tool_router]
    impl Echo {
        #[tool(description = "Answers pong.")]
        async fn ping(&self) -> String {
            "pong".into()
        }
    }

    #[tool_handler]
    impl rmcp::ServerHandler for Echo {}

    /// The server on a free port, and its address.
    async fn serve(token: &str) -> (String, CancellationToken) {
        serve_with(token, sessions().0).await
    }

    /// The server on a free port, keeping its sessions in `sessions`.
    async fn serve_with(token: &str, sessions: Sessions) -> (String, CancellationToken) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let cancel = CancellationToken::new();
        let router = secured(|| Echo, Arc::new(RwLock::new(token.to_owned())), sessions, port, cancel.child_token());
        let shutdown = cancel.clone();
        tokio::spawn(
            async move { axum::serve(listener, router).with_graceful_shutdown(shutdown.cancelled_owned()).await },
        );
        (format!("http://127.0.0.1:{port}{PATH}"), cancel)
    }

    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;

    async fn initialize(url: &str, headers: &[(&str, &str)]) -> reqwest::Response {
        let mut request = reqwest::Client::new()
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(INITIALIZE);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send().await.unwrap()
    }

    #[tokio::test]
    async fn answers_only_with_the_token() {
        install_crypto();
        let (url, cancel) = serve("s3cret").await;

        assert_eq!(initialize(&url, &[]).await.status(), 401);
        assert_eq!(initialize(&url, &[("authorization", "Bearer wrong")]).await.status(), 401);
        assert_eq!(initialize(&url, &[("authorization", "bearer s3cret")]).await.status(), 200);
        let ok = initialize(&url, &[("authorization", "Bearer s3cret")]).await;
        assert_eq!(ok.status(), 200);
        assert!(ok.text().await.unwrap().contains("serverInfo"));
        cancel.cancel();
    }

    #[tokio::test]
    async fn a_new_token_applies_at_once_and_the_old_one_stops_working() {
        install_crypto();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = Arc::new(RwLock::new("old".to_owned()));
        let cancel = CancellationToken::new();
        let router = secured(|| Echo, token.clone(), sessions().0, port, cancel.child_token());
        tokio::spawn(axum::serve(listener, router).into_future());
        let url = format!("http://127.0.0.1:{port}{PATH}");

        assert_eq!(initialize(&url, &[("authorization", "Bearer old")]).await.status(), 200);
        *token.write().unwrap() = "new".into();
        assert_eq!(initialize(&url, &[("authorization", "Bearer old")]).await.status(), 401);
        assert_eq!(initialize(&url, &[("authorization", "Bearer new")]).await.status(), 200);
        cancel.cancel();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stopped_server_stops_answering_at_once() {
        install_crypto();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let cancel = CancellationToken::new();
        let token = "only-this-test-uses-this-token";
        let router = secured(|| Echo, Arc::new(RwLock::new(token.into())), sessions().0, port, cancel.child_token());
        let shutdown = cancel.clone();
        let task = tauri::async_runtime::spawn(async move {
            let _ = axum::serve(listener, router).with_graceful_shutdown(shutdown.cancelled_owned()).await;
        });

        super::stop(Some(super::Running { cancel, task })).await;
        // It no longer answers its token. (Checking the port itself would race the
        // other tests, which take free ports at the same time and may get this one.)
        let url = format!("http://127.0.0.1:{port}{PATH}");
        let answer = reqwest::Client::new()
            .post(&url)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(INITIALIZE)
            .send()
            .await;
        assert!(answer.is_err() || answer.unwrap().status() != 200, "the stopped server still answers");
    }

    #[tokio::test]
    async fn refuses_web_pages_and_other_hosts() {
        install_crypto();
        let (url, cancel) = serve("s3cret").await;
        let auth = ("authorization", "Bearer s3cret");

        let from_page = initialize(&url, &[auth, ("origin", "https://evil.example")]).await;
        assert_eq!(from_page.status(), 403);
        let rebound = initialize(&url, &[auth, ("host", "evil.example")]).await;
        assert!(rebound.status().is_client_error(), "{}", rebound.status());
        cancel.cancel();
    }

    /// A JSON-RPC message sent within `session`.
    async fn send(url: &str, session: &str, body: &str) -> reqwest::Response {
        reqwest::Client::new()
            .post(url)
            .header("authorization", "Bearer s3cret")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18")
            .header("mcp-session-id", session)
            .body(body.to_owned())
            .send()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn an_agent_s_session_outlives_a_restart() {
        install_crypto();
        let (sessions, _dir) = sessions();
        let (url, cancel) = serve_with("s3cret", sessions.clone()).await;
        let connected = initialize(&url, &[("authorization", "Bearer s3cret")]).await;
        let session = connected.headers()["mcp-session-id"].to_str().unwrap().to_owned();
        let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(send(&url, &session, initialized).await.status().is_success());
        cancel.cancel();

        // The app reopens: a new server, on the same sessions.
        let (url, cancel) = serve_with("s3cret", sessions).await;
        let listed = send(&url, &session, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).await;
        assert_eq!(listed.status(), 200);
        assert!(listed.text().await.unwrap().contains("ping"));
        cancel.cancel();
    }
}
