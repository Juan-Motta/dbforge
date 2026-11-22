//! Asks before closing a console's tab, or quitting, with a transaction
//! open: what is pending (the connection, how long it has been open, its
//! changes and rows) and whether to roll it back (the default, on Enter),
//! commit it, or stay. The tab closes, or the app quits, only once every
//! transaction ended; one that fails keeps the dialog open with the error.

use std::time::Duration;

use dbforge_core::query::{
    ConsoleTransaction, TransactionEnd, commit_transaction, rollback_transaction, transaction_state,
};
use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled, Task, Window, actions, div,
    prelude::FluentBuilder, px,
};

use super::rules::clock;
use crate::backend::Backend;
use crate::i18n::Translate;
use crate::theme::ActiveTheme;
use crate::ui::modal::{modal, modal_body, modal_footer};
use crate::ui::notice::{Tone, notify};
use crate::ui::{Ellipsis, button, enabled, ghost_button, primary_button};

actions!(leave_transaction_dialog, [RollBackAll, Dismiss]);

const CONTEXT: &str = "LeaveTransactionDialog";

/// How often the time each has been open is redrawn.
const TICK: Duration = Duration::from_secs(1);

/// Enter rolls back, as the focused default button would; Escape stays.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("enter", RollBackAll, Some(CONTEXT)), KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

/// What is being left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leaving {
    /// A console's tab, by its title.
    Tab(String),
    /// The app, its window closing or quitting.
    Quit,
}

/// A transaction left open, and the title of its console's tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub title: String,
    pub transaction: ConsoleTransaction,
}

/// What the dialog tells its owner, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveEvent {
    /// Every transaction ended this way: close the tab, or quit, now.
    Ended(TransactionEnd),
}

pub struct LeaveDialog {
    leaving: Leaving,
    pending: Vec<Pending>,
    /// Commit or Rollback, going now.
    ending: Option<TransactionEnd>,
    error: Option<String>,
    focus: FocusHandle,
    _ticking: Task<()>,
}

impl EventEmitter<DismissEvent> for LeaveDialog {}
impl EventEmitter<LeaveEvent> for LeaveDialog {}

impl Focusable for LeaveDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl LeaveDialog {
    pub fn new(leaving: Leaving, pending: Vec<Pending>, cx: &mut Context<Self>) -> Self {
        let ticking = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self { leaving, pending, ending: None, error: None, focus: cx.focus_handle(), _ticking: ticking }
    }

    /// Takes the keyboard, so Enter and Escape reach the dialog.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    pub fn pending(&self) -> &[Pending] {
        &self.pending
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Commit is off while a statement failed in one of them: it can only be rolled back.
    pub fn can_commit(&self) -> bool {
        !self.pending.iter().any(|pending| pending.transaction.transaction.aborted)
    }

    /// Commits or rolls back every pending transaction, one after another,
    /// stopping at the first that fails. Those that ended leave the list.
    pub fn end(&mut self, end: TransactionEnd, cx: &mut Context<Self>) {
        if self.ending.is_some() || (end == TransactionEnd::Committed && !self.can_commit()) {
            return;
        }
        self.ending = Some(end);
        self.error = None;
        let sessions: Vec<String> = self.pending.iter().map(|p| p.transaction.session_id.clone()).collect();
        let ending = Backend::global(cx).spawn(cx, move |core| async move {
            let (open, log) = (core.query_sessions(), core.statement_log());
            for session in sessions {
                // One that ended meanwhile (a lost connection) has nothing left to end.
                if transaction_state(open, &session).is_none() {
                    continue;
                }
                let ended = match end {
                    TransactionEnd::Committed => commit_transaction(open, log, session).await,
                    TransactionEnd::RolledBack => rollback_transaction(open, log, session).await,
                };
                ended.map_err(|error| error.to_string())?;
            }
            Ok::<(), String>(())
        });
        cx.spawn(async move |this, cx| {
            let ended = ending.await;
            let _ = this.update(cx, |this, cx| this.finish(end, ended, cx));
        })
        .detach();
        cx.notify();
    }

