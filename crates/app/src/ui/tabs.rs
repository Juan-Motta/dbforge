//! A row of section tabs, one row high whatever they hold: the selected one
//! underlined, and a dot that keeps its room on each, lit when its setting
//! is on (SSL, an SSH tunnel), so turning one on never widens the tab.

use std::rc::Rc;

use gpui::{
    App, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use super::field::Tooltip;
use crate::theme::ActiveTheme;

/// The tabs' height.
pub const TABS_HEIGHT: f32 = 36.;

/// One tab: its label, and whether its setting is on (`title` says what).
#[derive(Clone)]
pub struct Tab {
    pub label: SharedString,
    pub active: bool,
    pub title: Option<SharedString>,
}

type SelectHandler = Rc<dyn Fn(&usize, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct SectionTabs {
    id: ElementId,
    tabs: Vec<Tab>,
    selected: usize,
    on_select: SelectHandler,
}

impl SectionTabs {
    pub fn new(
        id: impl Into<ElementId>,
        tabs: Vec<Tab>,
        selected: usize,
        on_select: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), tabs, selected, on_select: Rc::new(on_select) }
    }
}

impl RenderOnce for SectionTabs {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        div().id(self.id).h(px(TABS_HEIGHT)).flex().gap(px(4.)).children(self.tabs.into_iter().enumerate().map(
            move |(index, tab)| {
                let selected = index == self.selected;
                let on_select = self.on_select.clone();
                div()
                    .id(index)
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .border_b_2()
                    .border_color(if selected { theme.accent } else { gpui::transparent_black() })
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if selected { theme.text } else { theme.text2 })
                    .hover(|style| style.text_color(theme.text))
                    .cursor_pointer()
                    .on_click(move |_, window, cx| on_select(&index, window, cx))
                    .when_some(tab.title.filter(|_| tab.active), |tab, title| tab.tooltip(Tooltip::text(title)))
                    .child(tab.label)
                    .child(div().size(px(6.)).rounded(px(3.)).bg(if tab.active {
                        theme.ok
                    } else {
                        gpui::transparent_black()
                    }))
            },
        ))
    }
}
