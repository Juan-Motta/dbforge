//! A dialog's and a form's buttons: the accent's primary one (in
//! [`super::primary_button`]), a plain one, and a quiet one with no frame.
//! A disabled button is dimmed and takes no clicks; its owner leaves out the
//! click handler.

use gpui::{Div, FontWeight, ParentElement, Styled, div, prelude::FluentBuilder, px};

use super::{icon, shadow};
use crate::theme::Theme;

/// A framed glass button: Test connection, Browse…, Cancel in a confirmation.
pub fn button(theme: &Theme, icon_name: Option<&str>, label: &str) -> Div {
    div()
        .h(px(32.))
        .px(px(14.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(7.))
        .rounded(px(16.))
        .bg(theme.glass_hi)
        .border_1()
        .border_color(theme.rim)
        .shadow(shadow(8., 2., 0x0000001A))
        .text_color(theme.text)
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .when_some(icon_name, |button, name| button.child(icon(name, theme.text2, 14.)))
        .child(label.to_owned())
}

/// A button without a frame: Cancel beside a dialog's main action.
pub fn ghost_button(theme: &Theme, label: &str) -> Div {
    div()
        .h(px(32.))
        .px(px(14.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(16.))
        .text_color(theme.text2)
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .child(label.to_owned())
}

/// Dims a button that can't be pressed now; pressable ones show the pointer.
pub fn enabled(button: Div, enabled: bool) -> Div {
    if enabled { button.cursor_pointer() } else { button.opacity(0.45) }
}
