//! A whole PostgreSQL database to a .sql file and back, with PostgreSQL's own
//! tools: pg_dump writes the file, psql restores it. The tools are the ones
//! installed on the computer (see [`tools`]); they get the connection's
//! details and password in their environment, never on their command line,
//! where other users could read them.
//!
//! A restore runs in one transaction and stops at the first error, so a failed
//! one changes nothing; a database created for it is dropped again.

mod exports;
mod tool_paths;
mod tools;

pub use tool_paths::ToolPaths;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use self::tools::Tool;
use crate::connections::model::{Connection, Engine};
use crate::connections::store::ConnectionStore;
use crate::connections::transport::SslMode;
use crate::drivers::postgres;
use crate::error::{AppError, Result};
use crate::files::partial_path;
use crate::jobs::{Every, Jobs, cancelled};
use crate::statement_log::{Origin, StatementLog};
use crate::tunnel::Route;

/// What a dump holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Content {
    All,
    Structure,
    Data,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpOptions {
    pub content: Content,
    /// Drops each object before creating it, so the file restores over an existing database.
    pub clean: bool,
    /// Keeps `ALTER … OWNER` and `GRANT`, which need the same roles on the server restored to.
    pub keep_owners: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub bytes: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    /// The database didn't exist and was created for the restore.
    pub created: bool,
    pub elapsed_ms: u64,
}

/// Writes `database` to `path` as plain SQL. The dump goes to a file beside it
/// first, so a failed or cancelled one leaves `path` as it was.
pub async fn export(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    path: &Path,
    options: &DumpOptions,
    run: Run<'_>,
    on_line: impl FnMut(&str),
) -> Result<Exported> {
    let Run { tool, cancel } = run;
    postgres_only(connection)?;
    let started = Instant::now();
    // The default export folder may not exist yet.
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = partial_path(path);
    let session = session_name();
    let mut command = Command::new(&tool.path);
    let route = crate::tunnel::route(connection).await?;
    connect_with(&mut command, connection, &route, password, database, &session);
    command.args(["--format=plain", "--encoding=UTF8", "--verbose", "--no-password", "--file"]).arg(&partial);
    match options.content {
        Content::All => {}
        Content::Structure => {
            command.arg("--schema-only");
        }
        Content::Data => {
            command.arg("--data-only");
        }
    }
    if options.clean && options.content != Content::Data {
        command.args(["--clean", "--if-exists"]);
    }
    if !options.keep_owners {
        command.args(["--no-owner", "--no-privileges"]);
    }
    let outcome = tools::run(command, cancel, on_line).await;
    if let Err(error) = outcome {
        let _ = std::fs::remove_file(&partial);
        if cancel.is_cancelled() {
            postgres::end_sessions(connection, password, &session).await?;
        }
        return Err(with_version_hint(error));
    }
    std::fs::rename(&partial, path)?;
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or_default();
    Ok(Exported { bytes, elapsed_ms: started.elapsed().as_millis() as u64 })
}

