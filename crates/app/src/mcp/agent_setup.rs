//! Settings › Agents › the agents: on the left, the known agents, each with
//! how it has used DBForge (DBForge adds or removes none: the setup on the
//! right is what the user runs in the agent); on the right, the
//! agent picked and the setup that adds DBForge to it, as the former web
//! UI's `AgentSetup.tsx`: by default a prompt to paste into the agent, which
//! has it add DBForge itself, or else the steps to run, with how it reaches
//! DBForge and the note that has it use DBForge in a project. Over stdio
//! (the default) nothing secret is written in; over HTTP the token is, shown
//! as `<token>` until the token is shown, while the copies always carry it.

use dbforge_core::mcp::McpSettings;
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};

use super::agents::{Ago, Usage, shown_agent, usage};
use super::section::{McpSection, SetupMode, tile, value_row};
use super::setup::{AgentId, Endpoint, Step, StepKind, TRANSPORTS, Transport, agent_prompt, server_url, setup_steps};
use crate::i18n::{McpMessages, Translate};
use crate::theme::{MONO, Theme, color};
use crate::ui::field::Tooltip;
use crate::ui::modal::note;
use crate::ui::{Ellipsis, any, dot, enabled, icon, icon_button, primary_button, segment_face, segmented};

/// What stands for the token while it is hidden.
const PLACEHOLDER: &str = "<token>";

/// Steps and notes that run several lines: kept as written, scrolled when long.
const TALL_MAX_HEIGHT: f32 = 180.;

/// The prompt's box: one height for every agent and transport, scrolled inside.
const PROMPT_HEIGHT: f32 = 220.;

/// The agents' column.
const LIST_WIDTH: f32 = 210.;

/// The modes, in the order the choice shows them.
const MODES: [SetupMode; 2] = [SetupMode::Prompt, SetupMode::Command];

fn agent_label(agent: AgentId, m: &McpMessages) -> &'static str {
    match agent {
        AgentId::Claude => m.claude_code,
        AgentId::Codex => m.codex,
        AgentId::Opencode => m.opencode,
        AgentId::Pi => m.pi,
        AgentId::Other => m.other_agent,
    }
}

/// A stable id for the agent's row and blocks.
fn agent_key(agent: AgentId) -> String {
    format!("{agent:?}").to_lowercase()
}

/// How the agent has used DBForge, as its row says it.
fn usage_text(usage: Usage, m: &McpMessages) -> String {
    match usage {
        Usage::Connected => m.connected.to_owned(),
        Usage::UsedAgo(Ago::Minutes(minutes)) => (m.used_minutes_ago)(minutes),
        Usage::UsedAgo(Ago::Hours(hours)) => (m.used_hours_ago)(hours),
        Usage::UsedAgo(Ago::Days(days)) => (m.used_days_ago)(days),
        Usage::UsedBefore => m.used_before.to_owned(),
        Usage::NotUsed => m.not_used.to_owned(),
    }
}

/// The agent's mark: its initials on its colour, or a generic one for "Other agent".
fn mark(theme: &Theme, agent: AgentId, size: f32) -> impl IntoElement {
    let (initials, fill) = match agent {
        AgentId::Claude => (Some("CC"), color(0xC8643BFF)),
        AgentId::Codex => (Some("CX"), color(0x3A3D46FF)),
        AgentId::Opencode => (Some("OC"), color(0x2F6F8FFF)),
        AgentId::Pi => (Some("Pi"), color(0x5B4BB0FF)),
        AgentId::Other => (None, theme.glass_hi),
    };
    let known = agent != AgentId::Other;
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.3))
        .bg(fill)
        .border_1()
        .border_color(theme.rim)
        .text_size(px(size * 0.38))
        .font_weight(FontWeight::BOLD)
        .text_color(if known { gpui::white() } else { theme.text })
        .map(|mark| match initials {
            Some(initials) => mark.child(initials),
            None => mark.child(icon("code", theme.text2, size * 0.5)),
        })
}

fn step_title(step: &Step, m: &McpMessages) -> String {
    match step.kind {
        StepKind::Terminal => m.step_terminal.into(),
        StepKind::Profile => m.step_profile.into(),
        StepKind::Config => (m.step_config)(step.file.unwrap_or_default()),
        StepKind::Connect => m.step_connect.into(),
        StepKind::Launch => m.step_launch.into(),
    }
}

/// Where the agent reaches DBForge over `transport`, `secret` written in over HTTP.
fn endpoint<'a>(transport: Transport, executable: Option<&'a str>, url: &'a str, secret: &'a str) -> Endpoint<'a> {
    match (transport, executable) {
        (Transport::Stdio, Some(executable)) => Endpoint::Stdio { executable },
        _ => Endpoint::Http { url, token: secret },
    }
}

/// One block to copy: what it is shown as, and what the copy carries.
pub(super) struct Block {
    pub id: SharedString,
    pub title: String,
    pub shown: String,
    pub copied: String,
    tall: bool,
}

/// The setup of one agent: its steps, and the prompt that has it add DBForge.
pub(super) struct Setup {
    pub steps: Vec<Block>,
    pub prompt: Block,
}

