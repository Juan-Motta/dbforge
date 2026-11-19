//! A row of mutually exclusive choices in one frame, the one picked raised:
//! the interface language, the agent to set up and how it reaches DBForge.
//! Each choice is pressed to pick it; its owner adds the click.

use gpui::{
    Div, ElementId, FontWeight, InteractiveElement, ParentElement, SharedString, Stateful, Styled, div,
    prelude::FluentBuilder, px,
};

use crate::theme::Theme;

/// The frame the choices sit in, as wide as they are in a row (in a column, put it in a row).
pub fn choice_group(theme: &Theme) -> Div {
    div().flex().flex_wrap().flex_none().p(px(2.)).rounded(px(9.)).border_1().border_color(theme.line).bg(theme.sel)
}

/// One choice, raised when `checked`.
pub fn choice(theme: &Theme, id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.))
        .px(px(12.))
        .flex()
        .items_center()
        .rounded(px(7.))
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if checked { theme.text } else { theme.text2 })
        .when(checked, |choice| choice.bg(theme.accent.opacity(0.2)))
        .cursor_pointer()
        .child(label.into())
}
