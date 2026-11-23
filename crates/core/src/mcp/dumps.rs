//! Backups and restores of whole databases, for agents: a backup writes the
//! database into DBForge's exports folder under a name DBForge gives it, and a
//! restore reads only an export from that folder. The agent names no path.
//!
//! Both always wait for the user to approve them in the app, whatever the
//! connection's tag: a backup copies every row out of the database, a restore
//! changes its data. A backup needs agents' Read access to the connection, a
//! restore Read and write. They run as the app's own do ([`dump`]): logged as
//! dumps, cancellable through the job registry, and a database a failed
//! restore created is dropped again. Once approved, a run goes on to its end
//! even if the agent goes away, so it cleans up after itself as it would in the app.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Local, SecondsFormat, Utc};
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

use super::approvals::{ApprovalRequest, ApprovalSubject, DumpAction, DumpApproval};
use super::connections::Meanwhile;
use super::tools::Tools;
use crate::connections::model::{AgentAccess, Connection, Engine};
use crate::drivers;
use crate::dump::{
    self, Content, DumpEngine, DumpOptions, ExportRequest, ImportRequest, RestoreOptions, SqliteFormat,
    export_extension, export_file_name, list_exports, sqlite_name,
};

/// What the agent hears when the user says no to a backup, or nothing in time.
const DECLINED_BACKUP: &str = "The user declined this backup in DBForge, or didn't answer in time. Nothing ran.";
/// What the agent hears when the user says no to a restore, or nothing in time.
const DECLINED_RESTORE: &str = "The user declined this restore in DBForge, or didn't answer in time. Nothing ran.";

/// What the agent hears when it names a database for a SQLite connection.
const SQLITE_DATABASE: &str =
    "database is for server engines; a SQLite connection backs up and restores its own file. Leave it out.";

/// What the agent hears when the connection changed while the user was asked.
fn changed_meanwhile(name: &str) -> String {
    format!(
        "The connection {name} changed while waiting for approval, so nothing ran. Call list_connections and try again."
    )
}

/// What the agent hears when the connection was deleted while the user was asked.
fn deleted_meanwhile(name: &str) -> String {
    format!("The connection {name} was deleted while waiting for approval, so nothing ran.")
}

/// How long the restore's approval waits to learn whether the target database exists.
const LOOKUP_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListExportsRequest {
    /// Only the exports `import_database` can restore into this connection (from `list_connections`).
    pub connection_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportList {
    pub exports: Vec<ExportView>,
}

/// An export in DBForge's exports folder.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportView {
    /// What `import_database` takes as `file`.
    pub name: String,
    /// `sql`: a SQL script (PostgreSQL, MySQL, MariaDB or SQLite). `archive`: a
    /// gzipped MongoDB archive. `sqliteFile`: a copy of a SQLite database file,
    /// which opens as a connection rather than being restored.
    pub kind: String,
    /// The engines whose connections can restore it; none for a SQLite file copy.
    pub restores_into: Vec<String>,
    pub bytes: u64,
    /// When it was last written, RFC 3339 in UTC.
    pub modified: Option<String>,
}

/// What a dump holds.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ContentInput {
    All,
    Structure,
    Data,
}

