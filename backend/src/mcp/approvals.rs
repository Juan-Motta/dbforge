//! Changes an agent asks to make on a connection whose tag asks before
//! changes: the app shows the statement and waits for the user to approve or
//! decline it. Nothing runs until the user says yes; no answer within the
//! wait is a no.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

/// The event the window shows an approval dialog for.
const ASKED: &str = "mcp-approval";
/// How long a request waits for the user before it counts as declined.
const WAIT: Duration = Duration::from_secs(300);

/// What the dialog shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    /// The agent's own name for itself (e.g. `claude-code`), when it gave one.
    pub agent: Option<String>,
    pub connection_id: String,
    pub connection_name: String,
    pub tag: Option<String>,
    pub database: Option<String>,
    pub statement: String,
}

/// Requests waiting for an answer, by id.
#[derive(Default)]
pub struct Approvals {
    waiting: Mutex<HashMap<String, oneshot::Sender<bool>>>,
}

impl Approvals {
    /// Shows `request` in the app and waits for the user's answer.
    pub async fn ask(&self, app: &AppHandle, request: ApprovalRequest) -> bool {
        let (answer, answered) = oneshot::channel();
        let id = request.id.clone();
        self.lock().insert(id.clone(), answer);
        if app.emit(ASKED, &request).is_err() {
            self.lock().remove(&id);
            return false;
        }
        let approved = tokio::time::timeout(WAIT, answered).await.ok().and_then(Result::ok).unwrap_or(false);
        // Gone already when answered; still there after the wait ran out.
        self.lock().remove(&id);
        approved
    }

    /// The user's answer to request `id`; false when nothing waits under it any more.
    pub fn answer(&self, id: &str, approved: bool) -> bool {
        self.lock().remove(id).is_some_and(|waiting| waiting.send(approved).is_ok())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, oneshot::Sender<bool>>> {
        self.waiting.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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
}