/// Runs the SQL file at `path` into `database`, creating it first when it
/// doesn't exist. A cancel is checked between steps, so one that comes before
/// psql starts still stops the import.
pub async fn import(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    path: &Path,
    run: Run<'_>,
    on_line: impl FnMut(&str),
) -> Result<Imported> {
    let Run { tool, cancel } = run;
    postgres_only(connection)?;
    let database = database.trim();
    if database.is_empty() {
        return Err(AppError::Invalid("Name the database to restore into.".into()));
    }
    if !path.is_file() {
        return Err(AppError::Invalid(format!("There's no file at {}.", path.display())));
    }
    let started = Instant::now();
    let stop = || if cancel.is_cancelled() { Err(cancelled()) } else { Ok(()) };
    stop()?;
    let created = !postgres::database_exists(connection, password, database).await?;
    stop()?;
    if created {
        postgres::create_database(connection, password, database).await?;
    }
    let session = session_name();
    let mut command = Command::new(&tool.path);
    let route = crate::tunnel::route(connection).await?;
    connect_with(&mut command, connection, &route, password, database, &session);
    command
        .args(["--no-psqlrc", "--no-password", "--set", "ON_ERROR_STOP=1", "--single-transaction", "--file"])
        .arg(path);
    let Err(error) = tools::run(command, cancel, on_line).await else {
        return Ok(Imported { created, elapsed_ms: started.elapsed().as_millis() as u64 });
    };
    // Killing psql leaves its session working on the server: end it, which rolls the transaction back.
    let mut problems = Vec::new();
    if cancel.is_cancelled()
        && let Err(problem) = postgres::end_sessions(connection, password, &session).await
    {
        problems.push(problem.to_string());
    }
    // Nothing was restored. Leave no empty database behind.
    if created && let Err(problem) = postgres::force_drop_database(connection, password, database).await {
        problems.push(format!("The database {database} created for the import couldn't be removed: {problem}"));
    }
    if problems.is_empty() { Err(error) } else { Err(AppError::Invalid(format!("{error}\n{}", problems.join("\n")))) }
}

/// The `application_name` a tool's sessions carry, unique per run, so a
/// cancel can find them on the server.
fn session_name() -> String {
    format!("DBForge {}", uuid::Uuid::new_v4())
}

fn postgres_only(connection: &Connection) -> Result<()> {
    match connection.engine {
        Engine::Postgres => Ok(()),
        other => Err(AppError::Invalid(format!(
            "Exporting and importing whole databases is for PostgreSQL; this is {}.",
            other.label()
        ))),
    }
}

/// The connection for libpq, in the environment, to `route` (the server or its
/// tunnel). Inherited `PG…` settings (a service file, another host) are left
/// out so they can't redirect it.
fn connect_with(
    command: &mut Command,
    connection: &Connection,
    route: &Route,
    password: Option<&str>,
    database: &str,
    session: &str,
) {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("PG") {
            command.env_remove(name);
        }
    }
    command
        .env("PGHOST", &route.host)
        .env("PGPORT", route.port.to_string())
        .env("PGDATABASE", database)
        .env("PGAPPNAME", session)
        .env("PGCONNECT_TIMEOUT", "10");
    if !connection.username.is_empty() {
        command.env("PGUSER", &connection.username);
    }
    if let Some(password) = password {
        command.env("PGPASSWORD", password);
    }
    let ssl = &connection.ssl;
    command.env("PGSSLMODE", ssl_mode(ssl.mode));
    if !ssl.ca_file.is_empty() {
        command.env("PGSSLROOTCERT", &ssl.ca_file);
    } else if ssl.mode.verifies() {
        // As the app's own connections: the system's authorities (libpq 16 and later).
        command.env("PGSSLROOTCERT", "system");
    }
    if !ssl.cert_file.is_empty() {
        command.env("PGSSLCERT", &ssl.cert_file).env("PGSSLKEY", &ssl.key_file);
    }
}

/// libpq's name for `mode`.
fn ssl_mode(mode: SslMode) -> &'static str {
    match mode {
        SslMode::Disable => "disable",
        SslMode::Prefer => "prefer",
        SslMode::Require => "require",
        SslMode::VerifyCa => "verify-ca",
        SslMode::VerifyFull => "verify-full",
    }
}

/// pg_dump refuses servers newer than itself; say what to install.
fn with_version_hint(error: AppError) -> AppError {
    let message = error.to_string();
    if message.contains("server version mismatch") {
        AppError::Invalid(format!(
            "{message}\nInstall the client tools of the server's PostgreSQL version or newer, \
             e.g. `brew install postgresql@18`."
        ))
    } else {
        error
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub job_id: String,
    pub connection_id: String,
    pub database: String,
    pub path: String,
    pub options: DumpOptions,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub job_id: String,
    pub connection_id: String,
    pub database: String,
    pub path: String,
}

/// One line a running tool wrote, for the dialog to show.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    job_id: &'a str,
    line: &'a str,
}

