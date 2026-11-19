//! A framed list of choices, each a row with its mark (a radio or a
//! checkbox), an optional icon, a title and what it does under it: the
//! rows of an export's scope, a dump's content and options, a backup's
//! parts. The whole row picks it.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use super::icon;
use crate::theme::{ActiveTheme, Theme};

/// How a row's mark looks: one of several, or on and off on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Radio,
    Check,
}

type SelectHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct OptionItem {
    id: ElementId,
    mark: Mark,
    icon: Option<&'static str>,
    title: SharedString,
    description: Option<SharedString>,
    checked: bool,
    disabled: bool,
    on_select: Option<SelectHandler>,
}

impl OptionItem {
    pub fn new(id: impl Into<ElementId>, mark: Mark, title: impl Into<SharedString>, checked: bool) -> Self {
        Self {
            id: id.into(),
            mark,
            icon: None,
            title: title.into(),
            description: None,
            checked,
            disabled: false,
            on_select: None,
        }
    }

    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called when the row is pressed: a radio picks itself, a checkbox flips.
    pub fn on_select(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
}

fn mark(theme: &Theme, kind: Mark, checked: bool) -> Div {
    let frame = div().mt(px(1.)).size(px(16.)).flex_none().flex().items_center().justify_center().border_1();
    match (kind, checked) {
        (Mark::Radio, true) => frame
            .rounded(px(8.))
            .border_color(theme.accent)
            .bg(theme.accent)
            .child(div().size(px(6.)).rounded(px(3.)).bg(gpui::white())),
        (Mark::Radio, false) => frame.rounded(px(8.)).border_color(theme.line).bg(theme.sel),
        (Mark::Check, true) => {
            frame.rounded(px(5.)).border_color(theme.accent).bg(theme.accent).child(icon("check", gpui::white(), 12.))
        }
        (Mark::Check, false) => frame.rounded(px(5.)).border_color(theme.line).bg(theme.sel),
    }
}

impl RenderOnce for OptionItem {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let on_select = self.on_select.filter(|_| !self.disabled);
        div()
            .id(self.id)
            .w_full()
            .flex()
            .items_start()
            .gap(px(10.))
            .px(px(12.))
            .py(px(10.))
            .when(self.disabled, |row| row.opacity(0.55))
            .when_some(on_select, |row, on_select| {
                row.cursor_pointer()
                    .hover(|row| row.bg(theme.sel))
                    .on_click(move |event, window, cx| on_select(event, window, cx))
            })
            .child(mark(&theme, self.mark, self.checked))
            .children(self.icon.map(|name| div().mt(px(1.)).child(icon(name, theme.text3, 15.))))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(self.title),
                    )
                    .children(
                        self.description.map(|text| div().text_size(px(12.)).text_color(theme.text3).child(text)),
                    ),
            )
    }
}

/// The rows in one frame, a line between each.
pub fn option_list(theme: &Theme, items: Vec<AnyElement>) -> Div {
    let count = items.len();
    div().w_full().flex().flex_col().rounded(px(10.)).border_1().border_color(theme.line).overflow_hidden().children(
        items.into_iter().enumerate().map(|(index, item)| {
            div().when(index + 1 < count, |row| row.border_b_1().border_color(theme.line)).child(item)
        }),
    )
}
