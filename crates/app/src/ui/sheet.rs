//! A read-only table drawn with plain elements, for short lists (a table's
//! columns, a plan's nodes): its header stays on top while its rows scroll,
//! and the whole scrolls sideways when the space is narrower than it needs.

use gpui::{
    Div, ElementId, Font, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels, SharedString, Stateful,
    StatefulInteractiveElement, Styled, TextRun, Window, div, font, px,
};

use crate::theme::MONO;

/// A header over rows that scroll under it, `min_width` wide at least.
pub fn sheet(
    id: impl Into<ElementId>,
    min_width: Pixels,
    header: impl IntoElement,
    rows: impl IntoElement,
) -> Stateful<Div> {
    // Each scrolls its own axis only: a vertical wheel never moves the table sideways.
    let mut body = div().id("rows").flex_1().min_h_0().overflow_y_scroll().child(rows);
    body.style().restrict_scroll_to_axis = Some(true);
    let mut sheet = div().id(id).size_full().overflow_x_scroll();
    sheet.style().restrict_scroll_to_axis = Some(true);
    sheet.child(div().min_w(min_width).w_full().h_full().flex().flex_col().child(header).child(body))
}

/// How wide `text` is drawn on one line at `size`, in the monospace font
/// or the window's own, for sizing a column to its widest value.
pub fn text_width(text: &str, mono: bool, weight: FontWeight, size: f32, window: &Window) -> f32 {
    if text.is_empty() {
        return 0.;
    }
    let family: SharedString = if mono { MONO.into() } else { window.text_style().font_family.clone() };
    let run = TextRun {
        len: text.len(),
        font: Font { weight, ..font(family) },
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(SharedString::from(one_line(text)), px(size), &[run], None);
    f32::from(line.width)
}

/// `text` on one line: line breaks and tabs become spaces, as a cell shows it.
pub fn one_line(text: &str) -> String {
    text.chars().map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c }).collect()
}
