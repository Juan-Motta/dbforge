//! Drawing the palette: the search field, the saved queries offered with
//! the selected connection, its tables and views (read once, when it
//! opens) and the commands, each with its match highlighted, and the keys
//! that work it. ↵ runs a saved query (asking for its parameters first);
//! ⌘↵ (Ctrl+Enter) opens it in a tab.

use crate::ui::Ellipsis;
use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::{ObjectKind, Relation, RelationList, TableRef};
use dbforge_core::explorer;
use dbforge_core::saved_queries::SavedQuery;
use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::commands::{Command, CommandAction};
use super::search::{Match, parse_query, rank};
use crate::backend::Backend;
use crate::i18n::Translate;
use crate::saved::SavedStore;
use crate::saved::rules::{Place, offered_in_palette, scope_label};
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::{TextInput, TextInputEvent, glass, icon};

actions!(command_palette, [SelectPrevious, SelectNext, OpenInTab]);

const CONTEXT: &str = "CommandPalette";

/// Most databases fit; past this, typing narrows the list.
const MAX_TABLES: usize = 50;

/// The saved queries listed at most; typing narrows them.
const MAX_SAVED: usize = 20;

/// Each result row is this tall.
const ROW: f32 = 34.;

/// ↑ ↓ move through the results; ⌘↵ opens a saved query in a tab. Enter and
/// Escape reach it from the search field.
pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![
        KeyBinding::new("up", SelectPrevious, context),
        KeyBinding::new("down", SelectNext, context),
        KeyBinding::new("secondary-enter", OpenInTab, context),
    ]
}

/// Where tables are searched: a connection and, optionally, one of its databases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteSource {
    pub connection_id: String,
    pub connection_name: String,
    pub database: Option<String>,
}

/// What the palette asks of the window: close it and do this. Escape only
/// closes it ([`DismissEvent`]).
#[derive(Debug, Clone)]
pub enum PaletteEvent {
    OpenTable {
        connection_id: String,
        table: TableRef,
    },
    Run(CommandAction),
    /// Run this saved query, asking for its parameters first.
    RunSaved(SavedQuery),
    /// Open this saved query in a tab.
    OpenSaved(SavedQuery),
}

/// The tables and views to search, as far as they are read.
enum Relations {
    /// No connection is selected.
    None,
    Loading,
    Loaded(RelationList),
    Failed(String),
}

/// What a result does.
#[derive(Clone)]
enum Target {
    Saved(Box<SavedQuery>),
    Table(TableRef),
    Command(CommandAction),
}

/// One result, as listed.
#[derive(Clone)]
struct Entry {
    label: SharedString,
    detail: Option<SharedString>,
    icon: &'static str,
    target: Target,
}

/// The results for the query typed, in the order they are listed.
struct Results {
    commands_only: bool,
    saved: Vec<(Entry, Match)>,
    tables: Vec<(Entry, Match)>,
    commands: Vec<(Entry, Match)>,
}

impl Results {
    fn len(&self) -> usize {
        self.saved.len() + self.tables.len() + self.commands.len()
    }

    fn get(&self, index: usize) -> Option<&Entry> {
        self.saved.iter().chain(&self.tables).chain(&self.commands).nth(index).map(|(entry, _)| entry)
    }
}

