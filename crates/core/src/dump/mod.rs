//! A whole database to a file and back. PostgreSQL, MySQL and MariaDB, and
//! MongoDB use their own client tools, as installed on the computer (see
//! [`tools`]): pg_dump and psql, mysqldump and mysql, mongodump and
//! mongorestore. SQLite needs none: DBForge writes and runs its scripts itself.
//! SQL Server, Azure Cosmos DB and Redis have no standard client dump, so they
//! are refused.
//!
//! The tools get the connection's password where other users of the computer
//! can't read it: in the environment for PostgreSQL's, in a temporary file only
//! its owner reads for the others (see [`secret_file`]), never on a command line.
//!
//! An export is written to a file beside the chosen one first, which takes its
//! place only once the export succeeds: a failed or cancelled one leaves the
//! chosen file as it was and no partial file. A restore into a database it
//! created removes that database again when it fails.

mod exports;
mod extensions;
mod mongodb;
mod mysql;
mod postgres;
mod secret_file;
mod sqlite;
mod tool_paths;
mod tools;

pub use exports::{
    ExportFile, ExportFiles, ExportFolder, default_export_folder, export_extension, export_file_name, list_exports,
    sqlite_name,
};
pub use mongodb::MongodbOptions;
pub use mysql::MysqlOptions;
pub use postgres::PostgresOptions;
pub use sqlite::{SqliteFormat, SqliteOptions};
pub use tool_paths::ToolPaths;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use self::tools::{Flavour, Tool};
use crate::connections::model::{Connection, Engine};
use crate::connections::store::ConnectionStore;
use crate::error::{AppError, Result};
use crate::events::{Event, Events};
use crate::files::partial_path;
use crate::jobs::{Every, Jobs, cancelled};
use crate::statement_log::{Origin, StatementLog};

/// The engines whose databases DBForge exports and restores, each its own way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpEngine {
    Postgres,
    /// MySQL and MariaDB, with the same tools.
    Mysql,
    Mongodb,
    /// Built in: no tool to install.
    Sqlite,
}

impl DumpEngine {
    pub const ALL: [DumpEngine; 4] = [DumpEngine::Postgres, DumpEngine::Mysql, DumpEngine::Mongodb, DumpEngine::Sqlite];

    /// How `engine`'s databases are dumped, or `None` when they aren't.
    pub fn of(engine: Engine) -> Option<Self> {
        match engine {
            Engine::Postgres => Some(Self::Postgres),
            Engine::Mysql | Engine::Mariadb => Some(Self::Mysql),
            Engine::Mongodb => Some(Self::Mongodb),
            Engine::Sqlite => Some(Self::Sqlite),
            Engine::Sqlserver | Engine::Redis | Engine::Cosmosdb => None,
        }
    }

    /// The name settings group its tools under.
    pub fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::Mysql => "MySQL and MariaDB",
            Self::Mongodb => "MongoDB",
            Self::Sqlite => "SQLite",
        }
    }

    /// The client tools it runs, export's first: none for SQLite.
    pub fn tools(self) -> &'static [&'static str] {
        match self {
            Self::Postgres => &["pg_dump", "psql"],
            Self::Mysql => &["mysqldump", "mysql"],
            Self::Mongodb => &["mongodump", "mongorestore"],
            Self::Sqlite => &[],
        }
    }

    fn export_tool(self) -> Option<&'static str> {
        self.tools().first().copied()
    }

    fn import_tool(self) -> Option<&'static str> {
        self.tools().get(1).copied()
    }

    /// The extension of the files it restores, dot included: its exports'
    /// (SQLite restores scripts, not copies of a database file).
    pub fn restore_extension(self) -> &'static str {
        match self {
            Self::Postgres | Self::Mysql | Self::Sqlite => ".sql",
            Self::Mongodb => ".archive.gz",
        }
    }
}

/// Whether DBForge exports and imports whole databases of `engine`.
pub fn supports_dumps(engine: Engine) -> bool {
    DumpEngine::of(engine).is_some()
}

/// The client tools DBForge runs for `engine`'s dumps and restores; none when
/// it needs none (SQLite) or has no dumps.
pub fn dump_tools(engine: Engine) -> &'static [&'static str] {
    DumpEngine::of(engine).map_or(&[], DumpEngine::tools)
}

