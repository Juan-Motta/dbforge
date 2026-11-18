//! Changes an agent asks to make on a connection whose tag asks before
//! changes: the app shows the statement and waits for the user to approve or
//! decline it. Nothing runs until the user says yes; no answer within the
//! wait is a no.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::oneshot;

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
            connection_id: "c".into(),
            connection_name: "Shop".into(),
            tag: None,
            database: None,
            statement: "DELETE FROM orders".into(),
        }
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
