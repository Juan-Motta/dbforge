//! The changes agents are waiting to make, as the former web UI's `useApprovals`:
//! oldest first, and the first is the one the dialog shows. Answering a
//! request removes it, whatever the core says, since a request that already
//! timed out has nothing left to wait for; a request nobody waits on any
//! more (the agent left, or time ran out) is withdrawn. Nothing here ever
//! answers for the user.

use dbforge_core::mcp::ApprovalRequest;

#[derive(Debug, Default)]
pub struct ApprovalQueue {
    waiting: Vec<ApprovalRequest>,
}

impl ApprovalQueue {
    /// A request the core just asked: it waits behind the others.
    pub fn push(&mut self, request: ApprovalRequest) {
        self.waiting.push(request);
    }

    /// Takes request `id` off the queue, unanswered: nobody waits on it any more.
    pub fn withdraw(&mut self, id: &str) {
        self.waiting.retain(|request| request.id != id);
    }

    /// Takes request `id` off the queue to answer it; `None` when it is no
    /// longer there (withdrawn meanwhile), so there is nothing to answer.
    pub fn take(&mut self, id: &str) -> Option<ApprovalRequest> {
        let index = self.waiting.iter().position(|request| request.id == id)?;
        Some(self.waiting.remove(index))
    }

    /// The request the dialog shows: the oldest still waiting.
    pub fn current(&self) -> Option<&ApprovalRequest> {
        self.waiting.first()
    }

    pub fn len(&self) -> usize {
        self.waiting.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: id.into(),
            agent: Some("claude-code".into()),
            subject: dbforge_core::mcp::ApprovalSubject::Statement {
                connection_id: "shop".into(),
                connection_name: "Shop".into(),
                tag: Some("PROD".into()),
                database: None,
                statement: "DELETE FROM orders".into(),
            },
        }
    }

    fn ids(queue: &ApprovalQueue) -> Vec<&str> {
        queue.waiting.iter().map(|request| request.id.as_str()).collect()
    }

    #[test]
    fn shows_the_oldest_request_first_and_the_next_once_it_is_answered() {
        let mut queue = ApprovalQueue::default();
        assert!(queue.current().is_none());
        queue.push(request("a"));
        queue.push(request("b"));
        assert_eq!(queue.current().map(|request| request.id.as_str()), Some("a"));

        assert_eq!(queue.take("a").map(|request| request.id), Some("a".into()));
        assert_eq!(queue.current().map(|request| request.id.as_str()), Some("b"));
        assert!(queue.take("a").is_none(), "answered once only");
    }

    #[test]
    fn a_withdrawn_request_leaves_the_queue_unanswered_wherever_it_is() {
        let mut queue = ApprovalQueue::default();
        for id in ["a", "b", "c"] {
            queue.push(request(id));
        }
        queue.withdraw("b");
        assert_eq!(ids(&queue), ["a", "c"]);
        queue.withdraw("a");
        assert_eq!(queue.current().map(|request| request.id.as_str()), Some("c"));
        queue.withdraw("unknown");
        assert_eq!(queue.len(), 1);
        // A request withdrawn while its dialog was open has nothing left to answer.
        assert!(queue.take("a").is_none());
    }
}