pub struct CommandPalette {
    source: Option<PaletteSource>,
    /// The selected connection's engine, for the saved queries of any connection of it.
    engine: Option<Engine>,
    commands: Vec<Command>,
    relations: Relations,
    input: Entity<TextInput>,
    /// The highlighted result.
    active: usize,
    scroll: ScrollHandle,
    /// Where each result was drawn among the list's children (headings
    /// and status lines are children too), to scroll it into view.
    positions: Vec<usize>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for CommandPalette {}
impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Focusable for CommandPalette {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CommandPalette {
    pub fn new(
        source: Option<PaletteSource>,
        commands: Vec<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let m = &cx.messages().palette;
        let placeholder: SharedString = match &source {
            Some(source) => (m.search_placeholder)(&source.connection_name).into(),
            None => m.type_command.into(),
        };
        let input = cx.new(|cx| TextInput::new(placeholder, cx));
        let store = SavedStore::global(cx);
        let subscriptions =
            vec![cx.subscribe_in(&input, window, Self::on_input), cx.observe(&store, |_, _, cx| cx.notify())];
        let engine = source.as_ref().and_then(|source| {
            let connections = Backend::global(cx).core().connections().list();
            connections.into_iter().find(|c| c.id == source.connection_id).map(|c| c.engine)
        });
        let mut palette = Self {
            engine,
            relations: if source.is_some() { Relations::Loading } else { Relations::None },
            source,
            commands,
            input,
            active: 0,
            scroll: ScrollHandle::new(),
            positions: Vec::new(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        palette.load_relations(cx);
        palette
    }

    /// The search field takes the keyboard.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.input.read(cx).focus(window);
    }

    /// Types `query` into the search field, as the user would.
    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_text(query, cx));
        self.active = 0;
        cx.notify();
    }

    /// Reads the tables and views of the source's database, once.
    fn load_relations(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else { return };
        let listing = Backend::global(cx).spawn(cx, move |core| async move {
            explorer::list_relations(core.connections(), core.statement_log(), source.connection_id, source.database)
                .await
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let listed = listing.await;
            // Fails only when the palette closed meanwhile.
            let _ = this.update(cx, |this, cx| {
                this.relations = match listed {
                    Ok(list) => Relations::Loaded(list),
                    Err(message) => Relations::Failed(message),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn on_input(&mut self, _: &Entity<TextInput>, event: &TextInputEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Changed => {
                self.active = 0;
                self.scroll.scroll_to_item(0);
                cx.notify();
            }
            TextInputEvent::Submitted => self.run_active(false, cx),
            TextInputEvent::Cancelled => cx.emit(DismissEvent),
            TextInputEvent::Pasted(_) => {}
        }
    }

    /// The saved queries offered with the selected connection: those saved
    /// for any connection (they open on theirs), and those of any connection
    /// of its engine.
    fn saved_entries(&self, cx: &App) -> Vec<Entry> {
        let Some(store) = SavedStore::global_read(cx) else { return Vec::new() };
        let m = &cx.messages().saved;
        let here = self.source.as_ref().map(|source| source.connection_id.as_str());
        let place = here.zip(self.engine).map(|(connection_id, engine)| Place { connection_id, engine });
        let connections = Backend::global(cx).core().connections().list();
        let name_of = |id: &str| connections.iter().find(|c| c.id == id).map(|c| c.name.clone());
        store
            .queries()
            .iter()
            .filter(|query| offered_in_palette(query, place))
            .map(|query| Entry {
                label: query.name.clone().into(),
                detail: Some((m.palette_detail)(query.folder.as_deref(), &scope_label(query, here, name_of, m)).into()),
                icon: "play",
                target: Target::Saved(Box::new(query.clone())),
            })
            .collect()
    }

    fn table_entries(&self) -> Vec<Entry> {
        let (Some(source), Relations::Loaded(list)) = (&self.source, &self.relations) else { return Vec::new() };
        list.relations
            .iter()
            .map(|Relation { schema, name, kind }| Entry {
                label: name.clone().into(),
                detail: Some(format!("{} · {}.{schema}", source.connection_name, list.database).into()),
                icon: if *kind == ObjectKind::View { "view" } else { "table" },
                target: Target::Table(TableRef {
                    database: list.database.clone(),
                    schema: schema.clone(),
                    name: name.clone(),
                }),
            })
            .collect()
    }

    fn results(&self, cx: &App) -> Results {
        let query = parse_query(self.input.read(cx).text());
        let ranked = |entries: Vec<Entry>, limit| rank(entries, |entry| entry.label.as_ref(), &query.text, limit);
        let saved = if query.commands_only { Vec::new() } else { ranked(self.saved_entries(cx), MAX_SAVED) };
        let tables = if query.commands_only { Vec::new() } else { ranked(self.table_entries(), MAX_TABLES) };
        let commands = self
            .commands
            .iter()
            .map(|command| Entry {
                label: command.label.into(),
                detail: command.detail.clone().map(Into::into),
                icon: "right",
                target: Target::Command(command.action.clone()),
            })
            .collect();
        Results { commands_only: query.commands_only, saved, tables, commands: ranked(commands, usize::MAX) }
    }

    /// Moves the highlight `step` rows, wrapping around.
    fn step(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = self.results(cx).len();
        if count == 0 {
            return;
        }
        let current = self.active.min(count - 1) as isize;
        self.active = (current + step).rem_euclid(count as isize) as usize;
        if let Some(&position) = self.positions.get(self.active) {
            self.scroll.scroll_to_item(position);
        }
        cx.notify();
    }

    /// Does what the highlighted result does; a saved query opens in a tab
    /// instead of running when `in_tab` (⌘↵).
    fn run_active(&mut self, in_tab: bool, cx: &mut Context<Self>) {
        let results = self.results(cx);
        let index = self.active.min(results.len().saturating_sub(1));
        if let Some(entry) = results.get(index).cloned() {
            self.run(entry, in_tab, cx);
        }
    }

    /// Asks the window to close the palette and do what `entry` does.
    fn run(&mut self, entry: Entry, in_tab: bool, cx: &mut Context<Self>) {
        cx.emit(match entry.target {
            Target::Saved(query) if in_tab => PaletteEvent::OpenSaved(*query),
            Target::Saved(query) => PaletteEvent::RunSaved(*query),
            Target::Table(table) => {
                let connection_id = self.source.as_ref().map(|source| source.connection_id.clone()).unwrap_or_default();
                PaletteEvent::OpenTable { connection_id, table }
            }
            Target::Command(action) => PaletteEvent::Run(action),
        });
    }

    /// What the tables group says instead of (or above) its rows.
    fn table_status(&self, results: &Results, cx: &App) -> Option<String> {
        let m = &cx.messages().palette;
        if results.commands_only {
            return None;
        }
        match &self.relations {
            Relations::None => Some(m.select_connection.to_owned()),
            Relations::Loading => Some(m.loading_tables.to_owned()),
            Relations::Failed(error) => Some((m.load_tables_failed)(error)),
            Relations::Loaded(_) if results.tables.is_empty() => Some(m.no_tables.to_owned()),
            Relations::Loaded(_) => None,
        }
    }

    fn render_row(
        &self,
        theme: &Theme,
        index: usize,
        entry: &Entry,
        hit: &Match,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let end = hit.start + hit.length;
        let highlight = HighlightStyle {
            color: Some(theme.accent_text),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let label = StyledText::new(entry.label.clone())
            .with_highlights((hit.length > 0).then_some((hit.start..end, highlight)));
        let run = entry.clone();
        div()
            .id(("palette-row", index))
            .h(px(ROW))
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .rounded(px(10.))
            .text_size(px(13.))
            .text_color(if active { theme.text } else { theme.text2 })
            .when(active, |row| row.bg(theme.accent.opacity(0.2)))
            .cursor_pointer()
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.active != index {
                    this.active = index;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                this.run(run.clone(), event.modifiers().secondary(), cx)
            }))
            .child(icon(entry.icon, theme.text3, 15.))
            .child(div().min_w_0().ellipsis().child(label))
            .child(div().flex_1())
            .children(entry.detail.clone().map(|detail| {
                div().flex_none().max_w(px(280.)).ellipsis().text_size(px(12.)).text_color(theme.text3).child(detail)
            }))
    }
}

/// A group's heading in the list.
fn heading(theme: &Theme, title: &str) -> impl IntoElement {
    div()
        .h(px(26.))
        .flex_none()
        .px(px(12.))
        .flex()
        .items_end()
        .pb(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.text3)
        .child(title.to_uppercase())
}

/// A line under a heading saying why it has no rows.
fn status(theme: &Theme, text: String) -> impl IntoElement {
    div()
        .h(px(ROW))
        .flex_none()
        .px(px(12.))
        .flex()
        .items_center()
        .text_size(px(13.))
        .text_color(theme.text3)
        .child(text)
}

/// A key and what it does, in the footer.
fn key_hint(theme: &Theme, key: &str, what: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .child(
            div()
                .px(px(5.))
                .rounded(px(4.))
                .border_1()
                .border_color(theme.line)
                .font_family(MONO)
                .text_size(px(11.))
                .child(key.to_owned()),
        )
        .child(what.to_owned())
}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().palette;
        let results = self.results(cx);
        let current = self.active.min(results.len().saturating_sub(1));
        let table_status = self.table_status(&results, cx);
        // Every row of the list, in order: headings, status lines and results.
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut positions = Vec::with_capacity(results.len());
        // Saved queries first, when any is offered.
        if !results.saved.is_empty() {
            rows.push(heading(&theme, cx.messages().saved.saved_queries).into_any_element());
            for (index, (entry, hit)) in results.saved.iter().enumerate() {
                positions.push(rows.len());
                rows.push(self.render_row(&theme, index, entry, hit, index == current, cx).into_any_element());
            }
        }
        if !results.commands_only {
            rows.push(heading(&theme, m.tables_and_views).into_any_element());
            if let Some(text) = table_status {
                rows.push(status(&theme, text).into_any_element());
            }
            let offset = results.saved.len();
            for (i, (entry, hit)) in results.tables.iter().enumerate() {
                let index = offset + i;
                positions.push(rows.len());
                rows.push(self.render_row(&theme, index, entry, hit, index == current, cx).into_any_element());
            }
        }
        // An empty command list is noise unless commands are all that was asked for.
        if results.commands_only || !results.commands.is_empty() {
            rows.push(heading(&theme, m.commands).into_any_element());
            if results.commands.is_empty() {
                rows.push(status(&theme, m.no_commands.to_owned()).into_any_element());
            }
            let offset = results.saved.len() + results.tables.len();
            for (i, (entry, hit)) in results.commands.iter().enumerate() {
                let index = offset + i;
                positions.push(rows.len());
                rows.push(self.render_row(&theme, index, entry, hit, index == current, cx).into_any_element());
            }
        }
        self.positions = positions;
        glass(&theme)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &OpenInTab, _, cx| this.run_active(true, cx)))
            .bg(theme.dialog)
            .w(px(620.))
            .max_w_full()
            .rounded(px(18.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .h(px(52.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(theme.line)
                    .text_size(px(15.))
                    .text_color(theme.text)
                    .child(icon("search", theme.text3, 18.))
                    .child(div().flex_1().min_w_0().child(self.input.clone())),
            )
            .child(
                div()
                    .id("palette-results")
                    .h(px(360.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .child(
                div()
                    .h(px(36.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .border_t_1()
                    .border_color(theme.line)
                    .text_size(px(12.))
                    .text_color(theme.text3)
                    .child(key_hint(&theme, "↑↓", m.navigate))
                    .child(key_hint(&theme, "↵", m.open))
                    .child(key_hint(&theme, &crate::ui::shortcut("↵"), cx.messages().saved.open_in_tab))
                    .child(key_hint(&theme, "esc", m.close))
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(m.type_before)
                            .child(div().font_family(MONO).text_color(theme.text2).child(">"))
                            .child(m.type_after),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{TestAppContext, VisualTestContext};

    use dbforge_core::saved_queries::{SavedQueryInput, SavedScope};

    use super::*;
    use crate::i18n::Language;
    use crate::palette::commands::commands;
    use crate::test_support::{draw, setup, wait_until};
    use crate::theme::ThemePreference;

    fn open(
        source: Option<PaletteSource>,
        cx: &mut TestAppContext,
    ) -> (Entity<CommandPalette>, &mut VisualTestContext) {
        let listed = commands(None, &Language::En.messages().palette);
        cx.add_window_view(move |window, cx| CommandPalette::new(source, listed, window, cx))
    }

    fn heard(palette: &Entity<CommandPalette>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<PaletteEvent>>> {
        let events: Rc<RefCell<Vec<PaletteEvent>>> = Rc::default();
        let into = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(palette, move |_, event: &PaletteEvent, _| into.borrow_mut().push(event.clone())).detach()
        });
        events
    }

    #[gpui::test]
    fn runs_the_highlighted_command_and_limits_the_search_to_commands_after_a_greater_than(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (palette, cx) = open(None, cx);
        let events = heard(&palette, cx);
        draw(cx);
        palette.update(cx, |palette, cx| {
            palette.set_query("> theme", cx);
            let results = palette.results(cx);
            assert!(results.commands_only && results.tables.is_empty());
            let labels: Vec<&str> = results.commands.iter().map(|(entry, _)| entry.label.as_ref()).collect();
            assert_eq!(labels, ["Theme: Dark", "Theme: Light", "Theme: Match system"]);
            palette.step(1, cx);
            palette.step(1, cx);
            palette.step(1, cx);
            assert_eq!(palette.active, 0, "the highlight wraps around");
            palette.step(-1, cx);
            palette.run_active(false, cx);
        });
        draw(cx);
        let events = events.borrow();
        assert!(matches!(events.as_slice(), [PaletteEvent::Run(CommandAction::Theme(ThemePreference::System))]));
    }

    #[gpui::test]
    fn lists_the_saved_queries_offered_first_ranked_and_runs_or_opens_them(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let source = PaletteSource { connection_id: "gone".into(), connection_name: "local".into(), database: None };
        let (palette, cx) = open(Some(source), cx);
        let events = heard(&palette, cx);
        let on = |id: &str| SavedScope::Connection { connection_id: id.into() };
        cx.update(|_, cx| {
            let core = Backend::global(cx).core().clone();
            for (name, scope) in [
                ("Reorder report", on("gone")),
                ("Orders by status", on("gone")),
                ("Orders anywhere", SavedScope::Engine),
                ("Customers", on("other")),
            ] {
                let input = SavedQueryInput {
                    name: name.into(),
                    sql: "select 1".into(),
                    description: None,
                    folder: Some("Reports".into()),
                    scope,
                    engine: Engine::Postgres,
                };
                core.saved_queries().save(None, input).expect("saved");
            }
            crate::saved::store::reload(cx);
        });
        wait_until(cx, "the saved queries", |cx| {
            cx.update(|_, cx| SavedStore::global(cx).read(cx).queries().len() == 4)
        });
        palette.update(cx, |palette, cx| {
            palette.relations = Relations::Loaded(RelationList {
                database: "main".into(),
                relations: vec![Relation { schema: "main".into(), name: "orders".into(), kind: ObjectKind::Table }],
            });
            palette.set_query("order", cx);
            let results = palette.results(cx);
            let labels: Vec<&str> = results.saved.iter().map(|(entry, _)| entry.label.as_ref()).collect();
            assert_eq!(labels, ["Orders by status", "Reorder report"], "a query of any connection needs its engine");
            let detail = results.saved[0].0.detail.as_ref().map(|d| d.to_string());
            assert_eq!(detail.as_deref(), Some("Saved · Reports · This connection"));
            assert!(matches!(results.get(0).map(|entry| &entry.target), Some(Target::Saved(_))), "before the tables");
            assert_eq!(results.get(2).map(|entry| entry.label.to_string()).as_deref(), Some("orders"));
            palette.run_active(false, cx);
            palette.run_active(true, cx);
            palette.set_query("> order", cx);
            assert!(palette.results(cx).saved.is_empty(), "commands only");
        });
        draw(cx);
        let events = events.borrow();
        let [PaletteEvent::RunSaved(run), PaletteEvent::OpenSaved(open)] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert_eq!((run.name.as_str(), open.name.as_str()), ("Orders by status", "Orders by status"));
    }

    #[gpui::test]
    fn ranks_the_connection_s_tables_and_says_when_they_could_not_be_read(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let source = PaletteSource { connection_id: "gone".into(), connection_name: "local".into(), database: None };
        let (palette, cx) = open(Some(source), cx);
        wait_until(cx, "the tables to fail", |cx| {
            palette.read_with(cx, |palette, _| matches!(palette.relations, Relations::Failed(_)))
        });
        palette.read_with(cx, |palette, cx| {
            assert!(
                palette.table_status(&palette.results(cx), cx).is_some_and(|text| text.starts_with("Could not load"))
            );
        });
        draw(cx);
        let relation = |name: &str, kind| Relation { schema: "main".into(), name: name.into(), kind };
        let events = heard(&palette, cx);
        palette.update(cx, |palette, cx| {
            palette.relations = Relations::Loaded(RelationList {
                database: "main".into(),
                relations: vec![
                    relation("reorder_level", ObjectKind::Table),
                    relation("orders", ObjectKind::Table),
                    relation("v_orders", ObjectKind::View),
                ],
            });
            palette.set_query("order", cx);
            let results = palette.results(cx);
            let labels: Vec<&str> = results.tables.iter().map(|(entry, _)| entry.label.as_ref()).collect();
            assert_eq!(labels, ["orders", "v_orders", "reorder_level"]);
            assert_eq!(results.tables[1].0.icon, "view");
            assert_eq!(results.tables[0].0.detail.as_ref().map(|d| d.as_ref()), Some("local · main.main"));
            assert!(results.commands.is_empty(), "no command matches");
            palette.run_active(false, cx);
        });
        draw(cx);
        let events = events.borrow();
        let [PaletteEvent::OpenTable { connection_id, table }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!((connection_id.as_str(), table.name.as_str(), table.schema.as_str()), ("gone", "orders", "main"));
    }
}
