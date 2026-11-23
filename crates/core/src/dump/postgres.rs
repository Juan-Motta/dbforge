//! PostgreSQL, with its own tools: pg_dump writes the file, psql restores it.
//! They get the connection's details and password in their environment, never
//! on their command line, where other users could read them.
//!
//! A restore runs in one transaction and stops at the first error, so a failed
//! one changes nothing; a database created for it is dropped again. Extensions
//! the file creates that the server doesn't have are left out (see [`super::extensions`]).

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::{Content, Restored, Run, blocking, extensions, tools};
use crate::connections::model::Connection;
use crate::connections::transport::SslMode;
use crate::drivers::postgres;
use crate::error::{AppError, Result};
use crate::jobs::cancelled;
use crate::tunnel::Route;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostgresOptions {
    pub content: Content,
    /// Drops each object before creating it, so the file restores over an existing database.
    pub clean: bool,
    /// Keeps `ALTER … OWNER` and `GRANT`, which need the same roles on the server restored to.
    pub keep_owners: bool,
}

/// Writes `database` to `file` as plain SQL with pg_dump.
pub(super) async fn dump(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    file: &Path,
    options: &PostgresOptions,
    run: Run<'_>,
    on_line: impl FnMut(&str),
) -> Result<()> {
    let tool = run.tool()?;
    let session = session_name();
    let mut command = Command::new(&tool.path);
    let route = crate::tunnel::route(connection).await?;
    connect_with(&mut command, connection, &route, password, database, &session);
    command.args(["--format=plain", "--encoding=UTF8", "--verbose", "--no-password", "--file"]).arg(file);
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
    if let Err(error) = tools::run(command, run.cancel, on_line).await {
        if run.cancel.is_cancelled() {
            postgres::end_sessions(connection, password, &session).await?;
        }
        return Err(with_version_hint(error));
    }
    Ok(())
}

