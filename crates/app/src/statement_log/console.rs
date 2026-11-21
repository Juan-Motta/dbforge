//! Drawing the statement console: its bar (the count, the filter, copy all,
//! clear, close), the statements as a list that follows the newest while it
//! is scrolled to the bottom and stays put once scrolled up, and the top
//! edge that resizes it. It reads what the core recorded when it opens and
//! hears each new statement while it is open; closed, nothing listens.

use crate::ui::Ellipsis;
use std::collections::HashMap;

use dbforge_core::Event;
use dbforge_core::connections::model::Engine;
use dbforge_core::statement_log::{Entry, Origin, commands};
use gpui::{
    AnyElement, AppContext, BorrowAppContext, Context, DragMoveEvent, Empty, EventEmitter, FontWeight, HighlightStyle,
    Hsla, InteractiveElement, IntoElement, ListAlignment, ListState, ParentElement, Render, StatefulInteractiveElement,
    Styled, StyledText, Subscription, Window, div, list, prelude::FluentBuilder, px,
};

use super::height;
use super::log::{CAPACITY, Kind, clock_time, duration, kind_of, merge, runnable, script, visible};
use crate::backend::Backend;
use crate::events::EventHub;
use crate::i18n::{StatementMessages, Translate};
use crate::preferences::{Preferences, keys};
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::field::Tooltip;
use crate::ui::{any, glass, icon, icon_button, pill, segment_face, segmented, shortcut};

/// What the console tells the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementConsoleEvent {
    /// Its close button: hide it.
    Close,
}

/// Dragging the console's top edge.
struct ResizeStatements;

struct NoPreview;

impl Render for NoPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// A statement on one line, its runs of spaces and line breaks one space
/// each, as the former web UI's one-line row showed it.
fn on_one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What a statement was for, at a glance: the user's own, an agent's, changes, reads, metadata.
fn kind_tint(theme: &Theme, kind: Kind) -> Hsla {
    match kind {
        Kind::Console => theme.func,
        Kind::Agents => theme.accent_text,
        Kind::Changes => theme.warn,
        Kind::Reads => theme.ok,
        Kind::Metadata | Kind::All => theme.text3,
    }
}

fn kind_label(kind: Kind, m: &StatementMessages) -> &'static str {
    match kind {
        Kind::All => m.all,
        Kind::Console => m.console,
        Kind::Agents => m.agents,
        Kind::Changes => m.changes,
        Kind::Reads => m.reads,
        Kind::Metadata => m.metadata,
    }
}

fn origin_label(origin: Origin, m: &StatementMessages) -> &'static str {
    match origin {
        Origin::Console => m.origin_console,
        Origin::Browse => m.origin_browse,
        Origin::Edit => m.origin_edit,
        Origin::Metadata => m.origin_metadata,
        Origin::Admin => m.origin_admin,
        Origin::Export => m.origin_export,
        Origin::Import => m.origin_import,
        Origin::Dump => m.origin_dump,
        Origin::Connect => m.origin_connect,
        Origin::Agent => m.origin_agent,
    }
}

pub struct StatementConsole {
    entries: Vec<Entry>,
    kind: Kind,
    /// The statement shown whole.
    expanded: Option<u64>,
    /// The ids the list shows now, in order, to tell it what changed.
    shown: Vec<u64>,
    list: ListState,
    /// Each saved connection's name and engine, by id.
    connections: HashMap<String, (String, Engine)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<StatementConsoleEvent> for StatementConsole {}

impl StatementConsole {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Listening starts before the list is read, so nothing recorded in between is missed.
        let logged = cx.subscribe(&EventHub::global(cx), |this, _, event: &Event, cx| {
            if let Event::StatementLogged(entry) = event {
                this.add(vec![entry.clone()], cx);
            }
        });
        let recorded = Backend::global(cx).spawn_blocking(cx, |core| commands::list_statements(core.statement_log()));
        cx.spawn(async move |this, cx| {
            let recorded = recorded.await;
            let _ = this.update(cx, |this, cx| this.add(recorded, cx));
        })
        .detach();
        let mut console = Self {
            entries: Vec::new(),
            kind: Kind::All,
            expanded: None,
            shown: Vec::new(),
            list: ListState::new(0, ListAlignment::Bottom, px(200.)),
            connections: HashMap::new(),
            _subscriptions: vec![logged],
        };
        console.refresh_connections(cx);
        console
    }

