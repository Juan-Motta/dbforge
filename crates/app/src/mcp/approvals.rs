//! The changes agents wait to make, kept for the whole app: the core asks
//! ([`Event::McpApprovalAsked`]) when an agent wants to change data on a
//! connection whose tag asks first, and withdraws a request nobody waits on
//! any more ([`Event::McpApprovalWithdrawn`]). The window shows the oldest
//! in [`super::ApprovalDialog`], whose buttons are the only callers of
//! [`Approvals::answer`]. The core counts no answer within its wait as a
//! decline, so a request left alone never runs.

use dbforge_core::Event;
use dbforge_core::mcp::ApprovalRequest;
use dbforge_core::mcp::commands::answer_mcp_approval;
use gpui::{App, AppContext, Context, Entity, Global, Subscription};

use super::queue::ApprovalQueue;
use crate::backend::Backend;
use crate::events::EventHub;

pub struct Approvals {
    queue: ApprovalQueue,
    _events: Subscription,
}

struct GlobalApprovals(Entity<Approvals>);

impl Global for GlobalApprovals {}

impl Approvals {
    /// Starts listening to the core's events. Call it right after
    /// [`EventHub::start`], before the hub forwards anything, so no request
    /// sent while the window opens is missed.
    pub fn start(cx: &mut App) {
        let hub = EventHub::global(cx);
        let approvals = cx.new(|cx| Self {
            queue: ApprovalQueue::default(),
            _events: cx.subscribe(&hub, |this: &mut Self, _, event: &Event, cx| this.hear(event, cx)),
        });
        cx.set_global(GlobalApprovals(approvals));
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalApprovals>().0.clone()
    }

    fn hear(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::McpApprovalAsked(request) => self.queue.push(request.clone()),
            Event::McpApprovalWithdrawn { id } => self.queue.withdraw(id),
            _ => return,
        }
        cx.notify();
    }

    /// The request to show: the oldest still waiting.
    pub fn current(&self) -> Option<&ApprovalRequest> {
        self.queue.current()
    }

    pub fn waiting(&self) -> usize {
        self.queue.len()
    }

    /// The user's answer to request `id`, from the approval dialog's buttons
    /// only. It leaves the queue whatever the core says, as in the web UI;
    /// false when the core had nothing waiting under it any more.
    pub fn answer(&mut self, id: &str, approved: bool, cx: &mut Context<Self>) -> bool {
        self.queue.take(id);
        cx.notify();
        answer_mcp_approval(Backend::global(cx).core().mcp_approvals(), id.to_owned(), approved)
    }

    /// Development only: shows `request` as if an agent had asked; answering
    /// it reaches the core, which has nothing waiting under its id.
    #[cfg(debug_assertions)]
    pub fn show_for_development(&mut self, request: ApprovalRequest, cx: &mut Context<Self>) {
        self.queue.push(request);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dbforge_core::Events;
    use gpui::TestAppContext;

    use super::*;
    use crate::backend::runtime;
    use crate::events::channel;
    use crate::test_support::setup;

    fn request(id: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: id.into(),
            agent: Some("claude-code".into()),
            connection_id: "shop".into(),
            connection_name: "Shop".into(),
            tag: Some("PROD".into()),
            database: None,
            statement: "DELETE FROM orders".into(),
        }
    }

    /// The core's events reach a hub of the test's own, as the app's reach its hub.
    fn hub(cx: &mut TestAppContext) -> Arc<crate::events::ChannelEvents> {
        cx.update(|cx| {
            let (events, received) = channel();
            EventHub::start(received, cx);
            Approvals::start(cx);
            Arc::new(events)
        })
    }

    fn current(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| Approvals::global(cx).read(cx).current().map(|request| request.id.clone()))
    }

    #[gpui::test]
    fn the_core_s_request_waits_for_the_user_and_gets_exactly_their_answer(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let events = hub(cx);
        let core = cx.update(|cx| Backend::global(cx).core().clone());
        let asking = {
            let (core, events) = (core.clone(), events.clone());
            runtime().spawn(async move { core.mcp_approvals().ask(events.as_ref(), request("a")).await })
        };
        while current(cx).is_none() {
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!asking.is_finished(), "nothing runs before the user answers");

        let answered =
            cx.update(|cx| Approvals::global(cx).update(cx, |approvals, cx| approvals.answer("a", false, cx)));
        assert!(answered);
        assert!(!runtime().block_on(asking).unwrap(), "declined");
        assert_eq!(current(cx), None);
    }

    #[gpui::test]
    fn several_requests_wait_in_order_and_a_withdrawn_one_leaves(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let events = hub(cx);
        for id in ["a", "b", "c"] {
            events.emit(Event::McpApprovalAsked(request(id)));
        }
        cx.run_until_parked();
        assert_eq!(current(cx).as_deref(), Some("a"));

        events.emit(Event::McpApprovalWithdrawn { id: "a".into() });
        cx.run_until_parked();
        assert_eq!(current(cx).as_deref(), Some("b"));
        // Answering a request the core no longer waits on still takes it off the queue.
        let answered =
            cx.update(|cx| Approvals::global(cx).update(cx, |approvals, cx| approvals.answer("b", true, cx)));
        assert!(!answered, "the core had nothing waiting under it");
        assert_eq!(current(cx).as_deref(), Some("c"));
        assert_eq!(cx.update(|cx| Approvals::global(cx).read(cx).waiting()), 1);
    }
}
