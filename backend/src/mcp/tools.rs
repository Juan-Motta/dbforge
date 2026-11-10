//! What agents can do: the MCP tools, each checking the connection's agent
//! access and running in a statement-log scope, so every statement an agent
//! causes shows in the app's console as the agent's.
//!
//! Tools never return a password or anything the app keeps secret: agents
//! name a connection by its id, and the app opens it with its own credentials.

use rmcp::RoleServer;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::schemars::{self, JsonSchema};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::approvals::{ApprovalRequest, Approvals};
use crate::connections::model::{AgentAccess, Connection};
use crate::connections::store::ConnectionStore;
use crate::connections::tags::TagStore;
use crate::drivers::{self, ObjectKind, QueryOutcome, TableRef};
use crate::error::AppError;
use crate::statement_log::{self, Origin, StatementLog};

/// The MCP server's handler: one per agent session, all sharing the app.
#[derive(Clone)]
pub struct Tools {
    app: AppHandle,
}

impl Tools {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    /// The connection `id` with its password, when agents may at least read it.
    /// One they may not use reads as unknown, so its existence isn't given away.
    fn open(&self, id: &str) -> Result<(Connection, Option<String>), String> {
        let store = self.app.state::<ConnectionStore>();
        let connection =
            store.get(id).ok().filter(|connection| connection.agent_access != AgentAccess::None).ok_or_else(|| {
                format!("No connection with id {id}. Call list_connections to see the ones you can use.")
            })?;
        let password = store.password(&connection.id).map_err(|error| error.to_string())?;
        Ok((connection, password))
    }

    /// Runs `work` with its statements recorded as the agent's, on `connection`.
    async fn as_agent<T>(
        &self,
        connection: &Connection,
        work: impl Future<Output = crate::error::Result<T>>,
    ) -> Result<T, String> {
        let log = self.app.state::<StatementLog>();
        statement_log::scoped(&log, &connection.id, Origin::Agent, work).await.map_err(|error| error.to_string())
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

#[tool_router]
impl Tools {
    #[tool(
        description = "Lists the database connections saved in DBForge that you may use, with the language each one takes and whether you may change data. Call this first."
    )]
    async fn list_connections(&self) -> Json<ConnectionList> {
        let tags = self.app.state::<TagStore>().list();
        let connections = self
            .app
            .state::<ConnectionStore>()
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
    ) -> Result<Json<ObjectList>, String> {
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
    ) -> Result<Json<TableView>, String> {
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
    async fn query(&self, Parameters(request): Parameters<RunRequest>) -> Result<Json<RunResult>, String> {
        let (connection, password) = self.open(&request.connection_id)?;
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let read = drivers::run_read_only(
            &connection,
            password.as_deref(),
            request.database.as_deref(),
            request.container.as_deref(),
            &request.statement,
            limit,
        );
        // A read leaves nothing half done, so one taking too long is simply dropped.
        let read = async {
            tokio::time::timeout(drivers::READ_ONLY_TIME_LIMIT, read).await.unwrap_or_else(|_| {
                Err(AppError::Invalid("The query took longer than a minute and was stopped.".into()))
            })
        };
        Ok(Json(self.as_agent(&connection, read).await?.into()))
    }

    #[tool(
        description = "Runs a statement that changes data (INSERT, UPDATE, DELETE, DDL, or the engine's equivalent) on a connection you may write to. On a connection that asks before changes, the user approves it in DBForge first; a declined one doesn't run. For reading, use query."
    )]
    async fn execute(
        &self,
        Parameters(request): Parameters<RunRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<RunResult>, String> {
        let (connection, password) = self.open(&request.connection_id)?;
        if connection.agent_access != AgentAccess::Write {
            return Err(format!(
                "{} is read-only for agents. Use query, or ask the user to give agents Read and write access to it in DBForge.",
                connection.name
            ));
        }
        let tag = self
            .app
            .state::<TagStore>()
            .list()
            .into_iter()
            .find(|tag| Some(&tag.id) == connection.environment.as_ref());
        if tag.as_ref().is_some_and(|tag| tag.confirm) {
            let request = ApprovalRequest {
                id: uuid::Uuid::new_v4().to_string(),
                agent: context.peer.peer_info().map(|info| info.client_info.name.clone()),
                connection_id: connection.id.clone(),
                connection_name: connection.name.clone(),
                tag: tag.map(|tag| tag.name),
                database: request.database.clone(),
                statement: request.statement.clone(),
            };
            if !self.app.state::<Approvals>().ask(&self.app, request).await {
                return Err("The user declined this change in DBForge, or didn't answer in time. Nothing ran.".into());
            }
        }
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let run = async {
            let (database, container) = (request.database.as_deref(), request.container.as_deref());
            let mut session = drivers::open_session(&connection, password.as_deref(), database, container).await?;
            session.run(&request.statement, limit).await
        };
        Ok(Json(self.as_agent(&connection, run).await?.into()))
    }
}

#[tool_handler(
    name = "DBForge",
    instructions = "Query and change the databases saved in DBForge without their credentials. Start with list_connections; each connection says the language it takes and whether you may change data. The user sees every statement you run in DBForge's statement console."
)]
impl rmcp::ServerHandler for Tools {}
