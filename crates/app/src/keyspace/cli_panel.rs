//! redis-cli in a panel, as the former web UI's `CliPanel.tsx`: one command per
//! Enter, run in the tab's own session (it keeps the connection between
//! commands and closes with the tab), replies printed as redis-cli prints
//! them, ↑/↓ through the commands run, and a stop for a blocking command.

use crate::ui::Ellipsis;
use dbforge_core::query::{QueryScope, cancel_query, run_query};
use gpui::{
    AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, KeyBinding, ParentElement, Render,
    ScrollHandle, StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::cli::{CliHistory, ReplyTone, reply_tone};
use crate::backend::Backend;
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::field::Tooltip;
use crate::ui::{TextInput, TextInputEvent, icon_button};

actions!(key_cli, [Older, Newer]);

const CONTEXT: &str = "KeyCli";
/// The panel's height, as the former web UI's.
pub const CLI_HEIGHT: f32 = 200.;

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("up", Older, Some(CONTEXT)), KeyBinding::new("down", Newer, Some(CONTEXT))]
}

/// A command ran: what the browser shows may have changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    Command(String),
    Reply(String),
}

pub struct CliPanel {
    session_id: String,
    connection_id: String,
    db: i64,
    /// `host:port · db0`.
    address: String,
    lines: Vec<Line>,
    history: CliHistory,
    running: bool,
    input: Entity<TextInput>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CliEvent> for CliPanel {}

impl CliPanel {
    pub fn new(
        session_id: String,
        connection_id: String,
        db: i64,
        address: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::new(cx.messages().keyspace.cli_placeholder, cx));
        let subscriptions = vec![cx.subscribe_in(&input, window, |this, input, event: &TextInputEvent, _, cx| {
            if matches!(event, TextInputEvent::Submitted) && !this.running {
                let command = input.read(cx).text().to_owned();
                this.run(command, cx);
            }
        })];
        Self {
            session_id,
            connection_id,
            db,
            address,
            lines: Vec::new(),
            history: CliHistory::default(),
            running: false,
            input,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The lines printed, commands with their `› `.
    pub fn transcript(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|line| match line {
                Line::Command(text) => format!("› {text}"),
                Line::Reply(text) => text.clone(),
            })
            .collect()
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn run(&mut self, command: String, cx: &mut Context<Self>) {
        if command.trim().is_empty() {
            return;
        }
        self.lines.push(Line::Command(command.clone()));
        self.history.push(&command);
        self.input.update(cx, |input, cx| input.set_text("", cx));
        self.set_running(true, cx);
        let (session, connection) = (self.session_id.clone(), self.connection_id.clone());
        let scope = QueryScope { database: Some(self.db.to_string()), container: None };
        let running = Backend::global(cx).spawn(cx, move |core| async move {
            let (store, sessions, history, log) =
                (core.connections(), core.query_sessions(), core.history(), core.statement_log());
            run_query(store, sessions, history, log, session, connection, scope, command).await
        });
        cx.spawn(async move |this, cx| {
            let outcome = running.await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(outcome) => {
                        let replies = outcome.results.iter().flat_map(|result| {
                            result.rows.iter().map(|row| row.first().cloned().flatten().unwrap_or_default())
                        });
                        this.lines.extend(replies.map(Line::Reply));
                        if let Some(error) = outcome.error {
                            this.lines.push(Line::Reply(format!("(error) {error}")));
                        }
                        cx.emit(CliEvent);
                    }
                    Err(error) => this.lines.push(Line::Reply(format!("(error) {error}"))),
                }
                this.set_running(false, cx);
                this.scroll.scroll_to_bottom();
            });
        })
        .detach();
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        self.running = running;
        let m = &cx.messages().keyspace;
        let placeholder = if running { m.running } else { m.cli_placeholder };
        self.input.update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        cx.notify();
    }

    /// Stops a blocking command; it then fails with the server's error.
    fn cancel(&mut self, cx: &mut Context<Self>) {
        let session = self.session_id.clone();
        Backend::global(cx)
            .spawn(cx, move |core| async move {
                let _ = cancel_query(core.query_sessions(), core.statement_log(), session).await;
            })
            .detach();
    }

    fn browse(&mut self, older: bool, cx: &mut Context<Self>) {
        if let Some(text) = self.history.browse(older) {
            self.input.update(cx, |input, cx| input.set_text(&text, cx));
        }
    }

    /// The prompt takes the keyboard.
    pub fn focus(&self, window: &mut Window, cx: &gpui::App) {
        self.input.read(cx).focus(window);
    }
}

fn tone_color(theme: &Theme, tone: ReplyTone) -> gpui::Hsla {
    match tone {
        ReplyTone::Error => theme.err,
        ReplyTone::Number => theme.num,
        ReplyTone::String => theme.str,
        ReplyTone::Plain => theme.text,
    }
}

impl Render for CliPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().keyspace;
        let prompt = || div().flex_none().text_color(theme.accent_text).child("›");
        let lines = self.lines.iter().map(|line| match line {
            Line::Command(text) => div().flex().gap(px(6.)).child(prompt()).child(div().min_w_0().child(text.clone())),
            Line::Reply(text) => div().text_color(tone_color(&theme, reply_tone(text))).child(text.clone()),
        });
        div()
            .h(gpui::px(CLI_HEIGHT))
            .flex_none()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(theme.line)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Older, _, cx| this.browse(true, cx)))
            .on_action(cx.listener(|this, _: &Newer, _, cx| this.browse(false, cx)))
            .child(
                div()
                    .h(px(32.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .border_b_1()
                    .border_color(theme.line)
                    .text_size(px(11.5))
                    .child(div().text_color(theme.text2).font_weight(gpui::FontWeight::SEMIBOLD).child("CLI"))
                    .child(div().min_w_0().ellipsis().text_color(theme.text3).child(self.address.clone()))
                    .child(div().flex_1())
                    .when(self.running, |bar| {
                        bar.child(
                            icon_button(&theme, "cli-cancel", "close", true, false)
                                .tooltip(Tooltip::text(m.cancel_blocking))
                                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                        )
                    })
                    .child(
                        icon_button(&theme, "cli-clear", "trash", true, false)
                            .tooltip(Tooltip::text(m.clear_cli))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.lines.clear();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("cli-output")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(12.))
                    .py(px(8.))
                    .font_family(MONO)
                    .text_size(px(12.))
                    .line_height(px(19.))
                    .text_color(theme.text)
                    .children(lines)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .h(px(19.))
                            .child(prompt())
                            .child(div().flex_1().min_w_0().child(self.input.clone())),
                    ),
            )
    }
}