    fn finish(&mut self, end: TransactionEnd, ended: Result<(), String>, cx: &mut Context<Self>) {
        self.ending = None;
        let ended_here: Vec<Pending> = {
            let sessions = Backend::global(cx).core().query_sessions();
            let (still, gone): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending)
                .into_iter()
                .partition(|p| transaction_state(sessions, &p.transaction.session_id).is_some());
            self.pending = still;
            gone
        };
        match ended {
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
            Ok(()) => {
                // A tab's notice says what happened; a quitting app has nobody to tell.
                if let (Leaving::Tab(_), [pending]) = (&self.leaving, ended_here.as_slice()) {
                    let m = &cx.messages().transactions;
                    let (changes, rows) =
                        (pending.transaction.transaction.changes, pending.transaction.transaction.rows);
                    let text = match end {
                        TransactionEnd::Committed => (m.committed)(changes, rows),
                        TransactionEnd::RolledBack => (m.rolled_back)(changes, rows),
                    };
                    notify(text, Tone::Info, cx);
                }
                cx.emit(LeaveEvent::Ended(end));
            }
        }
    }

    fn row(&self, pending: &Pending, first: bool, now: i64, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = *cx.theme();
        let m = &cx.messages().transactions;
        let transaction = &pending.transaction.transaction;
        let detail = format!(
            "{} · {} · {}",
            (m.changes)(transaction.changes),
            (m.rows)(transaction.rows),
            (m.dialog.open_for)(&clock(now - transaction.started_at_ms)),
        );
        let title = match self.leaving {
            Leaving::Quit if !pending.title.is_empty() => {
                format!("{} · {}", pending.transaction.connection_name, pending.title)
            }
            _ => pending.transaction.connection_name.clone(),
        };
        div()
            .h(px(36.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .when(!first, |row| row.border_t_1().border_color(theme.line))
            .child(div().size(px(8.)).flex_none().rounded(px(4.)).bg(theme.txn))
            .child(div().flex_1().min_w_0().ellipsis().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(title))
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(if transaction.aborted { theme.err } else { theme.text2 })
                    .child(if transaction.aborted { m.failed.to_owned() } else { detail }),
            )
    }
}

impl Render for LeaveDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().transactions;
        let d = &m.dialog;
        let count = self.pending.len();
        let (title, rollback, commit) = match &self.leaving {
            Leaving::Tab(tab) => ((d.close_title)(tab), d.rollback_and_close.to_owned(), d.commit_and_close.to_owned()),
            Leaving::Quit => ((d.quit_title)(count), (d.rollback_and_quit)(count), (d.commit_and_quit)(count)),
        };
        let now = super::now_ms();
        let rows: Vec<_> =
            self.pending.iter().enumerate().map(|(index, pending)| self.row(pending, index == 0, now, cx)).collect();
        let ending = self.ending;
        let idle = ending.is_none();
        let can_commit = idle && self.can_commit();
        let rollback_label = if ending == Some(TransactionEnd::RolledBack) { d.rolling_back } else { &rollback };
        let commit_label = if ending == Some(TransactionEnd::Committed) { d.committing } else { &commit };
        modal(&theme, 540., title)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &RollBackAll, _, cx| this.end(TransactionEnd::RolledBack, cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                if this.ending.is_none() {
                    cx.emit(DismissEvent);
                }
            }))
            .child(
                modal_body("leave-transaction-body")
                    .gap(px(10.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text3)
                            .child(d.not_committed.to_uppercase()),
                    )
                    .child(
                        div()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(theme.txn.opacity(0.48))
                            .bg(theme.txn.opacity(0.1))
                            .flex()
                            .flex_col()
                            .children(rows),
                    )
                    .child(div().text_size(px(12.5)).line_height(px(18.)).text_color(theme.text2).child(d.explanation))
                    .when(!self.can_commit(), |body| {
                        body.child(div().text_size(px(12.5)).text_color(theme.txn).child(d.only_rollback))
                    })
                    // Room for an error is kept, so one appearing never moves the buttons.
                    .child(div().min_h(px(18.)).text_size(px(12.5)).text_color(theme.err).children(self.error.clone())),
            )
            .child(
                modal_footer(&theme)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .ellipsis()
                            .text_size(px(12.))
                            .text_color(theme.text3)
                            .child((d.keys)(&rollback)),
                    )
                    .child(
                        enabled(ghost_button(&theme, d.cancel), idle)
                            .id("leave-cancel")
                            .when(idle, |cancel| cancel.on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))),
                    )
                    // The default, on Enter: framed in the accent.
                    .child(
                        enabled(button(&theme, None, rollback_label).border_color(theme.accent_hi), idle)
                            .id("leave-rollback")
                            .when(idle, |button| {
                                button.on_click(cx.listener(|this, _, _, cx| this.end(TransactionEnd::RolledBack, cx)))
                            }),
                    )
                    .child(
                        enabled(primary_button(&theme, commit_label), can_commit).id("leave-commit").when(
                            can_commit,
                            |button| {
                                button.on_click(cx.listener(|this, _, _, cx| this.end(TransactionEnd::Committed, cx)))
                            },
                        ),
                    ),
            )
    }
}
