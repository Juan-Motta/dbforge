//! Changes an agent asks to make that wait for the user: a statement on a
//! connection whose tag asks before changes, every change to the saved
//! queries, every backup and restore of a database, and every change to the
//! saved connections. The app shows what the change does and waits for the user to
//! approve or decline it. Nothing happens until the user says yes; no answer
//! within the wait is a no.
//!
//! A connection's change may need secrets (its password, its SSH password or
//! key passphrase) that agents never send: the user types them in the dialog,
//! and they come back with the answer ([`ApprovalSecrets`]) straight to the
//! tool that asked, never to the agent, an event or the statement log.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::oneshot;

use crate::connections::model::{AgentAccess, Connection, Engine};
use crate::connections::transport::SshAuth;
use crate::dump::DumpOptions;
use crate::events::{Event, Events};

/// How long a request waits for the user before it counts as declined.
const WAIT: Duration = Duration::from_secs(300);

/// What the dialog shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    /// The agent's own name for itself (e.g. `claude-code`), when it gave one.
    pub agent: Option<String>,
    /// What the agent wants to do.
    pub subject: ApprovalSubject,
}

impl ApprovalRequest {
    /// The connection a statement, backup or restore would run on; none for a saved query's change.
    pub fn connection_id(&self) -> Option<&str> {
        match &self.subject {
            ApprovalSubject::Statement { connection_id, .. } => Some(connection_id),
            ApprovalSubject::Dump(dump) => Some(&dump.connection_id),
            ApprovalSubject::Connection(change) => Some(change.connection_id()),
            ApprovalSubject::SavedQuery(_) => None,
        }
    }
}

/// What an agent asks the user to approve.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ApprovalSubject {
    /// A statement that changes data, on a connection whose tag asks first.
    #[serde(rename_all = "camelCase")]
    Statement {
        connection_id: String,
        connection_name: String,
        tag: Option<String>,
        database: Option<String>,
        statement: String,
    },
    /// Creating, changing or deleting a saved query. Boxed: an update holds
    /// the query twice, and every event would be that large.
    SavedQuery(Box<SavedQueryChange>),
    /// Backing up a database to the exports folder, or restoring an export into one.
    Dump(Box<DumpApproval>),
    /// A connection to create, update or delete.
    Connection(Box<ConnectionChange>),
}

/// A change an agent wants to make to the saved connections. It never holds
/// a secret: the connections are as saved, without their passwords.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "camelCase")]
// Kept boxed in `ApprovalSubject`, so the larger update costs nothing elsewhere.
#[allow(clippy::large_enum_variant)]
pub enum ConnectionChange {
    /// A new connection; what agents may do with it is the user's choice in the dialog.
    Create {
        connection: Connection,
    },
    /// Connection `before` becoming `after`; its tag (by name) stays as it is.
    Update {
        before: Connection,
        after: Connection,
        tag: Option<String>,
    },
    Delete {
        connection: Connection,
        tag: Option<String>,
    },
}

/// The secrets the dialog asks the user for, for a connection being saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SecretFields {
    /// Its password (an account's key).
    pub password: bool,
    /// Its SSH password (`SshAuth::Password`) or key passphrase (`SshAuth::Key`).
    pub ssh: Option<SshAuth>,
    /// An update can't keep the saved password: the connection would point
    /// elsewhere ([`ConnectionField::DESTINATION`]), and the password isn't carried there unseen.
    pub password_dropped: bool,
    /// An update can't keep the saved SSH password or passphrase: the
    /// connection would point elsewhere, or its tunnel stops signing in the way it was for.
    pub ssh_dropped: bool,
}

impl SecretFields {
    /// What `connection` signs in with that DBForge keeps secret: a password
    /// for every engine but a SQLite file, and the tunnel's password or key
    /// passphrase when it goes through SSH without the system's agent.
    pub fn of(connection: &Connection) -> Self {
        let tunneled = connection.engine.tunnels() && connection.ssh.enabled;
        Self {
            password: connection.engine != Engine::Sqlite,
            ssh: (tunneled && connection.ssh.auth != SshAuth::Agent).then_some(connection.ssh.auth),
            password_dropped: false,
            ssh_dropped: false,
        }
    }

    /// The password must be typed: the saved one isn't kept, and the connection signs in with one.
    pub fn password_required(&self) -> bool {
        self.password && self.password_dropped
    }

    /// The SSH password must be typed: the saved one isn't kept. A key's
    /// passphrase never must, as a key may have none: left empty, there is none.
    pub fn ssh_required(&self) -> bool {
        self.ssh_dropped && self.ssh == Some(SshAuth::Password)
    }
}

