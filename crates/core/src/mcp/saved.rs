//! The user's saved queries, for agents: listed and run on the connections
//! agents may use, and created, changed or deleted once the user approves.
//!
//! A saved query is within an agent's reach when it can run somewhere the
//! agent may go: a query saved for one connection, when agents may use that
//! connection; one saved for any connection of an engine, when agents may use
//! at least one connection of it. Anything else reads as unknown, so its
//! existence isn't given away.

use std::collections::BTreeMap;

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

use super::approvals::{ApprovalRequest, ApprovalSubject, DraftScope, SavedQueryChange, SavedQueryDraft};
use super::tools::{RunRequest, RunResult, Tools};
use crate::connections::model::{AgentAccess, Connection, Engine};
use crate::events::Event;
use crate::saved_queries::params::{parameters, substitute};
use crate::saved_queries::{ParamKind, ParamValue, SavedQuery, SavedQueryInput, SavedScope, Stale, checked};

/// What the agent hears when the user says no, or nothing in time.
const DECLINED: &str = "The user declined this change in DBForge, or didn't answer in time. Nothing was changed.";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListSavedRequest {
    /// Only the queries that run on this connection (from `list_connections`).
    pub connection_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueryList {
    pub saved_queries: Vec<SavedQueryView>,
}

/// A saved query as agents see it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueryView {
    /// What `run_saved_query`, `save_query` and `delete_saved_query` take as `id`.
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// The folder the user keeps it in; none at the top level.
    pub folder: Option<String>,
    /// `connection`: saved for one connection. `engine`: for any connection of its engine.
    pub scope: String,
    /// The engine its SQL is written for.
    pub engine: String,
    /// The connections you may run it on.
    pub connections: Vec<ConnectionRef>,
    pub sql: String,
    /// Its `:name` parameters, in order of first use: each needs a value to run.
    pub parameters: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConnectionRef {
    pub id: String,
    pub name: String,
}

/// The type a parameter's value is written as in the SQL.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ValueKind {
    /// Quoted and escaped.
    Text,
    Number,
    /// `YYYY-MM-DD`.
    Date,
    /// `YYYY-MM-DD HH:MM[:SS]`.
    DateTime,
    /// `true` or `false`.
    Boolean,
    /// NULL; its `value` is ignored.
    Null,
}