/// Runs the SQL file at `path` into `database` with psql, creating it first
/// when it doesn't exist. A cancel is checked between steps, so one that comes
/// before psql starts still stops the import.
pub(super) async fn restore(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    path: &Path,
    run: Run<'_>,
    on_line: impl FnMut(&str),
) -> Result<Restored> {
    let tool = run.tool()?;
    let cancel = run.cancel;
    let stop = || if cancel.is_cancelled() { Err(cancelled()) } else { Ok(()) };
    stop()?;
    let script = Script::of(connection, password, path).await?;
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
        .arg(script.path(path));
    let Err(error) = tools::run(command, cancel, on_line).await.map_err(|error| script.as_written(error, path)) else {
        return Ok(Restored { created, skipped_extensions: script.skipped });
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

/// The script psql runs: the file itself, or a copy without the extensions the
/// server doesn't have.
struct Script {
    copy: Option<tempfile::NamedTempFile>,
    skipped: Vec<String>,
}

impl Script {
    async fn of(connection: &Connection, password: Option<&str>, path: &Path) -> Result<Self> {
        let file = path.to_path_buf();
        let requested = blocking(move || extensions::requested(&file)).await?;
        if requested.is_empty() {
            return Ok(Self { copy: None, skipped: Vec::new() });
        }
        let available = postgres::available_extensions(connection, password).await?;
        let missing: BTreeSet<String> = requested.into_iter().filter(|name| !available.contains(name)).collect();
        if missing.is_empty() {
            return Ok(Self { copy: None, skipped: Vec::new() });
        }
        let file = path.to_path_buf();
        let left_out = missing.clone();
        let copy = blocking(move || extensions::without(&file, &left_out)).await?;
        Ok(Self { copy: Some(copy), skipped: missing.into_iter().collect() })
    }

    fn path<'a>(&'a self, original: &'a Path) -> &'a Path {
        self.copy.as_ref().map_or(original, |copy| copy.path())
    }

    /// psql names the script it ran in its errors: the original file, not the copy.
    fn as_written(&self, error: AppError, original: &Path) -> AppError {
        match (&self.copy, error) {
            (Some(copy), AppError::Invalid(text)) => {
                AppError::Invalid(text.replace(&copy.path().display().to_string(), &original.display().to_string()))
            }
            (_, error) => error,
        }
    }
}

/// The `application_name` a tool's sessions carry, unique per run, so a
/// cancel can find them on the server.
fn session_name() -> String {
    format!("DBForge {}", uuid::Uuid::new_v4())
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

/// Runs against the local server and the client tools installed. Start the server and run
/// `cargo test -- --ignored`; see `drivers::postgres::tests` for the container.
#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use sqlx::{Connection as _, PgConnection};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::drivers::postgres::tests::{PASSWORD, local};
    use crate::dump::tools::{Flavour, Tool};
    use crate::dump::{DumpOptions, Imported, RestoreOptions, dump_file_exists, export};

    const PG: PostgresOptions = PostgresOptions { content: Content::All, clean: false, keep_owners: false };
    const ALL: DumpOptions = DumpOptions::Postgres(PG);

    /// A restore with the default options.
    async fn import(
        connection: &Connection,
        password: Option<&str>,
        database: &str,
        path: &Path,
        run: Run<'_>,
        on_line: impl FnMut(&str),
    ) -> Result<Imported> {
        crate::dump::import(connection, password, database, path, &RestoreOptions::default(), run, on_line).await
    }

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
            Run { tool: Some(&pg_dump().await), cancel: &CancellationToken::new() },
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
            let run = Run { tool: Some(&pg_dump().await), cancel: &CancellationToken::new() };
            export(&connection, None, "postgres", &file, &ALL, run, |_| {}).await
        };
        assert!(dump(certuser(trusting(SslMode::VerifyFull), "localhost")).await.is_err(), "no certificate");
        let error = dump(certuser(as_certuser(SslMode::VerifyFull), "127.0.0.1")).await.unwrap_err().to_string();
        assert!(error.contains("127.0.0.1"), "the name is checked: {error}");
        dump(certuser(as_certuser(SslMode::VerifyFull), "localhost")).await.unwrap();
        assert!(file.exists());
    }

    pub(crate) async fn pg_dump() -> Tool {
        tools::find("pg_dump", Flavour::Standard).await.unwrap()
    }

    async fn psql() -> Tool {
        tools::find("psql", Flavour::Standard).await.unwrap()
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
            Run { tool: Some(&pg_dump().await), cancel: &CancellationToken::new() },
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
            Run { tool: Some(&psql().await), cancel: &CancellationToken::new() },
            |l| restored.push(l.to_string()),
        )
        .await
        .unwrap();
        assert!(imported.created);
        assert_eq!(count("dbforge_dump_copy").await, 250);
        assert!(restored.iter().any(|l| l.starts_with("COPY 250")), "{restored:?}");

        // A dump that drops what it creates restores over the database it came from.
        let clean = dir.path().join("clean.sql");
        let options = DumpOptions::Postgres(PostgresOptions { clean: true, ..PG });
        export(
            &local(),
            PASSWORD,
            "dbforge_dump_source",
            &clean,
            &options,
            Run { tool: Some(&pg_dump().await), cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap();
        let again = import(
            &local(),
            PASSWORD,
            "dbforge_dump_copy",
            &clean,
            Run { tool: Some(&psql().await), cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap();
        assert!(!again.created);
        assert_eq!(count("dbforge_dump_copy").await, 250);

        let structure = dir.path().join("structure.sql");
        let options = DumpOptions::Postgres(PostgresOptions { content: Content::Structure, ..PG });
        export(
            &local(),
            PASSWORD,
            "dbforge_dump_source",
            &structure,
            &options,
            Run { tool: Some(&pg_dump().await), cancel: &CancellationToken::new() },
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
            Run { tool: Some(&psql().await), cancel: &CancellationToken::new() },
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
            Run { tool: Some(&psql().await), cancel: &CancellationToken::new() },
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
            Run { tool: Some(&psql().await), cancel: &CancellationToken::new() },
            |_| {},
        )
        .await;
        assert!(error.unwrap_err().to_string().starts_with("There's no file at"));
        postgres::force_drop_database(&local(), PASSWORD, "dbforge_dump_kept").await.unwrap();
    }

    /// As a dump of a managed server reads: its provider's extension, then one any server has.
    const WITH_EXTENSIONS: &str = "\
--
-- Name: dbforge_missing; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS dbforge_missing WITH SCHEMA pg_catalog;

--
-- Name: EXTENSION dbforge_missing; Type: COMMENT; Schema: -; Owner: -
--

COMMENT ON EXTENSION dbforge_missing IS 'Only on another server';

--
-- Name: plpgsql; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS plpgsql WITH SCHEMA pg_catalog;

CREATE TABLE kept (id int);
";

    #[tokio::test]
    #[ignore = "needs a local PostgreSQL on port 55432 and its client tools"]
    async fn leaves_out_extensions_the_server_lacks_and_errors_point_at_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("managed.sql");
        let restore = |database: &'static str, file: PathBuf| async move {
            let tool = psql().await;
            let run = Run { tool: Some(&tool), cancel: &CancellationToken::new() };
            import(&local(), PASSWORD, database, &file, run, |_| {}).await
        };

        std::fs::write(&file, WITH_EXTENSIONS).unwrap();
        fresh("dbforge_dump_extensions").await;
        let imported = restore("dbforge_dump_extensions", file.clone()).await.unwrap();
        assert_eq!(imported.skipped_extensions, ["dbforge_missing"]);
        sql("dbforge_dump_extensions", "SELECT * FROM kept").await;

        // Line 20 of the file, though the copy psql ran had lines blanked.
        std::fs::write(&file, format!("{WITH_EXTENSIONS}INSERT INTO missing VALUES (1);\n")).unwrap();
        fresh("dbforge_dump_extensions").await;
        let error = restore("dbforge_dump_extensions", file.clone()).await.unwrap_err().to_string();
        assert!(error.contains(&format!("psql:{}:20: ERROR:", file.display())), "{error}");
        postgres::force_drop_database(&local(), PASSWORD, "dbforge_dump_extensions").await.unwrap();
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
            Run { tool: Some(&pg_dump().await), cancel: &CancellationToken::new() },
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
            Run { tool: Some(&pg_dump().await), cancel: &CancellationToken::new() },
            |_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "-- an older dump");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no partial file is left");

        // Stopped before pg_dump even started.
        let stopped = CancellationToken::new();
        stopped.cancel();
        let error = export(
            &local(),
            PASSWORD,
            "shop",
            &file,
            &ALL,
            Run { tool: Some(&pg_dump().await), cancel: &stopped },
            |_| {},
        )
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
        let error =
            import(&local(), PASSWORD, database, &file, Run { tool: Some(&psql().await), cancel: &cancel }, |_| {})
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
            Run { tool: Some(&psql().await), cancel: &stopped },
            |_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "Cancelled.");
        assert!(!postgres::database_exists(&local(), PASSWORD, "dbforge_dump_never").await.unwrap());
    }
}