/// How `connection`'s databases are dumped, or why they aren't.
fn engine_of(connection: &Connection) -> Result<DumpEngine> {
    DumpEngine::of(connection.engine).ok_or(AppError::Unsupported(match connection.engine {
        Engine::Sqlserver => "Exporting and importing whole SQL Server databases",
        Engine::Redis => "Exporting and importing whole Redis databases",
        _ => "Exporting and importing whole Azure Cosmos DB databases",
    }))
}

/// What a dump holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Content {
    All,
    Structure,
    Data,
}

/// How to export, for the connection's engine (`"engine": "postgres"`, …, in JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "engine", rename_all = "camelCase")]
pub enum DumpOptions {
    Postgres(PostgresOptions),
    /// MySQL and MariaDB.
    Mysql(MysqlOptions),
    Mongodb(MongodbOptions),
    Sqlite(SqliteOptions),
}

impl DumpOptions {
    /// What a new export of `engine` starts with: everything, read consistently.
    pub fn defaults(engine: DumpEngine) -> Self {
        match engine {
            DumpEngine::Postgres => {
                Self::Postgres(PostgresOptions { content: Content::All, clean: false, keep_owners: false })
            }
            DumpEngine::Mysql => Self::Mysql(MysqlOptions {
                content: Content::All,
                add_drop_table: false,
                single_transaction: true,
                routines: true,
            }),
            DumpEngine::Mongodb => Self::Mongodb(MongodbOptions::default()),
            DumpEngine::Sqlite => Self::Sqlite(SqliteOptions { format: SqliteFormat::Script }),
        }
    }

    pub fn engine(&self) -> DumpEngine {
        match self {
            Self::Postgres(_) => DumpEngine::Postgres,
            Self::Mysql(_) => DumpEngine::Mysql,
            Self::Mongodb(_) => DumpEngine::Mongodb,
            Self::Sqlite(_) => DumpEngine::Sqlite,
        }
    }

    /// The extension of the file the export writes, dot included.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Postgres(_) | Self::Mysql(_) => ".sql",
            Self::Mongodb(_) => ".archive.gz",
            Self::Sqlite(options) => match options.format {
                SqliteFormat::Script => ".sql",
                SqliteFormat::File => ".db",
            },
        }
    }
}

/// How to restore. What doesn't apply to the connection's engine is ignored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RestoreOptions {
    /// MongoDB: drops each collection the archive holds before restoring it.
    pub drop_existing: bool,
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
    /// The database (a SQLite file) didn't exist and was created for the restore.
    pub created: bool,
    pub elapsed_ms: u64,
    /// Extensions the file creates that the server doesn't have, so the restore left them out (PostgreSQL).
    pub skipped_extensions: Vec<String>,
}

/// What an engine's restore did.
struct Restored {
    created: bool,
    skipped_extensions: Vec<String>,
}

impl Restored {
    fn created(created: bool) -> Self {
        Self { created, skipped_extensions: Vec::new() }
    }
}