impl From<ValueKind> for ParamKind {
    fn from(kind: ValueKind) -> Self {
        match kind {
            ValueKind::Text => ParamKind::Text,
            ValueKind::Number => ParamKind::Number,
            ValueKind::Date => ParamKind::Date,
            ValueKind::DateTime => ParamKind::DateTime,
            ValueKind::Boolean => ParamKind::Boolean,
            ValueKind::Null => ParamKind::Null,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValueInput {
    pub kind: ValueKind,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunSavedRequest {
    /// From `list_saved_queries`.
    pub id: String,
    /// Where to run it: one of the query's connections. Needed for a query
    /// saved for any connection of an engine; a query saved for one
    /// connection runs there.
    pub connection_id: Option<String>,
    /// A value for each parameter, by name without the colon, e.g.
    /// `{"status": {"kind": "text", "value": "paid"}}`.
    #[serde(default)]
    pub values: BTreeMap<String, ValueInput>,
    /// The database to run it in; the connection's own when left out.
    pub database: Option<String>,
    /// Cosmos DB only: the container a query reads.
    pub container: Option<String>,
    /// The most rows returned per result, 100 unless given; at most 1000.
    pub limit: Option<usize>,
    /// Run it as `execute` does, for a query that changes data. Left out, it
    /// runs as `query` does and a change is refused.
    #[serde(default)]
    pub allow_changes: bool,
}

/// Which connections a saved query is offered for: give one of the two.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScopeInput {
    /// Only this connection (from `list_connections`).
    pub connection_id: Option<String>,
    /// Any connection of this engine, as `list_connections` names it (e.g. `PostgreSQL`).
    pub engine: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveQueryRequest {
    /// The saved query to change; left out, a new one is created.
    pub id: Option<String>,
    /// Needed to create one; a change keeps the current name when left out.
    pub name: Option<String>,
    /// In the language of its connections, with `:name` placeholders for
    /// values asked when it runs. Needed to create one; a change keeps the
    /// current SQL when left out.
    pub sql: Option<String>,
    /// Left out, a change keeps the current one; empty removes it.
    pub description: Option<String>,
    /// Left out, a change keeps the current one; empty puts it at the top level.
    pub folder: Option<String>,
    /// Needed to create one; a change keeps the current scope when left out.
    pub scope: Option<ScopeInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteSavedRequest {
    /// From `list_saved_queries`.
    pub id: String,
}

/// The connections agents may use.
fn agent_connections(tools: &Tools) -> Vec<Connection> {
    tools
        .core
        .connections()
        .list()
        .into_iter()
        .filter(|connection| connection.agent_access != AgentAccess::None)
        .collect()
}

/// The connections among `connections` `query` runs on; none when it is out of reach.
fn runs_on<'a>(query: &SavedQuery, connections: &'a [Connection]) -> Vec<&'a Connection> {
    connections
        .iter()
        .filter(|connection| match &query.scope {
            SavedScope::Connection { connection_id } => connection.id == *connection_id,
            SavedScope::Engine => connection.engine == query.engine,
        })
        .collect()
}

fn not_found(id: &str) -> String {
    format!("No saved query with id {id}. Call list_saved_queries to see the ones you can use.")
}

/// What the agent hears when saved query `name` was changed in the app while its approval waited.
fn changed_meanwhile(name: &str) -> String {
    format!(
        "The saved query {name} changed while waiting for approval, so nothing was changed. Call list_saved_queries and try again."
    )
}

/// What the agent hears when saved query `name` was deleted in the app while its approval waited.
fn deleted_meanwhile(name: &str) -> String {
    format!(
        "The saved query {name} was deleted while waiting for approval, so nothing was changed. Call list_saved_queries to see the ones you can use."
    )
}

/// How `list_connections` and the saved queries' `engine` name it, or its id in the config.
fn names_engine(engine: Engine, text: &str) -> bool {
    let id = serde_json::to_value(engine).ok();
    engine.label().eq_ignore_ascii_case(text) || id.as_ref().and_then(|id| id.as_str()) == Some(&text.to_lowercase())
}

/// What the agent hears when saved query `name` was changed or deleted while its approval waited.
fn stale(name: &str, why: Stale) -> String {
    match why {
        Stale::Changed => changed_meanwhile(name),
        Stale::Deleted => deleted_meanwhile(name),
    }
}

impl Tools {
    /// Saved query `id` and the connections the agent may run it on, when it is within reach.
    fn reachable(&self, id: &str) -> Result<(SavedQuery, Vec<Connection>), String> {
        let query = self.core.saved_queries().get(id).map_err(|_| not_found(id))?;
        let connections = agent_connections(self);
        let on: Vec<Connection> = runs_on(&query, &connections).into_iter().cloned().collect();
        if on.is_empty() {
            return Err(not_found(id));
        }
        Ok((query, on))
    }

    fn view(query: SavedQuery, on: &[&Connection]) -> SavedQueryView {
        SavedQueryView {
            parameters: parameters(&query.sql, query.engine),
            scope: match query.scope {
                SavedScope::Connection { .. } => "connection",
                SavedScope::Engine => "engine",
            }
            .into(),
            engine: query.engine.label().into(),
            connections: on
                .iter()
                .map(|connection| ConnectionRef { id: connection.id.clone(), name: connection.name.clone() })
                .collect(),
            id: query.id,
            name: query.name,
            description: query.description,
            folder: query.folder,
            sql: query.sql,
        }
    }

    pub(super) fn list_saved(&self, request: ListSavedRequest) -> Result<SavedQueryList, String> {
        let only = match &request.connection_id {
            Some(id) => Some(self.open(id)?.0),
            None => None,
        };
        let connections = agent_connections(self);
        let saved_queries = self
            .core
            .saved_queries()
            .list()
            .into_iter()
            .filter_map(|query| {
                let on = runs_on(&query, &connections);
                let shown = match &only {
                    Some(only) => on.iter().any(|connection| connection.id == only.id),
                    None => !on.is_empty(),
                };
                shown.then(|| Self::view(query, &on))
            })
            .collect();
        Ok(SavedQueryList { saved_queries })
    }

    pub(super) async fn run_saved(&self, agent: Option<String>, request: RunSavedRequest) -> Result<RunResult, String> {
        let (query, on) = self.reachable(&request.id)?;
        let connection = match (&query.scope, request.connection_id.as_deref()) {
            (_, Some(id)) => on.iter().find(|connection| connection.id == id).ok_or_else(|| {
                let names: Vec<String> =
                    on.iter().map(|connection| format!("{} ({})", connection.name, connection.id)).collect();
                format!("{} doesn't run on connection {id}. It runs on: {}.", query.name, names.join(", "))
            })?,
            (SavedScope::Connection { .. }, None) => &on[0],
            (SavedScope::Engine, None) => {
                return Err(format!(
                    "{} runs on any {} connection: give connectionId, from its connections in list_saved_queries.",
                    query.name,
                    query.engine.label()
                ));
            }
        };
        let values: BTreeMap<String, ParamValue> = request
            .values
            .into_iter()
            .map(|(name, value)| (name, ParamValue { kind: value.kind.into(), value: value.value }))
            .collect();
        let sql = substitute(&query.sql, connection.engine, &values)
            .map_err(|error| {
                format!("{error} Give each parameter in values, e.g. {{\"kind\": \"text\", \"value\": \"paid\"}}.")
            })?
            .sql;
        let run = RunRequest {
            connection_id: connection.id.clone(),
            statement: sql,
            database: request.database,
            container: request.container,
            limit: request.limit,
        };
        let result = if request.allow_changes { self.change(agent, run).await? } else { self.read(run).await? };
        // The user's remembered values are theirs: an agent's run only counts as a run.
        if self.core.saved_queries().mark_run(&query.id, None).is_ok() {
            self.core.events().emit(Event::SavedQueriesChanged);
        }
        Ok(result)
    }

    /// The scope `input` names and the engine its SQL is written for, when the agent may reach it.
    fn scope_of(&self, input: ScopeInput) -> Result<(SavedScope, Engine), String> {
        match (input.connection_id, input.engine) {
            (Some(id), None) => {
                let (connection, _) = self.open(&id)?;
                Ok((SavedScope::Connection { connection_id: connection.id }, connection.engine))
            }
            (None, Some(name)) => {
                let connections = agent_connections(self);
                let engine = connections.iter().map(|connection| connection.engine).find(|engine| names_engine(*engine, &name));
                engine.map(|engine| (SavedScope::Engine, engine)).ok_or_else(|| {
                    let mut engines: Vec<&str> = connections.iter().map(|connection| connection.engine.label()).collect();
                    engines.sort_unstable();
                    engines.dedup();
                    format!("You may use no {name} connection. The engines you may use: {}.", engines.join(", "))
                })
            }
            _ => Err("Give scope as {\"connectionId\": …} for one connection, or {\"engine\": …} for any connection of an engine.".into()),
        }
    }

    /// A saved query made of `input`, as the approval dialog shows it.
    fn draft(&self, input: SavedQueryInput) -> SavedQueryDraft {
        let SavedQueryInput { name, sql, description, folder, scope, engine } = input;
        let scope = match scope {
            SavedScope::Connection { connection_id } => DraftScope::Connection {
                connection_name: self
                    .core
                    .connections()
                    .get(&connection_id)
                    .map_or_else(|_| connection_id.clone(), |connection| connection.name),
                connection_id,
            },
            SavedScope::Engine => DraftScope::Engine,
        };
        SavedQueryDraft { name, sql, description, folder, scope, engine }
    }

    /// Waits for the user to approve `change` in the app.
    async fn approve(&self, agent: Option<String>, change: SavedQueryChange) -> Result<(), String> {
        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            agent,
            subject: ApprovalSubject::SavedQuery(Box::new(change)),
        };
        match self.core.mcp_approvals().ask(self.core.events().as_ref(), request).await {
            true => Ok(()),
            false => Err(DECLINED.into()),
        }
    }

    pub(super) async fn save_saved(
        &self,
        agent: Option<String>,
        request: SaveQueryRequest,
    ) -> Result<SavedQueryView, String> {
        let existing = match &request.id {
            Some(id) => Some(self.reachable(id)?.0),
            None => None,
        };
        let required = |what: &str| format!("Give {what} to create a saved query.");
        let (scope, engine) = match (request.scope, &existing) {
            (Some(scope), _) => self.scope_of(scope)?,
            (None, Some(query)) => (query.scope.clone(), query.engine),
            (None, None) => return Err(required("scope")),
        };
        let input = SavedQueryInput {
            name: request
                .name
                .or_else(|| existing.as_ref().map(|query| query.name.clone()))
                .ok_or_else(|| required("name"))?,
            sql: request
                .sql
                .or_else(|| existing.as_ref().map(|query| query.sql.clone()))
                .ok_or_else(|| required("sql"))?,
            description: request.description.or_else(|| existing.as_ref().and_then(|query| query.description.clone())),
            folder: request.folder.or_else(|| existing.as_ref().and_then(|query| query.folder.clone())),
            scope,
            engine,
        };
        let input = checked(input).map_err(|error| error.to_string())?;
        let after = self.draft(input.clone());
        let change = match &existing {
            None => SavedQueryChange::Create { query: after },
            Some(query) => SavedQueryChange::Update { before: self.draft(query.content()), after },
        };
        if let Some(query) = existing.as_ref().filter(|_| change.changed_fields().is_empty() && !change.changes_sql()) {
            // Nothing to change, so nothing to ask.
            let connections = agent_connections(self);
            return Ok(Self::view(query.clone(), &runs_on(query, &connections)));
        }
        self.approve(agent, change).await?;
        let store = self.core.saved_queries();
        let saved = match &existing {
            None => store.save(None, input).map_err(|error| error.to_string())?,
            // Only onto the query the user approved the change of: a change
            // made meanwhile, by the user or another approved one, is kept.
            Some(query) => store
                .save_if_unchanged(&query.id, &query.content(), input)
                .map_err(|error| error.to_string())?
                .map_err(|why| stale(&query.name, why))?,
        };
        self.core.events().emit(Event::SavedQueriesChanged);
        let connections = agent_connections(self);
        let on = runs_on(&saved, &connections);
        Ok(Self::view(saved, &on))
    }

    pub(super) async fn delete_saved(
        &self,
        agent: Option<String>,
        request: DeleteSavedRequest,
    ) -> Result<String, String> {
        let (query, _) = self.reachable(&request.id)?;
        self.approve(agent, SavedQueryChange::Delete { query: self.draft(query.content()) }).await?;
        self.core
            .saved_queries()
            .remove_if_unchanged(&query.id, &query.content())
            .map_err(|error| error.to_string())?
            .map_err(|why| stale(&query.name, why))?;
        self.core.events().emit(Event::SavedQueriesChanged);
        Ok(format!("Deleted the saved query {}.", query.name))
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    use super::*;
    use crate::connections::transport::{Ssl, SslMode};
    use crate::core::{Core, Dirs};
    use crate::drivers;
    use crate::events::Events;
    use crate::mcp::SavedQueryField;

    /// What the app would hear, the approvals handed to the test to answer.
    pub(in crate::mcp) struct Heard {
        events: Mutex<Vec<Event>>,
        asked: UnboundedSender<ApprovalRequest>,
    }

    impl Events for Heard {
        fn emit(&self, event: Event) {
            if let Event::McpApprovalAsked(request) = &event {
                let _ = self.asked.send(request.clone());
            }
            self.events.lock().unwrap().push(event);
        }
    }

    impl Heard {
        fn changed(&self) -> usize {
            self.events.lock().unwrap().iter().filter(|event| matches!(event, Event::SavedQueriesChanged)).count()
        }
    }

    pub(in crate::mcp) struct Setup {
        _dir: tempfile::TempDir,
        pub(in crate::mcp) dir: std::path::PathBuf,
        pub(in crate::mcp) tools: Tools,
        heard: Arc<Heard>,
        asked: UnboundedReceiver<ApprovalRequest>,
    }

    impl Setup {
        pub(in crate::mcp) fn core(&self) -> &Core {
            &self.tools.core
        }

        /// Tools of their own, to call while [`Setup::answering`] holds the setup.
        pub(in crate::mcp) fn tools(&self) -> Tools {
            Tools::new(self.tools.core.clone())
        }

        /// A connection named `name` (also its id) agents may reach as `access`.
        pub(in crate::mcp) fn connection(
            &self,
            name: &str,
            engine: Engine,
            access: AgentAccess,
            tag: Option<&str>,
        ) -> Connection {
            let database = self.dir.join(format!("{name}.db"));
            if engine == Engine::Sqlite && !database.exists() {
                // An empty file is an empty SQLite database.
                std::fs::write(&database, b"").unwrap();
            }
            let connection = Connection {
                id: name.into(),
                name: name.into(),
                engine,
                environment: tag.map(Into::into),
                host: "localhost".into(),
                port: 5432,
                database: database.to_string_lossy().into_owned(),
                username: String::new(),
                ssl: Ssl { mode: SslMode::Disable, ..Default::default() },
                ssh: Default::default(),
                srv: false,
                agent_access: access,
            };
            self.core().connections().upsert(connection.clone()).unwrap();
            connection
        }

        fn saved(&self, name: &str, sql: &str, scope: SavedScope, engine: Engine) -> SavedQuery {
            let input =
                SavedQueryInput { name: name.into(), sql: sql.into(), description: None, folder: None, scope, engine };
            self.core().saved_queries().save(None, input).unwrap()
        }

        /// Runs `work`, answering every approval it asks for with `approve`;
        /// its outcome and the requests asked.
        pub(in crate::mcp) async fn answering<T>(
            &mut self,
            approve: bool,
            work: impl Future<Output = T>,
        ) -> (T, Vec<ApprovalRequest>) {
            let approvals = self.tools.core.clone();
            let asked = &mut self.asked;
            let seen = Mutex::new(Vec::new());
            let answer = async {
                while let Some(request) = asked.recv().await {
                    approvals.mcp_approvals().answer(&request.id, approve);
                    seen.lock().unwrap().push(request);
                }
            };
            let outcome = tokio::select! {
                outcome = Box::pin(work) => outcome,
                () = answer => unreachable!("the core keeps its events"),
            };
            (outcome, seen.into_inner().unwrap())
        }

        /// Runs `work`, answering every approval it asks for as `answer`
        /// says: approved or not, and what the user typed; its outcome and the requests asked.
        pub(in crate::mcp) async fn answering_with<T>(
            &mut self,
            answer: impl Fn(&ApprovalRequest) -> (bool, Option<crate::mcp::ApprovalSecrets>),
            work: impl Future<Output = T>,
        ) -> (T, Vec<ApprovalRequest>) {
            let approvals = self.tools.core.clone();
            let asked = &mut self.asked;
            let seen = Mutex::new(Vec::new());
            let answering = async {
                while let Some(request) = asked.recv().await {
                    let (approve, secrets) = answer(&request);
                    approvals.mcp_approvals().answer_with(&request.id, approve, secrets);
                    seen.lock().unwrap().push(request);
                }
            };
            let outcome = tokio::select! {
                outcome = Box::pin(work) => outcome,
                () = answering => unreachable!("the core keeps its events"),
            };
            (outcome, seen.into_inner().unwrap())
        }

        /// Every event the app would have heard so far.
        pub(in crate::mcp) fn events(&self) -> Vec<Event> {
            self.heard.events.lock().unwrap().clone()
        }
    }

    pub(in crate::mcp) fn setup() -> Setup {
        crate::tunnel::tests::setup();
        let dir = tempfile::tempdir().unwrap();
        let (asked_sender, asked) = unbounded_channel();
        let heard = Arc::new(Heard { events: Mutex::new(Vec::new()), asked: asked_sender });
        let dirs = Dirs { config: dir.path().join("config"), data: dir.path().join("data") };
        let core = Core::load(dirs, heard.clone()).unwrap();
        Setup { dir: dir.path().to_owned(), _dir: dir, tools: Tools::new(core), heard, asked }
    }

    fn on(connection: &Connection) -> SavedScope {
        SavedScope::Connection { connection_id: connection.id.clone() }
    }

    fn names(list: &SavedQueryList) -> Vec<&str> {
        list.saved_queries.iter().map(|query| query.name.as_str()).collect()
    }

    fn all(tools: &Tools) -> SavedQueryList {
        tools.list_saved(ListSavedRequest { connection_id: None }).unwrap()
    }

    #[tokio::test]
    async fn lists_only_the_queries_an_agent_may_reach() {
        let s = setup();
        let shop = s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let hidden = s.connection("hidden", Engine::Postgres, AgentAccess::None, None);
        s.connection("legacy", Engine::Mysql, AgentAccess::None, None);
        s.saved("On shop", "select :id", on(&shop), Engine::Postgres);
        s.saved("On hidden", "select 1", on(&hidden), Engine::Postgres);
        s.saved("Any postgres", "select 2", SavedScope::Engine, Engine::Postgres);
        s.saved("Any mysql", "select 3", SavedScope::Engine, Engine::Mysql);
        s.saved(
            "On a deleted one",
            "select 4",
            SavedScope::Connection { connection_id: "gone".into() },
            Engine::Postgres,
        );

        let list = all(&s.tools);
        assert_eq!(names(&list), ["Any postgres", "On shop"]);
        let any = &list.saved_queries[0];
        assert_eq!((any.scope.as_str(), any.engine.as_str()), ("engine", "PostgreSQL"));
        let ids: Vec<&str> = any.connections.iter().map(|connection| connection.id.as_str()).collect();
        assert_eq!(ids, ["shop"], "only the connections of the engine agents may use");
        let one = &list.saved_queries[1];
        assert_eq!(
            (one.scope.as_str(), one.parameters.as_slice(), one.sql.as_str()),
            ("connection", &["id".to_string()][..], "select :id")
        );
        assert_eq!(one.connections[0].name, "shop");
    }

    #[tokio::test]
    async fn lists_the_queries_of_one_connection_it_may_use() {
        let s = setup();
        let shop = s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let crm = s.connection("crm", Engine::Postgres, AgentAccess::Write, None);
        let hidden = s.connection("hidden", Engine::Postgres, AgentAccess::None, None);
        s.saved("On shop", "select 1", on(&shop), Engine::Postgres);
        s.saved("On crm", "select 1", on(&crm), Engine::Postgres);
        s.saved("Any postgres", "select 2", SavedScope::Engine, Engine::Postgres);

        let only = s.tools.list_saved(ListSavedRequest { connection_id: Some("crm".into()) }).unwrap();
        assert_eq!(names(&only), ["Any postgres", "On crm"]);
        let error = s.tools.list_saved(ListSavedRequest { connection_id: Some(hidden.id) }).unwrap_err();
        assert!(error.starts_with("No connection with id hidden."), "{error}");
    }

    fn run(id: &str, connection: Option<&str>, values: &[(&str, ValueKind, &str)]) -> RunSavedRequest {
        RunSavedRequest {
            id: id.into(),
            connection_id: connection.map(Into::into),
            values: values
                .iter()
                .map(|(name, kind, value)| (name.to_string(), ValueInput { kind: *kind, value: value.to_string() }))
                .collect(),
            database: None,
            container: None,
            limit: None,
            allow_changes: false,
        }
    }

    /// A table `items` with two rows.
    pub(in crate::mcp) async fn items(connection: &Connection) {
        let mut session = drivers::open_session(connection, None, None, None).await.unwrap();
        session.run("CREATE TABLE items (id INTEGER PRIMARY KEY, note TEXT); INSERT INTO items VALUES (1, 'a'), (2, 'it''s')", 10).await.unwrap();
    }

    pub(in crate::mcp) async fn count(connection: &Connection) -> String {
        let outcome =
            drivers::run_read_only(connection, None, None, None, "SELECT count(*) FROM items", 1).await.unwrap();
        outcome.results[0].rows[0][0].clone().unwrap()
    }

    #[tokio::test]
    async fn runs_a_saved_query_with_its_values_as_literals_and_keeps_the_user_s_values() {
        let s = setup();
        let db = s.connection("db", Engine::Sqlite, AgentAccess::Read, None);
        items(&db).await;
        let query = s.saved("By note", "SELECT id FROM items WHERE note = :note OR id = :id", on(&db), Engine::Sqlite);

        let result = s
            .tools
            .run_saved(None, run(&query.id, None, &[("note", ValueKind::Text, "it's"), ("id", ValueKind::Number, "1")]))
            .await
            .unwrap();
        assert_eq!(result.results[0].rows, [vec![Some("1".to_string())], vec![Some("2".to_string())]]);
        let after = s.core().saved_queries().get(&query.id).unwrap();
        assert!(after.last_run_at.is_some());
        assert!(after.last_values.is_empty() && after.recent_values.is_empty(), "the user's values stay theirs");
        assert_eq!(s.heard.changed(), 1, "the Library hears of the run");
    }

    #[tokio::test]
    async fn a_missing_or_invalid_value_is_named_and_nothing_runs() {
        let s = setup();
        let db = s.connection("db", Engine::Sqlite, AgentAccess::Read, None);
        let query = s.saved("By id", "SELECT :id, :day", on(&db), Engine::Sqlite);

        let missing =
            s.tools.run_saved(None, run(&query.id, None, &[("id", ValueKind::Number, "1")])).await.unwrap_err();
        assert!(missing.starts_with("Enter a value for :day."), "{missing}");
        let invalid = s
            .tools
            .run_saved(None, run(&query.id, None, &[("id", ValueKind::Number, "x"), ("day", ValueKind::Null, "")]))
            .await
            .unwrap_err();
        assert!(invalid.starts_with(":id: \"x\" is not a number."), "{invalid}");
        assert_eq!(s.core().saved_queries().get(&query.id).unwrap().last_run_at, None);
    }

    #[tokio::test]
    async fn a_read_refuses_changes_and_a_change_needs_write_access() {
        let mut s = setup();
        let read = s.connection("read", Engine::Sqlite, AgentAccess::Read, None);
        items(&read).await;
        let delete = s.saved("Delete one", "DELETE FROM items WHERE id = :id", SavedScope::Engine, Engine::Sqlite);
        let id = [("id", ValueKind::Number, "1")];

        assert!(s.tools.run_saved(None, run(&delete.id, Some("read"), &id)).await.is_err(), "a read changes nothing");
        let refused = s
            .tools
            .run_saved(None, RunSavedRequest { allow_changes: true, ..run(&delete.id, Some("read"), &id) })
            .await
            .unwrap_err();
        assert!(refused.starts_with("read is read-only for agents."), "{refused}");
        assert_eq!(count(&read).await, "2");

        let write = s.connection("write", Engine::Sqlite, AgentAccess::Write, None);
        items(&write).await;
        let request = RunSavedRequest { allow_changes: true, ..run(&delete.id, Some("write"), &id) };
        let tools = s.tools();
        let (outcome, asked) = s.answering(true, tools.run_saved(None, request)).await;
        outcome.unwrap();
        assert!(asked.is_empty(), "no tag asks first");
        assert_eq!(count(&write).await, "1");
    }

    #[tokio::test]
    async fn a_change_on_a_tag_that_asks_first_waits_for_the_user() {
        let mut s = setup();
        let prod = s.connection("prod", Engine::Sqlite, AgentAccess::Write, Some("production"));
        items(&prod).await;
        let delete = s.saved("Delete one", "DELETE FROM items WHERE id = :id", on(&prod), Engine::Sqlite);
        let request =
            || RunSavedRequest { allow_changes: true, ..run(&delete.id, None, &[("id", ValueKind::Number, "2")]) };

        let tools = s.tools();
        let (declined, asked) = s.answering(false, tools.run_saved(Some("codex".into()), request())).await;
        assert!(declined.unwrap_err().contains("Nothing ran."));
        let [asked] = asked.as_slice() else { panic!("{asked:?}") };
        assert_eq!(asked.agent.as_deref(), Some("codex"));
        assert!(
            matches!(&asked.subject, ApprovalSubject::Statement { statement, tag, .. } if statement == "DELETE FROM items WHERE id = 2" && tag.as_deref() == Some("PROD")),
            "{asked:?}"
        );
        assert_eq!(count(&prod).await, "2");

        let (approved, _) = s.answering(true, tools.run_saved(None, request())).await;
        approved.unwrap();
        assert_eq!(count(&prod).await, "1");
    }

    #[tokio::test]
    async fn runs_where_its_scope_allows_only() {
        let s = setup();
        let one = s.connection("one", Engine::Sqlite, AgentAccess::Read, None);
        let other = s.connection("other", Engine::Sqlite, AgentAccess::Read, None);
        let hidden = s.connection("hidden", Engine::Sqlite, AgentAccess::None, None);
        let saved_on_one = s.saved("On one", "SELECT 1", on(&one), Engine::Sqlite);
        let any = s.saved("Any", "SELECT 2", SavedScope::Engine, Engine::Sqlite);
        let on_hidden = s.saved("On hidden", "SELECT 3", on(&hidden), Engine::Sqlite);

        assert!(s.tools.run_saved(None, run(&saved_on_one.id, None, &[])).await.is_ok(), "runs on its connection");
        let elsewhere = s.tools.run_saved(None, run(&saved_on_one.id, Some(&other.id), &[])).await.unwrap_err();
        assert_eq!(elsewhere, "On one doesn't run on connection other. It runs on: one (one).");
        let no_connection = s.tools.run_saved(None, run(&any.id, None, &[])).await.unwrap_err();
        assert!(no_connection.contains("give connectionId"), "{no_connection}");
        assert!(s.tools.run_saved(None, run(&any.id, Some(&other.id), &[])).await.is_ok());
        let hidden_run = s.tools.run_saved(None, run(&any.id, Some(&hidden.id), &[])).await.unwrap_err();
        assert!(hidden_run.contains("doesn't run on connection hidden"), "{hidden_run}");
        let unknown = s.tools.run_saved(None, run(&on_hidden.id, None, &[])).await.unwrap_err();
        assert_eq!(unknown, not_found(&on_hidden.id), "out of reach reads as unknown");
    }

    fn create(name: &str, scope: ScopeInput) -> SaveQueryRequest {
        SaveQueryRequest {
            id: None,
            name: Some(name.into()),
            sql: Some("select * from orders where status = :status".into()),
            description: Some("By status".into()),
            folder: Some("Ops".into()),
            scope: Some(scope),
        }
    }

    fn connection_scope(id: &str) -> ScopeInput {
        ScopeInput { connection_id: Some(id.into()), engine: None }
    }

    fn engine_scope(engine: &str) -> ScopeInput {
        ScopeInput { connection_id: None, engine: Some(engine.into()) }
    }

    #[tokio::test]
    async fn creating_asks_the_user_and_a_declined_one_saves_nothing() {
        let mut s = setup();
        s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let tools = s.tools();

        let (declined, asked) = s
            .answering(false, tools.save_saved(Some("codex".into()), create("Orders", connection_scope("shop"))))
            .await;
        assert_eq!(declined.unwrap_err(), DECLINED);
        let [asked] = asked.as_slice() else { panic!("{asked:?}") };
        let expected = SavedQueryDraft {
            name: "Orders".into(),
            sql: "select * from orders where status = :status".into(),
            description: Some("By status".into()),
            folder: Some("Ops".into()),
            scope: DraftScope::Connection { connection_id: "shop".into(), connection_name: "shop".into() },
            engine: Engine::Postgres,
        };
        assert_eq!(
            asked.subject,
            ApprovalSubject::SavedQuery(Box::new(SavedQueryChange::Create { query: expected.clone() }))
        );
        assert_eq!(asked.agent.as_deref(), Some("codex"));
        assert!(s.core().saved_queries().list().is_empty());
        assert_eq!(s.heard.changed(), 0);

        let (created, _) = s.answering(true, tools.save_saved(None, create("Orders", connection_scope("shop")))).await;
        let created = created.unwrap();
        assert_eq!((created.name.as_str(), created.parameters.as_slice()), ("Orders", &["status".to_string()][..]));
        let stored = s.core().saved_queries().get(&created.id).unwrap();
        assert_eq!(
            stored.content(),
            SavedQueryInput {
                name: expected.name,
                sql: expected.sql,
                description: expected.description,
                folder: expected.folder,
                scope: SavedScope::Connection { connection_id: "shop".into() },
                engine: Engine::Postgres,
            }
        );
        assert_eq!(s.heard.changed(), 1, "the Library hears of it");
    }

    #[tokio::test]
    async fn an_engine_scope_names_an_engine_the_agent_may_use() {
        let mut s = setup();
        s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        s.connection("legacy", Engine::Mysql, AgentAccess::None, None);
        let tools = s.tools();

        let (created, _) = s.answering(true, tools.save_saved(None, create("Any", engine_scope("postgresql")))).await;
        let created = created.unwrap();
        assert_eq!((created.scope.as_str(), created.engine.as_str()), ("engine", "PostgreSQL"));
        let (by_id, _) = s.answering(true, tools.save_saved(None, create("Any too", engine_scope("postgres")))).await;
        assert!(by_id.is_ok(), "the engine's id names it too");

        let (mysql, asked) = s.answering(true, tools.save_saved(None, create("Mysql", engine_scope("MySQL")))).await;
        assert_eq!(mysql.unwrap_err(), "You may use no MySQL connection. The engines you may use: PostgreSQL.");
        assert!(asked.is_empty(), "nothing out of reach is asked");
        let (hidden, _) = s.answering(true, tools.save_saved(None, create("Hidden", connection_scope("legacy")))).await;
        assert!(hidden.unwrap_err().starts_with("No connection with id legacy."));
        let (both, _) = s
            .answering(
                true,
                tools.save_saved(
                    None,
                    create(
                        "Both",
                        ScopeInput { connection_id: Some("shop".into()), engine: Some("PostgreSQL".into()) },
                    ),
                ),
            )
            .await;
        assert!(both.unwrap_err().starts_with("Give scope as"));
        assert_eq!(s.core().saved_queries().list().len(), 2);
    }

    #[tokio::test]
    async fn creating_needs_a_name_sql_and_scope() {
        let mut s = setup();
        s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let tools = s.tools();
        for (request, missing) in [
            (
                SaveQueryRequest { scope: None, ..create("x", connection_scope("shop")) },
                "Give scope to create a saved query.",
            ),
            (
                SaveQueryRequest { name: None, ..create("x", connection_scope("shop")) },
                "Give name to create a saved query.",
            ),
            (
                SaveQueryRequest { sql: None, ..create("x", connection_scope("shop")) },
                "Give sql to create a saved query.",
            ),
            (create("   ", connection_scope("shop")), "Enter a name for the saved query."),
        ] {
            let (refused, asked) = s.answering(true, tools.save_saved(None, request)).await;
            assert_eq!(refused.unwrap_err(), missing);
            assert!(asked.is_empty());
        }
        assert!(s.core().saved_queries().list().is_empty());
    }

    #[tokio::test]
    async fn updating_shows_before_and_after_and_keeps_what_is_left_out() {
        let mut s = setup();
        let shop = s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let input = SavedQueryInput {
            name: "Orders".into(),
            sql: "select 1".into(),
            description: Some("All".into()),
            folder: Some("Ops".into()),
            scope: on(&shop),
            engine: Engine::Postgres,
        };
        let query = s.core().saved_queries().save(None, input.clone()).unwrap();
        let tools = s.tools();
        let update = || SaveQueryRequest {
            id: Some(query.id.clone()),
            name: Some("Paid orders".into()),
            sql: Some("select 2".into()),
            description: None,
            folder: Some(String::new()),
            scope: Some(engine_scope("PostgreSQL")),
        };

        let (declined, asked) = s.answering(false, tools.save_saved(None, update())).await;
        assert_eq!(declined.unwrap_err(), DECLINED);
        assert_eq!(s.core().saved_queries().get(&query.id).unwrap(), query, "nothing changed");
        let [asked] = asked.as_slice() else { panic!("{asked:?}") };
        let ApprovalSubject::SavedQuery(change) = &asked.subject else { panic!("{asked:?}") };
        let SavedQueryChange::Update { before, after } = change.as_ref() else { panic!("{change:?}") };
        assert_eq!((before.name.as_str(), after.name.as_str()), ("Orders", "Paid orders"));
        assert_eq!((before.sql.as_str(), after.sql.as_str()), ("select 1", "select 2"));
        assert_eq!(after.description.as_deref(), Some("All"), "left out, it is kept");
        assert_eq!((before.folder.as_deref(), after.folder.as_deref()), (Some("Ops"), None), "empty, it is removed");
        assert_eq!(change.changed_fields(), [SavedQueryField::Name, SavedQueryField::Folder, SavedQueryField::Scope]);

        let (approved, _) = s.answering(true, tools.save_saved(None, update())).await;
        assert_eq!(approved.unwrap().id, query.id);
        let stored = s.core().saved_queries().get(&query.id).unwrap();
        assert_eq!(
            (stored.name.as_str(), stored.scope.clone(), stored.folder),
            ("Paid orders", SavedScope::Engine, None)
        );
        assert_eq!(s.heard.changed(), 1);
    }

    #[tokio::test]
    async fn an_update_that_changes_nothing_asks_nothing() {
        let mut s = setup();
        let shop = s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let query = s.saved("Orders", "select 1", on(&shop), Engine::Postgres);
        let tools = s.tools();
        let same = SaveQueryRequest {
            id: Some(query.id.clone()),
            name: Some(" Orders ".into()),
            sql: None,
            description: None,
            folder: None,
            scope: None,
        };
        let (kept, asked) = s.answering(true, tools.save_saved(None, same)).await;
        assert_eq!(kept.unwrap().id, query.id);
        assert!(asked.is_empty());
        assert_eq!(s.core().saved_queries().get(&query.id).unwrap(), query);
    }

    #[tokio::test]
    async fn a_query_out_of_reach_can_t_be_changed_or_moved_out_of_reach() {
        let mut s = setup();
        let shop = s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let hidden = s.connection("hidden", Engine::Postgres, AgentAccess::None, None);
        let secret = s.saved("Secret", "select 1", on(&hidden), Engine::Postgres);
        let mine = s.saved("Mine", "select 1", on(&shop), Engine::Postgres);
        let tools = s.tools();

        let rename = SaveQueryRequest {
            id: Some(secret.id.clone()),
            name: Some("x".into()),
            sql: None,
            description: None,
            folder: None,
            scope: None,
        };
        let (refused, asked) = s.answering(true, tools.save_saved(None, rename)).await;
        assert_eq!(refused.unwrap_err(), not_found(&secret.id));
        let (deleted, _) =
            s.answering(true, tools.delete_saved(None, DeleteSavedRequest { id: secret.id.clone() })).await;
        assert_eq!(deleted.unwrap_err(), not_found(&secret.id));
        let mv = SaveQueryRequest {
            id: Some(mine.id.clone()),
            name: None,
            sql: None,
            description: None,
            folder: None,
            scope: Some(connection_scope("hidden")),
        };
        let (moved, more) = s.answering(true, tools.save_saved(None, mv)).await;
        assert!(moved.unwrap_err().starts_with("No connection with id hidden."));
        assert!(asked.is_empty() && more.is_empty());
        assert_eq!(s.core().saved_queries().list(), {
            let mut both = vec![mine, secret];
            both.sort_by(|a, b| a.name.cmp(&b.name));
            both
        });
    }

    #[tokio::test]
    async fn deleting_asks_the_user_first() {
        let mut s = setup();
        let shop = s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let query = s.saved("Orders", "select 1", on(&shop), Engine::Postgres);
        let tools = s.tools();
        let delete = || tools.delete_saved(Some("codex".into()), DeleteSavedRequest { id: query.id.clone() });

        let (declined, asked) = s.answering(false, delete()).await;
        assert_eq!(declined.unwrap_err(), DECLINED);
        let [asked] = asked.as_slice() else { panic!("{asked:?}") };
        assert!(
            matches!(&asked.subject, ApprovalSubject::SavedQuery(change) if matches!(change.as_ref(), SavedQueryChange::Delete { query: draft } if draft.name == "Orders" && draft.sql == "select 1")),
            "{asked:?}"
        );
        assert_eq!(s.core().saved_queries().list(), std::slice::from_ref(&query));

        let (deleted, _) = s.answering(true, delete()).await;
        assert_eq!(deleted.unwrap(), "Deleted the saved query Orders.");
        assert!(s.core().saved_queries().list().is_empty());
        assert_eq!(s.heard.changed(), 1);
    }

    fn rename(id: &str, name: &str) -> SaveQueryRequest {
        SaveQueryRequest {
            id: Some(id.into()),
            name: Some(name.into()),
            sql: None,
            description: None,
            folder: None,
            scope: None,
        }
    }

    fn resql(id: &str, sql: &str) -> SaveQueryRequest {
        SaveQueryRequest {
            id: Some(id.into()),
            name: None,
            sql: Some(sql.into()),
            description: None,
            folder: None,
            scope: None,
        }
    }

    /// Runs `work`, doing `meanwhile` to the core as each approval is asked
    /// and then approving it; its outcome.
    async fn approving_after<T>(s: &mut Setup, meanwhile: impl Fn(&Core), work: impl Future<Output = T>) -> T {
        let core = s.tools.core.clone();
        let asked = &mut s.asked;
        let answer = async {
            while let Some(request) = asked.recv().await {
                meanwhile(&core);
                core.mcp_approvals().answer(&request.id, true);
            }
        };
        tokio::select! {
            outcome = Box::pin(work) => outcome,
            () = answer => unreachable!("the core keeps its events"),
        }
    }

    #[tokio::test]
    async fn two_approved_updates_of_one_query_never_undo_each_other() {
        let mut s = setup();
        let shop = s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let query = s.saved("Orders", "select 1", on(&shop), Engine::Postgres);
        let (a, b) = (s.tools(), s.tools());

        let ((renamed, resqled), asked) = s
            .answering(true, async {
                tokio::join!(
                    a.save_saved(None, rename(&query.id, "Paid")),
                    b.save_saved(None, resql(&query.id, "select 2"))
                )
            })
            .await;
        assert_eq!(asked.len(), 2, "both were asked");
        let stored = s.core().saved_queries().get(&query.id).unwrap();
        let changed = changed_meanwhile("Orders");
        match (renamed, resqled) {
            (Ok(done), Err(refused)) => {
                assert_eq!(refused, changed);
                assert_eq!(
                    (done.name.as_str(), stored.name.as_str(), stored.sql.as_str()),
                    ("Paid", "Paid", "select 1")
                );
            }
            (Err(refused), Ok(done)) => {
                assert_eq!(refused, changed);
                assert_eq!(
                    (done.sql.as_str(), stored.name.as_str(), stored.sql.as_str()),
                    ("select 2", "Orders", "select 2")
                );
            }
            other => panic!("exactly one is applied: {other:?}"),
        }
        assert_eq!(s.heard.changed(), 1, "only the applied one is heard");
    }

    #[tokio::test]
    async fn a_user_edit_made_while_the_approval_waits_is_kept() {
        let mut s = setup();
        let shop = s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let query = s.saved("Orders", "select 1", on(&shop), Engine::Postgres);
        let tools = s.tools();
        let mut edit = query.content();
        edit.folder = Some("Mine".into());
        let user_edit = |core: &Core| {
            core.saved_queries().save(Some(&query.id), edit.clone()).unwrap();
        };

        let refused = approving_after(&mut s, user_edit, tools.save_saved(None, resql(&query.id, "select 2"))).await;
        assert_eq!(refused.unwrap_err(), changed_meanwhile("Orders"));
        let stored = s.core().saved_queries().get(&query.id).unwrap();
        assert_eq!(stored.content(), edit, "the user's edit stays");
        assert_eq!(s.heard.changed(), 0);
    }

    #[tokio::test]
    async fn a_delete_of_a_query_changed_or_deleted_meanwhile_does_nothing() {
        let mut s = setup();
        let shop = s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let query = s.saved("Orders", "select 1", on(&shop), Engine::Postgres);
        let tools = s.tools();
        let mut edit = query.content();
        edit.sql = "select 3".into();
        let user_edit = |core: &Core| {
            core.saved_queries().save(Some(&query.id), edit.clone()).unwrap();
        };
        let delete = || tools.delete_saved(None, DeleteSavedRequest { id: query.id.clone() });

        let refused = approving_after(&mut s, user_edit, delete()).await;
        assert_eq!(refused.unwrap_err(), changed_meanwhile("Orders"));
        assert_eq!(s.core().saved_queries().get(&query.id).unwrap().content(), edit);

        let user_delete = |core: &Core| {
            let _ = core.saved_queries().remove(&query.id);
        };
        let gone = approving_after(&mut s, user_delete, delete()).await;
        assert_eq!(gone.unwrap_err(), deleted_meanwhile("Orders"));
        let updated = approving_after(&mut s, |_: &Core| {}, tools.save_saved(None, rename(&query.id, "x"))).await;
        assert_eq!(updated.unwrap_err(), not_found(&query.id), "no word of connections");
        assert_eq!(s.heard.changed(), 0);
    }

    #[tokio::test]
    async fn an_agent_gone_before_the_answer_changes_nothing() {
        let s = setup();
        let shop = s.connection("shop", Engine::Postgres, AgentAccess::Read, None);
        let query = s.saved("Orders", "select 1", on(&shop), Engine::Postgres);
        let waiting = s.tools.delete_saved(None, DeleteSavedRequest { id: query.id.clone() });
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), waiting).await.is_err());
        assert_eq!(s.core().saved_queries().list(), [query]);
        let events = s.heard.events.lock().unwrap();
        assert!(matches!(events.last(), Some(Event::McpApprovalWithdrawn { .. })), "{events:?}");
    }
}