    /// Reads the saved connections' names again, after one was saved or deleted.
    pub fn refresh_connections(&mut self, cx: &mut Context<Self>) {
        let saved = Backend::global(cx).core().connections().list();
        self.connections =
            saved.into_iter().map(|connection| (connection.id, (connection.name, connection.engine))).collect();
        self.list.reset(self.shown.len());
        cx.notify();
    }

    fn engine_of(&self, connection_id: &str) -> Option<Engine> {
        self.connections.get(connection_id).map(|(_, engine)| *engine)
    }

    fn add(&mut self, incoming: Vec<Entry>, cx: &mut Context<Self>) {
        if merge(&mut self.entries, incoming, CAPACITY) {
            self.sync_list(cx);
        }
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.entries.clear();
        self.expanded = None;
        Backend::global(cx).spawn_blocking(cx, |core| commands::clear_statements(core.statement_log())).detach();
        self.sync_list(cx);
    }

    fn set_kind(&mut self, kind: Kind, cx: &mut Context<Self>) {
        if self.kind != kind {
            self.kind = kind;
            self.sync_list(cx);
        }
    }

    /// Tells the list what changed: statements dropped from the front (a
    /// full log drops the oldest) and added at the end keep the scroll
    /// where it is; anything else lays the list out again.
    fn sync_list(&mut self, cx: &mut Context<Self>) {
        let shown: Vec<u64> = visible(&self.entries, self.kind).into_iter().map(|entry| entry.id).collect();
        if shown == self.shown {
            return cx.notify();
        }
        let first = shown.first().copied();
        let dropped = self.shown.iter().take_while(|id| first.is_none_or(|first| **id < first)).count();
        let kept = &self.shown[dropped..];
        if shown.starts_with(kept) && !shown.is_empty() {
            self.list.splice(0..dropped, 0);
            self.list.splice(kept.len()..kept.len(), shown.len() - kept.len());
        } else {
            self.list.reset(shown.len());
        }
        self.shown = shown;
        cx.notify();
    }

    /// Shows statement `id` whole, or on one line again when it was.
    fn toggle(&mut self, id: u64, cx: &mut Context<Self>) {
        let before = self.expanded;
        self.expanded = if before == Some(id) { None } else { Some(id) };
        // The rows whose height changed are measured again.
        for changed in [Some(id), before.filter(|open| *open != id)].into_iter().flatten() {
            if let Some(row) = self.shown.iter().position(|shown| *shown == changed) {
                self.list.splice(row..row + 1, 1);
            }
        }
        cx.notify();
    }

    /// Puts `text` on the clipboard; the window's notice says so.
    fn copy(&mut self, text: String, cx: &mut Context<Self>) {
        crate::ui::notice::copy_to_clipboard(text, cx);
    }

    /// Keeps the height the user dragged the top edge to.
    fn resize(&mut self, height: f32, cx: &mut Context<Self>) {
        let height = super::height(Some(&height.round().to_string())).to_string();
        if Preferences::global(cx).get(keys::STATEMENTS_HEIGHT) != Some(height.as_str()) {
            cx.update_global::<Preferences, _>(|preferences, _| preferences.set(keys::STATEMENTS_HEIGHT, Some(height)));
            cx.notify();
        }
    }

