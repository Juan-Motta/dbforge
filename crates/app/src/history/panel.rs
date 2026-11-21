//! The history beside a console: the runs, newest first and grouped by day,
//! searched by their SQL, narrowed to errors, to this connection or not. A
//! click puts a run's SQL in the console; ⌘-click (Ctrl-click) opens it in a
//! new tab. While a new list loads the previous one stays, so typing in the
//! search never flickers.

use crate::ui::Ellipsis;
use chrono::Local;
use dbforge_core::history::{HistoryEntry, list_history};
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement,
    ListAlignment, ListState, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window, div,
    list, prelude::FluentBuilder, px,
};

use super::format::{first_line, format_duration, format_time, group_by_day, outcome_label};
use crate::backend::Backend;
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::{TextInput, TextInputEvent, any, capsule, dot, group, icon, segment, spinner};

/// The panel's width.
pub const WIDTH: f32 = 320.;

/// How a click that opens a new tab is written on this OS.
const NEW_TAB_CLICK: &str = if cfg!(target_os = "macos") { "⌘-click" } else { "Ctrl-click" };

/// What the panel asks of its console.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryEvent {
    /// Put this run's SQL in the console, without running it.
    Load(HistoryEntry),
    /// Open this run's SQL in a new console on its own connection.
    OpenNew(HistoryEntry),
    Close,
}

/// One row of the list: a day's heading or a run.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    Day(String),
    Entry(usize),
}

pub struct HistoryPanel {
    /// The console's connection, whose runs "This connection" keeps to.
    connection_id: String,
    search: Entity<TextInput>,
    errors_only: bool,
    this_connection: bool,
    entries: Vec<HistoryEntry>,
    rows: Vec<Row>,
    list: ListState,
    loading: bool,
    /// Counts loads, so only the latest one's answer is shown.
    generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<HistoryEvent> for HistoryPanel {}

impl HistoryPanel {
    pub fn new(connection_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = cx.messages().query.history_panel.search_placeholder;
        let search = cx.new(|cx| TextInput::new(placeholder, cx));
        let subscriptions = vec![cx.subscribe_in(&search, window, |this, _, event: &TextInputEvent, _, cx| {
            if *event == TextInputEvent::Changed {
                this.reload(cx);
            }
        })];
        let mut panel = Self {
            connection_id,
            search,
            errors_only: false,
            this_connection: true,
            entries: Vec::new(),
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(200.)),
            loading: false,
            generation: 0,
            _subscriptions: subscriptions,
        };
        panel.reload(cx);
        panel
    }

