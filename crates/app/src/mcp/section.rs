//! Settings › Agents (MCP), the combined design of the former web UI's
//! `McpSettings.tsx` and its agents list: a status card (the server on or
//! off, its state and address, the agents connected, their statements today
//! and the changes waiting), with the port and the token agents send behind
//! "Port and token"; the known agents, each with how it has used DBForge
//! and the setup that adds DBForge to it ([`super::agent_setup`]). What
//! agents may reach on each saved connection is its own section,
//! Agent access ([`super::access`]). The core saves the settings and starts, restarts or stops the server to match.
//!
//! The token: read from the core when the section opens and kept only in
//! this view while it is open; shown as bullets until the eye shows it, put
//! on the clipboard only by a copy button, replaced by Regenerate after a
//! second press. It never reaches the preferences, the events or a log.

use std::time::Duration;

use dbforge_core::Event;
use dbforge_core::mcp::commands::{mcp_agents, mcp_state, mcp_token, regenerate_mcp_token, set_mcp_settings};
use dbforge_core::mcp::{McpSettings, Status};
use dbforge_core::statement_log::commands::list_statements;
use gpui::{
    AnyElement, AppContext, ClipboardItem, Context, Entity, EventEmitter, Focusable, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, div, prelude::FluentBuilder, px,
};

use super::Approvals;
use super::agents::{Listed, connected, listed, statements_today};
use super::setup::{AgentId, Transport, parse_port};
use crate::backend::Backend;
use crate::events::EventHub;
use crate::i18n::{McpMessages, Translate};
use crate::statement_log::log::{CAPACITY, Kind, kind_of};
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::field::{Tooltip, field, help, label, text_box};
use crate::ui::modal::form_section;
use crate::ui::{
    Ellipsis, TextInput, TextInputEvent, button, enabled, icon, icon_button, pill, primary_button, switch,
};

/// Bullets in place of the token while it is hidden: as many as the former web UI shows.
const HIDDEN_TOKEN_LENGTH: usize = 32;

/// How often the agents are read again, so "Connected" wears off without an event.
const REFRESH_EVERY: Duration = Duration::from_secs(30);

/// The status card's second line: room for two lines whatever it says, so
/// the card keeps its height when the server starts, stops or fails.
const STATUS_LINE_HEIGHT: f32 = 17.;

/// The server's settings and state, as the core last reported them.
pub(super) struct Loaded {
    pub settings: McpSettings,
    pub status: Status,
    /// The executable agents start over stdio; `None` leaves HTTP as the only way.
    pub executable: Option<String>,
}

/// How the agent shown is set up: by pasting a prompt into it (the default) or by running the steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SetupMode {
    #[default]
    Prompt,
    Command,
}

/// What the section tells the settings dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpSectionEvent {
    /// A connection's agent access was saved: the explorer reads the connections again.
    ConnectionsChanged,
}

