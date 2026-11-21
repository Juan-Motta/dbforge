//! Drawing the explorer: its header, the filter, the tree's rows (only those
//! in view, so a schema with thousands of tables scrolls smoothly) and the
//! edge that resizes it.

use crate::ui::Ellipsis;
use gpui::{
    AnyElement, AppContext, Context, DragMoveEvent, Empty, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
    uniform_list,
};

use super::tree::{NodeKind, Tone, TreeRow};
use super::{Activate, CONTEXT, Collapse, Expand, Explorer, ExplorerEvent, SelectNext, SelectPrevious, width};
use crate::i18n::Translate;
use crate::preferences::{Preferences, keys};
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::title_bar::{keeps_window, moves_window};
use crate::ui::{any, engine_logo, glass, group, icon, popover, segment, spinner, tag_badge};

/// Every row is this tall, which lets the list draw only those in view.
const ROW: f32 = 28.;
/// How far each level is indented.
const INDENT: f32 = 12.;

/// Dragging the explorer's edge.
struct ResizeExplorer;

/// What is drawn under the pointer while dragging the edge: nothing.
struct NoPreview;

impl Render for NoPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

fn kind_icon(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Connection | NodeKind::Database => "database",
        NodeKind::Schema => "schema",
        NodeKind::Folder => "folder",
        NodeKind::Table => "table",
        NodeKind::View => "view",
    }
}