const PROGRESS: &str = "dump-progress";
/// The dialog shows the last line the tool wrote; psql writes one per
/// statement, so lines are sent this often at most, not one event for each.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

#[tauri::command]
pub async fn export_database(
    app: AppHandle,
    store: State<'_, ConnectionStore>,
    jobs: State<'_, Jobs>,
    chosen: State<'_, ToolPaths>,
    log: State<'_, StatementLog>,
    request: ExportRequest,
) -> Result<Exported> {
    let opened = store.open(&request.connection_id)?;
    let tool = tools::resolve("pg_dump", chosen.get("pg_dump").as_deref()).await?;
    let job = jobs.start(&request.job_id);
    let mut every = Every::new(PROGRESS_EVERY);
    let report = |line: &str| {
        if every.due() {
            let _ = app.emit(PROGRESS, Progress { job_id: &request.job_id, line });
        }
    };
    let path = Path::new(&request.path);
    let run = Run { tool: &tool, cancel: job.token() };
    opened
        .run(&log, Origin::Dump, async |connection, password| {
            export(connection, password, &request.database, path, &request.options, run, report).await
        })
        .await
}

#[tauri::command]
pub async fn import_database(
    app: AppHandle,
    store: State<'_, ConnectionStore>,
    jobs: State<'_, Jobs>,
    chosen: State<'_, ToolPaths>,
    log: State<'_, StatementLog>,
    request: ImportRequest,
) -> Result<Imported> {
    let opened = store.open(&request.connection_id)?;
    let tool = tools::resolve("psql", chosen.get("psql").as_deref()).await?;
    let job = jobs.start(&request.job_id);
    let mut every = Every::new(PROGRESS_EVERY);
    let report = |line: &str| {
        if every.due() {
            let _ = app.emit(PROGRESS, Progress { job_id: &request.job_id, line });
        }
    };
    let path = Path::new(&request.path);
    let run = Run { tool: &tool, cancel: job.token() };
    opened
        .run(&log, Origin::Dump, async |connection, password| {
            import(connection, password, &request.database, path, run, report).await
        })
        .await
}

/// How an export or import runs: with which tool, and what stops it.
#[derive(Clone, Copy)]
pub struct Run<'a> {
    pub tool: &'a Tool,
    pub cancel: &'a CancellationToken,
}

/// The tool an export or import runs, as the dialogs show it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    path: String,
    /// `major.minor`.
    version: String,
    /// Chosen by the user rather than found.
    custom: bool,
}

impl From<Tool> for ToolInfo {
    fn from(tool: Tool) -> Self {
        Self {
            path: tool.path.display().to_string(),
            version: format!("{}.{}", tool.version.0, tool.version.1),
            custom: tool.custom,
        }
    }
}

/// Which `pg_dump` or `psql` DBForge will run, or why there is none to run.
#[tauri::command]
pub async fn dump_tool(chosen: State<'_, ToolPaths>, name: String) -> Result<ToolInfo> {
    let name = tools::check_name(&name)?;
    Ok(tools::resolve(name, chosen.get(name).as_deref()).await?.into())
}

/// Runs `path` for `name` from now on, once it proves to be that tool; `None` goes back to the one found.
#[tauri::command]
pub async fn set_dump_tool(chosen: State<'_, ToolPaths>, name: String, path: Option<String>) -> Result<ToolInfo> {
    let name = tools::check_name(&name)?;
    let path = path.map(PathBuf::from);
    // Refused before it is saved: a path that doesn't run would only fail later.
    let tool = tools::resolve(name, path.as_deref()).await?;
    chosen.set(name, path)?;
    Ok(tool.into())
}

/// Whether a file is already at `path`, so the export dialog can ask before replacing it.
#[tauri::command]
pub fn dump_file_exists(path: String) -> bool {
    Path::new(&path).exists()
}

/// How many exports are in `folder` and how much they take.
#[tauri::command(async)]
pub fn export_folder_usage(folder: String) -> Result<exports::ExportFiles> {
    exports::usage(Path::new(&folder))
}