impl ConnectionChange {
    /// The connection changed: its id.
    pub fn connection_id(&self) -> &str {
        &self.connection().id
    }

    /// The connection as the dialog names it: as it is now, or as it will be when new.
    pub fn connection(&self) -> &Connection {
        match self {
            Self::Create { connection } | Self::Delete { connection, .. } => connection,
            Self::Update { before, .. } => before,
        }
    }

    /// Its tag's name, if any; a new connection has none.
    pub fn tag(&self) -> Option<&str> {
        match self {
            Self::Create { .. } => None,
            Self::Update { tag, .. } | Self::Delete { tag, .. } => tag.as_deref(),
        }
    }

    /// The fields an update changes, in the order the dialog lists them.
    pub fn changed_fields(&self) -> Vec<ConnectionField> {
        let Self::Update { before, after, .. } = self else {
            return Vec::new();
        };
        ConnectionField::ALL.into_iter().filter(|field| field.differs(before, after)).collect()
    }

    /// The secrets the user may type: none for a delete. On an update they
    /// are the new connection's, and one left empty keeps the saved one,
    /// unless the update drops it (see [`SecretFields::password_dropped`]).
    pub fn secrets(&self) -> SecretFields {
        match self {
            Self::Create { connection } => SecretFields::of(connection),
            Self::Update { before, after, .. } => {
                let elsewhere = ConnectionField::DESTINATION.iter().any(|field| field.differs(before, after));
                let switched = before.ssh.auth != after.ssh.auth && before.ssh.auth != SshAuth::Agent;
                SecretFields {
                    password_dropped: elsewhere,
                    ssh_dropped: elsewhere || switched,
                    ..SecretFields::of(after)
                }
            }
            Self::Delete { .. } => SecretFields::default(),
        }
    }
}

/// A field of a connection the dialog lists, besides its secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionField {
    Name,
    Engine,
    Host,
    Port,
    Database,
    Username,
    Srv,
    SslMode,
    CaFile,
    CertFile,
    KeyFile,
    Ssh,
    SshHost,
    SshPort,
    SshUser,
    SshAuth,
    SshKeyFile,
}

impl ConnectionField {
    pub const ALL: [ConnectionField; 17] = [
        Self::Name,
        Self::Engine,
        Self::Host,
        Self::Port,
        Self::Database,
        Self::Username,
        Self::Srv,
        Self::SslMode,
        Self::CaFile,
        Self::CertFile,
        Self::KeyFile,
        Self::Ssh,
        Self::SshHost,
        Self::SshPort,
        Self::SshUser,
        Self::SshAuth,
        Self::SshKeyFile,
    ];

    /// The fields that say where a connection points: a change to any of them
    /// takes it to another server (or through another tunnel).
    pub const DESTINATION: [ConnectionField; 8] =
        [Self::Engine, Self::Host, Self::Port, Self::Srv, Self::Ssh, Self::SshHost, Self::SshPort, Self::SshUser];

    /// Whether `before` and `after` differ in this field.
    fn differs(self, before: &Connection, after: &Connection) -> bool {
        match self {
            Self::Name => before.name != after.name,
            Self::Engine => before.engine != after.engine,
            Self::Host => before.host != after.host,
            Self::Port => before.port != after.port,
            Self::Database => before.database != after.database,
            Self::Username => before.username != after.username,
            Self::Srv => before.srv != after.srv,
            Self::SslMode => before.ssl.mode != after.ssl.mode,
            Self::CaFile => before.ssl.ca_file != after.ssl.ca_file,
            Self::CertFile => before.ssl.cert_file != after.ssl.cert_file,
            Self::KeyFile => before.ssl.key_file != after.ssl.key_file,
            Self::Ssh => before.ssh.enabled != after.ssh.enabled,
            Self::SshHost => before.ssh.host != after.ssh.host,
            Self::SshPort => before.ssh.port != after.ssh.port,
            Self::SshUser => before.ssh.username != after.ssh.username,
            Self::SshAuth => before.ssh.auth != after.ssh.auth,
            Self::SshKeyFile => before.ssh.key_file != after.ssh.key_file,
        }
    }
}

/// What the user typed in a connection's approval: its secrets and, for a
/// new connection, what agents may do with it. It reaches only the tool that
/// asked: it is neither serialized nor printed, and its text is wiped when dropped.
#[derive(Default)]
pub struct ApprovalSecrets {
    /// The password; `None` or empty keeps the saved one (none, for a new connection).
    pub password: Option<String>,
    /// The SSH password or key passphrase, kept the same way.
    pub ssh_secret: Option<String>,
    /// A new connection's agent access; Read only when not given.
    pub agent_access: Option<AgentAccess>,
}

