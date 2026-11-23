//! Changes an agent asks to make that wait for the user: a statement on a
//! connection whose tag asks before changes, every change to the saved
//! queries, and every backup and restore of a database. The app shows what the change does and waits for the user to
//! approve or decline it. Nothing happens until the user says yes; no answer
//! within the wait is a no.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::oneshot;

use crate::connections::model::Engine;
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

/// Requests waiting for an answer, by id.
#[derive(Default)]
pub struct Approvals {
    waiting: Mutex<HashMap<String, oneshot::Sender<bool>>>,
}

impl Approvals {
    /// Shows `request` in the app ([`Event::McpApprovalAsked`]) and waits for
    /// the user's answer. If the wait ends without one (time ran out, or the
    /// agent went away and this future was dropped), the dialog is withdrawn
    /// ([`Event::McpApprovalWithdrawn`]).
    pub async fn ask(&self, events: &dyn Events, request: ApprovalRequest) -> bool {
        let (answer, answered) = oneshot::channel();
        self.lock().insert(request.id.clone(), answer);
        let _pending = Pending { approvals: self, events, id: &request.id };
        events.emit(Event::McpApprovalAsked(request.clone()));
        tokio::time::timeout(WAIT, answered).await.ok().and_then(Result::ok).unwrap_or(false)
    }

    /// The user's answer to request `id`; false when nothing waits under it any more.
    pub fn answer(&self, id: &str, approved: bool) -> bool {
        self.lock().remove(id).is_some_and(|waiting| waiting.send(approved).is_ok())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, oneshot::Sender<bool>>> {
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
        let (sender, mut receiver) = oneshot::channel();
        approvals.lock().insert("a".into(), sender);

        assert!(!approvals.answer("unknown", true));
        assert!(approvals.answer("a", true));
        assert_eq!(receiver.try_recv(), Ok(true));
        assert!(!approvals.answer("a", false), "answered once only");
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