impl McpSection {
    /// The transport in use: the one picked, or HTTP when there is no executable to start.
    pub(super) fn effective_transport(&self) -> Transport {
        match self.loaded.as_ref().and_then(|loaded| loaded.executable.as_ref()) {
            Some(_) => self.transport,
            None => Transport::Http,
        }
    }

    /// The agent shown: the one picked, else the most recently used, else Claude Code.
    pub(super) fn shown_agent(&self) -> AgentId {
        shown_agent(self.picked, &self.agents)
    }

    pub(super) fn setup(&self, m: &McpMessages) -> Setup {
        let agent = self.shown_agent();
        let port = self.loaded.as_ref().map_or(McpSettings::default().port, |loaded| loaded.settings.port);
        let url = server_url(port);
        let executable = self.loaded.as_ref().and_then(|loaded| loaded.executable.as_deref());
        let transport = self.effective_transport();
        let token = self.token.as_deref();
        let shown_secret = match token {
            Some(token) if self.shown => token,
            _ => PLACEHOLDER,
        };
        let shown = endpoint(transport, executable, &url, shown_secret);
        let copied = endpoint(transport, executable, &url, token.unwrap_or_default());
        let key = agent_key(agent);
        let steps = setup_steps(agent, shown)
            .iter()
            .zip(setup_steps(agent, copied))
            .enumerate()
            .map(|(index, (step, copy))| Block {
                id: format!("{key}-step-{index}").into(),
                title: step_title(step, m),
                tall: step.text.contains('\n'),
                shown: step.text.clone(),
                copied: copy.text,
            })
            .collect();
        Setup {
            steps,
            prompt: Block {
                id: format!("{key}-prompt").into(),
                title: String::new(),
                shown: agent_prompt(agent, shown),
                copied: agent_prompt(agent, copied),
                tall: true,
            },
        }
    }

    fn pick(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        self.picked = Some(agent);
        cx.notify();
    }