impl ApprovalSecrets {
    /// The typed password, taken out; `None` when nothing was typed.
    pub fn take_password(&mut self) -> Option<String> {
        self.password.take().filter(|password| !password.is_empty())
    }

    /// The typed SSH password or passphrase, taken out; `None` when nothing was typed.
    pub fn take_ssh_secret(&mut self) -> Option<String> {
        self.ssh_secret.take().filter(|secret| !secret.is_empty())
    }
}

impl std::fmt::Debug for ApprovalSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let typed = |secret: &Option<String>| if secret.is_some() { "<typed>" } else { "<none>" };
        f.debug_struct("ApprovalSecrets")
            .field("password", &typed(&self.password))
            .field("ssh_secret", &typed(&self.ssh_secret))
            .field("agent_access", &self.agent_access)
            .finish()
    }
}

impl Drop for ApprovalSecrets {
    fn drop(&mut self) {
        wipe(&mut self.password);
        wipe(&mut self.ssh_secret);
    }
}

/// Overwrites a secret's text before its memory is freed.
pub fn wipe(secret: &mut Option<String>) {
    if let Some(secret) = secret.take() {
        let mut bytes = secret.into_bytes();
        bytes.fill(0);
        std::hint::black_box(&bytes);
    }
}

/// The user's answer to a request.
#[derive(Debug, Default)]
pub struct Answer {
    pub approved: bool,
    /// What the user typed, for an approved connection change only.
    pub secrets: Option<ApprovalSecrets>,
}

/// A backup or restore an agent wants to run.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpApproval {
    pub connection_id: String,
    pub connection_name: String,
    /// The connection's tag (e.g. PROD), if any.
    pub tag: Option<String>,
    pub engine: Engine,
    /// The database backed up or restored into; for SQLite, its file's name.
    pub database: String,
    /// The export's file name in the exports folder: written by a backup, read by a restore.
    pub file: String,
    pub action: DumpAction,
}

/// Which way a [`DumpApproval`] goes, with its options.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DumpAction {
    /// Writes the database to `file`.
    Export { options: DumpOptions },
    /// Restores `file` into the database, changing its data.
    #[serde(rename_all = "camelCase")]
    Import {
        /// MongoDB: drops each collection the archive holds before restoring it.
        drop_existing: bool,
        /// Whether the database is already there; `None` when that couldn't be told.
        target_exists: Option<bool>,
    },
}

/// A change an agent wants to make to the saved queries.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum SavedQueryChange {
    Create {
        query: SavedQueryDraft,
    },
    /// Saved query `before` becoming `after`.
    Update {
        before: SavedQueryDraft,
        after: SavedQueryDraft,
    },
    Delete {
        query: SavedQueryDraft,
    },
}

impl SavedQueryChange {
    /// The name the dialog calls the query by: its current one.
    pub fn name(&self) -> &str {
        match self {
            Self::Create { query } | Self::Delete { query } => &query.name,
            Self::Update { before, .. } => &before.name,
        }
    }

    /// The engine its SQL is written for, the new one on an update.
    pub fn engine(&self) -> Engine {
        match self {
            Self::Create { query } | Self::Delete { query } => query.engine,
            Self::Update { after, .. } => after.engine,
        }
    }

    /// The fields an update changes, besides the SQL, in the order the dialog lists them.
    pub fn changed_fields(&self) -> Vec<SavedQueryField> {
        let Self::Update { before, after } = self else {
            return Vec::new();
        };
        SavedQueryField::ALL.into_iter().filter(|field| field.differs(before, after)).collect()
    }

    /// Whether an update changes the SQL.
    pub fn changes_sql(&self) -> bool {
        matches!(self, Self::Update { before, after } if before.sql != after.sql)
    }
}

/// A saved query as the approval dialog shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueryDraft {
    pub name: String,
    pub sql: String,
    pub description: Option<String>,
    pub folder: Option<String>,
    pub scope: DraftScope,
    pub engine: Engine,
}

/// Which connections a saved query is offered for, named for the dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DraftScope {
    /// One connection, by its name (its id when it no longer exists).
    #[serde(rename_all = "camelCase")]
    Connection { connection_id: String, connection_name: String },
    /// Any connection of the query's engine.
    Engine,
}

/// A field of a saved query an update can change, besides its SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SavedQueryField {
    Name,
    Folder,
    Scope,
    Description,
}

impl SavedQueryField {
    pub const ALL: [SavedQueryField; 4] = [Self::Name, Self::Folder, Self::Scope, Self::Description];