    /// Back to the default height (double-click on the edge), kept as the former web UI kept it.
    fn reset_height(&mut self, cx: &mut Context<Self>) {
        let height = super::height(None).to_string();
        cx.update_global::<Preferences, _>(|preferences, _| preferences.set(keys::STATEMENTS_HEIGHT, Some(height)));
        cx.notify();
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().statements;
        let Some(entry) = self.shown.get(index).and_then(|id| self.entries.iter().find(|entry| entry.id == *id)) else {
            return Empty.into_any_element();
        };
        let open = self.expanded == Some(entry.id);
        let connection = self.connections.get(&entry.connection_id);
        let name = connection.map_or(m.unknown_connection.to_owned(), |(name, _)| name.clone());
        let where_ = match &entry.database {
            Some(database) => format!("{name} · {database}"),
            None => name.clone(),
        };
        let mut meta = duration(entry.elapsed_ms);
        if let (None, Some(rows)) = (&entry.error, entry.rows) {
            meta = format!("{meta} · {}", (m.rows)(rows));
        }
        let kind = kind_of(entry.origin);
        let (id, text) = (entry.id, runnable(entry, connection.map(|(_, engine)| *engine)));
        let failed = entry.error.is_some();
        div()
            .id(("statement", id))
            .group("statement")
            .w_full()
            .px(px(12.))
            .py(px(3.))
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme.line)
            .text_size(px(12.))
            .hover(|row| row.bg(theme.sel))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        div()
                            .id(("statement-time", id))
                            .w(px(88.))
                            .flex_none()
                            .pt(px(3.))
                            .font_family(MONO)
                            .text_size(px(11.5))
                            .text_color(theme.text3)
                            .tooltip(Tooltip::text(where_.clone()))
                            .child(clock_time(entry.at, &chrono::Local)),
                    )
                    .child(
                        // A flex column, so the pill keeps its label's width instead of the column's.
                        div()
                            .w(px(70.))
                            .flex_none()
                            .flex()
                            .pt(px(2.))
                            .child(pill(origin_label(entry.origin, m), kind_tint(&theme, kind))),
                    )
                    .child({
                        // The connection, then its database in a quieter colour.
                        let quiet = HighlightStyle { color: Some(theme.text3), ..Default::default() };
                        let database = (name.len()..where_.len(), quiet);
                        div()
                            .w(px(150.))
                            .flex_none()
                            .pt(px(3.))
                            .ellipsis()
                            .text_color(theme.text2)
                            .child(StyledText::new(where_.clone()).with_highlights([database]))
                    })
                    .child(
                        div()
                            .id(("statement-text", id))
                            .flex_1()
                            .min_w_0()
                            .py(px(3.))
                            .font_family(MONO)
                            .text_color(if failed { theme.err } else { theme.text })
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle(id, cx)))
                            .map(|text| {
                                if open {
                                    text.child(entry.text.clone())
                                } else {
                                    text.ellipsis().child(on_one_line(&entry.text))
                                }
                            }),
                    )
                    .child(
                        // The time and rows, and over them on hover the copy button, so the
                        // row keeps no empty space at its end for it.
                        div()
                            .w(px(116.))
                            .flex_none()
                            .relative()
                            .child(
                                div()
                                    .pt(px(3.))
                                    .flex()
                                    .justify_end()
                                    .font_family(MONO)
                                    .text_size(px(11.5))
                                    .text_color(theme.text3)
                                    .group_hover("statement", |meta| meta.opacity(0.))
                                    .child(meta),
                            )
                            .child(
                                icon_button(&theme, ("statement-copy", id), "duplicate", true, false)
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .size(px(24.))
                                    .opacity(0.)
                                    .group_hover("statement", |button| button.opacity(1.))
                                    .tooltip(Tooltip::text(m.copy))
                                    .on_click(cx.listener(move |this, _, _, cx| this.copy(text.clone(), cx))),
                            ),
                    ),
            )
            .children(entry.error.clone().map(|error| {
                // Under the statement, from its column on.
                div().pl(px(88. + 70. + 150. + 30.)).pb(px(4.)).text_color(theme.err).child(error)
            }))
            .into_any_element()
    }

    fn render_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().statements;
        let shown = self.shown.len();
        let kinds = Kind::ALL
            .into_iter()
            .map(|kind| {
                any(segment_face(theme, None, kind_label(kind, m), self.kind == kind)
                    .id(("statement-kind", kind as usize))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.set_kind(kind, cx))))
            })
            .collect();
        let all = script(visible(&self.entries, self.kind), |connection_id| self.engine_of(connection_id));
        let quiet = |id: &'static str, icon_name: &'static str, label: &'static str, on: bool| {
            div()
                .h(px(30.))
                .px(px(10.))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .rounded(px(15.))
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text2)
                .child(icon(icon_name, theme.text2, 14.))
                .child(label)
                .id(id)
                .when(on, |button| button.cursor_pointer().hover(|button| button.bg(theme.sel)))
                .when(!on, |button| button.opacity(0.45))
        };
        div()
            .h(px(44.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(theme.line)
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(m.title))
            .child(div().text_size(px(12.)).text_color(theme.text3).child((m.count)(shown)))
            .child(segmented(theme, "statement-kinds", Kind::ALL.iter().position(|kind| *kind == self.kind), kinds))
            .child(div().flex_1())
            .child(quiet("statements-copy-all", "duplicate", m.copy_all, shown > 0).when(shown > 0, |button| {
                button.on_click(cx.listener(move |this, _, _, cx| this.copy(all.clone(), cx)))
            }))
            .child(
                quiet("statements-clear", "trash", m.clear, !self.entries.is_empty())
                    .when(!self.entries.is_empty(), |button| {
                        button.on_click(cx.listener(|this, _, _, cx| this.clear(cx)))
                    }),
            )
            .child(
                icon_button(theme, "statements-close", "close", true, false)
                    .tooltip(Tooltip::text((m.close)(&shortcut("j"))))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(StatementConsoleEvent::Close))),
            )
    }
}

