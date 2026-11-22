//! The tab strip: each open tab with its icon, title and close button, the
//! shown one raised by a thumb that slides to the next one shown, and + for a new query on the selected connection.
//! A middle click closes a tab, as in browsers. A console whose text differs
//! from its saved query shows a dot in place of its close button, which
//! comes back on hover, so the tab never changes width; one with a
//! transaction open shows an amber dot there instead, which wins over the
//! other, its tooltip saying closing asks first.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement,
    Styled, div, prelude::FluentBuilder, px,
};

use super::Workspace;
use crate::i18n::Translate;
use crate::theme::ActiveTheme;
use crate::ui::field::Tooltip;
use crate::ui::segmented::sliding;
use crate::ui::{any, dot, enabled, glass, icon, segment, segment_face};

/// The strip's height.
pub const STRIP_HEIGHT: f32 = 40.;

impl Workspace {
    pub(super) fn strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = cx.messages();
        let active = self.tabs.active_id().map(str::to_owned);
        let shown = self.tabs.tabs().iter().position(|tab| active.as_deref() == Some(tab.id.as_str()));
        let mut tabs: Vec<AnyElement> = self
            .tabs
            .tabs()
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let shown = active.as_deref() == Some(tab.id.as_str());
                let console = self.console(&tab.id).map(|console| console.read(cx));
                let edited = console.is_some_and(|console| console.is_edited());
                let pending = console.is_some_and(|console| console.transaction().is_some());
                let marker = if pending { Some(theme.txn) } else { edited.then_some(theme.text2) };
                let tooltip = if pending {
                    format!("{} · {}", m.transactions.tab_dot, (m.query.close_tab)(&tab.title))
                } else {
                    (m.query.close_tab)(&tab.title)
                };
                let (select, middle, close) = (tab.id.clone(), tab.id.clone(), tab.id.clone());
                let kind_icon = match tab.kind {
                    super::tabs::TabKind::Query { .. } => "code",
                    super::tabs::TabKind::Table { .. } => "table",
                    super::tabs::TabKind::Diagram { .. } => "diagram",
                    super::tabs::TabKind::Keyspace { .. } => "key",
                };
                any(segment_face(&theme, Some(kind_icon), &tab.title, shown)
                    .id(("tab", index))
                    .flex_none()
                    .pr(px(6.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(&select, window, cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close_tab(&middle, window, cx)),
                    )
                    .child(
                        div()
                            .id(("close-tab", index))
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .relative()
                            .group("close-tab")
                            .rounded(px(9.))
                            .hover(|button| button.bg(theme.sel))
                            .tooltip(Tooltip::text(tooltip))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(&close, window, cx);
                            }))
                            .map(|button| {
                                let Some(tint) = marker else {
                                    return button.child(icon("close", theme.text3, 11.));
                                };
                                button.child(dot(tint).group_hover("close-tab", |dot| dot.opacity(0.))).child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .opacity(0.)
                                        .group_hover("close-tab", |close| close.opacity(1.))
                                        .child(icon("close", theme.text3, 11.)),
                                )
                            }),
                    ))
            })
            .collect();
        let can_query = self.can_query();
        // With no tab open, + says what it does.
        let label = if self.tabs.tabs().is_empty() { m.new_query } else { "" };
        let new_query = enabled(segment(&theme, Some("plus"), label, false), can_query)
            .id("new-query")
            .flex_none()
            .tooltip(Tooltip::text(m.new_query))
            .when(can_query, |button| {
                button.on_click(cx.listener(|this, _, window, cx| this.new_query_on_target(window, cx)))
            });
        // After the tabs, so the thumb's choices are the tabs alone.
        tabs.push(any(new_query));
        div()
            .h(px(STRIP_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .child(
                glass(&theme)
                    .id("tab-strip")
                    .h(px(34.))
                    .max_w_full()
                    .rounded(px(17.))
                    .p(px(3.))
                    .overflow_x_scroll()
                    .child(sliding(&theme, "tab-strip-thumb", shown, tabs)),
            )
            .into_any_element()
    }
}
