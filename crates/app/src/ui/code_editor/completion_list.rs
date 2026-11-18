//! The suggestions under the caret: a short list over everything else, the
//! picked one marked, each with what it is and a detail (a column's type).
//! ↑ ↓ pick, Enter or Tab insert, Escape closes; a click inserts.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, anchored,
    deferred, div, point, prelude::FluentBuilder, px,
};

use super::CodeEditor;
use crate::sql::completion::ItemKind;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::glass;

/// Rows shown at once; the list scrolls to keep the picked one in them.
const ROWS: usize = 8;
const ROW_HEIGHT: f32 = 24.;

impl CodeEditor {
    pub(super) fn completion_list(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.completion.as_ref()?;
        let caret = self.layout.caret?;
        let theme = *cx.theme();
        let items = &open.completion.items;
        let first = open.selected.saturating_sub(ROWS - 1);
        let rows = items.iter().enumerate().skip(first).take(ROWS).map(|(index, item)| {
            let picked = index == open.selected;
            let (mark, tint) = match item.kind {
                ItemKind::Keyword => ("K", theme.kw),
                ItemKind::Schema => ("S", theme.num),
                ItemKind::Table => ("T", theme.func),
                ItemKind::View => ("V", theme.func),
                ItemKind::Column => ("C", theme.str),
            };
            div()
                .id(index)
                .h(px(ROW_HEIGHT))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(6.))
                .when(picked, |row| row.bg(theme.accent.opacity(0.22)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.accept(index, cx);
                    }),
                )
                .child(
                    div()
                        .w(px(16.))
                        .flex_none()
                        .text_size(px(10.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(tint)
                        .child(mark),
                )
                .child(div().flex_1().min_w_0().truncate().text_color(theme.text).child(item.label.clone()))
                .child(div().flex_none().text_size(px(11.)).text_color(theme.text3).child(item.detail.clone()))
        });
        let list = glass(&theme)
            .id("completions")
            .occlude()
            .w(px(300.))
            .p(px(4.))
            .rounded(px(10.))
            .font_family(MONO)
            .text_size(px(12.5))
            .children(rows);
        let position = point(caret.left() - px(12.), caret.bottom() + px(4.));
        Some(
            deferred(anchored().position(position).snap_to_window_with_margin(px(8.)).child(list))
                .with_priority(1)
                .into_any_element(),
        )
    }
}
