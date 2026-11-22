//! What the core tells the UI while it works: progress of long jobs, each
//! statement as it is logged, the MCP approvals waiting on the user, and the
//! agents' changes to the saved queries. The UI implements [`Events`] and gets
//! each one as it happens, from any thread.

use crate::credentials::SwitchProgress;
use crate::mcp::ApprovalRequest;
use crate::statement_log::Entry;

/// Something the UI should hear about.
#[derive(Debug, Clone)]
pub enum Event {
    /// Rows an export (`export_table`) has written so far.
    ExportProgress { job_id: String, rows: u64 },
    /// Rows an import (`import_table`) has added so far.
    ImportProgress { job_id: String, rows: u64 },
    /// The last line `pg_dump` or `psql` wrote, for a database export or import.
    DumpProgress { job_id: String, line: String },
    /// The password being moved now, while the credential storage changes.
    CredentialProgress(SwitchProgress),
    /// A statement just recorded in the statement log.
    StatementLogged(Entry),
    /// An agent's change waits for the user to approve or decline it
    /// ([`crate::mcp::commands::answer_mcp_approval`]).
    McpApprovalAsked(ApprovalRequest),
    /// Nobody waits on the approval `id` any more (the agent left, or time ran out): its dialog closes.
    McpApprovalWithdrawn { id: String },
    /// An agent used the MCP server for the first time in this run, or again a
    /// while after it was last announced: the agents list
    /// ([`crate::mcp::commands::mcp_agents`]) has changed.
    McpAgentsChanged,
    /// An agent created, changed, deleted or ran a saved query: the list
    /// ([`crate::saved_queries::commands::list_saved_queries`]) has changed.
    SavedQueriesChanged,
}

/// Where the core sends its events: the UI.
pub trait Events: Send + Sync + 'static {
    fn emit(&self, event: Event);
}

/// Events nobody listens to: for tests and for running the core without a UI.
pub struct NoEvents;

impl Events for NoEvents {
    fn emit(&self, _event: Event) {}
}