pub struct McpSection {
    pub(super) loaded: Option<Loaded>,
    port: Entity<TextInput>,
    /// `None` until it is read; the copy buttons wait for it.
    pub(super) token: Option<String>,
    /// The token is written in, instead of bullets and `<token>`.
    pub(super) shown: bool,
    /// Regenerate was pressed once: the row asks to confirm.
    regenerating: bool,
    /// "Port and token" is open: the port field and the token show under the status.
    pub(super) secrets_open: bool,
    pub(super) error: Option<String>,
    /// The copy button last pressed, which shows a check.
    pub(super) copied: Option<SharedString>,
    /// The agents' list, with what was last read of each.
    pub(super) agents: Vec<Listed>,
    /// The agent the user picked; until they pick one, the most recently used, else Claude Code.
    pub(super) picked: Option<AgentId>,
    pub(super) mode: SetupMode,
    /// The transport picked; HTTP whatever is picked while there is no executable.
    pub(super) transport: Transport,
    /// Each statement agents ran (its id, and when it started), as the statement log keeps them.
    agent_statements: Vec<(u64, u64)>,
    /// The settings a running save writes; the latest asked meanwhile wait for it.
    saving: Option<McpSettings>,
    queued: Option<McpSettings>,
    _refresh: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<McpSectionEvent> for McpSection {}

impl McpSection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let port = cx.new(|cx| TextInput::new("", cx));
        let subscriptions = vec![
            cx.subscribe(&port, |this, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Submitted => this.apply_port(cx),
                TextInputEvent::Changed => cx.notify(),
                _ => {}
            }),
            cx.on_blur(&port.focus_handle(cx), window, |this, _, cx| this.apply_port(cx)),
            cx.subscribe(&EventHub::global(cx), |this, _, event: &Event, cx| this.hear(event, cx)),
            cx.observe(&Approvals::global(cx), |_, _, cx| cx.notify()),
        ];
        // Read again now and then: an agent stops counting as connected without telling anyone.
        let refresh = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH_EVERY).await;
                if this.update(cx, |this, cx| this.load_agents(cx)).is_err() {
                    break;
                }
            }
        });
        let mut section = Self {
            loaded: None,
            port,
            token: None,
            shown: false,
            regenerating: false,
            secrets_open: false,
            error: None,
            copied: None,
            agents: listed(&[]),
            picked: None,
            mode: SetupMode::default(),
            transport: Transport::Stdio,
            agent_statements: Vec::new(),
            saving: None,
            queued: None,
            _refresh: refresh,
            _subscriptions: subscriptions,
        };
        section.load(cx);
        section.load_agents(cx);
        section.load_statements(cx);
        section
    }

    fn hear(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::McpAgentsChanged => self.load_agents(cx),
            Event::StatementLogged(entry) if kind_of(entry.origin) == Kind::Agents => {
                self.agent_statements.push((entry.id, entry.at));
                if self.agent_statements.len() > CAPACITY {
                    self.agent_statements.remove(0);
                }
                cx.notify();
            }
            _ => {}
        }
    }

    /// Reads the server's state and the token, each off the UI thread.
    fn load(&mut self, cx: &mut Context<Self>) {
        let backend = Backend::global(cx);
        let state = backend.spawn_blocking(cx, |core| {
            let state = mcp_state(core.mcp_settings(), core.mcp_server());
            Loaded { settings: state.settings, status: state.status, executable: state.executable }
        });
        let token = backend.spawn_blocking(cx, |core| mcp_token(core.connections()).map_err(|error| error.to_string()));
        cx.spawn(async move |this, cx| {
            let loaded = state.await;
            let _ = this.update(cx, |this, cx| {
                let port = loaded.settings.port.to_string();
                this.port.update(cx, |input, cx| input.set_text(&port, cx));
                this.loaded = Some(loaded);
                cx.notify();
            });
            let token = token.await;
            let _ = this.update(cx, |this, cx| {
                match token {
                    Ok(token) => this.token = Some(token),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads the agents that used DBForge, off the UI thread.
    pub(super) fn load_agents(&mut self, cx: &mut Context<Self>) {
        let seen = Backend::global(cx).spawn_blocking(cx, |core| mcp_agents(core.mcp_agents(), core.mcp_sessions()));
        cx.spawn(async move |this, cx| {
            let seen = seen.await;
            let _ = this.update(cx, |this, cx| {
                this.agents = listed(&seen);
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads when the statements agents ran started, from the statement log.
    fn load_statements(&mut self, cx: &mut Context<Self>) {
        let times = Backend::global(cx).spawn_blocking(cx, |core| {
            let entries = list_statements(core.statement_log());
            let agents = entries.into_iter().filter(|entry| kind_of(entry.origin) == Kind::Agents);
            agents.map(|entry| (entry.id, entry.at)).collect()
        });
        cx.spawn(async move |this, cx| {
            let mut statements: Vec<(u64, u64)> = times.await;
            let _ = this.update(cx, |this, cx| {
                // Statements logged while the list was read came as events too: keep each once.
                statements.append(&mut this.agent_statements);
                statements.sort_unstable_by_key(|(id, _)| *id);
                statements.dedup_by_key(|(id, _)| *id);
                this.agent_statements = statements;
                cx.notify();
            });
        })
        .detach();
    }

    /// Saves `settings`, the core starting, restarting or stopping the
    /// server to match; one save at a time, the latest asked going next.
    fn save(&mut self, settings: McpSettings, cx: &mut Context<Self>) {
        self.error = None;
        cx.notify();
        if self.saving.is_some() {
            self.queued = Some(settings);
            return;
        }
        self.saving = Some(settings);
        let saving = Backend::global(cx).spawn(cx, move |core| async move {
            set_mcp_settings(&core, settings).await.map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let saved = saving.await;
            let _ = this.update(cx, |this, cx| {
                match saved {
                    Ok(state) => {
                        this.loaded = Some(Loaded {
                            settings: state.settings,
                            status: state.status,
                            executable: state.executable,
                        })
                    }
                    Err(error) => this.error = Some(error),
                }
                this.saving = None;
                if let Some(next) = this.queued.take() {
                    this.save(next, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The settings a change starts from: the last asked, saved or not yet.
    fn latest(&self) -> Option<McpSettings> {
        self.queued.or(self.saving).or(self.loaded.as_ref().map(|loaded| loaded.settings))
    }

    fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if let Some(latest) = self.latest() {
            self.save(McpSettings { enabled, ..latest }, cx);
        }
    }

    /// Enter or leaving the field: a valid port other than the last one asked is saved.
    fn apply_port(&mut self, cx: &mut Context<Self>) {
        let Some(latest) = self.latest() else { return };
        let Some(port) = parse_port(self.port.read(cx).text()) else { return };
        if port != latest.port {
            self.save(McpSettings { port, ..latest }, cx);
        }
    }

    /// Whether the server is set to run (running or failing to): only then are its port and token offered.
    fn enabled(&self) -> bool {
        self.latest().is_some_and(|latest| latest.enabled)
    }

    /// Puts `text` on the clipboard; the button `id` shows a check.
    pub(super) fn copy(&mut self, id: SharedString, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = Some(id);
        cx.notify();
    }

    /// Replaces the token (the second press of Regenerate): agents using the
    /// old one are refused from now on. The new one is shown at once.
    fn regenerate(&mut self, cx: &mut Context<Self>) {
        self.regenerating = false;
        cx.notify();
        let regenerating = Backend::global(cx).spawn(cx, |core| async move {
            let token = regenerate_mcp_token(core.connections(), core.mcp_server()).await;
            // The server keeps running with the new token; read its state again rather than assume it.
            let state = mcp_state(core.mcp_settings(), core.mcp_server());
            token
                .map(|token| {
                    (token, Loaded { settings: state.settings, status: state.status, executable: state.executable })
                })
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let regenerated = regenerating.await;
            let _ = this.update(cx, |this, cx| {
                match regenerated {
                    Ok((token, loaded)) => {
                        this.token = Some(token);
                        this.shown = true;
                        this.loaded = Some(loaded);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The pill, the card's title and its second line, in the state's tint.
    fn status_text(
        &self,
        m: &McpMessages,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> (&'static str, &'static str, String, Hsla) {
        let failed = |error: &str| (m.status_failed, m.failed, error.to_owned(), theme.err);
        if let Some(error) = &self.error {
            return failed(error);
        }
        match self.loaded.as_ref().map(|loaded| (&loaded.status, loaded.settings.port)) {
            Some((Status::Running { .. }, port)) => {
                let now = chrono::Local::now();
                let mut parts = vec![
                    (m.server_on)(&format!("127.0.0.1:{port}")),
                    (m.agents_connected)(connected(&self.agents, now.timestamp_millis())),
                    (m.statements_today)(statements_today(
                        self.agent_statements.iter().map(|(_, at)| *at),
                        now.timestamp_millis(),
                        &chrono::Local,
                    )),
                ];
                let waiting = Approvals::global(cx).read(cx).waiting();
                if waiting > 0 {
                    parts.push((m.changes_waiting)(waiting));
                }
                (m.status_running, m.status_on, parts.join(" · "), theme.ok)
            }
            Some((Status::Failed { error }, _)) => failed(error),
            Some((Status::Stopped, _)) | None => {
                (m.status_stopped, m.status_off, m.enable_description.to_owned(), theme.text3)
            }
        }
    }

    fn render_status(&self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let (state, title, detail, tint) = self.status_text(m, theme, cx);
        let on = self.enabled();
        let ready = self.loaded.is_some();
        let secrets = self.secrets_open && on;
        let summary = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(
                div()
                    .size(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.))
                    .bg(tint.opacity(0.14))
                    .child(icon("code", tint, 18.)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .min_w_0()
                                    .ellipsis()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(pill(state, tint).flex_none()),
                    )
                    .child(
                        div()
                            .h(px(STATUS_LINE_HEIGHT * 2.))
                            .overflow_hidden()
                            .line_clamp(2)
                            .text_ellipsis()
                            .text_size(px(12.5))
                            .line_height(px(STATUS_LINE_HEIGHT))
                            .text_color(theme.text3)
                            .child(detail),
                    ),
            )
            .child(
                // Kept in place while the server is off, so the switch doesn't move.
                button(theme, Some("key"), m.port_and_token)
                    .when(secrets, |button| button.border_color(theme.accent))
                    .id("mcp-port-and-token")
                    .when(!on, |button| button.invisible())
                    .when(on, |button| {
                        button.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                            this.secrets_open = !this.secrets_open;
                            cx.notify();
                        }))
                    }),
            )
            .child(
                enabled(switch(theme, on), ready)
                    .id("mcp-enable")
                    .tooltip(Tooltip::text(m.enable))
                    .when(ready, |toggle| {
                        toggle.on_click(cx.listener(move |this, _, _, cx| this.set_enabled(!on, cx)))
                    }),
            );
        tile(theme)
            .px(px(18.))
            .py(px(14.))
            .flex()
            .flex_col()
            .border_color(tint.opacity(0.22))
            .bg(tint.opacity(0.05))
            .child(summary)
            .when(secrets, |card| card.child(self.render_secrets(theme, window, cx)))
    }

    /// Behind "Port and token": the port field and the token's row.
    fn render_secrets(&self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let invalid = parse_port(self.port.read(cx).text()).is_none();
        let port = field(
            label(theme, "mcp-port-label", m.port, None),
            div().w(px(160.)).child(text_box(&self.port, invalid, true, window, cx)),
            Some(help(theme, if invalid { m.port_invalid } else { m.port_help }, invalid.then_some(theme.err))),
        );
        div()
            .mt(px(14.))
            .pt(px(14.))
            .border_t_1()
            .border_color(theme.line)
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(port)
            .child(self.render_token(theme, cx))
    }

    fn render_token(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let confirming = self.regenerating;
        let ready = self.token.is_some();
        let value: SharedString = if confirming {
            m.regenerate_confirm.into()
        } else if self.shown {
            self.token.clone().unwrap_or_default().into()
        } else {
            "•".repeat(HIDDEN_TOKEN_LENGTH).into()
        };
        let actions: Vec<AnyElement> = if confirming {
            vec![
                quiet_button(theme, "mcp-regenerate-cancel", None, m.cancel, true)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.regenerating = false;
                        cx.notify();
                    }))
                    .into_any_element(),
                primary_button(theme, m.regenerate)
                    .id("mcp-regenerate-confirm")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.regenerate(cx)))
                    .into_any_element(),
            ]
        } else {
            let copied = self.copied.as_ref().is_some_and(|id| id == "token");
            vec![
                icon_button(theme, "mcp-token-show", "view", ready, self.shown)
                    .tooltip(Tooltip::text(if self.shown { m.hide } else { m.show }))
                    .when(ready, |show| {
                        show.on_click(cx.listener(|this, _, _, cx| {
                            this.shown = !this.shown;
                            cx.notify();
                        }))
                    })
                    .into_any_element(),
                icon_button(theme, "mcp-token-copy", if copied { "check" } else { "duplicate" }, ready, false)
                    .tooltip(Tooltip::text(m.copy))
                    .when(ready, |copy| {
                        copy.on_click(cx.listener(|this, _, _, cx| {
                            if let Some(token) = this.token.clone() {
                                this.copy("token".into(), token, cx);
                            }
                        }))
                    })
                    .into_any_element(),
                quiet_button(theme, "mcp-regenerate", Some("refresh"), m.regenerate, ready)
                    .when(ready, |regenerate| {
                        regenerate.on_click(cx.listener(|this, _, _, cx| {
                            this.regenerating = true;
                            cx.notify();
                        }))
                    })
                    .into_any_element(),
            ]
        };
        form_section(theme, m.token).child(
            value_row(theme, confirming)
                .child(icon(
                    if confirming { "warning" } else { "key" },
                    if confirming { theme.warn } else { theme.text3 },
                    14.,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(theme.text2)
                        .text_size(px(12.))
                        .when(!confirming, |value| value.font_family(MONO))
                        .child(value),
                )
                .children(actions),
        )
    }
}

/// A value with its actions, one height whether it shows the value or asks
/// to confirm: the token, and each block of the agent setup.
pub(super) fn value_row(theme: &Theme, confirming: bool) -> gpui::Div {
    div()
        .w_full()
        .min_h(px(40.))
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(12.))
        .pr(px(4.))
        .py(px(4.))
        .rounded(px(8.))
        .border_1()
        .text_color(theme.text3)
        .map(|row| {
            if confirming {
                row.border_color(theme.warn.opacity(0.45)).bg(theme.warn.opacity(0.1))
            } else {
                row.border_color(theme.line)
            }
        })
}

/// A frameless button with an optional icon: Regenerate, and Cancel beside
/// its confirmation. Dimmed and deaf while `on` is false; the owner adds the click.
fn quiet_button(
    theme: &Theme,
    id: &'static str,
    icon_name: Option<&'static str>,
    label: &str,
    on: bool,
) -> gpui::Stateful<gpui::Div> {
    let button = div()
        .h(px(32.))
        .px(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .rounded(px(16.))
        .text_color(theme.text2)
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .when_some(icon_name, |button, name| button.child(icon(name, theme.text2, 14.)))
        .child(label.to_owned());
    enabled(button, on).id(id).when(on, |button| button.hover(|button| button.bg(theme.sel)))
}

/// A framed part of the section: the status card, the agents, what they can reach.
pub(super) fn tile(theme: &Theme) -> gpui::Div {
    div().rounded(px(14.)).border_1().border_color(theme.line)
}

impl Render for McpSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        div()
            .pt(px(6.))
            .pb(px(14.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(self.render_status(&theme, window, cx))
            .child(self.render_agents(&theme, cx))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::test_support::{draw, setup, wait_until};

    /// A port nothing listens on, high and on the loopback address only.
    fn free_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()
    }

    fn opened(port: u16, cx: &mut TestAppContext) -> (Entity<McpSection>, &mut VisualTestContext) {
        cx.update(|cx| {
            let settings = McpSettings { enabled: false, port };
            Backend::global(cx).core().mcp_settings().set(settings).unwrap();
        });
        let (section, cx) = cx.add_window_view(McpSection::new);
        wait_until(cx, "the state and the token", |cx| {
            section.read_with(cx, |section, _| section.loaded.is_some() && section.token.is_some())
        });
        draw(cx);
        (section, cx)
    }

    fn status(section: &Entity<McpSection>, cx: &mut VisualTestContext) -> Option<Status> {
        section.read_with(cx, |section, _| section.loaded.as_ref().map(|loaded| loaded.status.clone()))
    }

    #[gpui::test]
    fn reads_the_settings_and_keeps_the_token_hidden_until_shown(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let port = free_port();
        let (section, cx) = opened(port, cx);
        section.read_with(cx, |section, cx| {
            assert_eq!(section.port.read(cx).text(), port.to_string());
            assert_eq!(section.token.as_ref().map(String::len), Some(64));
            assert!(!section.shown && !section.regenerating);
        });
    }

    #[gpui::test]
    fn applies_only_a_valid_new_port_and_turns_the_server_on_and_off_there(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (first, second) = (free_port(), free_port());
        let (section, cx) = opened(first, cx);
        let saved = |cx: &mut VisualTestContext| cx.update(|_, cx| Backend::global(cx).core().mcp_settings().get());

        section.update(cx, |section, cx| {
            section.port.update(cx, |input, cx| input.set_text("80", cx));
            section.apply_port(cx);
        });
        cx.run_until_parked();
        assert_eq!(saved(cx).port, first, "a privileged port is not applied");

        // Turned on and moved at once: the saves run one after the other, the last one winning.
        section.update(cx, |section, cx| {
            section.set_enabled(true, cx);
            section.port.update(cx, |input, cx| input.set_text(&second.to_string(), cx));
            section.apply_port(cx);
        });
        let url = format!("http://127.0.0.1:{second}/mcp");
        wait_until(cx, "the server on the new port", |cx| {
            status(&section, cx) == Some(Status::Running { url: url.clone() })
                && section.read_with(cx, |section, _| section.saving.is_none())
        });
        assert_eq!(saved(cx), McpSettings { enabled: true, port: second });

        section.update(cx, |section, cx| section.set_enabled(false, cx));
        wait_until(cx, "the server stopped", |cx| status(&section, cx) == Some(Status::Stopped));
        assert!(!saved(cx).enabled);
    }

    #[gpui::test]
    fn regenerating_asks_first_then_shows_the_new_token(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (section, cx) = opened(free_port(), cx);
        let old = section.read_with(cx, |section, _| section.token.clone());

        section.update(cx, |section, cx| {
            section.regenerating = true;
            cx.notify();
        });
        draw(cx);
        section.read_with(cx, |section, _| assert_eq!(section.token, old, "the first press only asks"));

        section.update(cx, |section, cx| section.regenerate(cx));
        wait_until(cx, "the new token", |cx| section.read_with(cx, |section, _| section.token != old));
        let stored =
            cx.update(|_, cx| dbforge_core::mcp::commands::mcp_token(Backend::global(cx).core().connections()));
        section.read_with(cx, |section, _| {
            assert!(section.shown && !section.regenerating);
            assert_eq!(section.token.as_deref(), stored.ok().as_deref(), "the core keeps the new one");
        });
    }

    #[gpui::test]
    fn copies_the_token_only_from_its_button(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (section, cx) = opened(free_port(), cx);
        let token = section.read_with(cx, |section, _| section.token.clone().unwrap());
        section.update(cx, |section, cx| section.copy("token".into(), token.clone(), cx));
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()), Some(token));
        section.read_with(cx, |section, _| assert_eq!(section.copied.as_ref().map(|id| id.as_ref()), Some("token")));
    }

    #[gpui::test]
    fn shows_the_prompt_by_default_and_the_command_once_chosen(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (section, cx) = opened(free_port(), cx);
        section.read_with(cx, |section, _| {
            assert_eq!(section.mode, SetupMode::Prompt);
            assert_eq!(section.shown_agent(), AgentId::Claude, "no agent used yet");
        });
        assert!(cx.debug_bounds("mcp-prompt").is_some(), "the prompt shows");
        assert!(cx.debug_bounds("mcp-command").is_none());

        let choice = cx.debug_bounds("mcp-mode-Command").expect("the Run a command choice");
        cx.simulate_click(choice.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        section.read_with(cx, |section, _| assert_eq!(section.mode, SetupMode::Command));
        // GPUI keeps the bounds of elements drawn in earlier frames, so only what appears is checked.
        assert!(cx.debug_bounds("mcp-command").is_some(), "the command shows");
    }

    #[gpui::test]
    fn the_prompt_shows_a_placeholder_over_http_and_copies_the_real_token(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (section, cx) = opened(free_port(), cx);
        section.update(cx, |section, cx| {
            section.transport = Transport::Http;
            cx.notify();
        });
        let (prompt, token) = section
            .read_with(cx, |section, cx| (section.setup(&cx.messages().mcp).prompt, section.token.clone().unwrap()));
        assert!(prompt.shown.contains("<token>") && !prompt.shown.contains(&token));
        assert!(prompt.copied.contains(&token) && !prompt.copied.contains("<token>"));
        section.update(cx, |section, cx| section.copy(prompt.id.clone(), prompt.copied.clone(), cx));
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()), Some(prompt.copied));
    }

    #[gpui::test]
    fn lists_the_agents_seen_and_hears_when_they_change(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (section, cx) = opened(free_port(), cx);
        section.read_with(cx, |section, _| assert!(section.agents.iter().all(|row| !row.used)));
        cx.update(|_, cx| {
            let core = Backend::global(cx).core();
            core.mcp_agents().record(&dbforge_core::events::NoEvents, "codex-mcp-client", Some("1.0"));
            EventHub::global(cx).update(cx, |_, cx| cx.emit(Event::McpAgentsChanged));
        });
        wait_until(cx, "the agent listed", |cx| {
            section.read_with(cx, |section, _| section.agents.iter().any(|row| row.used))
        });
        section.read_with(cx, |section, _| {
            assert_eq!(section.agents.len(), 5, "the known agents, always");
            assert_eq!(section.shown_agent(), AgentId::Codex, "the most recently active");
            let now = chrono::Local::now().timestamp_millis();
            assert_eq!(connected(&section.agents, now), 1);
        });
        draw(cx);
    }

    #[gpui::test]
    fn counts_the_agent_statements_logged_today(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (section, cx) = opened(free_port(), cx);
        let now = chrono::Local::now().timestamp_millis() as u64;
        let entry = |id: u64, origin: dbforge_core::statement_log::Origin| dbforge_core::statement_log::Entry {
            id,
            at: now,
            connection_id: "c".into(),
            database: None,
            origin,
            text: "SELECT 1".into(),
            elapsed_ms: 1,
            rows: None,
            error: None,
        };
        cx.update(|_, cx| {
            EventHub::global(cx).update(cx, |_, cx| {
                cx.emit(Event::StatementLogged(entry(1, dbforge_core::statement_log::Origin::Agent)));
                cx.emit(Event::StatementLogged(entry(2, dbforge_core::statement_log::Origin::Console)));
                cx.emit(Event::StatementLogged(entry(3, dbforge_core::statement_log::Origin::Agent)));
            })
        });
        cx.run_until_parked();
        section.read_with(cx, |section, _| {
            let times = section.agent_statements.iter().map(|(_, at)| *at);
            assert_eq!(statements_today(times, now as i64, &chrono::Local), 2);
        });
    }
}
