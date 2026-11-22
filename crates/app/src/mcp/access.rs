//! Settings › Agent access: each saved connection with what agents may do
//! with it (hidden, read only, or read and write), shared by every agent.
//! Changing it here saves the connection's own access, the same one its
//! settings edit. A section of its own, so Agents (MCP) fits without
//! scrolling.

use dbforge_core::connections::commands::set_agent_access;
use dbforge_core::connections::model::AgentAccess;
use dbforge_core::connections::model::Connection;
use dbforge_core::connections::tags::Tag;
use gpui::{
    AnyElement, Context, EventEmitter, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use super::section::{McpSectionEvent, tile};
use crate::backend::Backend;
use crate::i18n::{McpMessages, Translate};
use crate::tags::tag_of;
use crate::theme::{ActiveTheme, Theme};
use crate::ui::modal::note;
use crate::ui::{Ellipsis, any, engine_logo, segment_face, segmented, tag_badge};

/// The accesses, in the order the choice shows them.
const ACCESSES: [AgentAccess; 3] = [AgentAccess::None, AgentAccess::Read, AgentAccess::Write];

fn access_label(access: AgentAccess, m: &McpMessages) -> &'static str {
    match access {
        AgentAccess::None => m.access_hidden,
        AgentAccess::Read => m.access_read,
        AgentAccess::Write => m.access_write,
    }
}

/// The Agent access section: the saved connections, their tags, and the last
/// save that failed.
pub struct AccessSection {
    connections: Vec<Connection>,
    tags: Vec<Tag>,
    error: Option<String>,
}

impl EventEmitter<McpSectionEvent> for AccessSection {}

impl AccessSection {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut section = Self { connections: Vec::new(), tags: Vec::new(), error: None };
        section.load_connections(cx);
        section
    }

    /// Reads the saved connections and their tags, off the UI thread.
    fn load_connections(&mut self, cx: &mut Context<Self>) {
        let read = Backend::global(cx).spawn_blocking(cx, |core| (core.connections().list(), core.tags().list()));
        cx.spawn(async move |this, cx| {
            let (connections, tags) = read.await;
            let _ = this.update(cx, |this, cx| {
                this.connections = connections;
                this.tags = tags;
                cx.notify();
            });
        })
        .detach();
    }

    /// Saves what agents may do with connection `id`. Shown at once, and put
    /// back with the error when the core couldn't save it.
    fn set_access(&mut self, id: String, access: AgentAccess, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.iter_mut().find(|connection| connection.id == id) else { return };
        if connection.agent_access == access {
            return;
        }
        let before = std::mem::replace(&mut connection.agent_access, access);
        self.error = None;
        cx.notify();
        let saving = Backend::global(cx).spawn_blocking(cx, {
            let id = id.clone();
            move |core| set_agent_access(core.connections(), &id, access).map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let saved = saving.await;
            let _ = this.update(cx, |this, cx| {
                let shown = this.connections.iter_mut().find(|connection| connection.id == id);
                match saved {
                    Ok(saved) => {
                        if let Some(shown) = shown {
                            *shown = saved;
                        }
                        cx.emit(McpSectionEvent::ConnectionsChanged);
                    }
                    Err(error) => {
                        // Only when nothing newer was asked meanwhile.
                        if let Some(shown) = shown.filter(|shown| shown.agent_access == access) {
                            shown.agent_access = before;
                        }
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_access(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let rows: Vec<AnyElement> = self
            .connections
            .iter()
            .enumerate()
            .map(|(index, connection)| {
                let tag = tag_of(&self.tags, connection.environment.as_deref());
                let segments = ACCESSES
                    .iter()
                    .map(|&access| {
                        let id = connection.id.clone();
                        any(segment_face(theme, None, access_label(access, m), connection.agent_access == access)
                            .id(SharedString::from(format!("mcp-access-{}-{access:?}", connection.id)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.set_access(id.clone(), access, cx))))
                    })
                    .collect();
                let first = index == 0;
                div()
                    .min_h(px(48.))
                    .px(px(14.))
                    .py(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .when(!first, |row| row.border_t_1().border_color(theme.line))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(engine_logo(connection.engine, 16.))
                            .child(div().min_w_0().ellipsis().text_size(px(13.)).child(connection.name.clone()))
                            .children(tag.map(|tag| tag_badge(theme, tag))),
                    )
                    .child(div().flex_none().child(segmented(
                        theme,
                        SharedString::from(format!("mcp-access-{}", connection.id)),
                        ACCESSES.iter().position(|access| *access == connection.agent_access),
                        segments,
                    )))
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        tile(theme)
            .overflow_hidden()
            .flex()
            .flex_col()
            .children(rows)
            .when(empty, |tile| {
                tile.child(
                    div().px(px(14.)).py(px(12.)).text_size(px(12.5)).text_color(theme.text3).child(m.no_connections),
                )
            })
            .child(
                div().px(px(14.)).py(px(12.)).border_t_1().border_color(theme.line).child(note(theme, m.access_note)),
            )
    }
}

impl Render for AccessSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        div()
            .pt(px(6.))
            .pb(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .children(self.error.clone().map(|error| div().text_size(px(12.5)).text_color(theme.err).child(error)))
            .child(self.render_access(&theme, cx))
    }
}
