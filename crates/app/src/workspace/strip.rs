//! The tab strip: each open tab with its icon, title and close button, the
//! shown one raised, and + for a new query on the selected connection.
//! A middle click closes a tab, as in browsers.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement,
    Styled, div, prelude::FluentBuilder, px,
};

use super::Workspace;
use crate::i18n::Translate;
use crate::theme::ActiveTheme;
use crate::ui::field::Tooltip;
use crate::ui::{any, enabled, glass, icon, segment};

/// The strip's height.
pub const STRIP_HEIGHT: f32 = 40.;

impl Workspace {
    pub(super) fn strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = cx.messages();
        let active = self.tabs.active_id().map(str::to_owned);
        let tabs = self.tabs.tabs().iter().enumerate().map(|(index, tab)| {
            let shown = active.as_deref() == Some(tab.id.as_str());
            let (select, middle, close) = (tab.id.clone(), tab.id.clone(), tab.id.clone());
            let kind_icon = match tab.kind {
                super::tabs::TabKind::Query { .. } => "code",
                super::tabs::TabKind::Table { .. } => "table",
            };
            any(segment(&theme, Some(kind_icon), &tab.title, shown)
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
                        .rounded(px(9.))
                        .hover(|button| button.bg(theme.sel))
                        .tooltip(Tooltip::text((m.query.close_tab)(&tab.title)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_tab(&close, window, cx);
                        }))
                        .child(icon("close", theme.text3, 11.)),
                ))
        });
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
                    .flex()
                    .gap(px(2.))
                    .overflow_x_scroll()
                    .children(tabs)
                    .child(new_query),
            )
            .into_any_element()
    }
}