/// Deletes the exports in `folder` (other files stay), and says what was deleted.
#[tauri::command(async)]
pub fn clear_export_folder(folder: String) -> Result<exports::ExportFiles> {
    exports::clear(Path::new(&folder))
}

/// Stops a running export or import. Harmless once it has finished.
/// Runs against the local server and the client tools installed. Start the server and run
/// `cargo test -- --ignored`; see `drivers::postgres::tests` for the container.
#[cfg(test)]
pub(crate) mod tests {
    use sqlx::{Connection as _, PgConnection};

    #[test]
    fn progress_is_sent_at_most_once_per_period() {
        let mut every = Every::new(Duration::from_millis(50));
        assert!(every.due(), "the first line goes at once");
        assert!(!every.due() && !every.due(), "the lines right after it wait");
        std::thread::sleep(Duration::from_millis(60));
        assert!(every.due());
        assert!(!every.due());
    }

    use super::*;
    use crate::drivers::postgres::tests::{PASSWORD, local};

    const ALL: DumpOptions = DumpOptions { content: Content::All, clean: false, keep_owners: false };

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432"]
    async fn exports_into_a_folder_that_doesnt_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new folder").join("shop.sql");
        assert!(!dump_file_exists(file.display().to_string()));
        export(
            &local(),
            PASSWORD,
            "shop",
            &file,
            &ALL,
            Run { tool: &pg_dump().await, cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap();
        assert!(dump_file_exists(file.display().to_string()));
    }

    #[tokio::test]
    #[ignore = "needs the TLS PostgreSQL container on port 55433 and the client tools"]
    async fn pg_dump_uses_the_ssl_mode_and_the_client_certificate() {
        use crate::connections::transport::SslMode;
        use crate::drivers::test_certs::{as_certuser, trusting};
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tls.sql");
        let certuser = |ssl, host: &str| Connection {
            host: host.into(),
            port: 55433,
            username: "certuser".into(),
            ssl,
            ..local()
        };
        let dump = async |connection: Connection| {
            let run = Run { tool: &pg_dump().await, cancel: &CancellationToken::new() };
            export(&connection, None, "postgres", &file, &ALL, run, |_| {}).await
        };
        assert!(dump(certuser(trusting(SslMode::VerifyFull), "localhost")).await.is_err(), "no certificate");
        let error = dump(certuser(as_certuser(SslMode::VerifyFull), "127.0.0.1")).await.unwrap_err().to_string();
        assert!(error.contains("127.0.0.1"), "the name is checked: {error}");
        dump(certuser(as_certuser(SslMode::VerifyFull), "localhost")).await.unwrap();
        assert!(file.exists());
    }

    /// Exports `database` of `connection` to `file`, everything included.
    pub(crate) async fn export_to(
        connection: &Connection,
        password: Option<&str>,
        database: &str,
        file: &Path,
    ) -> Result<Exported> {
        let run = Run { tool: &pg_dump().await, cancel: &CancellationToken::new() };
        export(connection, password, database, file, &ALL, run, |_| {}).await
    }

    async fn pg_dump() -> Tool {
        tools::find("pg_dump").await.unwrap()
    }

    async fn psql() -> Tool {
        tools::find("psql").await.unwrap()
    }

    async fn sql(database: &str, statements: &str) {
        let url = format!("postgres://postgres:dbforge@127.0.0.1:55432/{database}");
        let mut conn = PgConnection::connect(&url).await.unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(statements.to_string())).execute(&mut conn).await.unwrap();
        conn.close().await.unwrap();
    }

    async fn count(database: &str) -> i64 {
        let url = format!("postgres://postgres:dbforge@127.0.0.1:55432/{database}");
        let mut conn = PgConnection::connect(&url).await.unwrap();
        let n = sqlx::query_scalar("SELECT count(*) FROM dumped.items").fetch_one(&mut conn).await.unwrap();
        conn.close().await.unwrap();
        n
    }

