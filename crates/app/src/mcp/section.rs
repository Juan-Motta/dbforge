//! Settings › Agents (MCP), as the former web UI's `McpSettings.tsx`: the MCP
//! server on or off, its port, the token agents send, and the steps that add
//! DBForge to an agent ([`super::agent_setup`]). The core saves the settings
//! and starts, restarts or stops the server to match.
//!
//! The token: read from the core when the section opens and kept only in
//! this view while it is open; shown as bullets until the eye shows it, put
//! on the clipboard only by a copy button, replaced by Regenerate after a
//! second press. It never reaches the preferences, the events or a log.

use dbforge_core::mcp::commands::{mcp_state, mcp_token, regenerate_mcp_token, set_mcp_settings};
use dbforge_core::mcp::{McpSettings, Status};
use gpui::{
    AnyElement, AppContext, ClipboardItem, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};

use super::setup::{AgentId, Transport, parse_port};
use crate::backend::Backend;
use crate::i18n::{McpMessages, Translate};
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::callout::{Callout, Tone, callout_space};
use crate::ui::field::{Tooltip, field, help, label, text_box};
use crate::ui::modal::{form_section, note};
use crate::ui::option_list::{Mark, OptionItem, option_list};
use crate::ui::{TextInput, TextInputEvent, enabled, icon, icon_button, primary_button};

/// Bullets in place of the token while it is hidden: as many as the former web UI shows.
const HIDDEN_TOKEN_LENGTH: usize = 32;

/// The server's settings and state, as the core last reported them.
pub(super) struct Loaded {
    pub settings: McpSettings,
    pub status: Status,
    /// The executable agents start over stdio; `None` leaves HTTP as the only way.
    pub executable: Option<String>,
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
    error: Option<String>,
    /// The copy button last pressed, which shows a check.
    pub(super) copied: Option<SharedString>,
    pub(super) agent: AgentId,
    /// The transport picked; HTTP whatever is picked while there is no executable.
    pub(super) transport: Transport,
    /// The settings a running save writes; the latest asked meanwhile wait for it.
    saving: Option<McpSettings>,
    queued: Option<McpSettings>,
    _subscriptions: Vec<Subscription>,
}

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
        ];
        let mut section = Self {
            loaded: None,
            port,
            token: None,
            shown: false,
            regenerating: false,
            error: None,
            copied: None,
            agent: AgentId::Claude,
            transport: Transport::Stdio,
            saving: None,
            queued: None,
            _subscriptions: subscriptions,
        };
        section.load(cx);
        section
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

    fn render_status(&self, m: &McpMessages) -> AnyElement {
        let callout = |tone, title: &str, detail: &str| {
            Callout::new("mcp-status", tone, title.to_owned()).detail(detail.to_owned()).into_any_element()
        };
        if let Some(error) = &self.error {
            return callout(Tone::Failed, m.failed, error);
        }
        match self.loaded.as_ref().map(|loaded| &loaded.status) {
            Some(Status::Running { url }) => {
                Callout::new("mcp-status", Tone::Done, m.listening).detail(url.clone()).mono(true).into_any_element()
            }
            Some(Status::Failed { error }) => callout(Tone::Failed, m.failed, error),
            Some(Status::Stopped) | None => callout_space().into_any_element(),
        }
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

impl Render for McpSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().mcp;
        let enabled = self.loaded.as_ref().is_some_and(|loaded| loaded.settings.enabled);
        let invalid = parse_port(self.port.read(cx).text()).is_none();
        let toggle = OptionItem::new("mcp-enable", Mark::Check, m.enable, enabled)
            .icon("code")
            .description(m.enable_description)
            .disabled(self.loaded.is_none())
            .on_select(cx.listener(move |this, _, _, cx| this.set_enabled(!enabled, cx)));
        let port = field(
            label(&theme, "mcp-port-label", m.port, None),
            text_box(&self.port, invalid, true, window, cx),
            Some(help(&theme, if invalid { m.port_invalid } else { m.port_help }, invalid.then_some(theme.err))),
        );
        div()
            .pt(px(16.))
            .pb(px(14.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(option_list(&theme, vec![toggle.into_any_element()]))
            .child(self.render_status(m))
            .child(port)
            .child(self.render_token(&theme, cx))
            .child(self.render_agent_setup(&theme, cx))
            .child(note(&theme, m.access_note))
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
}
