//! Settings › Agents › what agents can reach: each saved connection with
//! its agent access (hidden, read only, or read and write), shared by every
//! agent. Changing it here saves the connection's own access, the same one
//! its settings edit.

use dbforge_core::connections::commands::set_agent_access;
use dbforge_core::connections::model::AgentAccess;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};

use super::section::{McpSection, McpSectionEvent, tile};
use crate::backend::Backend;
use crate::i18n::{McpMessages, Translate};
use crate::tags::tag_of;
use crate::theme::Theme;
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

impl McpSection {
    /// Reads the saved connections and their tags, off the UI thread.
    pub(super) fn load_connections(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn set_access(&mut self, id: String, access: AgentAccess, cx: &mut Context<Self>) {
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

    pub(super) fn render_access(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let rows: Vec<AnyElement> = self
            .connections
            .iter()
            .map(|connection| {
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
                div()
                    .min_h(px(48.))
                    .px(px(14.))
                    .py(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .border_t_1()
                    .border_color(theme.line)
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
            .child(
                div()
                    .px(px(14.))
                    .pt(px(12.))
                    .pb(px(10.))
                    .flex()
                    .items_baseline()
                    .gap(px(10.))
                    .child(div().flex_none().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child(m.reach_title))
                    .child(div().min_w_0().ellipsis().text_size(px(12.)).text_color(theme.text3).child(m.reach_hint)),
            )
            .children(rows)
            .when(empty, |tile| {
                tile.child(
                    div()
                        .px(px(14.))
                        .py(px(12.))
                        .border_t_1()
                        .border_color(theme.line)
                        .text_size(px(12.5))
                        .text_color(theme.text3)
                        .child(m.no_connections),
                )
            })
            .child(
                div().px(px(14.)).py(px(12.)).border_t_1().border_color(theme.line).child(note(theme, m.access_note)),
            )
    }
}