    fn render_block(&self, theme: &Theme, block: Block, ready: bool, cx: &mut Context<Self>) -> AnyElement {
        let m = &cx.messages().mcp;
        let copied = self.copied.as_ref() == Some(&block.id);
        let Block { id, title, shown, copied: copy, tall } = block;
        let text = div()
            .id(SharedString::from(format!("{id}-text")))
            .flex_1()
            .min_w_0()
            .text_color(theme.text2)
            .font_family(MONO)
            .text_size(px(12.))
            .when(tall, |text| text.max_h(px(TALL_MAX_HEIGHT)).overflow_y_scroll())
            .child(shown);
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().text_size(px(12.)).text_color(theme.text2).child(title))
            .child(
                value_row(theme, false).child(text).child(
                    icon_button(
                        theme,
                        SharedString::from(format!("{id}-copy")),
                        if copied { "check" } else { "duplicate" },
                        ready,
                        false,
                    )
                    .tooltip(Tooltip::text(m.copy))
                    .when(ready, |button| {
                        button.on_click(cx.listener(move |this, _, _, cx| this.copy(id.clone(), copy.clone(), cx)))
                    }),
                ),
            )
            .into_any_element()
    }

    /// The agents tile: the list on the left, the agent picked on the right.
    pub(super) fn render_agents(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        tile(theme).flex().overflow_hidden().child(self.render_list(theme, cx)).child(self.render_detail(theme, cx))
    }

    fn render_list(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let shown = self.shown_agent();
        let now = chrono::Local::now().timestamp_millis();
        let rows: Vec<AnyElement> = self
            .agents
            .iter()
            .map(|row| {
                let agent = row.agent;
                let usage = usage(row, now);
                let tint = match usage {
                    Usage::Connected => theme.ok,
                    Usage::NotUsed => theme.text3.opacity(0.5),
                    _ => theme.text3,
                };
                let status = div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .text_color(theme.text3)
                    .child(dot(tint).size(px(7.)).flex_none())
                    .child(div().min_w_0().ellipsis().child(usage_text(usage, m)));
                let text = div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().ellipsis().child(agent_label(agent, m)))
                    .child(status);
                list_row(theme, agent, agent == shown, 46.)
                    .child(text)
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(agent, cx)))
                    .into_any_element()
            })
            .collect();
        div()
            .w(px(LIST_WIDTH))
            .flex_none()
            .px(px(10.))
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .border_r_1()
            .border_color(theme.line)
            .child(
                div()
                    .pt(px(2.))
                    .pb(px(8.))
                    .px(px(10.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text3)
                    .child(m.agents_heading.to_uppercase()),
            )
            .children(rows)
            .child(div().pt(px(10.)).px(px(10.)).text_size(px(11.5)).text_color(theme.text3).child(m.connected_help))
    }

    fn render_detail(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let agent = self.shown_agent();
        let name = agent_label(agent, m);
        let transport = self.effective_transport();
        let header = div().flex().items_center().gap(px(12.)).child(mark(theme, agent, 36.)).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().ellipsis().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(name))
                .child(
                    div()
                        .ellipsis()
                        .text_size(px(12.))
                        .text_color(theme.text3)
                        .child(if transport == Transport::Stdio { m.reaches_stdio } else { m.reaches_http }),
                ),
        );
        let modes = MODES
            .iter()
            .map(|&mode| {
                let label = if mode == SetupMode::Prompt { m.paste_prompt } else { m.run_command };
                any(segment_face(theme, None, label, self.mode == mode)
                    .id(if mode == SetupMode::Prompt { "mcp-mode-prompt" } else { "mcp-mode-command" })
                    .debug_selector(move || format!("mcp-mode-{mode:?}"))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.mode = mode;
                        cx.notify();
                    })))
            })
            .collect();
        let hint = match self.mode {
            SetupMode::Prompt => (m.prompt_hint)(name),
            SetupMode::Command => (m.command_hint)(name),
        };
        let choice = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().flex_none().child(segmented(
                theme,
                "mcp-setup-mode",
                MODES.iter().position(|mode| *mode == self.mode),
                modes,
            )))
            // Two lines kept whichever mode says what, so the setup below doesn't move.
            .child(div().flex_1().min_w_0().h(px(32.)).flex().items_center().child(
                div().line_clamp(2).text_size(px(12.)).line_height(px(16.)).text_color(theme.text3).child(hint),
            ));
        let body = match self.mode {
            SetupMode::Prompt => self.render_prompt(theme, cx).into_any_element(),
            SetupMode::Command => self.render_command(theme, cx).into_any_element(),
        };
        div()
            .flex_1()
            .min_w_0()
            .px(px(18.))
            .py(px(16.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(header)
            .child(choice)
            .child(body)
    }

    /// Whether the copies can be made: over stdio nothing waits on the token.
    fn copies_ready(&self) -> bool {
        self.effective_transport() == Transport::Stdio || self.token.is_some()
    }

    fn render_prompt(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let ready = self.copies_ready();
        let Block { id, shown, copied: copy, .. } = self.setup(m).prompt;
        let copied = self.copied.as_ref() == Some(&id);
        let text = div()
            .id(SharedString::from(format!("{id}-text")))
            .debug_selector(|| "mcp-prompt".into())
            .h(px(PROMPT_HEIGHT))
            .px(px(14.))
            .py(px(12.))
            .overflow_y_scroll()
            .rounded(px(12.))
            .border_1()
            .border_color(theme.line)
            .bg(theme.glass)
            .font_family(MONO)
            .text_size(px(12.))
            .line_height(px(19.))
            .text_color(theme.text2)
            .child(shown);
        let copy_button = enabled(
            primary_button(theme, m.copy_prompt).child(icon(
                if copied { "check" } else { "duplicate" },
                gpui::white(),
                14.,
            )),
            ready,
        )
        .id("mcp-copy-prompt")
        .when(ready, |button| {
            button.on_click(cx.listener(move |this, _, _, cx| this.copy(id.clone(), copy.clone(), cx)))
        });
        let help = if self.effective_transport() == Transport::Stdio { m.prompt_help_stdio } else { m.prompt_help };
        div().flex().flex_col().gap(px(10.)).child(text).child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(copy_button)
                .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(theme.text3).child(help)),
        )
    }

    fn render_command(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let has_executable = self.loaded.as_ref().is_some_and(|loaded| loaded.executable.is_some());
        let transport = self.effective_transport();
        let ready = self.copies_ready();
        let transports = has_executable.then(|| {
            let options = TRANSPORTS
                .iter()
                .map(|&option| {
                    let label = if option == Transport::Stdio { m.transport_stdio } else { m.transport_http };
                    any(segment_face(theme, None, label, transport == option)
                        .id(SharedString::from(format!("mcp-transport-{option:?}")))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.transport = option;
                            cx.notify();
                        })))
                })
                .collect();
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_size(px(12.)).text_color(theme.text2).child(m.transport))
                .child(div().flex().child(segmented(
                    theme,
                    "mcp-transport",
                    TRANSPORTS.iter().position(|option| *option == transport),
                    options,
                )))
                .child(note(
                    theme,
                    if transport == Transport::Stdio { m.transport_stdio_note } else { m.transport_http_note },
                ))
        });
        let Setup { steps, .. } = self.setup(m);
        let steps: Vec<AnyElement> =
            steps.into_iter().map(|block| self.render_block(theme, block, ready, cx)).collect();
        div()
            .id("mcp-command")
            .debug_selector(|| "mcp-command".into())
            .flex()
            .flex_col()
            .gap(px(14.))
            .children(transports)
            .children(steps)
    }
}

/// A row of the agents' list: the agent's mark, then what the caller adds.
fn list_row(theme: &Theme, agent: AgentId, selected: bool, height: f32) -> gpui::Stateful<gpui::Div> {
    let tint: Hsla = if selected { theme.text } else { theme.text2 };
    div()
        .id(SharedString::from(format!("mcp-agent-{}", agent_key(agent))))
        .h(px(height))
        .flex_none()
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(px(10.))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(tint)
        .cursor_pointer()
        .when(selected, |row| row.bg(theme.accent.opacity(0.2)))
        .when(!selected, |row| row.hover(|row| row.bg(theme.sel)))
        .child(mark(theme, agent, 26.))
}
