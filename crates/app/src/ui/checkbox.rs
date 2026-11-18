//! A checkbox with what it does under its label: the whole row toggles it.

use std::rc::Rc;

use gpui::{
    App, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use super::icon;
use crate::theme::ActiveTheme;

type ChangeHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    label: SharedString,
    help: Option<(SharedString, Option<Hsla>)>,
    checked: bool,
    disabled: bool,
    on_change: Option<ChangeHandler>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Self {
        Self { id: id.into(), label: label.into(), help: None, checked, disabled: false, on_change: None }
    }

    /// What it does, under the label; in `tint` when given (a warning).
    pub fn help(mut self, help: impl Into<SharedString>, tint: Option<Hsla>) -> Self {
        self.help = Some((help.into(), tint));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called with the new state when the user toggles it.
    pub fn on_change(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let checked = self.checked;
        let mark = div()
            .mt(px(1.))
            .size(px(16.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .border_1()
            .map(|mark| {
                if checked {
                    mark.bg(theme.accent).border_color(theme.accent).child(icon("check", gpui::white(), 12.))
                } else {
                    mark.bg(theme.sel).border_color(theme.line)
                }
            });
        let on_change = self.on_change.filter(|_| !self.disabled);
        div()
            .id(self.id)
            .flex()
            .items_start()
            .gap(px(8.))
            .text_size(px(13.))
            .text_color(if self.disabled { theme.text3 } else { theme.text })
            .when(self.disabled, |row| row.opacity(0.7))
            .when_some(on_change, |row, on_change| {
                row.cursor_pointer().on_click(move |_, window, cx| on_change(&!checked, window, cx))
            })
            .child(mark)
            .child(div().flex().flex_col().min_w_0().child(self.label).children(self.help.map(|(help, tint)| {
                div().mt(px(2.)).text_size(px(12.)).text_color(tint.unwrap_or(theme.text3)).child(help)
            })))
    }
}
