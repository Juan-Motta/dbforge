//! The frame of a tall dialog (an export, an import, a dump): its title on
//! top, a body that scrolls when the window is shorter than it, and its
//! buttons in a footer that stays. The dialog sits in the shell's dialog
//! layer, which keeps it inside the window.

use crate::ui::Ellipsis;
use gpui::{Div, FontWeight, InteractiveElement, ParentElement, Stateful, StatefulInteractiveElement, Styled, div, px};

use super::glass;
use crate::theme::Theme;

/// The dialog's card, `width` wide at most; it never grows past the window.
pub fn modal(theme: &Theme, width: f32, title: impl Into<gpui::SharedString>) -> Div {
    glass(theme)
        .bg(theme.dialog)
        .w(px(width))
        .max_w_full()
        .max_h_full()
        .rounded(px(22.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .px(px(22.))
                .pt(px(20.))
                .pb(px(4.))
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .ellipsis()
                .child(title.into()),
        )
}

/// The part between the title and the buttons, scrolling when it must.
pub fn modal_body(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(22.))
        .pt(px(14.))
        .pb(px(8.))
        .flex()
        .flex_col()
        .gap(px(16.))
}

/// `path` cut to its last `max` characters behind an ellipsis, so a long
/// one shows its end (the file's name) rather than its start.
pub fn path_tail(path: &str, max: usize) -> String {
    let count = path.chars().count();
    if count <= max {
        return path.to_owned();
    }
    let tail: String = path.chars().skip(count - max.saturating_sub(1)).collect();
    format!("…{tail}")
}

/// The box saying how a run goes, under the body and over the buttons:
/// always in sight, however the body is scrolled.
pub fn modal_status(status: impl gpui::IntoElement) -> Div {
    div().flex_none().px(px(22.)).pt(px(6.)).pb(px(12.)).child(status)
}

/// The buttons, on the right.
pub fn modal_footer(theme: &Theme) -> Div {
    div()
        .flex_none()
        .px(px(22.))
        .py(px(14.))
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .border_t_1()
        .border_color(theme.line)
}

/// A titled part of a form.
pub fn form_section(theme: &Theme, title: &str) -> Div {
    div().flex().flex_col().gap(px(8.)).child(
        div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(theme.text2).child(title.to_owned()),
    )
}

/// A quiet note under a form's part.
pub fn note(theme: &Theme, text: impl Into<gpui::SharedString>) -> Div {
    div().text_size(px(12.)).line_height(px(17.)).text_color(theme.text3).child(text.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_end_of_a_long_path() {
        assert_eq!(path_tail("/a/b.csv", 20), "/a/b.csv");
        assert_eq!(path_tail("/very/long/folder/shop.sql", 10), "…/shop.sql");
    }
}