    /// Whether `before` and `after` differ in this field. The scope covers
    /// the engine too: "any PostgreSQL connection" isn't "any MySQL connection".
    fn differs(self, before: &SavedQueryDraft, after: &SavedQueryDraft) -> bool {
        match self {
            Self::Name => before.name != after.name,
            Self::Folder => before.folder != after.folder,
            Self::Description => before.description != after.description,
            Self::Scope => (&before.scope, before.engine) != (&after.scope, after.engine),
        }
    }
}

/// A request waiting for its answer.
struct Waiting {
    answer: oneshot::Sender<Answer>,
    /// A connection's change: the answer may bring secrets.
    takes_secrets: bool,
}

/// Requests waiting for an answer, by id.
#[derive(Default)]
pub struct Approvals {
    waiting: Mutex<HashMap<String, Waiting>>,
}

impl Approvals {
    /// Shows `request` in the app ([`Event::McpApprovalAsked`]) and waits for
    /// the user's answer. If the wait ends without one (time ran out, or the
    /// agent went away and this future was dropped), the dialog is withdrawn
    /// ([`Event::McpApprovalWithdrawn`]).
    pub async fn ask(&self, events: &dyn Events, request: ApprovalRequest) -> bool {
        self.ask_for_answer(events, request).await.approved
    }

    /// As [`Approvals::ask`], with what the user typed along with a
    /// connection's approval. A decline brings nothing.
    pub async fn ask_for_answer(&self, events: &dyn Events, request: ApprovalRequest) -> Answer {
        let (answer, answered) = oneshot::channel();
        let takes_secrets = matches!(request.subject, ApprovalSubject::Connection(_));
        self.lock().insert(request.id.clone(), Waiting { answer, takes_secrets });
        let _pending = Pending { approvals: self, events, id: &request.id };
        events.emit(Event::McpApprovalAsked(request.clone()));
        tokio::time::timeout(WAIT, answered).await.ok().and_then(Result::ok).unwrap_or_default()
    }

    /// The user's answer to request `id`; false when nothing waits under it any more.
    pub fn answer(&self, id: &str, approved: bool) -> bool {
        self.answer_with(id, approved, None)
    }

