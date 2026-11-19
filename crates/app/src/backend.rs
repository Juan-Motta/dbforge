//! The core and the tokio runtime it works on, for views to call: each call
//! runs off the UI thread and its result comes back as a GPUI [`Task`] the
//! view awaits on the foreground.

use std::future::Future;
use std::sync::{Arc, OnceLock};

use dbforge_core::Core;
use gpui::{App, Global, Task};
use tokio::runtime::Runtime;

/// The process's tokio runtime: built once and never dropped, so the core's
/// tunnels, MCP server and queries outlive any window.
pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("dbforge-core")
            // A file import's future nests the drivers' deeply (a debug build
            // overflows tokio's 2 MiB default); room as a main thread has.
            .thread_stack_size(8 * 1024 * 1024)
            .build()
            .expect("DBForge needs a tokio runtime to run its core")
    })
}

/// The loaded core, reachable from any view as a GPUI global.
#[derive(Clone)]
pub struct Backend {
    core: Arc<Core>,
}

impl Global for Backend {}

impl Backend {
    pub fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// The core, for what is only memory and returns at once (a connection list).
    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }

    /// Runs one of the core's async functions on the runtime; await the task
    /// on the foreground (`cx.spawn`) for its result.
    pub fn spawn<F, Fut, T>(&self, cx: &App, call: F) -> Task<T>
    where
        F: FnOnce(Arc<Core>) -> Fut + Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let core = self.core.clone();
        let running = runtime().spawn(async move { call(core).await });
        cx.background_executor().spawn(async move { joined(running.await) })
    }

    /// Runs one of the core's blocking functions (files, the keychain) on the
    /// runtime's blocking threads, never the UI thread.
    pub fn spawn_blocking<F, T>(&self, cx: &App, call: F) -> Task<T>
    where
        F: FnOnce(&Core) -> T + Send + 'static,
        T: Send + 'static,
    {
        let core = self.core.clone();
        let running = runtime().spawn_blocking(move || call(&core));
        cx.background_executor().spawn(async move { joined(running.await) })
    }
}

/// A finished call's value; a call that panicked panics its awaiting task too,
/// as it would have had it run there.
fn joined<T>(result: Result<T, tokio::task::JoinError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        // The runtime is never shut down, so nothing cancels a call.
        Err(error) => panic!("a core call was cancelled: {error}"),
    }
}

/// A core over empty directories, for tests that draw views calling it.
#[cfg(test)]
pub fn test_backend(cx: &mut App) -> tempfile::TempDir {
    use dbforge_core::Dirs;
    let dir = tempfile::tempdir().expect("a temporary directory");
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).expect("a config directory");
    // Secrets in the encrypted file, never the keychain.
    std::fs::write(config.join("settings.json"), r#"{"credentialStorage":"file"}"#).expect("the settings");
    let dirs = Dirs { config, data: dir.path().join("data") };
    let core = {
        let _runtime = runtime().enter();
        Core::load(dirs, Arc::new(NoEvents)).expect("a core")
    };
    cx.set_global(Backend::new(core));
    dir
}

#[cfg(test)]
struct NoEvents;

#[cfg(test)]
impl dbforge_core::Events for NoEvents {
    fn emit(&self, _event: dbforge_core::Event) {}
}