/// Writes `database` to `path`: the connection's whole file for SQLite. The
/// dump goes to a file beside it first, so a failed or cancelled one leaves
/// `path` as it was.
pub async fn export(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    path: &Path,
    options: &DumpOptions,
    run: Run<'_>,
    on_line: impl FnMut(&str),
) -> Result<Exported> {
    let engine = engine_of(connection)?;
    if options.engine() != engine {
        return Err(AppError::Invalid(format!(
            "These export options are for {}; the connection is {}.",
            options.engine().label(),
            connection.engine.label()
        )));
    }
    let started = Instant::now();
    // The default export folder may not exist yet.
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = partial_path(path);
    // One left by a run that couldn't clean up would be in the way of tools that won't overwrite.
    let _ = std::fs::remove_file(&partial);
    let outcome = match options {
        DumpOptions::Postgres(options) => {
            postgres::dump(connection, password, database, &partial, options, run, on_line).await
        }
        DumpOptions::Mysql(options) => {
            mysql::dump(connection, password, database, &partial, options, run, on_line).await
        }
        DumpOptions::Mongodb(options) => {
            mongodb::dump(connection, password, database, &partial, options, run, on_line).await
        }
        DumpOptions::Sqlite(options) => sqlite::dump(connection, &partial, options, run.cancel, on_line).await,
    };
    if let Err(error) = outcome {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    std::fs::rename(&partial, path)?;
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or_default();
    Ok(Exported { bytes, elapsed_ms: started.elapsed().as_millis() as u64 })
}

/// Restores the file at `path` into `database` (for SQLite, the file to
/// restore into: the connection's own, or a new one), creating it first when
/// it doesn't exist.
pub async fn import(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    path: &Path,
    options: &RestoreOptions,
    run: Run<'_>,
    on_line: impl FnMut(&str),
) -> Result<Imported> {
    let engine = engine_of(connection)?;
    let database = database.trim();
    if database.is_empty() {
        return Err(AppError::Invalid("Name the database to restore into.".into()));
    }
    if !path.is_file() {
        return Err(AppError::Invalid(format!("There's no file at {}.", path.display())));
    }
    let started = Instant::now();
    let restored = match engine {
        DumpEngine::Postgres => postgres::restore(connection, password, database, path, run, on_line).await?,
        DumpEngine::Mysql => mysql::restore(connection, password, database, path, run, on_line).await?,
        DumpEngine::Mongodb => mongodb::restore(connection, password, database, path, options, run, on_line).await?,
        DumpEngine::Sqlite => sqlite::restore(connection, database, path, run.cancel, on_line).await?,
    };
    Ok(Imported {
        created: restored.created,
        elapsed_ms: started.elapsed().as_millis() as u64,
        skipped_extensions: restored.skipped_extensions,
    })
}

/// Runs file work off the async threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> std::io::Result<T> + Send + 'static) -> Result<T> {
    Ok(tokio::task::spawn_blocking(work).await.map_err(|error| AppError::Invalid(error.to_string()))??)
}

/// An error unless `cancel` was cancelled.
fn stop_if(cancel: &CancellationToken) -> Result<()> {
    if cancel.is_cancelled() { Err(cancelled()) } else { Ok(()) }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub job_id: String,
    pub connection_id: String,
    /// Ignored for SQLite, whose connection is one database.
    pub database: String,
    pub path: String,
    pub options: DumpOptions,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub job_id: String,
    pub connection_id: String,
    /// The database to restore into; for SQLite, the file: the connection's own or a new one.
    pub database: String,
    pub path: String,
    #[serde(default)]
    pub options: RestoreOptions,
}

/// The dialog shows the last line the tool wrote; psql writes one per
/// statement, so lines are sent this often at most, not one event for each.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// The tool `name` for `connection`, or `None` when its engine needs none.
async fn tool_for(name: Option<&str>, chosen: &ToolPaths, connection: &Connection) -> Result<Option<Tool>> {
    let Some(name) = name else { return Ok(None) };
    let flavour = if connection.engine == Engine::Mariadb { Flavour::Mariadb } else { Flavour::Standard };
    Ok(Some(tools::resolve(name, chosen.get(name).as_deref(), flavour).await?))
}

pub async fn export_database(
    events: &dyn Events,
    store: &ConnectionStore,
    jobs: &Jobs,
    chosen: &ToolPaths,
    log: &StatementLog,
    request: ExportRequest,
) -> Result<Exported> {
    let opened = store.open(&request.connection_id)?;
    let engine = engine_of(&opened.connection)?;
    let tool = tool_for(engine.export_tool(), chosen, &opened.connection).await?;
    let job = jobs.start(&request.job_id);
    let mut every = Every::new(PROGRESS_EVERY);
    let report = |line: &str| {
        if every.due() {
            events.emit(Event::DumpProgress { job_id: request.job_id.clone(), line: line.to_owned() });
        }
    };
    let path = Path::new(&request.path);
    let run = Run { tool: tool.as_ref(), cancel: job.token() };
    opened
        .run(log, Origin::Dump, async |connection, password| {
            export(connection, password, &request.database, path, &request.options, run, report).await
        })
        .await
}

