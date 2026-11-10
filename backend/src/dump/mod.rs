//! A whole PostgreSQL database to a .sql file and back, with PostgreSQL's own
//! tools: pg_dump writes the file, psql restores it. The tools are the ones
//! installed on the computer (see [`tools`]); they get the connection's
//! details and password in their environment, never on their command line,
//! where other users could read them.
//!
//! A restore runs in one transaction and stops at the first error, so a failed
//! one changes nothing; a database created for it is dropped again.

mod tools;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::connections::model::{Connection, Engine};
use crate::connections::store::ConnectionStore;
use crate::drivers::postgres;
use crate::error::{AppError, Result};

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
    cancel: &CancellationToken,
    on_line: impl FnMut(&str),
) -> Result<Exported> {
    postgres_only(connection)?;
    let started = Instant::now();
    let tool = tools::find("pg_dump").await?;
    let partial = partial_path(path);
    let session = session_name();
    let mut command = Command::new(&tool.path);
    connect_with(&mut command, connection, password, database, &session);
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
    cancel: &CancellationToken,
    on_line: impl FnMut(&str),
) -> Result<Imported> {
    postgres_only(connection)?;
    let database = database.trim();
    if database.is_empty() {
        return Err(AppError::Invalid("Name the database to restore into.".into()));
    }
    if !path.is_file() {
        return Err(AppError::Invalid(format!("There's no file at {}.", path.display())));
    }
    let started = Instant::now();
    let tool = tools::find("psql").await?;
    let stop = || if cancel.is_cancelled() { Err(tools::cancelled()) } else { Ok(()) };
    stop()?;
    let created = !postgres::database_exists(connection, password, database).await?;
    stop()?;
    if created {
        postgres::create_database(connection, password, database).await?;
    }
    let session = session_name();
    let mut command = Command::new(&tool.path);
    connect_with(&mut command, connection, password, database, &session);
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
    if created && let Err(problem) = postgres::drop_database(connection, password, database).await {
        problems.push(format!("The database {database} created for the import couldn't be removed: {problem}"));
    }
    if problems.is_empty() { Err(error) } else { Err(AppError::Invalid(format!("{error}\n{}", problems.join("\n")))) }
}

