//! A table's or view's tab: its actions on the left of a bar and its views
//! on the right, and the view shown under them. Data (the rows) is the one
//! view for now; the structure's sections and the diagram join it with
//! their feature.

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::{RowFilter, TableRef};
use gpui::{
    AnyElement, AnyView, AppContext, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, px,
};

use super::view::{TableEvent, TableView};
use crate::i18n::Translate;
use crate::theme::ActiveTheme;
use crate::ui::{any, card, group, segment};

/// What a table's tab asks of the workspace.
#[derive(Debug, Clone)]
pub enum TablePageEvent {
    /// Open this table of the same connection, filtered (a foreign key followed).
    OpenTable { table: TableRef, filter: RowFilter },
    /// Show this dialog over the window, or close the one shown.
    Dialog(Option<AnyView>),
}

pub struct TablePage {
    data: Entity<TableView>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TablePageEvent> for TablePage {}

impl TablePage {
    pub fn new(
        connection_id: String,
        engine: Engine,
        table: TableRef,
        filter: RowFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let data = cx.new(|cx| TableView::new(connection_id, engine, table, filter, window, cx));
        let subscriptions = vec![
            // The bar's actions follow the rows: what is selected, what is pending.
            cx.observe(&data, |_, _, cx| cx.notify()),
            cx.subscribe(&data, |_, _, event: &TableEvent, cx| {
                cx.emit(match event.clone() {
                    TableEvent::OpenTable { table, filter } => TablePageEvent::OpenTable { table, filter },
                    TableEvent::Dialog(dialog) => TablePageEvent::Dialog(dialog),
                });
            }),
        ];
        Self { data, _subscriptions: subscriptions }
    }

    pub fn data(&self) -> &Entity<TableView> {
        &self.data
    }

    /// The rows take the keyboard.
    pub fn focus(&self, window: &mut Window, cx: &gpui::App) {
        self.data.read(cx).focus(window, cx);
    }

    fn bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().table;
        let actions = self.data.update(cx, |data, cx| data.actions(cx));
        div()
            .h(px(34.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .children(actions)
            .child(div().flex_1())
            .child(group(&theme, vec![any(segment(&theme, None, m.data, true))]))
            .into_any_element()
    }
}

impl Render for TablePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(self.bar(cx))
            .child(card(&theme).child(self.data.clone()))
    }
}