    /// Reads the list again: after a run, or when a filter changed.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        let search = self.search.read(cx).text().to_owned();
        let connection = self.this_connection.then(|| self.connection_id.clone());
        let errors_only = self.errors_only;
        let listing = Backend::global(cx)
            .spawn_blocking(cx, move |core| list_history(core.history(), search, connection, errors_only));
        cx.spawn(async move |this, cx| {
            let entries = listing.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.show(entries, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn show(&mut self, entries: Vec<HistoryEntry>, cx: &mut Context<Self>) {
        let m = &cx.messages().query.history_panel;
        let mut rows = Vec::new();
        let mut index = 0;
        for (label, group) in group_by_day(&entries, &Local::now(), m) {
            rows.push(Row::Day(label));
            for _ in group {
                rows.push(Row::Entry(index));
                index += 1;
            }
        }
        self.entries = entries;
        self.rows = rows;
        self.list.reset(self.rows.len());
        self.loading = false;
        cx.notify();
    }

    fn toggle_errors(&mut self, cx: &mut Context<Self>) {
        self.errors_only = !self.errors_only;
        self.reload(cx);
    }

    fn set_scope(&mut self, this_connection: bool, cx: &mut Context<Self>) {
        if self.this_connection != this_connection {
            self.this_connection = this_connection;
            self.reload(cx);
        }
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().query.history_panel;
        match self.rows.get(index) {
            Some(Row::Day(label)) => div()
                .px(px(14.))
                .pt(px(10.))
                .pb(px(4.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text3)
                .child(label.clone())
                .into_any_element(),
            Some(Row::Entry(entry_index)) => {
                let entry = &self.entries[*entry_index];
                let failed = entry.error.is_some();
                let mut meta = entry.connection_name.clone();
                if let Some(database) = &entry.database {
                    meta.push_str(&format!(" · {database}"));
                }
                meta.push_str(&format!(" · {} · ", format_duration(entry.elapsed_ms)));
                let clicked = entry.clone();
                div()
                    .id(("history-entry", *entry_index))
                    .mx(px(6.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .hover(|row| row.bg(theme.sel))
                    .on_click(cx.listener(move |_, event: &gpui::ClickEvent, _, cx| {
                        if event.modifiers().secondary() {
                            cx.emit(HistoryEvent::OpenNew(clicked.clone()));
                        } else {
                            cx.emit(HistoryEvent::Load(clicked.clone()));
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(dot(if failed { theme.err } else { theme.ok }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .ellipsis()
                                    .font_family(MONO)
                                    .text_size(px(12.))
                                    .text_color(theme.text)
                                    .child(first_line(&entry.sql).to_owned()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(11.))
                                    .text_color(theme.text3)
                                    .child(format_time(entry.executed_at, &Local)),
                            ),
                    )
                    .child(
                        div()
                            .pl(px(16.))
                            .flex()
                            .text_size(px(11.))
                            .text_color(theme.text3)
                            .child(div().min_w_0().ellipsis().child(meta))
                            .child(
                                div()
                                    .flex_none()
                                    .when(failed, |label| label.text_color(theme.err))
                                    .child(outcome_label(entry, m)),
                            ),
                    )
                    .into_any_element()
            }
            None => div().into_any_element(),
        }
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().query.history_panel;
        let note =
            |text: String, tint| div().p(px(14.)).text_size(px(12.5)).text_color(tint).child(text).into_any_element();
        if self.entries.is_empty() {
            if self.loading {
                return div().into_any_element();
            }
            let filtered = self.errors_only || !self.search.read(cx).text().is_empty();
            return note(if filtered { m.no_runs_match } else { m.empty_history }.into(), theme.text3);
        }
        let this = cx.entity().downgrade();
        list(self.list.clone(), move |index, _window, cx: &mut App| {
            this.update(cx, |this, cx| this.render_row(index, cx)).unwrap_or_else(|_| div().into_any_element())
        })
        .size_full()
        .into_any_element()
    }
}

impl Render for HistoryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().query.history_panel;
        let toggle =
            |id: &'static str, label: &'static str, on: bool| segment(&theme, None, label, on).id(id).cursor_pointer();
        div()
            .w(px(WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(44.))
                    .flex_none()
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(13.)).child(m.title))
                    // The spinner keeps its room, so the header never moves while a list loads.
                    .child(
                        div()
                            .size(px(14.))
                            .when(self.loading, |slot| slot.child(spinner("history-loading", theme.text3, 14.))),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("close-history")
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(13.))
                            .cursor_pointer()
                            .hover(|button| button.bg(theme.sel))
                            .tooltip(crate::ui::field::Tooltip::text(m.close))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(HistoryEvent::Close)))
                            .child(icon("close", theme.text2, 13.)),
                    ),
            )
            .child(
                div()
                    .px(px(10.))
                    .flex()
                    .gap(px(6.))
                    .child(
                        capsule(&theme)
                            .flex_1()
                            .min_w_0()
                            .h(px(32.))
                            .px(px(12.))
                            .child(icon("search", theme.text3, 14.))
                            .child(self.search.clone()),
                    )
                    .child(group(
                        &theme,
                        vec![any(toggle("errors-only", m.errors, self.errors_only)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_errors(cx))))],
                    )),
            )
            .child(div().px(px(10.)).pt(px(8.)).child(group(
                &theme,
                vec![
                        any(toggle("scope-connection", m.this_connection, self.this_connection)
                            .on_click(cx.listener(|this, _, _, cx| this.set_scope(true, cx)))),
                        any(toggle("scope-all", m.all, !self.this_connection)
                            .on_click(cx.listener(|this, _, _, cx| this.set_scope(false, cx)))),
                    ],
            )))
            .child(div().flex_1().min_h_0().pt(px(4.)).child(self.body(cx)))
            .child(
                div()
                    .flex_none()
                    .px(px(14.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(theme.line)
                    .text_size(px(11.))
                    .text_color(theme.text3)
                    .child((m.hint)(NEW_TAB_CLICK)),
            )
    }
}