    /// The user's answer to request `id`, with what they typed for a
    /// connection's change. Secrets reach only an approved connection's
    /// request; any other drops them unread.
    pub fn answer_with(&self, id: &str, approved: bool, secrets: Option<ApprovalSecrets>) -> bool {
        let Some(waiting) = self.lock().remove(id) else { return false };
        let secrets = secrets.filter(|_| approved && waiting.takes_secrets);
        waiting.answer.send(Answer { approved, secrets }).is_ok()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Waiting>> {
        self.waiting.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A request still waiting when the asking ends: removed, and its dialog closed.
struct Pending<'a> {
    approvals: &'a Approvals,
    events: &'a dyn Events,
    id: &'a str,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        // Already gone when the user answered.
        if self.approvals.lock().remove(self.id).is_some() {
            self.events.emit(Event::McpApprovalWithdrawn { id: self.id.to_owned() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_reaches_only_a_request_still_waiting() {
        let approvals = Approvals::default();
        let (answer, mut receiver) = oneshot::channel();
        approvals.lock().insert("a".into(), Waiting { answer, takes_secrets: false });

        assert!(!approvals.answer("unknown", true));
        assert!(approvals.answer("a", true));
        assert!(receiver.try_recv().unwrap().approved);
        assert!(!approvals.answer("a", false), "answered once only");
    }

    fn typed() -> ApprovalSecrets {
        ApprovalSecrets { password: Some("s3cret".into()), ssh_secret: None, agent_access: Some(AgentAccess::Write) }
    }

    #[test]
    fn secrets_reach_only_an_approved_connection_change() {
        let approvals = Approvals::default();
        let mut receivers = Vec::new();
        for (id, takes_secrets) in [("statement", false), ("declined", true), ("approved", true)] {
            let (answer, receiver) = oneshot::channel();
            approvals.lock().insert(id.into(), Waiting { answer, takes_secrets });
            receivers.push(receiver);
        }
        assert!(approvals.answer_with("statement", true, Some(typed())));
        assert!(approvals.answer_with("declined", false, Some(typed())));
        assert!(approvals.answer_with("approved", true, Some(typed())));
        let answers: Vec<Answer> = receivers.iter_mut().map(|receiver| receiver.try_recv().unwrap()).collect();
        assert!(answers[0].approved && answers[0].secrets.is_none(), "a statement takes no secrets");
        assert!(!answers[1].approved && answers[1].secrets.is_none(), "a decline brings nothing");
        let mut secrets = answers.into_iter().nth(2).unwrap().secrets.expect("the typed secrets");
        assert_eq!(secrets.take_password().as_deref(), Some("s3cret"));
        assert_eq!(secrets.agent_access, Some(AgentAccess::Write));
    }

    #[test]
    fn typed_secrets_never_print() {
        let shown = format!("{:?}", typed());
        assert!(!shown.contains("s3cret"), "{shown}");
        assert!(shown.contains("<typed>"));
        let mut empty = ApprovalSecrets { password: Some(String::new()), ssh_secret: None, agent_access: None };
        assert_eq!(empty.take_password(), None, "an empty password keeps the saved one");
    }

    /// The events the UI would hear, in order.
    #[derive(Default)]
    struct Heard(Mutex<Vec<Event>>);

    impl Events for Heard {
        fn emit(&self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }

    fn request(id: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: id.into(),
            agent: None,
            subject: ApprovalSubject::Statement {
                connection_id: "c".into(),
                connection_name: "Shop".into(),
                tag: None,
                database: None,
                statement: "DELETE FROM orders".into(),
            },
        }
    }

    fn draft(name: &str) -> SavedQueryDraft {
        SavedQueryDraft {
            name: name.into(),
            sql: "select 1".into(),
            description: None,
            folder: None,
            scope: DraftScope::Engine,
            engine: Engine::Postgres,
        }
    }

    #[test]
    fn an_update_lists_only_the_fields_it_changes() {
        let before = draft("Orders");
        let same = SavedQueryChange::Update { before: before.clone(), after: before.clone() };
        assert!(same.changed_fields().is_empty());
        assert!(!same.changes_sql());

        let after = SavedQueryDraft {
            name: "Paid orders".into(),
            sql: "select 2".into(),
            folder: Some("Ops".into()),
            scope: DraftScope::Connection { connection_id: "c".into(), connection_name: "Shop".into() },
            ..before.clone()
        };
        let change = SavedQueryChange::Update { before, after };
        assert_eq!(change.changed_fields(), [SavedQueryField::Name, SavedQueryField::Folder, SavedQueryField::Scope]);
        assert!(change.changes_sql());
        assert_eq!(change.name(), "Orders", "called by the name the user knows");
    }

    #[test]
    fn moving_to_another_engine_changes_the_scope() {
        let before = draft("Orders");
        let after = SavedQueryDraft { engine: Engine::Mysql, ..before.clone() };
        let change = SavedQueryChange::Update { before, after };
        assert_eq!(change.changed_fields(), [SavedQueryField::Scope]);
        assert_eq!(change.engine(), Engine::Mysql);
    }

    #[test]
    fn only_a_statement_names_a_connection() {
        assert_eq!(request("a").connection_id(), Some("c"));
        let saved = ApprovalRequest {
            id: "b".into(),
            agent: None,
            subject: ApprovalSubject::SavedQuery(Box::new(SavedQueryChange::Delete { query: draft("Orders") })),
        };
        assert_eq!(saved.connection_id(), None);
        assert!(SavedQueryChange::Create { query: draft("x") }.changed_fields().is_empty());
    }

    #[tokio::test]
    async fn the_ui_hears_the_request_and_its_answer_reaches_the_agent() {
        let (approvals, heard) = (Approvals::default(), Heard::default());
        let asking = approvals.ask(&heard, request("a"));
        let answering = async {
            while approvals.lock().is_empty() {
                tokio::task::yield_now().await;
            }
            assert!(approvals.answer("a", true));
        };
        let (approved, ()) = tokio::join!(asking, answering);

        assert!(approved);
        let heard = heard.0.into_inner().unwrap();
        assert!(matches!(heard.as_slice(), [Event::McpApprovalAsked(asked)] if asked.id == "a"), "{heard:?}");
    }

    #[tokio::test]
    async fn a_request_nobody_waits_on_any_more_is_withdrawn() {
        let (approvals, heard) = (Approvals::default(), Heard::default());
        let asking = Box::pin(approvals.ask(&heard, request("a")));
        // The agent goes away before the user answers.
        assert!(tokio::time::timeout(Duration::from_millis(10), asking).await.is_err());

        let heard = heard.0.into_inner().unwrap();
        assert!(
            matches!(heard.as_slice(), [Event::McpApprovalAsked(_), Event::McpApprovalWithdrawn { id }] if id == "a"),
            "{heard:?}"
        );
        assert!(!approvals.answer("a", true), "nothing waits under it any more");
    }
}