impl From<ContentInput> for Content {
    fn from(content: ContentInput) -> Self {
        match content {
            ContentInput::All => Content::All,
            ContentInput::Structure => Content::Structure,
            ContentInput::Data => Content::Data,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SqliteFormatInput {
    /// A SQL script of the schema and the data, which `import_database` restores.
    Script,
    /// A copy of the database file.
    File,
}

/// How to back up; each option is for the engines it names, and one given for
/// another engine is refused. What is left out takes the default.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportOptionsInput {
    /// PostgreSQL, MySQL and MariaDB: `all` (default), `structure` or `data`.
    pub content: Option<ContentInput>,
    /// PostgreSQL: drop each object before creating it. Default false.
    pub clean: Option<bool>,
    /// PostgreSQL: keep owners and privileges. Default false.
    pub keep_owners: Option<bool>,
    /// MySQL and MariaDB: `DROP TABLE IF EXISTS` before each table. Default false.
    pub add_drop_table: Option<bool>,
    /// MySQL and MariaDB: read InnoDB tables in one consistent snapshot. Default true.
    pub single_transaction: Option<bool>,
    /// MySQL and MariaDB: stored procedures and functions, triggers and events. Default true.
    pub routines: Option<bool>,
    /// MongoDB: only these collections. Default: the whole database.
    pub collections: Option<Vec<String>>,
    /// SQLite: `script` (default) or `file`.
    pub format: Option<SqliteFormatInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportDatabaseRequest {
    /// From `list_connections`.
    pub connection_id: String,
    /// The database to back up; the connection's own when left out. Not for
    /// SQLite, whose connection is one database file.
    pub database: Option<String>,
    pub options: Option<ExportOptionsInput>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportedView {
    /// The export's name in DBForge's exports folder, which `import_database` takes as `file`.
    pub file: String,
    pub bytes: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportDatabaseRequest {
    /// From `list_connections`; agents need Read and write access to it.
    pub connection_id: String,
    /// The name of an export in DBForge's exports folder, from `list_exports`: no folder, just the name.
    pub file: String,
    /// The database to restore into, created when it doesn't exist; needed
    /// except for SQLite, which restores into the connection's own file and takes none.
    pub database: Option<String>,
    /// MongoDB only: drop each collection the archive holds before restoring it. Default false.
    pub drop_existing: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportedView {
    pub file: String,
    /// The database restored into; for SQLite, the connection's file's name.
    pub database: String,
    /// The database didn't exist and was created for the restore.
    pub created: bool,
    pub elapsed_ms: u64,
    /// PostgreSQL: extensions the file creates that the server doesn't have, so the restore left them out.
    pub skipped_extensions: Vec<String>,
}

/// How `connection`'s databases are backed up, or why they aren't.
fn engine_of(connection: &Connection) -> Result<DumpEngine, String> {
    DumpEngine::of(connection.engine)
        .ok_or_else(|| format!("DBForge can't back up or restore whole {} databases.", connection.engine.label()))
}

/// What an export's file holds, and the engines that restore it.
fn kind_of(extension: &str) -> (&'static str, Vec<String>) {
    let engines = |engines: &[Engine]| engines.iter().map(|engine| engine.label().to_owned()).collect();
    match extension {
        ".archive.gz" => ("archive", engines(&[Engine::Mongodb])),
        ".db" => ("sqliteFile", Vec::new()),
        _ => ("sql", engines(&[Engine::Postgres, Engine::Mysql, Engine::Mariadb, Engine::Sqlite])),
    }
}

/// `input` over `engine`'s defaults; an option given for another engine is refused.
fn options_for(engine: DumpEngine, label: &str, input: Option<ExportOptionsInput>) -> Result<DumpOptions, String> {
    let mut options = DumpOptions::defaults(engine);
    let Some(input) = input else { return Ok(options) };
    let ExportOptionsInput {
        content,
        clean,
        keep_owners,
        add_drop_table,
        single_transaction,
        routines,
        collections,
        format,
    } = input;
    // The options given, by name, that this engine's backup has no use for.
    let mut unused = Vec::new();
    let mut not_for = |name: &'static str, given: bool| {
        if given {
            unused.push(name);
        }
    };
    match &mut options {
        DumpOptions::Postgres(postgres) => {
            postgres.content = content.map_or(postgres.content, Into::into);
            postgres.clean = clean.unwrap_or(postgres.clean);
            postgres.keep_owners = keep_owners.unwrap_or(postgres.keep_owners);
            not_for("addDropTable", add_drop_table.is_some());
            not_for("singleTransaction", single_transaction.is_some());
            not_for("routines", routines.is_some());
            not_for("collections", collections.is_some());
            not_for("format", format.is_some());
        }
        DumpOptions::Mysql(mysql) => {
            mysql.content = content.map_or(mysql.content, Into::into);
            mysql.add_drop_table = add_drop_table.unwrap_or(mysql.add_drop_table);
            mysql.single_transaction = single_transaction.unwrap_or(mysql.single_transaction);
            mysql.routines = routines.unwrap_or(mysql.routines);
            not_for("clean", clean.is_some());
            not_for("keepOwners", keep_owners.is_some());
            not_for("collections", collections.is_some());
            not_for("format", format.is_some());
        }
        DumpOptions::Mongodb(mongodb) => {
            let collections = collections.unwrap_or_default();
            if collections.iter().any(|name| name.trim().is_empty()) {
                return Err("A collection's name can't be empty.".into());
            }
            mongodb.collections = collections;
            not_for("content", content.is_some());
            not_for("clean", clean.is_some());
            not_for("keepOwners", keep_owners.is_some());
            not_for("addDropTable", add_drop_table.is_some());
            not_for("singleTransaction", single_transaction.is_some());
            not_for("routines", routines.is_some());
            not_for("format", format.is_some());
        }
        DumpOptions::Sqlite(sqlite) => {
            if let Some(format) = format {
                sqlite.format = match format {
                    SqliteFormatInput::Script => SqliteFormat::Script,
                    SqliteFormatInput::File => SqliteFormat::File,
                };
            }
            not_for("content", content.is_some());
            not_for("clean", clean.is_some());
            not_for("keepOwners", keep_owners.is_some());
            not_for("addDropTable", add_drop_table.is_some());
            not_for("singleTransaction", single_transaction.is_some());
            not_for("routines", routines.is_some());
            not_for("collections", collections.is_some());
        }
    }
    match unused.as_slice() {
        [] => Ok(options),
        unused => Err(format!("{} don't apply to {label} backups. Leave them out.", unused.join(", "))),
    }
}

/// Checks `file` is the bare name of an export `engine` restores.
fn restorable_name(file: &str, engine: DumpEngine, label: &str) -> Result<(), String> {
    let bare = !file.is_empty()
        && !file.contains(['/', '\\', '\0'])
        && !file.contains("..")
        && Path::new(file).file_name().and_then(|name| name.to_str()) == Some(file);
    if !bare {
        return Err(
            "Give file as the name of an export in DBForge's exports folder, from list_exports, without a folder."
                .into(),
        );
    }
    let Some(extension) = export_extension(file) else {
        return Err(format!(
            "{file} isn't the name of a DBForge export. Call list_exports to see the exports you can restore."
        ));
    };
    let expected = engine.restore_extension();
    if extension != expected {
        return Err(format!("A {label} connection restores {expected} exports, and {file} isn't one."));
    }
    Ok(())
}

impl Tools {
    /// The exports folder, or why there is none.
    fn exports_folder(&self) -> Result<PathBuf, String> {
        self.core.export_folder().get().ok_or_else(|| {
            "DBForge has no exports folder: choose one in DBForge's Settings › Export and import.".to_owned()
        })
    }

    /// The name of `connection`'s tag, if it has one.
    pub(super) fn tag_name(&self, connection: &Connection) -> Option<String> {
        let tags = self.core.tags().list();
        tags.into_iter().find(|tag| Some(&tag.id) == connection.environment.as_ref()).map(|tag| tag.name)
    }

    /// Waits for the user to approve `approval` in the app.
    async fn approve_dump(&self, agent: Option<String>, approval: DumpApproval) -> bool {
        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            agent,
            subject: ApprovalSubject::Dump(Box::new(approval)),
        };
        self.core.mcp_approvals().ask(self.core.events().as_ref(), request).await
    }

    /// Once approved: `connection` still as the dialog showed it, agents' access included.
    fn still_as_shown(&self, connection: &Connection) -> Result<(), String> {
        self.still_as(connection).map_err(|meanwhile| match meanwhile {
            Meanwhile::Deleted => deleted_meanwhile(&connection.name),
            Meanwhile::Changed => changed_meanwhile(&connection.name),
        })
    }

    /// Whether agents may be offered the export at `path`: DBForge made it,
    /// from a connection agents may reach.
    fn offered(&self, path: &Path) -> bool {
        self.made_by(path).is_some()
    }

    /// The reachable connection DBForge made the export at `path` from, if any.
    fn made_by(&self, path: &Path) -> Option<Connection> {
        let record = self.core.export_records().of(path)?;
        self.reachable_connection(&record.connection_id).ok()
    }

    /// Whether the export at `path` came from a connection of `engine`'s kind: a .sql
    /// script of one engine is no script for another.
    fn restores_into(&self, path: &Path, engine: DumpEngine) -> bool {
        self.made_by(path).is_some_and(|source| DumpEngine::of(source.engine) == Some(engine))
    }

    pub(super) fn exports(&self, request: ListExportsRequest) -> Result<ExportList, String> {
        let engine = match &request.connection_id {
            Some(id) => Some(engine_of(&self.open(id)?.0)?),
            None => None,
        };
        let folder = self.exports_folder()?;
        let exports = list_exports(&folder)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|file| engine.is_none_or(|engine| file.extension == engine.restore_extension()))
            .filter(|file| self.offered(&folder.join(&file.name)))
            .filter(|file| engine.is_none_or(|engine| self.restores_into(&folder.join(&file.name), engine)))
            .map(|file| {
                let (kind, restores_into) = kind_of(file.extension);
                ExportView {
                    kind: kind.into(),
                    restores_into,
                    bytes: file.bytes,
                    modified: file
                        .modified
                        .map(|time| DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Secs, true)),
                    name: file.name,
                }
            })
            .collect();
        Ok(ExportList { exports })
    }

    pub(super) async fn backup(
        &self,
        agent: Option<String>,
        request: ExportDatabaseRequest,
    ) -> Result<ExportedView, String> {
        let (connection, _) = self.open(&request.connection_id)?;
        let engine = engine_of(&connection)?;
        let options = options_for(engine, connection.engine.label(), request.options)?;
        // What the file is named after and the dialog calls it: a SQLite connection's file, by its name.
        let (database, shown) = match engine {
            DumpEngine::Sqlite if request.database.is_some() => return Err(SQLITE_DATABASE.into()),
            DumpEngine::Sqlite => (connection.database.clone(), sqlite_name(&connection.database)),
            _ => {
                let database = request
                    .database
                    .map(|database| database.trim().to_owned())
                    .filter(|database| !database.is_empty())
                    .or_else(|| Some(connection.database.trim().to_owned()).filter(|database| !database.is_empty()))
                    .ok_or("Give database: the database to back up.")?;
                (database.clone(), database)
            }
        };
        let folder = self.exports_folder()?;
        let file = export_file_name(&shown, Local::now().naive_local(), options.extension());
        // Named only as an export at the top of the folder, whatever the database is called.
        if export_extension(&file).is_none()
            || Path::new(&file).file_name().and_then(|name| name.to_str()) != Some(&file)
        {
            return Err(format!("A backup of {shown} can't be named as a file."));
        }
        let path = folder.join(&file);
        let taken = || {
            path.symlink_metadata().is_ok().then(|| {
                format!("There's already an export named {file}. Try again in a minute, when the name is free.")
            })
        };
        if let Some(taken) = taken() {
            return Err(taken);
        }
        let path_text = path.to_str().ok_or("The exports folder's path isn't valid UTF-8.")?.to_owned();
        let approval = DumpApproval {
            connection_id: connection.id.clone(),
            connection_name: connection.name.clone(),
            tag: self.tag_name(&connection),
            engine: connection.engine,
            database: shown,
            file: file.clone(),
            action: DumpAction::Export { options: options.clone() },
        };
        if !self.approve_dump(agent, approval).await {
            return Err(DECLINED_BACKUP.into());
        }
        self.still_as_shown(&connection)?;
        // Made meanwhile, by the user or another backup: left as it is.
        if let Some(taken) = taken() {
            return Err(taken);
        }
        let core = self.core.clone();
        let request = ExportRequest {
            job_id: format!("mcp-{}", uuid::Uuid::new_v4()),
            connection_id: connection.id,
            database,
            path: path_text,
            options,
        };
        let exported = tokio::spawn(async move {
            let (events, store, jobs) = (core.events().as_ref(), core.connections(), core.jobs());
            let (tools, log, records) = (core.tool_paths(), core.statement_log(), core.export_records());
            dump::export_database(events, store, jobs, tools, log, records, request).await
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
        Ok(ExportedView { file, bytes: exported.bytes, elapsed_ms: exported.elapsed_ms })
    }

    /// Whether `database` is already on `connection`'s server; `None` when that couldn't be told in time.
    async fn database_exists(&self, connection: &Connection, password: Option<&str>, database: &str) -> Option<bool> {
        let listed = Box::pin(self.as_agent(connection, drivers::list_objects(connection, password, &[])));
        let databases = tokio::time::timeout(LOOKUP_LIMIT, listed).await.ok()?.ok()?;
        Some(databases.iter().any(|object| object.name == database))
    }

    pub(super) async fn restore(
        &self,
        agent: Option<String>,
        request: ImportDatabaseRequest,
    ) -> Result<ImportedView, String> {
        let (connection, password) = self.open(&request.connection_id)?;
        if connection.agent_access != AgentAccess::Write {
            return Err(format!(
                "{} is read-only for agents, and a restore changes data. Ask the user to give agents Read and write access to it in DBForge.",
                connection.name
            ));
        }
        let engine = engine_of(&connection)?;
        let label = connection.engine.label();
        restorable_name(&request.file, engine, label)?;
        let drop_existing = request.drop_existing.unwrap_or(false);
        if drop_existing && engine != DumpEngine::Mongodb {
            return Err(format!("dropExisting is for MongoDB restores; leave it out for a {label} connection."));
        }
        let (database, shown) = match engine {
            DumpEngine::Sqlite if request.database.is_some() => return Err(SQLITE_DATABASE.into()),
            DumpEngine::Sqlite => (connection.database.clone(), sqlite_name(&connection.database)),
            _ => {
                let database = request
                    .database
                    .map(|database| database.trim().to_owned())
                    .filter(|database| !database.is_empty())
                    .ok_or("Give database: the database to restore into. It is created when it doesn't exist.")?;
                (database.clone(), database)
            }
        };
        let folder = self.exports_folder()?;
        let path = folder.join(&request.file);
        let missing = || {
            // Not followed: a link named like an export is not one DBForge wrote. One
            // DBForge has no record of, or of a connection agents may not reach, reads as missing.
            let file = path.symlink_metadata().is_ok_and(|metadata| metadata.is_file());
            (!file || !self.offered(&path)).then(|| {
                format!(
                    "There's no export named {} in DBForge's exports folder. Call list_exports to see them.",
                    request.file
                )
            })
        };
        if let Some(missing) = missing() {
            return Err(missing);
        }
        if !self.restores_into(&path, engine)
            && let Some(source) = self.made_by(&path)
        {
            return Err(format!(
                "{} is a {} export, so it can't be restored into a {label} connection. Restore it into a {} connection.",
                request.file,
                source.engine.label(),
                source.engine.label()
            ));
        }
        let path_text = path.to_str().ok_or("The exports folder's path isn't valid UTF-8.")?.to_owned();
        let target_exists = match engine {
            DumpEngine::Sqlite => Some(Path::new(&connection.database).exists()),
            _ => self.database_exists(&connection, password.as_deref(), &database).await,
        };
        let approval = DumpApproval {
            connection_id: connection.id.clone(),
            connection_name: connection.name.clone(),
            tag: self.tag_name(&connection),
            engine: connection.engine,
            database: shown.clone(),
            file: request.file.clone(),
            action: DumpAction::Import { drop_existing, target_exists },
        };
        if !self.approve_dump(agent, approval).await {
            return Err(DECLINED_RESTORE.into());
        }
        self.still_as_shown(&connection)?;
        if let Some(missing) = missing() {
            return Err(missing);
        }
        let core = self.core.clone();
        let import = ImportRequest {
            job_id: format!("mcp-{}", uuid::Uuid::new_v4()),
            connection_id: connection.id,
            database,
            path: path_text,
            options: RestoreOptions { drop_existing },
        };
        let imported = tokio::spawn(async move {
            let (events, store, jobs) = (core.events().as_ref(), core.connections(), core.jobs());
            dump::import_database(events, store, jobs, core.tool_paths(), core.statement_log(), import).await
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
        Ok(ImportedView {
            file: request.file,
            database: shown,
            created: imported.created,
            elapsed_ms: imported.elapsed_ms,
            skipped_extensions: imported.skipped_extensions,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::connections::model::ConnectionParts;
    use crate::mcp::saved::tests::{Setup, count, items, setup};

    /// The setup with its exports folder in its own temporary directory.
    fn exporting() -> (Setup, PathBuf) {
        let s = setup();
        let folder = s.dir.join("exports");
        s.core().export_folder().set(Some(folder.clone()));
        (s, folder)
    }

    fn files(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = match std::fs::read_dir(folder) {
            Ok(entries) => entries.map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names
    }

    fn export(connection: &str) -> ExportDatabaseRequest {
        ExportDatabaseRequest { connection_id: connection.into(), database: None, options: None }
    }

    fn import(connection: &str, file: &str) -> ImportDatabaseRequest {
        ImportDatabaseRequest {
            connection_id: connection.into(),
            file: file.into(),
            database: None,
            drop_existing: None,
        }
    }

    /// An export `name` in `folder` holding `text`, recorded as made from connection `by`.
    fn made(s: &Setup, folder: &Path, name: &str, by: &str, text: &str) {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(folder.join(name), text).unwrap();
        s.core().export_records().record(&folder.join(name), by, by).unwrap();
    }

    fn dump_of(request: &ApprovalRequest) -> &DumpApproval {
        match &request.subject {
            ApprovalSubject::Dump(dump) => dump,
            other => panic!("not a dump: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_connection_agents_may_not_use_reads_as_unknown() {
        let (s, folder) = exporting();
        s.connection("hidden", Engine::Sqlite, AgentAccess::None, None);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("hidden-2026-11-10-1432.sql"), "SELECT 1;").unwrap();
        let tools = s.tools();

        let unknown = "No connection with id hidden. Call list_connections to see the ones you can use.";
        let request = ListExportsRequest { connection_id: Some("hidden".into()) };
        assert_eq!(tools.exports(request).unwrap_err(), unknown);
        assert_eq!(tools.backup(None, export("hidden")).await.unwrap_err(), unknown);
        let error = tools.restore(None, import("hidden", "hidden-2026-11-10-1432.sql")).await.unwrap_err();
        assert_eq!(error, unknown);
        assert_eq!(files(&folder), ["hidden-2026-11-10-1432.sql"], "nothing written");
    }

    #[tokio::test]
    async fn read_access_backs_up_but_doesn_t_restore() {
        let (mut s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Read, Some("production"));
        items(&shop).await;

        let tools = s.tools();
        let (backup, asked) = s.answering(true, tools.backup(Some("codex".into()), export("shop"))).await;
        let backup = backup.unwrap();
        let [asked] = asked.as_slice() else { panic!("{asked:?}") };
        assert_eq!(asked.agent.as_deref(), Some("codex"));
        assert_eq!(asked.connection_id(), Some("shop"));
        let dump = dump_of(asked);
        assert_eq!((dump.connection_name.as_str(), dump.tag.as_deref()), ("shop", Some("PROD")));
        assert_eq!((dump.engine, dump.database.as_str()), (Engine::Sqlite, "shop"));
        assert_eq!(dump.file, backup.file);
        assert_eq!(dump.action, DumpAction::Export { options: DumpOptions::defaults(DumpEngine::Sqlite) });
        assert_eq!(files(&folder), std::slice::from_ref(&backup.file));

        let (error, asked) = s.answering(true, tools.restore(None, import("shop", &backup.file))).await;
        assert!(error.unwrap_err().starts_with("shop is read-only for agents"));
        assert!(asked.is_empty(), "nothing asked");
        assert_eq!(count(&shop).await, "2");
    }

    #[tokio::test]
    async fn a_backup_writes_only_into_the_exports_folder_under_the_name_dbforge_gives_it() {
        let (mut s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Read, None);
        items(&shop).await;
        let before = files(&s.dir);

        let tools = s.tools();
        let (backup, _) = s.answering(true, tools.backup(None, export("shop"))).await;
        let backup = backup.unwrap();
        assert!(backup.file.starts_with("shop-") && backup.file.ends_with(".sql"), "{}", backup.file);
        assert_eq!(export_extension(&backup.file), Some(".sql"));
        assert_eq!(files(&folder), std::slice::from_ref(&backup.file));
        assert_eq!(std::fs::metadata(folder.join(&backup.file)).unwrap().len(), backup.bytes);
        let beside: Vec<String> = files(&s.dir).into_iter().filter(|name| name != "exports").collect();
        assert_eq!(beside, before, "nothing else written beside it");
        let script = std::fs::read_to_string(folder.join(&backup.file)).unwrap();
        assert!(script.contains("CREATE TABLE items"), "{script}");
        let record = s.core().export_records().of(&folder.join(&backup.file)).expect("recorded");
        assert_eq!(record.connection_id, "shop", "who made it is recorded");

        // A second one the same minute would take its name: refused, the first left as it is.
        let (again, asked) = s.answering(true, tools.backup(None, export("shop"))).await;
        // Unless the minute turned meanwhile, giving it a name of its own.
        if let Err(error) = again {
            assert!(error.starts_with("There's already an export named"), "{error}");
            assert!(asked.is_empty(), "refused before asking");
            assert_eq!(files(&folder), std::slice::from_ref(&backup.file));
        }
    }

    #[tokio::test]
    async fn a_declined_backup_or_restore_runs_nothing() {
        let (mut s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Write, None);
        items(&shop).await;
        let tools = s.tools();

        let (declined, asked) = s.answering(false, tools.backup(None, export("shop"))).await;
        assert_eq!(declined.unwrap_err(), DECLINED_BACKUP);
        assert_eq!(asked.len(), 1);
        assert!(files(&folder).is_empty(), "no file written");

        let file = "other-2026-11-10-1432.sql";
        made(&s, &folder, file, "shop", "DELETE FROM items;");
        let (declined, asked) = s.answering(false, tools.restore(None, import("shop", file))).await;
        assert_eq!(declined.unwrap_err(), DECLINED_RESTORE);
        let [asked] = asked.as_slice() else { panic!("{asked:?}") };
        assert_eq!(dump_of(asked).action, DumpAction::Import { drop_existing: false, target_exists: Some(true) });
        assert_eq!(count(&shop).await, "2", "nothing restored");
    }

    #[tokio::test]
    async fn an_agent_gone_before_the_answer_runs_nothing() {
        let (s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Write, None);
        items(&shop).await;
        let tools = s.tools();
        let waiting = tokio::time::timeout(Duration::from_millis(20), tools.backup(None, export("shop"))).await;
        assert!(waiting.is_err(), "still waiting for the user");
        assert!(files(&folder).is_empty());
        assert!(!s.core().mcp_approvals().answer("anything", true));
    }

    #[tokio::test]
    async fn a_restore_reads_only_an_export_of_the_engine_s_kind_from_the_folder() {
        let (mut s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Write, None);
        items(&shop).await;
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("shop-2026-11-10-1432.archive.gz"), "x").unwrap();
        std::fs::write(folder.join("shop-2026-11-10-1432.db"), "x").unwrap();
        std::fs::write(folder.join("shop-2026-11-10-1432.sql.dbforge-partial"), "DELETE FROM items;").unwrap();
        std::fs::write(folder.join("notes.sql"), "DELETE FROM items;").unwrap();
        // A file outside the folder, named as an export.
        std::fs::write(s.dir.join("outside-2026-11-10-1432.sql"), "DELETE FROM items;").unwrap();
        let tools = s.tools();

        let path =
            "Give file as the name of an export in DBForge's exports folder, from list_exports, without a folder.";
        let cases = [
            ("../outside-2026-11-10-1432.sql", path.to_owned()),
            (&*s.dir.join("outside-2026-11-10-1432.sql").to_string_lossy().into_owned().leak(), path.to_owned()),
            ("sub\\outside-2026-11-10-1432.sql", path.to_owned()),
            ("..", path.to_owned()),
            ("", path.to_owned()),
            (
                "notes.sql",
                "notes.sql isn't the name of a DBForge export. Call list_exports to see the exports you can restore."
                    .to_owned(),
            ),
            (
                "shop-2026-11-10-1432.sql.dbforge-partial",
                "shop-2026-11-10-1432.sql.dbforge-partial isn't the name of a DBForge export. Call list_exports to see the exports you can restore.".to_owned(),
            ),
            (
                "shop-2026-11-10-1432.archive.gz",
                "A SQLite connection restores .sql exports, and shop-2026-11-10-1432.archive.gz isn't one.".to_owned(),
            ),
            (
                "shop-2026-11-10-1432.db",
                "A SQLite connection restores .sql exports, and shop-2026-11-10-1432.db isn't one.".to_owned(),
            ),
            (
                "gone-2026-11-10-1432.sql",
                "There's no export named gone-2026-11-10-1432.sql in DBForge's exports folder. Call list_exports to see them.".to_owned(),
            ),
        ];
        for (file, expected) in cases {
            let (error, asked) = s.answering(true, tools.restore(None, import("shop", file))).await;
            assert_eq!(error.unwrap_err(), expected, "{file}");
            assert!(asked.is_empty(), "{file}: refused before asking");
        }
        let drop = ImportDatabaseRequest { drop_existing: Some(true), ..import("shop", "x-2026-11-10-1432.sql") };
        let (error, _) = s.answering(true, tools.restore(None, drop)).await;
        assert_eq!(error.unwrap_err(), "dropExisting is for MongoDB restores; leave it out for a SQLite connection.");
        assert_eq!(count(&shop).await, "2", "nothing restored");
    }

    #[tokio::test]
    async fn options_for_another_engine_are_refused_before_asking() {
        let (mut s, folder) = exporting();
        s.connection("shop", Engine::Sqlite, AgentAccess::Read, None);
        let tools = s.tools();
        let options = ExportOptionsInput {
            content: Some(ContentInput::Data),
            routines: Some(false),
            ..ExportOptionsInput::default()
        };
        let request = ExportDatabaseRequest { options: Some(options), ..export("shop") };
        let (error, asked) = s.answering(true, tools.backup(None, request)).await;
        assert_eq!(error.unwrap_err(), "content, routines don't apply to SQLite backups. Leave them out.");
        assert!(asked.is_empty());
        assert!(files(&folder).is_empty());

        let mysql = options_for(
            DumpEngine::Mysql,
            "MySQL",
            Some(ExportOptionsInput { add_drop_table: Some(true), ..ExportOptionsInput::default() }),
        )
        .unwrap();
        assert_eq!(
            mysql,
            DumpOptions::Mysql(dump::MysqlOptions {
                content: Content::All,
                add_drop_table: true,
                single_transaction: true,
                routines: true
            })
        );
        let error = options_for(
            DumpEngine::Postgres,
            "PostgreSQL",
            Some(ExportOptionsInput { collections: Some(vec!["a".into()]), ..ExportOptionsInput::default() }),
        )
        .unwrap_err();
        assert_eq!(error, "collections don't apply to PostgreSQL backups. Leave them out.");
    }

    #[tokio::test]
    async fn engines_without_dumps_are_refused() {
        let (s, _) = exporting();
        s.connection("cache", Engine::Redis, AgentAccess::Write, None);
        let error = s.tools().backup(None, export("cache")).await.unwrap_err();
        assert_eq!(error, "DBForge can't back up or restore whole Redis databases.");
    }

    #[tokio::test]
    async fn a_sqlite_database_backs_up_and_restores_through_the_tools() {
        let (mut s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Read, None);
        items(&shop).await;
        let copy = s.connection("copy", Engine::Sqlite, AgentAccess::Write, None);
        std::fs::write(folder.parent().unwrap().join("ignored.sql"), "").unwrap();
        let tools = s.tools();

        let (backup, _) = s.answering(true, tools.backup(Some("claude-code".into()), export("shop"))).await;
        let backup = backup.unwrap();
        let file_copy = ExportDatabaseRequest {
            options: Some(ExportOptionsInput { format: Some(SqliteFormatInput::File), ..Default::default() }),
            ..export("shop")
        };
        let copied = s.answering(true, tools.backup(None, file_copy)).await.0.unwrap();
        assert!(copied.file.ends_with(".db"), "{}", copied.file);

        let listed = tools.exports(ListExportsRequest { connection_id: None }).unwrap();
        let kinds: Vec<(&str, &str)> =
            listed.exports.iter().map(|export| (export.name.as_str(), export.kind.as_str())).collect();
        assert_eq!(kinds, [(copied.file.as_str(), "sqliteFile"), (backup.file.as_str(), "sql")]);
        assert!(listed.exports.iter().all(|export| export.modified.is_some()));
        let restorable = tools.exports(ListExportsRequest { connection_id: Some("copy".into()) }).unwrap();
        let names: Vec<&str> = restorable.exports.iter().map(|export| export.name.as_str()).collect();
        assert_eq!(names, [backup.file.as_str()], "only what restores into a SQLite connection");

        let (restored, asked) =
            s.answering(true, tools.restore(Some("codex".into()), import("copy", &backup.file))).await;
        let restored = restored.unwrap();
        assert_eq!((restored.database.as_str(), restored.created), ("copy", false));
        let [asked] = asked.as_slice() else { panic!("{asked:?}") };
        let dump = dump_of(asked);
        assert_eq!((dump.file.as_str(), dump.database.as_str()), (backup.file.as_str(), "copy"));
        assert_eq!(count(&copy).await, "2");
        assert_eq!(count(&shop).await, "2", "the source is untouched");
    }

    #[tokio::test]
    async fn a_sqlite_connection_takes_no_database() {
        let (mut s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Write, None);
        items(&shop).await;
        made(&s, &folder, "shop-2026-11-10-1432.sql", "shop", "DELETE FROM items;");
        let tools = s.tools();
        let expected =
            "database is for server engines; a SQLite connection backs up and restores its own file. Leave it out.";

        let request = ExportDatabaseRequest { database: Some("main".into()), ..export("shop") };
        let (error, asked) = s.answering(true, tools.backup(None, request)).await;
        assert_eq!(error.unwrap_err(), expected);
        assert!(asked.is_empty(), "refused before asking");
        let request =
            ImportDatabaseRequest { database: Some("other".into()), ..import("shop", "shop-2026-11-10-1432.sql") };
        let (error, asked) = s.answering(true, tools.restore(None, request)).await;
        assert_eq!(error.unwrap_err(), expected);
        assert!(asked.is_empty(), "refused before asking");
        assert_eq!(files(&folder), ["shop-2026-11-10-1432.sql"], "nothing written");
        assert_eq!(count(&shop).await, "2", "nothing restored");
    }

    #[test]
    fn requests_with_a_field_the_tools_don_t_know_are_refused() {
        use serde_json::{Value, from_value, json};
        fn refused<T: serde::de::DeserializeOwned>(value: Value) -> bool {
            from_value::<T>(value).is_err()
        }
        assert!(refused::<ExportDatabaseRequest>(json!({ "connectionId": "shop", "path": "/tmp/x.sql" })));
        assert!(refused::<ImportDatabaseRequest>(
            json!({ "connectionId": "shop", "file": "f", "drop_existing": true })
        ));
        assert!(refused::<ListExportsRequest>(json!({ "connection": "shop" })));
        assert!(refused::<super::super::connections::ConnectionDetailsRequest>(
            json!({ "connectionId": "shop", "secrets": true })
        ));
        assert!(refused::<super::super::connections::DeleteConnectionRequest>(json!({ "id": "shop", "force": true })));

        assert!(from_value::<ExportDatabaseRequest>(json!({ "connectionId": "shop", "database": "d" })).is_ok());
        let import: ImportDatabaseRequest =
            from_value(json!({ "connectionId": "shop", "file": "f", "dropExisting": true })).unwrap();
        assert_eq!(import.drop_existing, Some(true));
        assert!(from_value::<ListExportsRequest>(json!({})).is_ok());
    }

    #[tokio::test]
    async fn a_connection_changed_while_the_user_was_asked_runs_nothing() {
        let (mut s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Write, None);
        items(&shop).await;
        let file = "shop-2026-11-10-1432.sql";
        made(&s, &folder, file, "shop", "DELETE FROM items;");
        let tools = s.tools();
        let store = s.tools();
        let changed = "The connection shop changed while waiting for approval, so nothing ran. Call list_connections and try again.";

        // Made read-only for agents while the backup waited.
        let read_only = Connection { agent_access: AgentAccess::Read, ..shop.clone() };
        let (error, asked) = s
            .answering_with(
                |_| {
                    store.core.connections().upsert(read_only.clone()).unwrap();
                    (true, None)
                },
                tools.restore(None, import("shop", file)),
            )
            .await;
        assert_eq!(error.unwrap_err(), changed);
        assert_eq!(asked.len(), 1);
        assert_eq!(count(&shop).await, "2", "nothing restored");

        // Pointed at another file while the backup waited.
        let moved = Connection { database: s.dir.join("other.db").to_string_lossy().into_owned(), ..read_only.clone() };
        let (error, _) = s
            .answering_with(
                |_| {
                    store.core.connections().upsert(moved.clone()).unwrap();
                    (true, None)
                },
                tools.backup(None, export("shop")),
            )
            .await;
        assert_eq!(error.unwrap_err(), changed);
        assert_eq!(files(&folder), [file], "no backup written");

        let (error, _) = s
            .answering_with(
                |_| {
                    store.core.connections().remove("shop").unwrap();
                    (true, None)
                },
                tools.backup(None, export("shop")),
            )
            .await;
        assert_eq!(error.unwrap_err(), "The connection shop was deleted while waiting for approval, so nothing ran.");
        assert_eq!(files(&folder), [file], "no backup written");
    }

    #[tokio::test]
    async fn two_exports_to_one_file_at_once_leave_the_first_to_finish() {
        let (s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Read, None);
        items(&shop).await;
        let core = s.core();
        let path = folder.join("shop-2026-11-10-1432.sql").to_string_lossy().into_owned();
        let request = |job: &str| ExportRequest {
            job_id: job.into(),
            connection_id: "shop".into(),
            database: shop.database.clone(),
            path: path.clone(),
            options: DumpOptions::defaults(DumpEngine::Sqlite),
        };
        let run = |job: &'static str| {
            let (events, store, jobs) = (core.events().as_ref(), core.connections(), core.jobs());
            let (tools, log, records) = (core.tool_paths(), core.statement_log(), core.export_records());
            dump::export_database(events, store, jobs, tools, log, records, request(job))
        };
        let (first, second) = tokio::join!(run("first"), run("second"));
        let mut outcomes =
            [first.map(|_| ()).map_err(|e| e.to_string()), second.map(|_| ()).map_err(|e| e.to_string())];
        outcomes.sort_by_key(Result::is_err);
        assert!(outcomes[0].is_ok(), "{outcomes:?}");
        let refused = outcomes[1].as_ref().unwrap_err();
        assert!(refused.starts_with("A backup with this name is already being written"), "{refused}");
        assert_eq!(files(&folder), ["shop-2026-11-10-1432.sql"], "the first finished, no partial file left");
        let script = std::fs::read_to_string(&path).unwrap();
        assert!(script.contains("CREATE TABLE items"), "{script}");
    }

    #[tokio::test]
    async fn agents_are_offered_only_exports_dbforge_made_from_connections_they_reach() {
        let (mut s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Write, None);
        items(&shop).await;
        s.connection("hidden", Engine::Sqlite, AgentAccess::None, None);
        made(&s, &folder, "shop-2026-11-10-1432.sql", "shop", "SELECT 1;");
        made(&s, &folder, "hidden-2026-11-10-1432.sql", "hidden", "DELETE FROM items;");
        made(&s, &folder, "gone-2026-11-10-1432.sql", "gone", "DELETE FROM items;");
        // Made outside DBForge, or before it kept records.
        std::fs::write(folder.join("outside-2026-11-10-1432.sql"), "DELETE FROM items;").unwrap();
        let tools = s.tools();

        for connection_id in [None, Some("shop".to_owned())] {
            let listed = tools.exports(ListExportsRequest { connection_id }).unwrap();
            let names: Vec<&str> = listed.exports.iter().map(|export| export.name.as_str()).collect();
            assert_eq!(names, ["shop-2026-11-10-1432.sql"]);
        }
        for file in ["hidden-2026-11-10-1432.sql", "gone-2026-11-10-1432.sql", "outside-2026-11-10-1432.sql"] {
            let (error, asked) = s.answering(true, tools.restore(None, import("shop", file))).await;
            let expected =
                format!("There's no export named {file} in DBForge's exports folder. Call list_exports to see them.");
            assert_eq!(error.unwrap_err(), expected);
            assert!(asked.is_empty(), "{file}: refused before asking");
        }
        assert_eq!(count(&shop).await, "2", "nothing restored");
        let (restored, _) = s.answering(true, tools.restore(None, import("shop", "shop-2026-11-10-1432.sql"))).await;
        restored.unwrap();
    }

    #[tokio::test]
    async fn an_export_restores_only_into_the_engine_it_came_from() {
        let (mut s, folder) = exporting();
        let shop = s.connection("shop", Engine::Sqlite, AgentAccess::Write, None);
        items(&shop).await;
        // A PostgreSQL script: a .sql file too, but not one SQLite can run.
        s.connection("warehouse", Engine::Postgres, AgentAccess::Read, None);
        made(&s, &folder, "warehouse-2026-11-10-1432.sql", "warehouse", "DELETE FROM items;");
        let tools = s.tools();

        let listed = tools.exports(ListExportsRequest { connection_id: Some("shop".into()) }).unwrap();
        assert!(listed.exports.is_empty(), "not offered for a SQLite connection: {:?}", listed.exports);
        let (error, asked) =
            s.answering(true, tools.restore(None, import("shop", "warehouse-2026-11-10-1432.sql"))).await;
        assert_eq!(
            error.unwrap_err(),
            "warehouse-2026-11-10-1432.sql is a PostgreSQL export, so it can't be restored into a SQLite connection. Restore it into a PostgreSQL connection."
        );
        assert!(asked.is_empty(), "refused before asking");
        assert_eq!(count(&shop).await, "2", "nothing restored");
    }

    /// Saves `connection` with `password` in the setup's own credential file.
    fn saved(s: &Setup, connection: Connection, password: Option<&str>) {
        let parts = ConnectionParts { connection, password: password.map(Into::into), ssh_secret: None };
        s.core().connections().save(parts).unwrap();
    }

    /// Exports `source` through the tools and restores it into `target` on the same connection.
    async fn through_the_tools(s: &mut Setup, connection: &str, source: &str, target: &str, drop: Option<bool>) {
        let tools = s.tools();
        let request = ExportDatabaseRequest { database: Some(source.into()), ..export(connection) };
        let (backup, _) = s.answering(true, tools.backup(None, request)).await;
        let backup = backup.unwrap();
        let request = ImportDatabaseRequest {
            database: Some(target.into()),
            drop_existing: drop,
            ..import(connection, &backup.file)
        };
        let (restored, asked) = s.answering(true, tools.restore(None, request)).await;
        assert!(restored.unwrap().created);
        let [asked] = asked.as_slice() else { panic!("{asked:?}") };
        assert_eq!(
            dump_of(asked).action,
            DumpAction::Import { drop_existing: drop.unwrap_or(false), target_exists: Some(false) }
        );
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306 and its client tools"]
    async fn mysql_backs_up_and_restores_through_the_tools() {
        use crate::drivers::mysql::tests::{PASSWORD, local};
        let (mut s, _) = exporting();
        let server = Connection { agent_access: AgentAccess::Write, ..local() };
        saved(&s, server.clone(), PASSWORD);
        let run = async |sql: &str| {
            let mut session = drivers::open_session(&server, PASSWORD, None, None).await.unwrap();
            session.run(sql, 10).await.unwrap()
        };
        run("DROP DATABASE IF EXISTS dbforge_mcp_source; DROP DATABASE IF EXISTS dbforge_mcp_copy; \
             CREATE DATABASE dbforge_mcp_source; CREATE TABLE dbforge_mcp_source.items (id INT PRIMARY KEY); \
             INSERT INTO dbforge_mcp_source.items VALUES (1), (2);")
        .await;
        through_the_tools(&mut s, &server.id, "dbforge_mcp_source", "dbforge_mcp_copy", None).await;
        let counted = run("SELECT COUNT(*) FROM dbforge_mcp_copy.items").await;
        assert_eq!(counted.results[0].rows[0][0].as_deref(), Some("2"));
        run("DROP DATABASE dbforge_mcp_source; DROP DATABASE dbforge_mcp_copy;").await;
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017 and the Database Tools"]
    async fn mongodb_backs_up_and_restores_through_the_tools() {
        use mongodb::bson::doc;

        use crate::drivers::mongodb::tests::{PASSWORD, database, local};
        let (mut s, _) = exporting();
        database("dbforge_mcp_source", &[("items", vec![doc! { "n": 1 }, doc! { "n": 2 }])]).await;
        let server = Connection { agent_access: AgentAccess::Write, ..local() };
        let _ = drivers::mongodb::drop_database(&server, PASSWORD, "dbforge_mcp_copy").await;
        saved(&s, server.clone(), PASSWORD);
        through_the_tools(&mut s, &server.id, "dbforge_mcp_source", "dbforge_mcp_copy", Some(true)).await;
        let objects = drivers::list_objects(&server, PASSWORD, &["dbforge_mcp_copy".into()]).await.unwrap();
        assert!(objects.iter().any(|object| object.name == "items"), "{objects:?}");
        let _ = drivers::mongodb::drop_database(&server, PASSWORD, "dbforge_mcp_source").await;
        let _ = drivers::mongodb::drop_database(&server, PASSWORD, "dbforge_mcp_copy").await;
    }
}
