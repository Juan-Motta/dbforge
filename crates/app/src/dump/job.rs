//! An export or restore run as a dialog follows it: its job id while it runs
//! (Stop cancels it by that id), when it started, and the last line the
//! tool wrote, from the core's [`Event::DumpProgress`].

use std::time::Instant;

use dbforge_core::Event;
use gpui::{AnyElement, App, Context, IntoElement, Subscription};

use super::Outcome;
use crate::backend::Backend;
use crate::events::EventHub;
use crate::ui::callout::{Callout, Tone, callout_space, elapsed};

#[derive(Default)]
pub struct DumpJob {
    id: Option<String>,
    started: Option<Instant>,
    line: String,
    listening: Option<Subscription>,
}

impl DumpJob {
    pub fn running(&self) -> bool {
        self.id.is_some()
    }

    /// Starts following a new run and returns its id; `job` finds this job in its owner.
    pub fn start<T: 'static>(&mut self, job: fn(&mut T) -> &mut DumpJob, cx: &mut Context<T>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.id = Some(id.clone());
        self.started = Some(Instant::now());
        self.line.clear();
        let followed = id.clone();
        self.listening = Some(cx.subscribe(&EventHub::global(cx), move |this, _, event: &Event, cx| {
            if let Event::DumpProgress { job_id, line } = event
                && *job_id == followed
            {
                job(this).line = line.clone();
                cx.notify();
            }
        }));
        id
    }

    /// The run is over, however it ended.
    pub fn end(&mut self) {
        self.id = None;
        self.listening = None;
    }

    /// Asks the core to stop the run; it then ends as stopped.
    pub fn cancel(&self, cx: &App) {
        if let Some(id) = &self.id {
            Backend::global(cx).core().jobs().cancel(id);
        }
    }

    /// The box under the form: the run in progress (the tool's last line and
    /// the time so far), else `outcome`, else its room kept.
    pub fn status(&self, running_title: String, outcome: Option<&Outcome>) -> AnyElement {
        if let (true, Some(started)) = (self.running(), self.started) {
            let line = if self.line.is_empty() { " ".to_owned() } else { self.line.clone() };
            return Callout::new("dump-status", Tone::Progress, running_title)
                .aside(elapsed(started))
                .detail(line)
                .mono(true)
                .one_line()
                .into_any_element();
        }
        match outcome {
            Some(outcome) => Callout::new("dump-status", outcome.tone, outcome.title.clone())
                .detail_opt(outcome.detail.clone())
                .mono(outcome.mono)
                .into_any_element(),
            None => callout_space().into_any_element(),
        }
    }
}
