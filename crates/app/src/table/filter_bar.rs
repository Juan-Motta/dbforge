//! The bar above a table's rows: a `WHERE` and an `ORDER BY` (or MongoDB's
//! filter and sort), applied with Enter, Escape putting back what applies;
//! and a button that clears both, always there so the fields keep their
//! width. While changes are pending it is locked: another filter would hide them.

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::RowFilter;
use gpui::{
    AppContext, Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};

use super::filter::{filter_labels, is_filtered};
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::field::{Tooltip, frame};
use crate::ui::{TextInput, TextInputEvent, icon};

/// The user applied this filter (Enter, or the clear button).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyFilter(pub RowFilter);

pub struct FilterBar {
    engine: Engine,
    /// What the rows are shown with.
    applied: RowFilter,
    filter: Entity<TextInput>,
    order: Entity<TextInput>,
    locked: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ApplyFilter> for FilterBar {}

impl FilterBar {
    pub fn new(engine: Engine, applied: RowFilter, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let labels = filter_labels(engine, &cx.messages().table);
        let field = |hint: &'static str, text: &str, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut input = TextInput::new(hint, cx);
                input.set_text(text, cx);
                input
            })
        };
        let filter = field(labels.filter_hint, &applied.filter, cx);
        let order = field(labels.order_hint, &applied.order, cx);
        let subscriptions =
            vec![cx.subscribe_in(&filter, window, Self::on_field), cx.subscribe_in(&order, window, Self::on_field)];
        Self { engine, applied, filter, order, locked: false, _subscriptions: subscriptions }
    }

    fn on_field(&mut self, _: &Entity<TextInput>, event: &TextInputEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Submitted => {
                let draft = RowFilter {
                    filter: self.filter.read(cx).text().to_owned(),
                    order: self.order.read(cx).text().to_owned(),
                };
                cx.emit(ApplyFilter(draft));
            }
            TextInputEvent::Cancelled => self.show(self.applied.clone(), cx),
            _ => {}
        }
    }

    /// The filter the rows are shown with now; a header click changes the order from outside.
    pub fn set_applied(&mut self, applied: RowFilter, cx: &mut Context<Self>) {
        self.show(applied.clone(), cx);
        self.applied = applied;
    }

    fn show(&mut self, filter: RowFilter, cx: &mut Context<Self>) {
        self.filter.update(cx, |input, cx| input.set_text(&filter.filter, cx));
        self.order.update(cx, |input, cx| input.set_text(&filter.order, cx));
        cx.notify();
    }

    /// Locked while changes are pending.
    pub fn set_locked(&mut self, locked: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked != locked {
            self.locked = locked;
            self.filter.update(cx, |input, cx| input.set_disabled(locked, window, cx));
            self.order.update(cx, |input, cx| input.set_disabled(locked, window, cx));
            cx.notify();
        }
    }

    fn field(&self, label: &'static str, input: &Entity<TextInput>, window: &Window, cx: &Context<Self>) -> gpui::Div {
        let theme = cx.theme();
        let field = input.read(cx);
        frame(theme, field.is_focused(window), false, field.is_disabled())
            .font_family(MONO)
            .text_size(px(12.5))
            .text_color(theme.text)
            .child(
                div()
                    .flex_none()
                    .font_family(".SystemUIFont")
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text3)
                    .child(label),
            )
            .child(div().flex_1().min_w_0().child(input.clone()))
    }
}

impl Render for FilterBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().table;
        let labels = filter_labels(self.engine, m);
        let clearable = !self.locked && is_filtered(&self.applied);
        div()
            .id("filter-bar")
            .h(px(44.))
            .flex_none()
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(theme.line)
            .when(self.locked, |bar| bar.tooltip(Tooltip::text(m.filter_locked)))
            .child(self.field(labels.filter, &self.filter, window, cx).flex_1().min_w_0())
            .child(self.field(labels.order, &self.order, window, cx).flex_shrink().w(px(280.)).min_w(px(140.)))
            .child(
                div()
                    .id("clear-filter")
                    .size(px(28.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(14.))
                    .tooltip(Tooltip::text(m.clear_filter))
                    .when(!clearable, |button| button.opacity(0.4))
                    .when(clearable, |button| {
                        button.cursor_pointer().hover(|button| button.bg(theme.sel)).on_click(cx.listener(
                            |_, _, _, cx| {
                                cx.emit(ApplyFilter(RowFilter::default()));
                            },
                        ))
                    })
                    .child(icon("close", theme.text2, 13.)),
            )
    }
}
