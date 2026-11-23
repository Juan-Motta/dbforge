//! What agents can do: the MCP tools, each checking the connection's agent
//! access and running in a statement-log scope, so every statement an agent
//! causes shows in the app's console as the agent's.
//!
//! Tools never return a password or anything the app keeps secret: agents
//! name a connection by its id, and the app opens it with its own credentials.
//! Nor do they take one: a connection an agent creates or changes gets its
//! secrets from the user, typed in the approval dialog.

use rmcp::RoleServer;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::{self, JsonSchema};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::approvals::{ApprovalRequest, ApprovalSubject};
use super::connections::{ConnectionDetails, ConnectionDetailsRequest, DeleteConnectionRequest, SaveConnectionRequest};
use super::dumps::{
    ExportDatabaseRequest, ExportList, ExportedView, ImportDatabaseRequest, ImportedView, ListExportsRequest,
};
use super::saved::{
    DeleteSavedRequest, ListSavedRequest, RunSavedRequest, SaveQueryRequest, SavedQueryList, SavedQueryView,
};
use crate::Core;
use crate::connections::model::{AgentAccess, Connection};
use crate::drivers::{self, ObjectKind, QueryOutcome, TableRef};
use crate::statement_log::{self, Origin};

/// The MCP server's handler: one per agent session, all sharing the app.
#[derive(Clone)]
pub struct Tools {
    pub(super) core: Arc<Core>,
}

impl Tools {
    pub fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    /// The connection `id` with its password, when agents may at least read it.
    /// One they may not use reads as unknown, so its existence isn't given away.
    pub(super) fn open(&self, id: &str) -> Result<(Connection, Option<String>), String> {
        let store = self.core.connections();
        let connection =
            store.get(id).ok().filter(|connection| connection.agent_access != AgentAccess::None).ok_or_else(|| {
                format!("No connection with id {id}. Call list_connections to see the ones you can use.")
            })?;
        let password = store.password(&connection.id).map_err(|error| error.to_string())?;
        Ok((connection, password))
    }

    /// Records the call as the agent's activity, so the app lists it; the
    /// agent's own name for itself, when it sent its client info.
    pub(super) fn note_agent(&self, context: &RequestContext<RoleServer>) -> Option<String> {
        let client = context.peer.peer_info().map(|info| info.client_info.clone())?;
        let version = Some(client.version.as_str()).filter(|version| !version.is_empty());
        self.core.mcp_agents().record(self.core.events().as_ref(), &client.name, version);
        Some(client.name)
    }

    /// Runs `work` with its statements recorded as the agent's, on `connection`.
    pub(super) async fn as_agent<T>(
        &self,
        connection: &Connection,
        work: impl Future<Output = crate::error::Result<T>>,
    ) -> Result<T, String> {
        statement_log::scoped(self.core.statement_log(), &connection.id, Origin::Agent, work)
            .await
            .map_err(|error| error.to_string())
    }

    /// Runs `request`'s statement on a session the server keeps from writing.
    pub(super) async fn read(&self, request: RunRequest) -> Result<RunResult, String> {
        let (connection, password) = self.open(&request.connection_id)?;
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        // Drivers' futures are boxed here and below: kept on the stack, they
        // overflowed a 2 MB thread's stack (a refused SQLite write, in a debug build).
        let read = Box::pin(drivers::run_read_only(
            &connection,
            password.as_deref(),
            request.database.as_deref(),
            request.container.as_deref(),
            &request.statement,
            limit,
        ));
        // A read leaves nothing half done, so one taking too long is simply dropped.
        let read = async {
            tokio::time::timeout(drivers::READ_ONLY_TIME_LIMIT, read)
                .await
                .unwrap_or_else(|_| Err(drivers::read_only_timed_out()))
        };
        Ok(self.as_agent(&connection, read).await?.into())
    }

    /// Runs `request`'s statement, which may change data, for `agent`: only
    /// on a connection agents may write to, and once the user approved it
    /// when its tag asks before changes.
    pub(super) async fn change(&self, agent: Option<String>, request: RunRequest) -> Result<RunResult, String> {
        let (connection, password) = self.open(&request.connection_id)?;
        if connection.agent_access != AgentAccess::Write {
            return Err(format!(
                "{} is read-only for agents. Use query, or ask the user to give agents Read and write access to it in DBForge.",
                connection.name
            ));
        }
        let tag = self.core.tags().list().into_iter().find(|tag| Some(&tag.id) == connection.environment.as_ref());
        if tag.as_ref().is_some_and(|tag| tag.confirm) {
            let request = ApprovalRequest {
                id: uuid::Uuid::new_v4().to_string(),
                agent,
                subject: ApprovalSubject::Statement {
                    connection_id: connection.id.clone(),
                    connection_name: connection.name.clone(),
                    tag: tag.map(|tag| tag.name),
                    database: request.database.clone(),
                    statement: request.statement.clone(),
                },
            };
            if !self.core.mcp_approvals().ask(self.core.events().as_ref(), request).await {
                return Err("The user declined this change in DBForge, or didn't answer in time. Nothing ran.".into());
            }
        }
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let run = async {
            let (database, container) = (request.database.as_deref(), request.container.as_deref());
            let mut session =
                Box::pin(drivers::open_session(&connection, password.as_deref(), database, container)).await?;
            Box::pin(session.run(&request.statement, limit)).await
        };
        Ok(self.as_agent(&connection, run).await?.into())
    }
}

