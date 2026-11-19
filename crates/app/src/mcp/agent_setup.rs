//! Settings › Agents › Add DBForge to an agent, as the web UI's
//! `AgentSetup.tsx`: pick an agent and how it reaches DBForge, then the
//! steps that add DBForge to it, a prompt that has the agent do it itself,
//! and the note that has it use DBForge in a project. Over stdio (the
//! default) nothing secret is written in; over HTTP the token is, shown as
//! `<token>` until the token is shown, while the copies always carry it.

use dbforge_core::mcp::McpSettings;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, div, prelude::FluentBuilder, px,
};

use super::section::{McpSection, value_row};
use super::setup::{
    AGENTS, AgentId, Endpoint, Step, StepKind, TRANSPORTS, Transport, agent_prompt, instructions_file, project_note,
    server_url, setup_steps,
};
use crate::i18n::{McpMessages, Translate};
use crate::theme::{MONO, Theme};
use crate::ui::field::Tooltip;
use crate::ui::modal::{form_section, note};
use crate::ui::{choice, choice_group, icon_button};

/// What stands for the token while it is hidden.
const PLACEHOLDER: &str = "<token>";

/// Prompts and notes run several lines: kept as written, scrolled when long.
const TALL_MAX_HEIGHT: f32 = 180.;

fn agent_label(agent: AgentId, m: &McpMessages) -> &'static str {
    match agent {
        AgentId::Claude => m.claude_code,
        AgentId::Codex => m.codex,
        AgentId::Opencode => m.opencode,
        AgentId::Pi => m.pi,
        AgentId::Other => m.other_agent,
    }
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
struct Block {
    id: SharedString,
    title: String,
    shown: String,
    copied: String,
    tall: bool,
}

impl McpSection {
    /// The transport in use: the one picked, or HTTP when there is no executable to start.
    fn effective_transport(&self) -> Transport {
        match self.loaded.as_ref().and_then(|loaded| loaded.executable.as_ref()) {
            Some(_) => self.transport,
            None => Transport::Http,
        }
    }

    fn blocks(&self, m: &McpMessages) -> Vec<Block> {
        let agent = self.agent;
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
        let steps = setup_steps(agent, shown);
        let copies = setup_steps(agent, copied);
        let key = format!("{agent:?}").to_lowercase();
        let mut blocks: Vec<Block> = steps
            .iter()
            .zip(copies)
            .enumerate()
            .map(|(index, (step, copy))| Block {
                id: format!("{key}-step-{index}").into(),
                title: step_title(step, m),
                tall: step.text.contains('\n'),
                shown: step.text.clone(),
                copied: copy.text,
            })
            .collect();
        blocks.push(Block {
            id: format!("{key}-prompt").into(),
            title: m.prompt_title.into(),
            shown: agent_prompt(agent, shown),
            copied: agent_prompt(agent, copied),
            tall: true,
        });
        blocks.push(Block {
            id: format!("{key}-note").into(),
            title: (m.note_title)(instructions_file(agent)),
            shown: project_note(),
            copied: project_note(),
            tall: true,
        });
        blocks
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

    pub(super) fn render_agent_setup(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let has_executable = self.loaded.as_ref().is_some_and(|loaded| loaded.executable.is_some());
        let transport = self.effective_transport();
        // Over stdio nothing waits on the token, so the blocks copy at once.
        let ready = transport == Transport::Stdio || self.token.is_some();
        let agents = div().flex().child(choice_group(theme).children(AGENTS.map(|agent| {
            choice(
                theme,
                SharedString::from(format!("mcp-agent-{agent:?}")),
                agent_label(agent, m),
                self.agent == agent,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.agent = agent;
                cx.notify();
            }))
        })));
        let transports = has_executable.then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_size(px(12.)).text_color(theme.text2).child(m.transport))
                .child(div().flex().child(choice_group(theme).children(TRANSPORTS.map(|option| {
                    let label = if option == Transport::Stdio { m.transport_stdio } else { m.transport_http };
                    choice(theme, SharedString::from(format!("mcp-transport-{option:?}")), label, transport == option)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.transport = option;
                            cx.notify();
                        }))
                }))))
                .child(note(
                    theme,
                    if transport == Transport::Stdio { m.transport_stdio_note } else { m.transport_http_note },
                ))
        });
        let mut blocks = self.blocks(m);
        let project = blocks.pop().map(|block| self.render_block(theme, block, ready, cx));
        let prompt = blocks.pop().map(|block| self.render_block(theme, block, ready, cx));
        let steps: Vec<AnyElement> =
            blocks.into_iter().map(|block| self.render_block(theme, block, ready, cx)).collect();
        form_section(theme, m.add_to_agents)
            .child(agents)
            .children(transports)
            .children(steps)
            .children(prompt)
            .child(note(theme, if transport == Transport::Stdio { m.prompt_help_stdio } else { m.prompt_help }))
            .children(project)
    }
}