/// Where a dump is written until it is complete: beside the file asked for.
fn partial_path(path: &Path) -> PathBuf {
    let mut partial = path.as_os_str().to_owned();
    partial.push(".dbforge-partial");
    PathBuf::from(partial)
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

/// The connection for libpq, in the environment. Inherited `PG…` settings
/// (a service file, another host) are left out so they can't redirect it.
fn connect_with(command: &mut Command, connection: &Connection, password: Option<&str>, database: &str, session: &str) {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("PG") {
            command.env_remove(name);
        }
    }
    command
        .env("PGHOST", &connection.host)
        .env("PGPORT", connection.port.to_string())
        .env("PGDATABASE", database)
        .env("PGAPPNAME", session)
        .env("PGCONNECT_TIMEOUT", "10");
    if !connection.username.is_empty() {
        command.env("PGUSER", &connection.username);
    }
    if let Some(password) = password {
        command.env("PGPASSWORD", password);
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

/// Running exports and imports, by job id, so they can be cancelled.
#[derive(Default)]
pub struct DumpJobs(Mutex<HashMap<String, CancellationToken>>);

impl DumpJobs {
    fn start(&self, id: &str) -> CancellationToken {
        let cancel = CancellationToken::new();
        self.0.lock().unwrap().insert(id.to_string(), cancel.clone());
        cancel
    }

    fn finish(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
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

#[tauri::command]
pub async fn export_database(
    app: AppHandle,
    store: State<'_, ConnectionStore>,
    jobs: State<'_, DumpJobs>,
    request: ExportRequest,
) -> Result<Exported> {
    let connection = store.get(&request.connection_id)?;
    let password = store.password(&connection.id)?;
    let cancel = jobs.start(&request.job_id);
    let report = |line: &str| {
        let _ = app.emit(PROGRESS, Progress { job_id: &request.job_id, line });
    };
    let path = Path::new(&request.path);
    let result =
        export(&connection, password.as_deref(), &request.database, path, &request.options, &cancel, report).await;
    jobs.finish(&request.job_id);
    result
}

#[tauri::command]
pub async fn import_database(
    app: AppHandle,
    store: State<'_, ConnectionStore>,
    jobs: State<'_, DumpJobs>,
    request: ImportRequest,
) -> Result<Imported> {
    let connection = store.get(&request.connection_id)?;
    let password = store.password(&connection.id)?;
    let cancel = jobs.start(&request.job_id);
    let report = |line: &str| {
        let _ = app.emit(PROGRESS, Progress { job_id: &request.job_id, line });
    };
    let path = Path::new(&request.path);
    let result = import(&connection, password.as_deref(), &request.database, path, &cancel, report).await;
    jobs.finish(&request.job_id);
    result
}

/// Stops a running export or import. Harmless once it has finished.
#[tauri::command]
pub fn cancel_dump(jobs: State<'_, DumpJobs>, job_id: String) {
    if let Some(cancel) = jobs.0.lock().unwrap().get(&job_id) {
        cancel.cancel();
    }
}

/// Runs against the local server and the client tools installed. Start the server and run
/// `cargo test -- --ignored`; see `drivers::postgres::tests` for the container.
#[cfg(test)]
mod tests {
    use sqlx::{Connection as _, PgConnection};

    use super::*;
    use crate::drivers::postgres::tests::{PASSWORD, local};

    const ALL: DumpOptions = DumpOptions { content: Content::All, clean: false, keep_owners: false };

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
        postgres::drop_database(&local(), PASSWORD, name).await.unwrap();
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
        let exported = export(&local(), PASSWORD, "dbforge_dump_source", &file, &ALL, &CancellationToken::new(), |l| {
            lines.push(l.to_string())
        })
        .await
        .unwrap();
        assert!(exported.bytes > 1000, "{exported:?}");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("CREATE TABLE dumped.items"), "{text}");
        assert!(!text.contains("OWNER TO"), "owners are left out by default");
        assert!(lines.iter().any(|l| l.contains("dumping contents of table")), "{lines:?}");

        postgres::drop_database(&local(), PASSWORD, "dbforge_dump_copy").await.unwrap();
        let mut restored = Vec::new();
        let imported = import(&local(), PASSWORD, "dbforge_dump_copy", &file, &CancellationToken::new(), |l| {
            restored.push(l.to_string())
        })
        .await
        .unwrap();
        assert!(imported.created);
        assert_eq!(count("dbforge_dump_copy").await, 250);
        assert!(restored.iter().any(|l| l.starts_with("COPY 250")), "{restored:?}");

        // A dump that drops what it creates restores over the database it came from.
        let clean = dir.path().join("clean.sql");
        let options = DumpOptions { clean: true, ..ALL };
        export(&local(), PASSWORD, "dbforge_dump_source", &clean, &options, &CancellationToken::new(), |_| {})
            .await
            .unwrap();
        let again =
            import(&local(), PASSWORD, "dbforge_dump_copy", &clean, &CancellationToken::new(), |_| {}).await.unwrap();
        assert!(!again.created);
        assert_eq!(count("dbforge_dump_copy").await, 250);

        let structure = dir.path().join("structure.sql");
        let options = DumpOptions { content: Content::Structure, ..ALL };
        export(&local(), PASSWORD, "dbforge_dump_source", &structure, &options, &CancellationToken::new(), |_| {})
            .await
            .unwrap();
        assert!(!std::fs::read_to_string(&structure).unwrap().contains("COPY dumped.items"));

        for name in ["dbforge_dump_source", "dbforge_dump_copy"] {
            postgres::drop_database(&local(), PASSWORD, name).await.unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432 and its client tools"]
    async fn a_failed_restore_changes_nothing_and_drops_the_database_it_created() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("broken.sql");
        std::fs::write(&file, "CREATE TABLE ok (id int);\nINSERT INTO missing VALUES (1);\n").unwrap();

        postgres::drop_database(&local(), PASSWORD, "dbforge_dump_broken").await.unwrap();
        let error = import(&local(), PASSWORD, "dbforge_dump_broken", &file, &CancellationToken::new(), |_| {})
            .await
            .unwrap_err();
        assert!(error.to_string().contains("relation \"missing\" does not exist"), "{error}");
        assert!(!postgres::database_exists(&local(), PASSWORD, "dbforge_dump_broken").await.unwrap());

        // Into an existing database, the transaction leaves it as it was.
        fresh("dbforge_dump_kept").await;
        import(&local(), PASSWORD, "dbforge_dump_kept", &file, &CancellationToken::new(), |_| {}).await.unwrap_err();
        assert!(postgres::database_exists(&local(), PASSWORD, "dbforge_dump_kept").await.unwrap());
        let url = "postgres://postgres:dbforge@127.0.0.1:55432/dbforge_dump_kept";
        let mut conn = PgConnection::connect(url).await.unwrap();
        let tables: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE tablename = 'ok'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(tables, 0);
        conn.close().await.unwrap();

        let error =
            import(&local(), PASSWORD, "x", &dir.path().join("none.sql"), &CancellationToken::new(), |_| {}).await;
        assert!(error.unwrap_err().to_string().starts_with("There's no file at"));
        postgres::drop_database(&local(), PASSWORD, "dbforge_dump_kept").await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432 and its client tools"]
    async fn a_wrong_password_reads_as_libpq_says_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.sql");
        let error = export(&local(), Some("wrong"), "shop", &file, &ALL, &CancellationToken::new(), |_| {})
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
        export(&local(), Some("wrong"), "shop", &file, &ALL, &CancellationToken::new(), |_| {}).await.unwrap_err();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "-- an older dump");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no partial file is left");

        // Stopped before pg_dump even started.
        let stopped = CancellationToken::new();
        stopped.cancel();
        let error = export(&local(), PASSWORD, "shop", &file, &ALL, &stopped, |_| {}).await.unwrap_err();
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
        let error = import(&local(), PASSWORD, database, &file, &cancel, |_| {}).await.unwrap_err();
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
        postgres::drop_database(&local(), PASSWORD, "dbforge_dump_stopped").await.unwrap();
        let error = stopped_import("dbforge_dump_stopped", std::time::Duration::from_millis(1500)).await;
        assert_eq!(error.to_string(), "Cancelled.");
        assert!(!postgres::database_exists(&local(), PASSWORD, "dbforge_dump_stopped").await.unwrap());

        // Into an existing database: its session ends and the table it began is rolled back.
        fresh("dbforge_dump_stopped_kept").await;
        let error = stopped_import("dbforge_dump_stopped_kept", std::time::Duration::from_millis(1500)).await;
        assert_eq!(error.to_string(), "Cancelled.");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(sessions_on("dbforge_dump_stopped_kept").await, 0);
        postgres::drop_database(&local(), PASSWORD, "dbforge_dump_stopped_kept").await.unwrap();

        // Stopped before psql started: nothing is created.
        postgres::drop_database(&local(), PASSWORD, "dbforge_dump_never").await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.sql");
        std::fs::write(&file, "SELECT 1;").unwrap();
        let stopped = CancellationToken::new();
        stopped.cancel();
        let error = import(&local(), PASSWORD, "dbforge_dump_never", &file, &stopped, |_| {}).await.unwrap_err();
        assert_eq!(error.to_string(), "Cancelled.");
        assert!(!postgres::database_exists(&local(), PASSWORD, "dbforge_dump_never").await.unwrap());
    }
}