impl Explorer {
    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        match &self.rows[index] {
            TreeRow::Message { depth, text, tone, .. } => div()
                .h(px(ROW))
                .w_full()
                .pl(px(30. + *depth as f32 * INDENT))
                .pr(px(10.))
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(if *tone == Tone::Error { theme.err } else { theme.text3 })
                .child(div().ellipsis().child(text.clone()))
                .into_any_element(),
            TreeRow::Node { node, depth, expanded } => {
                let selected = self.selected.as_deref() == Some(node.id.as_str());
                let tag = (node.kind == NodeKind::Connection)
                    .then(|| self.connection(&node.id).and_then(|(_, tag)| tag.cloned()))
                    .flatten();
                let chevron =
                    div().id(("chevron", index)).size(px(14.)).flex_none().flex().items_center().justify_center().when(
                        node.expandable,
                        |chevron| {
                            let node = node.clone();
                            chevron.child(icon(if *expanded { "down" } else { "right" }, theme.text3, 12.)).on_click(
                                cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    window.focus(&this.focus);
                                    this.select(&node, cx);
                                    this.toggle(&node, cx);
                                }),
                            )
                        },
                    );
                let kind = if node.kind == NodeKind::Connection {
                    div()
                        .size(px(15.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(engine_logo(node.engine, 16.))
                } else {
                    div().flex_none().child(icon(kind_icon(node.kind), theme.text3, 15.))
                };
                let (clicked, menu) = (node.clone(), node.clone());
                div()
                    .id(("row", index))
                    .h(px(ROW))
                    .w_full()
                    .pl(px(6. + *depth as f32 * INDENT))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(9.))
                    .text_size(px(13.))
                    .text_color(if selected { theme.text } else { theme.text2 })
                    .when(node.kind == NodeKind::Connection, |row| row.font_weight(FontWeight::MEDIUM))
                    .when(selected, |row| row.bg(theme.accent.opacity(0.2)))
                    .when(!selected, |row| row.hover(|style| style.bg(theme.sel)))
                    .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                        window.focus(&this.focus);
                        this.select(&clicked, cx);
                        if event.click_count() == 2 {
                            this.activate(&clicked, window, cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.select(&menu, cx);
                            this.open_menu(&menu, event.position, window, cx);
                        }),
                    )
                    .child(chevron)
                    .child(kind)
                    .child(div().min_w_0().ellipsis().child(node.name.clone()))
                    .when(node.loading, |row| row.child(spinner(("spinner", index), theme.text3, 11.)))
                    .children(tag.map(|tag| tag_badge(&theme, &tag)))
                    .child(div().flex_1())
                    .children(node.meta.clone().map(|meta| {
                        div().flex_none().font_family(MONO).text_size(px(11.)).text_color(theme.text3).child(meta)
                    }))
                    .into_any_element()
            }
        }
    }

    fn render_header(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // The top row leaves the traffic lights their corner on macOS.
        moves_window(div()).h(px(52.)).flex().items_center().justify_end().px(px(12.)).child(keeps_window(div()).child(
            group(
                theme,
                vec![
                any(segment(theme, Some("plus"), "", false)
                    .id("explorer-new-connection")
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ExplorerEvent::NewConnection)))),
                any(segment(theme, Some("refresh"), "", false)
                    .id("explorer-refresh")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(None, cx)))),
                any(segment(theme, Some("collapse"), "", false)
                    .id("explorer-collapse")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.collapse_all(cx)))),
            ],
            ),
        ))
    }

    fn render_filter(&self, theme: &Theme) -> impl IntoElement {
        div().px(px(12.)).pb(px(10.)).child(
            div()
                .h(px(32.))
                .rounded(px(16.))
                .bg(theme.sel)
                .border_1()
                .border_color(theme.rim)
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(11.))
                .text_size(px(13.))
                .text_color(theme.text)
                .child(icon("search", theme.text3, 15.))
                .child(self.filter.clone()),
        )
    }

    /// The tree, or what to say when there is none to show.
    fn render_tree(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let m = cx.messages();
        let note = |text: String| {
            div().px(px(22.)).py(px(6.)).text_size(px(13.)).text_color(theme.text3).child(text).into_any_element()
        };
        if self.connections.is_empty() {
            return note(m.no_connections.to_owned());
        }
        if self.rows.is_empty() {
            return note((m.no_matches)(&self.query(cx)));
        }
        uniform_list(
            "explorer-rows",
            self.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                // A list item sizes to its content; the inset lane makes every row as wide as the list.
                range.map(|index| div().w_full().px(px(8.)).child(this.render_row(index, cx))).collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .flex_1()
        .pb(px(8.))
        .into_any_element()
    }

    /// The right edge, dragged to resize the explorer and double-clicked to reset it.
    fn render_edge(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("explorer-edge")
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(6.))
            .cursor_col_resize()
            .on_drag(ResizeExplorer, |_, _, _, cx| cx.new(|_| NoPreview))
            .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    this.reset_width(cx);
                }
            }))
    }
}

impl Render for Explorer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = cx.messages();
        let explorer_width = width(Preferences::global(cx).get(keys::EXPLORER_WIDTH));
        glass(&theme)
            .id("explorer")
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.step(false, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(true, cx)))
            .on_action(cx.listener(|this, _: &Expand, _, cx| this.expand(cx)))
            .on_action(cx.listener(|this, _: &Collapse, _, cx| this.collapse(cx)))
            .on_action(cx.listener(|this, _: &Activate, window, cx| {
                if let Some(node) = this.selected_row().and_then(|row| this.node_at(row)) {
                    this.activate(&node, window, cx);
                }
            }))
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<ResizeExplorer>, _, cx| {
                let left: f32 = event.bounds.left().into();
                let pointer: f32 = event.event.position.x.into();
                this.resize(pointer - left, cx);
            }))
            .relative()
            .w(px(explorer_width))
            .flex_none()
            .rounded(px(26.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(self.render_header(&theme, cx))
            .child(self.render_filter(&theme))
            .child(
                div()
                    .px(px(22.))
                    .h(px(24.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text3)
                    .child(m.connections.to_uppercase()),
            )
            .child(self.render_tree(&theme, cx))
            .child(self.render_edge(cx))
            .children(self.menu.as_ref().map(|open| popover(open.menu.clone(), open.position)))
    }
}