pub async fn import_database(
    events: &dyn Events,
    store: &ConnectionStore,
    jobs: &Jobs,
    chosen: &ToolPaths,
    log: &StatementLog,
    request: ImportRequest,
) -> Result<Imported> {
    let opened = store.open(&request.connection_id)?;
    let engine = engine_of(&opened.connection)?;
    let tool = tool_for(engine.import_tool(), chosen, &opened.connection).await?;
    let job = jobs.start(&request.job_id);
    let mut every = Every::new(PROGRESS_EVERY);
    let report = |line: &str| {
        if every.due() {
            events.emit(Event::DumpProgress { job_id: request.job_id.clone(), line: line.to_owned() });
        }
    };
    let path = Path::new(&request.path);
    let run = Run { tool: tool.as_ref(), cancel: job.token() };
    opened
        .run(log, Origin::Dump, async |connection, password| {
            import(connection, password, &request.database, path, &request.options, run, report).await
        })
        .await
}

/// How an export or import runs: with which tool (none for SQLite), and what stops it.
#[derive(Clone, Copy)]
pub struct Run<'a> {
    pub tool: Option<&'a Tool>,
    pub cancel: &'a CancellationToken,
}

impl<'a> Run<'a> {
    /// The tool, which every engine but SQLite needs.
    fn tool(&self) -> Result<&'a Tool> {
        self.tool.ok_or_else(|| AppError::Invalid("No client tool was found to run.".into()))
    }
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
    /// MariaDB's build of the tool (mariadb-dump, or a mysqldump MariaDB ships).
    mariadb: bool,
}

impl ToolInfo {
    /// Where the program is.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// `major.minor`.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Chosen by the user rather than found.
    pub fn custom(&self) -> bool {
        self.custom
    }

    /// MariaDB's build of the tool.
    pub fn mariadb(&self) -> bool {
        self.mariadb
    }
}

impl From<Tool> for ToolInfo {
    fn from(tool: Tool) -> Self {
        Self {
            path: tool.path.display().to_string(),
            version: format!("{}.{}", tool.version.0, tool.version.1),
            custom: tool.custom,
            mariadb: tool.mariadb,
        }
    }
}

/// Which tool `name` (`pg_dump`, `mysqldump`, `mongorestore`, …) DBForge will
/// run, or why there is none to run. Of MySQL's and MariaDB's builds, MySQL's
/// is shown; a MariaDB connection prefers MariaDB's when both are installed.
pub async fn dump_tool(chosen: &ToolPaths, name: String) -> Result<ToolInfo> {
    let name = tools::check_name(&name)?;
    Ok(tools::resolve(name, chosen.get(name).as_deref(), Flavour::Standard).await?.into())
}

/// Runs `path` for `name` from now on, once it proves to be that tool; `None` goes back to the one found.
pub async fn set_dump_tool(chosen: &ToolPaths, name: String, path: Option<String>) -> Result<ToolInfo> {
    let name = tools::check_name(&name)?;
    let path = path.map(PathBuf::from);
    // Refused before it is saved: a path that doesn't run would only fail later.
    let tool = tools::resolve(name, path.as_deref(), Flavour::Standard).await?;
    chosen.set(name, path)?;
    Ok(tool.into())
}

/// Whether a file is already at `path`, so the export dialog can ask before replacing it.
pub fn dump_file_exists(path: String) -> bool {
    Path::new(&path).exists()
}

/// How many exports are in `folder` and how much they take.
pub fn export_folder_usage(folder: String) -> Result<exports::ExportFiles> {
    exports::usage(Path::new(&folder))
}