/// A connection as agents see it: enough to choose it and write to it in its language.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    /// What every other tool takes as `connectionId`.
    pub id: String,
    pub name: String,
    /// PostgreSQL, MySQL, MariaDB, SQLite, SQL Server, MongoDB, Redis or Azure Cosmos DB.
    pub engine: String,
    /// The language statements are written in for this engine.
    pub language: String,
    /// The tag the user gave it (e.g. PROD), if any.
    pub tag: Option<String>,
    /// `read`: `query` only. `write`: `execute` too.
    pub access: String,
    /// Changes wait for the user to approve them in the app.
    pub asks_before_changes: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConnectionList {
    pub connections: Vec<ConnectionView>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListObjectsRequest {
    /// From `list_connections`.
    pub connection_id: String,
    /// Where to look, as names from the root: `[]` lists databases, `["shop"]`
    /// its schemas (or tables, for engines without schemas), `["shop", "public"]`
    /// the tables and views of that schema.
    #[serde(default)]
    pub path: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ObjectView {
    pub name: String,
    /// `database`, `schema`, `table` or `view`.
    pub kind: String,
    /// Approximate rows, when the server knows it.
    pub row_estimate: Option<i64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ObjectList {
    pub objects: Vec<ObjectView>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DescribeTableRequest {
    pub connection_id: String,
    pub database: String,
    /// The schema; for engines without schemas, the same as `database`.
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableView {
    /// `table` or `view`.
    pub kind: String,
    pub row_estimate: Option<i64>,
    /// A script that creates it: columns, keys, indexes, foreign keys, triggers.
    pub definition: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub connection_id: String,
    /// In the connection's language (see `list_connections`).
    pub statement: String,
    /// The database to run it in; the connection's own when left out.
    pub database: Option<String>,
    /// Cosmos DB only: the container a query reads.
    pub container: Option<String>,
    /// The most rows returned per result, 100 unless given; at most 1000.
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResultSet {
    pub columns: Vec<String>,
    /// Every value as text, `null` being NULL. Values over 2000 characters are cut, ending in `…`.
    pub rows: Vec<Vec<Option<String>>>,
    /// More rows came back than `limit`; the rest were left out.
    pub truncated: bool,
    /// Rows returned, or changed by a statement that returns none.
    pub rows_affected: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    /// One per statement that ran.
    pub results: Vec<ResultSet>,
    pub elapsed_ms: u64,
}

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1000;
/// Long values (JSON documents, blobs as text) would fill an agent's context.
const MAX_VALUE: usize = 2000;

fn cut(value: String) -> String {
    match value.char_indices().nth(MAX_VALUE) {
        Some((end, _)) => format!("{}…", &value[..end]),
        None => value,
    }
}

impl From<QueryOutcome> for RunResult {
    fn from(outcome: QueryOutcome) -> Self {
        let results = outcome
            .results
            .into_iter()
            .map(|result| ResultSet {
                columns: result.columns.into_iter().map(|column| column.name).collect(),
                rows: result
                    .rows
                    .into_iter()
                    .map(|row| row.into_iter().map(|value| value.map(cut)).collect())
                    .collect(),
                truncated: result.truncated,
                rows_affected: result.rows_affected,
            })
            .collect();
        Self { results, elapsed_ms: outcome.elapsed_ms }
    }
}

fn kind_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Database => "database",
        ObjectKind::Schema => "schema",
        ObjectKind::Table => "table",
        ObjectKind::View => "view",
    }
}

fn language_of(connection: &Connection) -> &'static str {
    use crate::connections::model::Engine;
    match connection.engine {
        Engine::Postgres => "PostgreSQL SQL",
        Engine::Mysql | Engine::Mariadb => "MySQL SQL",
        Engine::Sqlite => "SQLite SQL",
        Engine::Sqlserver => "T-SQL",
        Engine::Mongodb => "mongosh, e.g. db.getCollection(\"orders\").find({ status: \"paid\" })",
        Engine::Redis => "one redis-cli command per call, e.g. HGETALL user:1",
        Engine::Cosmosdb => "Cosmos DB SQL on one container, e.g. SELECT * FROM c WHERE c.status = \"paid\"",
    }
}

#[tool_router(vis = "pub(crate)")]
impl Tools {
    #[tool(
        description = "Lists the database connections saved in DBForge that you may use, with the language each one takes and whether you may change data. Call this first."
    )]
    async fn list_connections(&self, context: RequestContext<RoleServer>) -> Json<ConnectionList> {
        self.note_agent(&context);
        let tags = self.core.tags().list();
        let connections = self
            .core
            .connections()
            .list()
            .into_iter()
            .filter(|connection| connection.agent_access != AgentAccess::None)
            .map(|connection| {
                let tag = tags.iter().find(|tag| Some(&tag.id) == connection.environment.as_ref());
                ConnectionView {
                    language: language_of(&connection).into(),
                    engine: connection.engine.label().into(),
                    tag: tag.map(|tag| tag.name.clone()),
                    access: match connection.agent_access {
                        AgentAccess::Write => "write",
                        _ => "read",
                    }
                    .into(),
                    asks_before_changes: tag.is_some_and(|tag| tag.confirm),
                    id: connection.id,
                    name: connection.name,
                }
            })
            .collect();
        Json(ConnectionList { connections })
    }

    #[tool(description = "Lists what a connection holds at a path: databases, then schemas, then tables and views.")]
    async fn list_objects(
        &self,
        Parameters(request): Parameters<ListObjectsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ObjectList>, String> {
        self.note_agent(&context);
        let (connection, password) = self.open(&request.connection_id)?;
        let objects = self
            .as_agent(&connection, drivers::list_objects(&connection, password.as_deref(), &request.path))
            .await?
            .into_iter()
            .map(|object| ObjectView {
                name: object.name,
                kind: kind_name(object.kind).into(),
                row_estimate: object.row_estimate,
            })
            .collect();
        Ok(Json(ObjectList { objects }))
    }

    #[tool(
        description = "Describes a table or view: a script that creates it, with its columns, types, keys, indexes and foreign keys."
    )]
    async fn describe_table(
        &self,
        Parameters(request): Parameters<DescribeTableRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<TableView>, String> {
        self.note_agent(&context);
        let (connection, password) = self.open(&request.connection_id)?;
        let table = TableRef { database: request.database, schema: request.schema, name: request.table };
        let structure =
            self.as_agent(&connection, drivers::table_structure(&connection, password.as_deref(), &table)).await?;
        Ok(Json(TableView {
            kind: kind_name(structure.kind).into(),
            row_estimate: structure.row_estimate,
            definition: structure.ddl,
        }))
    }

    #[tool(
        description = "Runs one statement that only reads (SELECT, find, GET…) on a connection and returns its rows. The server keeps it from changing anything; a statement that would is refused. Takes at most a minute."
    )]
    async fn query(
        &self,
        Parameters(request): Parameters<RunRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<RunResult>, String> {
        self.note_agent(&context);
        Ok(Json(self.read(request).await?))
    }

    #[tool(
        description = "Runs a statement that changes data (INSERT, UPDATE, DELETE, DDL, or the engine's equivalent) on a connection you may write to. On a connection that asks before changes, the user approves it in DBForge first; a declined one doesn't run. For reading, use query."
    )]
    async fn execute(
        &self,
        Parameters(request): Parameters<RunRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<RunResult>, String> {
        let agent = self.note_agent(&context);
        Ok(Json(self.change(agent, request).await?))
    }

    #[tool(
        description = "Lists the queries the user saved in DBForge that you may run: each with its SQL, its :name parameters, and the connections it runs on (one connection, or any connection of an engine). Give connectionId to list only the ones for that connection."
    )]
    async fn list_saved_queries(
        &self,
        Parameters(request): Parameters<ListSavedRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<SavedQueryList>, String> {
        self.note_agent(&context);
        Ok(Json(self.list_saved(request)?))
    }

    #[tool(
        description = "Runs a saved query by id with a value for each of its :name parameters, written into the SQL as literals of the connection's language. It only reads, as query does, unless you set allowChanges, which runs it as execute does: on a connection you may write to, approved by the user when it asks before changes."
    )]
    async fn run_saved_query(
        &self,
        Parameters(request): Parameters<RunSavedRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<RunResult>, String> {
        let agent = self.note_agent(&context);
        Ok(Json(self.run_saved(agent, request).await?))
    }

    #[tool(
        description = "Saves a query in DBForge's library so the user can run it again, with :name parameters for the values that change; with id, changes that saved query instead. The user approves every change in DBForge first; a declined one changes nothing."
    )]
    async fn save_query(
        &self,
        Parameters(request): Parameters<SaveQueryRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<SavedQueryView>, String> {
        let agent = self.note_agent(&context);
        Ok(Json(self.save_saved(agent, request).await?))
    }

    #[tool(
        description = "Deletes a saved query by id. The user approves it in DBForge first; a declined one deletes nothing."
    )]
    async fn delete_saved_query(
        &self,
        Parameters(request): Parameters<DeleteSavedRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let agent = self.note_agent(&context);
        self.delete_saved(agent, request).await
    }

    #[tool(
        description = "Shows a connection's settings, without any secret: its engine, host, port, database, user, SSL mode and files, SSH tunnel (host, port, user, how it signs in, key file), tag and what agents may do with it. Its password, SSH password or passphrase and keys are never shown."
    )]
    async fn connection_details(
        &self,
        Parameters(request): Parameters<ConnectionDetailsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ConnectionDetails>, String> {
        self.note_agent(&context);
        Ok(Json(self.details_of(request)?))
    }

    #[tool(
        description = "Creates a connection in DBForge, or with id changes one you may use, keeping what you leave out. You never send a password or other secret: the user types them in DBForge when approving (on a change, typing none keeps the saved ones). You can't set what agents may do with a connection nor its tag: the user chooses a new connection's access when approving. The user approves every create and change in DBForge first; a declined one changes nothing. A connection with a console's transaction open can't be changed."
    )]
    async fn save_connection(
        &self,
        Parameters(request): Parameters<SaveConnectionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ConnectionDetails>, String> {
        let agent = self.note_agent(&context);
        Ok(Json(self.save_conn(agent, request).await?))
    }

    #[tool(
        description = "Deletes a connection you may use, with its saved password, from DBForge. The user approves it in DBForge first; a declined one deletes nothing. A connection with a console's transaction open can't be deleted."
    )]
    async fn delete_connection(
        &self,
        Parameters(request): Parameters<DeleteConnectionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let agent = self.note_agent(&context);
        self.delete_conn(agent, request).await
    }

    #[tool(
        description = "Lists the backups (exports) in DBForge's exports folder that import_database can restore: each with its kind, the engines it restores into, its size and when it was written. Give connectionId to list only the ones that restore into that connection."
    )]
    async fn list_exports(
        &self,
        Parameters(request): Parameters<ListExportsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ExportList>, String> {
        self.note_agent(&context);
        Ok(Json(self.exports(request)?))
    }

    #[tool(
        description = "Backs up a whole database (PostgreSQL, MySQL, MariaDB, MongoDB or SQLite) into a new file in DBForge's exports folder, named by DBForge after the database and the minute; you can't choose the path. Options are per engine and default to everything. The user approves every backup in DBForge first; a declined one runs nothing. Waits until the backup ends and returns the file's name, which import_database takes."
    )]
    async fn export_database(
        &self,
        Parameters(request): Parameters<ExportDatabaseRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ExportedView>, String> {
        let agent = self.note_agent(&context);
        Ok(Json(self.backup(agent, request).await?))
    }

    #[tool(
        description = "Restores an export from DBForge's exports folder (a name from list_exports) into a database of a connection you may write to, creating the database when it doesn't exist; SQLite restores into the connection's own file. It changes data, and objects already there may be replaced. The user approves every restore in DBForge first; a declined one runs nothing. A failed restore removes a database it created."
    )]
    async fn import_database(
        &self,
        Parameters(request): Parameters<ImportDatabaseRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ImportedView>, String> {
        let agent = self.note_agent(&context);
        Ok(Json(self.restore(agent, request).await?))
    }
}

/// What the agent is told about the server, the same from the app and from the stdio bridge.
const INSTRUCTIONS: &str = "Query and change the databases saved in DBForge without their credentials. Start with list_connections; each connection says the language it takes and whether you may change data. The user sees every statement you run in DBForge's statement console. The user's saved queries are there too: list_saved_queries and run_saved_query run them with values for their :name parameters, and save_query and delete_saved_query change them once the user approves. export_database backs up a whole database into DBForge's exports folder and import_database restores one of those exports (list_exports lists them); the user approves every backup and restore first. connection_details shows a connection's settings without its secrets, and save_connection and delete_connection create, change or delete connections once the user approves; you never send or see a password, which the user types in DBForge.";

/// How DBForge's MCP server introduces itself, wherever the agent reaches it.
pub(crate) fn server_info() -> ServerConfig {
    ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        .with_server_info(Implementation::new("DBForge", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS.to_owned())
}

#[tool_handler]
impl rmcp::ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        server_info()
    }
}