    async fn fresh(name: &str) {
        postgres::force_drop_database(&local(), PASSWORD, name).await.unwrap();
        postgres::create_database(&local(), PASSWORD, name).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432 and its client tools"]
    async fn exports_a_database_and_restores_it_into_a_new_one() {
        fresh("dbforge_dump_source").await;
        sql(
            "dbforge_dump_source",
            "CREATE SCHEMA dumped;
             CREATE TABLE dumped.items (id int PRIMARY KEY, name text NOT NULL, tags jsonb);
             INSERT INTO dumped.items SELECT n, 'item ' || n, '[\"a\"]' FROM generate_series(1, 250) n;",
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("source.sql");
        let mut lines = Vec::new();
        let exported = export(
            &local(),
            PASSWORD,
            "dbforge_dump_source",
            &file,
            &ALL,
            Run { tool: &pg_dump().await, cancel: &CancellationToken::new() },
            |l| lines.push(l.to_string()),
        )
        .await
        .unwrap();
        assert!(exported.bytes > 1000, "{exported:?}");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("CREATE TABLE dumped.items"), "{text}");
        assert!(!text.contains("OWNER TO"), "owners are left out by default");
        assert!(lines.iter().any(|l| l.contains("dumping contents of table")), "{lines:?}");

        postgres::force_drop_database(&local(), PASSWORD, "dbforge_dump_copy").await.unwrap();
        let mut restored = Vec::new();
        let imported = import(
            &local(),
            PASSWORD,
            "dbforge_dump_copy",
            &file,
            Run { tool: &psql().await, cancel: &CancellationToken::new() },
            |l| restored.push(l.to_string()),
        )
        .await
        .unwrap();
        assert!(imported.created);
        assert_eq!(count("dbforge_dump_copy").await, 250);
        assert!(restored.iter().any(|l| l.starts_with("COPY 250")), "{restored:?}");

        // A dump that drops what it creates restores over the database it came from.
        let clean = dir.path().join("clean.sql");
        let options = DumpOptions { clean: true, ..ALL };
        export(
            &local(),
            PASSWORD,
            "dbforge_dump_source",
            &clean,
            &options,
            Run { tool: &pg_dump().await, cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap();
        let again = import(
            &local(),
            PASSWORD,
            "dbforge_dump_copy",
            &clean,
            Run { tool: &psql().await, cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap();
        assert!(!again.created);
        assert_eq!(count("dbforge_dump_copy").await, 250);

        let structure = dir.path().join("structure.sql");
        let options = DumpOptions { content: Content::Structure, ..ALL };
        export(
            &local(),
            PASSWORD,
            "dbforge_dump_source",
            &structure,
            &options,
            Run { tool: &pg_dump().await, cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap();
        assert!(!std::fs::read_to_string(&structure).unwrap().contains("COPY dumped.items"));

        for name in ["dbforge_dump_source", "dbforge_dump_copy"] {
            postgres::force_drop_database(&local(), PASSWORD, name).await.unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432 and its client tools"]
    async fn a_failed_restore_changes_nothing_and_drops_the_database_it_created() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("broken.sql");
        std::fs::write(&file, "CREATE TABLE ok (id int);\nINSERT INTO missing VALUES (1);\n").unwrap();

        postgres::force_drop_database(&local(), PASSWORD, "dbforge_dump_broken").await.unwrap();
        let error = import(
            &local(),
            PASSWORD,
            "dbforge_dump_broken",
            &file,
            Run { tool: &psql().await, cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("relation \"missing\" does not exist"), "{error}");
        assert!(!postgres::database_exists(&local(), PASSWORD, "dbforge_dump_broken").await.unwrap());

        // Into an existing database, the transaction leaves it as it was.
        fresh("dbforge_dump_kept").await;
        import(
            &local(),
            PASSWORD,
            "dbforge_dump_kept",
            &file,
            Run { tool: &psql().await, cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(postgres::database_exists(&local(), PASSWORD, "dbforge_dump_kept").await.unwrap());
        let url = "postgres://postgres:dbforge@127.0.0.1:55432/dbforge_dump_kept";
        let mut conn = PgConnection::connect(url).await.unwrap();
        let tables: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE tablename = 'ok'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(tables, 0);
        conn.close().await.unwrap();

        let error = import(
            &local(),
            PASSWORD,
            "x",
            &dir.path().join("none.sql"),
            Run { tool: &psql().await, cancel: &CancellationToken::new() },
            |_| {},
        )
        .await;
        assert!(error.unwrap_err().to_string().starts_with("There's no file at"));
        postgres::force_drop_database(&local(), PASSWORD, "dbforge_dump_kept").await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432 and its client tools"]
    async fn a_wrong_password_reads_as_libpq_says_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.sql");
        let error = export(
            &local(),
            Some("wrong"),
            "shop",
            &file,
            &ALL,
            Run { tool: &pg_dump().await, cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("password authentication failed"), "{error}");
        assert!(!file.exists());
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432 and its client tools"]
    async fn a_failed_export_leaves_the_file_that_was_there() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("kept.sql");
        std::fs::write(&file, "-- an older dump").unwrap();
        export(
            &local(),
            Some("wrong"),
            "shop",
            &file,
            &ALL,
            Run { tool: &pg_dump().await, cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "-- an older dump");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no partial file is left");

        // Stopped before pg_dump even started.
        let stopped = CancellationToken::new();
        stopped.cancel();
        let error =
            export(&local(), PASSWORD, "shop", &file, &ALL, Run { tool: &pg_dump().await, cancel: &stopped }, |_| {})
                .await
                .unwrap_err();
        assert_eq!(error.to_string(), "Cancelled.");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "-- an older dump");
    }

    /// Imports a file that sleeps on the server, stopped after `after`.
    async fn stopped_import(database: &str, after: std::time::Duration) -> AppError {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("slow.sql");
        std::fs::write(&file, "CREATE TABLE slow (id int);\nSELECT pg_sleep(30);\n").unwrap();
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            stop.cancel();
        });
        let started = Instant::now();
        let error = import(&local(), PASSWORD, database, &file, Run { tool: &psql().await, cancel: &cancel }, |_| {})
            .await
            .unwrap_err();
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "the cancel stopped the sleep");
        error
    }

    async fn sessions_on(database: &str) -> i64 {
        let mut conn = PgConnection::connect("postgres://postgres:dbforge@127.0.0.1:55432/postgres").await.unwrap();
        let n = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = $1")
            .bind(database)
            .fetch_one(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
        n
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432 and its client tools"]
    async fn a_stopped_import_ends_its_session_and_removes_the_database_it_created() {
        postgres::force_drop_database(&local(), PASSWORD, "dbforge_dump_stopped").await.unwrap();
        let error = stopped_import("dbforge_dump_stopped", std::time::Duration::from_millis(1500)).await;
        assert_eq!(error.to_string(), "Cancelled.");
        assert!(!postgres::database_exists(&local(), PASSWORD, "dbforge_dump_stopped").await.unwrap());

        // Into an existing database: its session ends and the table it began is rolled back.
        fresh("dbforge_dump_stopped_kept").await;
        let error = stopped_import("dbforge_dump_stopped_kept", std::time::Duration::from_millis(1500)).await;
        assert_eq!(error.to_string(), "Cancelled.");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(sessions_on("dbforge_dump_stopped_kept").await, 0);
        postgres::force_drop_database(&local(), PASSWORD, "dbforge_dump_stopped_kept").await.unwrap();

        // Stopped before psql started: nothing is created.
        postgres::force_drop_database(&local(), PASSWORD, "dbforge_dump_never").await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.sql");
        std::fs::write(&file, "SELECT 1;").unwrap();
        let stopped = CancellationToken::new();
        stopped.cancel();
        let error = import(
            &local(),
            PASSWORD,
            "dbforge_dump_never",
            &file,
            Run { tool: &psql().await, cancel: &stopped },
            |_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "Cancelled.");
        assert!(!postgres::database_exists(&local(), PASSWORD, "dbforge_dump_never").await.unwrap());
    }
}
