//! What every form field is made of: its label (with an ⓘ that explains it
//! on hover), the framed box its control sits in, and the help line under
//! it. Each keeps its height whatever it shows, so a form never moves.

use gpui::{
    AnyView, App, AppContext, Context, Div, ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use super::text_input::TextInput;
use super::{glass, icon};
use crate::theme::{ActiveTheme, Theme};

/// A control's height: text fields, dropdowns, file fields.
pub const CONTROL_HEIGHT: f32 = 32.;

/// A hover note: what a field's ⓘ explains.
pub struct Tooltip {
    text: SharedString,
}

impl Tooltip {
    /// A builder for an element's `.tooltip(..)`.
    pub fn text(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let text = text.into();
        move |_, cx| cx.new(|_| Tooltip { text: text.clone() }).into()
    }
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        glass(theme)
            .bg(theme.card)
            .rounded(px(10.))
            .max_w(px(300.))
            .px(px(10.))
            .py(px(7.))
            .text_size(px(12.))
            .line_height(px(16.))
            .font_weight(FontWeight::NORMAL)
            .text_color(theme.text2)
            .child(self.text.clone())
    }
}

/// A field's label, with an ⓘ that shows `info` on hover. `id` must be
/// unique in the form.
pub fn label(theme: &Theme, id: impl Into<ElementId>, text: impl Into<SharedString>, info: Option<&str>) -> Div {
    div()
        .h(px(16.))
        .mb(px(6.))
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text2)
        .child(text.into())
        .children(info.map(|info| {
            div()
                .id(id)
                .flex()
                .items_center()
                .cursor(gpui::CursorStyle::Arrow)
                .tooltip(Tooltip::text(info.to_owned()))
                .child(icon("info", theme.text3, 12.))
        }))
}

/// The help under a field: one line, in a quiet colour (or `tint`).
pub fn help(theme: &Theme, text: impl Into<SharedString>, tint: Option<Hsla>) -> Div {
    div()
        .mt(px(5.))
        .min_h(px(16.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(tint.unwrap_or(theme.text3))
        .child(text.into())
}

/// The framed box a control sits in, `CONTROL_HEIGHT` high: its border
/// shows focus, or a value that can't be used.
pub fn frame(theme: &Theme, focused: bool, invalid: bool, disabled: bool) -> Div {
    let border = if invalid {
        theme.err
    } else if focused {
        theme.accent
    } else {
        theme.line
    };
    div()
        .h(px(CONTROL_HEIGHT))
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .rounded(px(10.))
        .border_1()
        .border_color(border)
        .bg(theme.sel)
        .text_size(px(13.))
        .when(disabled, |frame| frame.opacity(0.6))
}

/// A text field in its frame, in the monospace face when `mono`.
pub fn text_box(input: &Entity<TextInput>, invalid: bool, mono: bool, window: &Window, cx: &App) -> Div {
    let theme = cx.theme();
    let field = input.read(cx);
    frame(theme, field.is_focused(window), invalid, field.is_disabled())
        .text_color(theme.text)
        .when(mono, |frame| frame.font_family(crate::theme::MONO).text_size(px(12.5)))
        .child(input.clone())
}

/// A label, a control and its help, stacked: one field of a form.
pub fn field(label: Div, control: impl IntoElement, help: Option<Div>) -> Div {
    div().flex().flex_col().min_w_0().child(label).child(control).children(help)
}