impl Render for StatementConsole {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().statements;
        let console_height = height(Preferences::global(cx).get(keys::STATEMENTS_HEIGHT));
        let body = if self.shown.is_empty() {
            let text = if self.entries.is_empty() { m.empty } else { m.empty_filtered };
            div().px(px(14.)).py(px(18.)).text_size(px(12.)).text_color(theme.text3).child(text).into_any_element()
        } else {
            list(self.list.clone(), cx.processor(|this, index: usize, _window, cx| this.render_row(index, cx)))
                .flex_1()
                .min_h_0()
                .into_any_element()
        };
        glass(&theme)
            .id("statements")
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<ResizeStatements>, _, cx| {
                let bottom: f32 = event.bounds.bottom().into();
                let pointer: f32 = event.event.position.y.into();
                this.resize(bottom - pointer, cx);
            }))
            .relative()
            .h(px(console_height))
            .flex_none()
            .rounded(px(22.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(self.render_bar(&theme, cx))
            .child(body)
            .child(
                div()
                    .id("statements-edge")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(6.))
                    .cursor_row_resize()
                    .tooltip(Tooltip::text(m.resize))
                    .on_drag(ResizeStatements, |_, _, _, cx| cx.new(|_| NoPreview))
                    .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            this.reset_height(cx);
                        }
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::test_support::{draw, setup};

    fn entry(id: u64, origin: Origin) -> Entry {
        Entry {
            id,
            at: 1_700_000_000_000 + id,
            connection_id: "gone".into(),
            database: Some("shop".into()),
            origin,
            text: format!("SELECT {id}\nFROM orders"),
            elapsed_ms: 1240,
            rows: Some(id),
            error: (origin == Origin::Edit).then(|| "permission denied".to_owned()),
        }
    }

    fn logged(entry: Entry, cx: &mut VisualTestContext) {
        cx.update(|_, cx| EventHub::global(cx).update(cx, |_, cx| cx.emit(Event::StatementLogged(entry))));
        cx.run_until_parked();
    }

    #[gpui::test]
    fn shows_each_statement_once_as_it_is_logged_filtered_by_what_it_was_for(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (console, cx) = cx.add_window_view(|_, cx| StatementConsole::new(cx));
        cx.run_until_parked();
        draw(cx);
        logged(entry(1, Origin::Console), cx);
        logged(entry(2, Origin::Edit), cx);
        // The same statement heard twice (read at start and as an event) shows once.
        logged(entry(1, Origin::Console), cx);
        console.read_with(cx, |console, _| assert_eq!(console.shown, [1, 2]));
        draw(cx);
        console.update(cx, |console, cx| console.set_kind(Kind::Changes, cx));
        console.read_with(cx, |console, _| assert_eq!(console.shown, [2]));
        // Shown whole, then on one line again.
        console.update(cx, |console, cx| console.toggle(2, cx));
        console.read_with(cx, |console, _| assert_eq!(console.expanded, Some(2)));
        draw(cx);
        console.update(cx, |console, cx| console.toggle(2, cx));
        console.read_with(cx, |console, _| assert_eq!(console.expanded, None));
        console.update(cx, |console, cx| console.clear(cx));
        console.read_with(cx, |console, _| assert!(console.shown.is_empty() && console.entries.is_empty()));
        draw(cx);
    }

    #[test]
    fn writes_a_statement_on_one_line_as_a_row_shows_it() {
        assert_eq!(on_one_line("SELECT name\n    FROM t\t WHERE x"), "SELECT name FROM t WHERE x");
    }

    #[gpui::test]
    fn names_an_unsaved_connection_and_copies_a_statement_as_it_runs(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (console, cx) = cx.add_window_view(|_, cx| StatementConsole::new(cx));
        logged(entry(7, Origin::Console), cx);
        let text = console.read_with(cx, |console, _| {
            assert!(console.engine_of("gone").is_none(), "no saved connection has that id");
            runnable(&console.entries[0], console.engine_of("gone"))
        });
        console.update(cx, |console, cx| console.copy(text.clone(), cx));
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()), Some(text));
        let notice = cx.update(|_, cx| {
            crate::ui::notice::Notices::global(cx).read(cx).current().map(|notice| notice.text.to_string())
        });
        assert_eq!(notice.as_deref(), Some("Copied to the clipboard."), "the window says it was copied");
        draw(cx);
    }
}