/// Deletes the exports in `folder` (other files stay), and says what was deleted.
pub fn clear_export_folder(folder: String) -> Result<exports::ExportFiles> {
    exports::clear(Path::new(&folder))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn progress_is_sent_at_most_once_per_period() {
        let mut every = Every::new(Duration::from_millis(50));
        assert!(every.due(), "the first line goes at once");
        assert!(!every.due() && !every.due(), "the lines right after it wait");
        std::thread::sleep(Duration::from_millis(60));
        assert!(every.due());
        assert!(!every.due());
    }

    #[test]
    fn dumps_are_for_the_engines_with_a_way_to_make_them() {
        for engine in [Engine::Postgres, Engine::Mysql, Engine::Mariadb, Engine::Mongodb, Engine::Sqlite] {
            assert!(supports_dumps(engine), "{engine:?}");
        }
        for engine in [Engine::Sqlserver, Engine::Redis, Engine::Cosmosdb] {
            assert!(!supports_dumps(engine), "{engine:?}");
            assert!(dump_tools(engine).is_empty());
        }
        assert_eq!(dump_tools(Engine::Postgres), ["pg_dump", "psql"]);
        assert_eq!(dump_tools(Engine::Mysql), ["mysqldump", "mysql"]);
        assert_eq!(dump_tools(Engine::Mariadb), ["mysqldump", "mysql"]);
        assert_eq!(dump_tools(Engine::Mongodb), ["mongodump", "mongorestore"]);
        assert!(dump_tools(Engine::Sqlite).is_empty());
        // Every tool listed is one DBForge knows how to find.
        for engine in DumpEngine::ALL {
            for tool in engine.tools() {
                assert_eq!(tools::check_name(tool).unwrap(), *tool);
            }
        }
    }

    #[tokio::test]
    async fn other_engines_and_options_for_another_engine_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.sql");
        let cancel = CancellationToken::new();
        let run = Run { tool: None, cancel: &cancel };
        let sqlserver = Connection { engine: Engine::Sqlserver, ..crate::drivers::postgres::tests::local() };
        let options = DumpOptions::Sqlite(SqliteOptions { format: SqliteFormat::Script });
        let error = export(&sqlserver, None, "x", &file, &options, run, |_| {}).await.unwrap_err();
        assert!(matches!(error, AppError::Unsupported(_)), "{error:?}");
        assert_eq!(error.to_string(), "Exporting and importing whole SQL Server databases is not supported yet");
        std::fs::write(&file, "SELECT 1;").unwrap();
        let error = import(&sqlserver, None, "x", &file, &RestoreOptions::default(), run, |_| {}).await.unwrap_err();
        assert!(matches!(error, AppError::Unsupported(_)), "{error:?}");

        let postgres = crate::drivers::postgres::tests::local();
        let error = export(&postgres, None, "x", &file, &options, run, |_| {}).await.unwrap_err();
        assert_eq!(error.to_string(), "These export options are for SQLite; the connection is PostgreSQL.");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "SELECT 1;", "the file is left alone");
    }

    #[test]
    fn options_name_their_engine_and_the_file_extension() {
        let json = |text: &str| serde_json::from_str::<DumpOptions>(text).unwrap();
        let postgres = json(r#"{"engine":"postgres","content":"all","clean":false,"keepOwners":true}"#);
        assert!(matches!(postgres, DumpOptions::Postgres(PostgresOptions { keep_owners: true, .. })));
        let mysql = json(
            r#"{"engine":"mysql","content":"structure","addDropTable":true,"singleTransaction":true,"routines":false}"#,
        );
        assert_eq!(mysql.extension(), ".sql");
        assert!(matches!(mysql, DumpOptions::Mysql(MysqlOptions { content: Content::Structure, .. })));
        let mongodb = json(r#"{"engine":"mongodb","collections":["orders"]}"#);
        assert_eq!(mongodb.extension(), ".archive.gz");
        assert_eq!(json(r#"{"engine":"sqlite","format":"file"}"#).extension(), ".db");
        assert_eq!(json(r#"{"engine":"sqlite","format":"script"}"#).extension(), ".sql");

        let request: ImportRequest =
            serde_json::from_str(r#"{"jobId":"j","connectionId":"c","database":"d","path":"p"}"#).unwrap();
        assert!(!request.options.drop_existing, "options default when left out");
    }

    /// Exports `database` of `connection` (PostgreSQL) to `file`, everything included.
    pub(crate) async fn export_to(
        connection: &Connection,
        password: Option<&str>,
        database: &str,
        file: &Path,
    ) -> Result<Exported> {
        let options =
            DumpOptions::Postgres(PostgresOptions { content: Content::All, clean: false, keep_owners: false });
        let tool = postgres::tests::pg_dump().await;
        let run = Run { tool: Some(&tool), cancel: &CancellationToken::new() };
        export(connection, password, database, file, &options, run, |_| {}).await
    }
}
