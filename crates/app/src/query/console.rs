//! A SQL console: the database it queries, the editor, the results of its
//! last run below it and, on ⌘Y, the history beside them. Each console has
//! its own core session (a connection kept open between runs, so a
//! transaction or a `SET` carries over); closing its tab closes it.
//!
//! ⌘↵ (Ctrl+Enter) and the Run button run the selection, or the whole
//! script, as the web console did. Explain shows the plan of the same text
//! instead; with Analyze it runs the statement to measure it, inside a
//! transaction the core rolls back.

use std::rc::Rc;
use std::time::{Duration, Instant};

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::ObjectKind;
use dbforge_core::explorer::{completion_schema, list_objects};
use dbforge_core::history::HistoryEntry;
use dbforge_core::query::{QueryScope, cancel_query, close_session, explain_query, run_query};
use gpui::{
    AnyElement, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Pixels, Point, Render, StatefulInteractiveElement, Styled, Subscription,
    Window, actions, div, linear_color_stop, linear_gradient, point, prelude::FluentBuilder, px,
};

use super::plan::parse_plan;
use super::results::{LastRun, ResultsPanel};
use super::shortcuts;
use crate::backend::Backend;
use crate::history::{HistoryEvent, HistoryPanel};
use crate::i18n::Translate;
use crate::sql::Dialect;
use crate::sql::completion::CompletionNames;
use crate::sql::statements::changes_structure;
use crate::theme::{ActiveTheme, color};
use crate::ui::field::Tooltip;
use crate::ui::{
    Checkbox, CodeEditor, MenuItem, PopoverMenu, any, button, capsule, card, enabled, group, icon, popover, segment,
    shadow,
};

actions!(query_console, [Run, ToggleHistory]);

const CONTEXT: &str = "QueryConsole";

/// How often the running time is redrawn.
const TICK: Duration = Duration::from_millis(100);

/// Space between the console's cards.
const GAP: f32 = 10.;

pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![KeyBinding::new("secondary-enter", Run, context), KeyBinding::new("secondary-y", ToggleHistory, context)]
}

/// What a console asks of the workspace.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsoleEvent {
    /// Open this past run in a new console on its own connection.
    OpenQuery(HistoryEntry),
}

/// A run going now.
struct Running {
    cancelling: bool,
}

pub struct QueryConsole {
    /// The core session's id: the tab's.
    session_id: String,
    connection_id: String,
    engine: Engine,
    dialect: Dialect,
    /// The database queried; the connection's own when `None`.
    database: Option<String>,
    /// The databases the picker offers; `None` while they load.
    databases: Option<Vec<String>>,
    editor: Entity<CodeEditor>,
    results: Entity<ResultsPanel>,
    history: Option<Entity<HistoryPanel>>,
    running: Option<Running>,
    /// An explain going now.
    explaining: bool,
    /// Explain runs the statement to measure it (in a transaction rolled back).
    analyze: bool,
    cancel_error: Option<String>,
    database_menu: Option<(Entity<PopoverMenu>, Point<Pixels>, Subscription)>,
    /// Counts completion-name loads, so a stale one is dropped.
    names_generation: u64,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConsoleEvent> for QueryConsole {}

impl Focusable for QueryConsole {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl QueryConsole {
    /// A console `session_id` on `connection_id` (a SQL engine) and
    /// `database`, its editor starting with `sql`.
    pub fn new(
        session_id: String,
        connection_id: String,
        engine: Engine,
        database: Option<String>,
        sql: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let dialect = Dialect::of(engine).unwrap_or(Dialect::Postgres);
        let editor = cx.new(|cx| CodeEditor::new(sql.as_deref().unwrap_or(""), Some(dialect), cx));
        let mut console = Self {
            session_id,
            connection_id,
            engine,
            dialect,
            database,
            databases: None,
            editor,
            results: cx.new(ResultsPanel::new),
            history: None,
            running: None,
            explaining: false,
            analyze: false,
            cancel_error: None,
            database_menu: None,
            names_generation: 0,
            focus: cx.focus_handle(),
            _subscriptions: Vec::new(),
        };
        console.load_databases(cx);
        console.load_names(cx);
        console
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    pub fn editor(&self) -> &Entity<CodeEditor> {
        &self.editor
    }

    pub fn results(&self) -> &Entity<ResultsPanel> {
        &self.results
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn is_explaining(&self) -> bool {
        self.explaining
    }

    /// A run or an explain is going: neither starts another.
    fn busy(&self) -> bool {
        self.running.is_some() || self.explaining
    }

    pub fn set_analyze(&mut self, analyze: bool, cx: &mut Context<Self>) {
        self.analyze = analyze;
        cx.notify();
    }

    /// The editor takes the keyboard.
    pub fn focus(&self, window: &mut Window, cx: &gpui::App) {
        self.editor.read(cx).focus(window);
    }

    fn scope(&self) -> QueryScope {
        QueryScope { database: self.database.clone(), container: None }
    }

    /// The databases the picker offers. A SQLite file is its one database.
    fn load_databases(&mut self, cx: &mut Context<Self>) {
        if self.engine == Engine::Sqlite {
            self.databases = Some(Vec::new());
            return;
        }
        let connection = self.connection_id.clone();
        let listing = Backend::global(cx).spawn(cx, move |core| async move {
            list_objects(core.connections(), core.statement_log(), connection, Vec::new()).await
        });
        cx.spawn(async move |this, cx| {
            // Without the list, the console keeps querying the database it opened on.
            let databases = listing
                .await
                .map(|objects| objects.into_iter().filter(|o| o.kind == ObjectKind::Database).map(|o| o.name).collect())
                .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.databases = Some(databases);
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads the names the editor completes; a failure leaves keywords alone.
    fn load_names(&mut self, cx: &mut Context<Self>) {
        self.names_generation += 1;
        let generation = self.names_generation;
        let (connection, database, dialect) = (self.connection_id.clone(), self.database.clone(), self.dialect);
        let reading = Backend::global(cx).spawn(cx, move |core| async move {
            completion_schema(core.connections(), core.statement_log(), connection, database).await
        });
        cx.spawn(async move |this, cx| {
            let Ok(schema) = reading.await else { return };
            let names = Rc::new(CompletionNames::new(&schema, dialect));
            let _ = this.update(cx, |this, cx| {
                if this.names_generation == generation {
                    this.editor.update(cx, |editor, cx| editor.set_completion_names(Some(names), cx));
                }
            });
        })
        .detach();
    }

    /// Queries `database` from the next run on.
    fn pick_database(&mut self, database: Option<String>, cx: &mut Context<Self>) {
        if self.database != database {
            self.database = database;
            self.load_names(cx);
            cx.notify();
        }
    }

    /// What ⌘↵ and Run run: the selection, or the whole script.
    pub fn text_to_run(&self, cx: &gpui::App) -> String {
        let editor = self.editor.read(cx);
        editor.selected_text().unwrap_or(editor.text()).to_owned()
    }

    /// Runs `sql` in the console's session. The last results stay until the
    /// new ones arrive; a script that may have changed the tables reads
    /// their names again.
    pub fn run(&mut self, sql: String, cx: &mut Context<Self>) {
        if sql.trim().is_empty() || self.busy() {
            return;
        }
        let started = Instant::now();
        self.running = Some(Running { cancelling: false });
        self.cancel_error = None;
        self.results.update(cx, |results, cx| results.set_running(Some(started), cx));
        let (session, connection, scope) = (self.session_id.clone(), self.connection_id.clone(), self.scope());
        let script = sql.clone();
        let run = Backend::global(cx).spawn(cx, move |core| async move {
            let (store, sessions, history, log) =
                (core.connections(), core.query_sessions(), core.history(), core.statement_log());
            run_query(store, sessions, history, log, session, connection, scope, script).await
        });
        // Redraws the running time until the run ends.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let going = this.update(cx, |this, cx| {
                    this.results.update(cx, |_, cx| cx.notify());
                    this.running.is_some()
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let outcome = run.await;
            let _ = this.update(cx, |this, cx| {
                this.running = None;
                let last = match outcome {
                    Ok(outcome) => LastRun::Outcome(outcome),
                    Err(error) => LastRun::Failed(error.to_string()),
                };
                this.results.update(cx, |results, cx| results.show(last, cx));
                if let Some(history) = &this.history {
                    history.update(cx, |history, cx| history.reload(cx));
                }
                if changes_structure(&sql) {
                    this.load_names(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Shows the plan of `sql` in place of the results. It runs only with
    /// Analyze, inside a transaction the core rolls back; a plan that can't
    /// be read shows as an error.
    pub fn explain(&mut self, sql: String, cx: &mut Context<Self>) {
        if sql.trim().is_empty() || self.busy() {
            return;
        }
        self.explaining = true;
        self.results.update(cx, |results, cx| results.set_running(Some(Instant::now()), cx));
        let (connection, scope, analyze) = (self.connection_id.clone(), self.scope(), self.analyze);
        let explaining = Backend::global(cx).spawn(cx, move |core| async move {
            explain_query(core.connections(), core.statement_log(), connection, scope, sql, analyze).await
        });
        cx.spawn(async move |this, cx| {
            let plan = explaining.await;
            let _ = this.update(cx, |this, cx| {
                this.explaining = false;
                let last = match plan {
                    Ok(json) => match parse_plan(&json, &cx.messages().query.plan) {
                        Ok(_) => LastRun::Plan(Rc::new(json)),
                        Err(error) => LastRun::Failed(error),
                    },
                    Err(error) => LastRun::Failed(error.to_string()),
                };
                this.results.update(cx, |results, cx| results.show(last, cx));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Stops the run going; it then fails with the server's cancel error.
    fn cancel(&mut self, cx: &mut Context<Self>) {
        let Some(running) = self.running.as_mut() else { return };
        if running.cancelling {
            return;
        }
        running.cancelling = true;
        let session = self.session_id.clone();
        let cancelling = Backend::global(cx).spawn(cx, move |core| async move {
            cancel_query(core.query_sessions(), core.statement_log(), session).await
        });
        cx.spawn(async move |this, cx| {
            if let Err(error) = cancelling.await {
                // The query keeps running; the user may try again.
                let _ = this.update(cx, |this, cx| {
                    this.cancel_error = Some(error.to_string());
                    if let Some(running) = this.running.as_mut() {
                        running.cancelling = false;
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// Releases the console's connection, its tab being closed; uncommitted
    /// work is rolled back. Nobody is left to show an error.
    pub fn close_session(session: String, cx: &gpui::App) {
        Backend::global(cx)
            .spawn(cx, move |core| async move {
                let _ = close_session(core.query_sessions(), core.statement_log(), session).await;
            })
            .detach();
    }

    /// Shows or hides the history beside the console.
    pub fn toggle_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.take().is_some() {
            self.focus(window, cx);
        } else {
            let connection = self.connection_id.clone();
            let history = cx.new(|cx| HistoryPanel::new(connection, window, cx));
            self._subscriptions.push(cx.subscribe_in(&history, window, Self::on_history));
            self.history = Some(history);
        }
        cx.notify();
    }

    fn on_history(
        &mut self,
        _: &Entity<HistoryPanel>,
        event: &HistoryEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            HistoryEvent::Load(entry) => self.load(entry, window, cx),
            HistoryEvent::OpenNew(entry) => {
                // A run whose connection was deleted can't open; it loads here instead.
                let exists =
                    Backend::global(cx).core().connections().list().iter().any(|c| c.id == entry.connection_id);
                if exists {
                    cx.emit(ConsoleEvent::OpenQuery(entry.clone()));
                } else {
                    self.load(entry, window, cx);
                }
            }
            HistoryEvent::Close => {
                self.history = None;
                self.focus(window, cx);
                cx.notify();
            }
        }
    }

    /// Puts a past run's SQL in the editor, without running it.
    fn load(&mut self, entry: &HistoryEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| editor.set_text(&entry.sql, cx));
        self.focus(window, cx);
    }

    fn open_database_menu(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(databases) = self.databases.clone() else { return };
        let this = cx.entity().downgrade();
        let m = &cx.messages().query;
        let mut choices: Vec<(Option<String>, String)> = Vec::new();
        if self.database.is_none() {
            choices.push((None, m.default_database.to_owned()));
        }
        // A database the list leaves out (one the user can't see listed) stays selectable.
        if let Some(current) = self.database.as_ref().filter(|current| !databases.contains(current)) {
            choices.push((Some(current.clone()), current.clone()));
        }
        choices.extend(databases.into_iter().map(|name| (Some(name.clone()), name)));
        let items = choices
            .into_iter()
            .map(|(value, label)| {
                let this = this.clone();
                let checked = value == self.database;
                MenuItem::new(label, move |_, cx| {
                    let value = value.clone();
                    let _ = this.update(cx, |this, cx| this.pick_database(value, cx));
                })
                .checked(checked)
            })
            .collect();
        let menu = cx.new(|cx| PopoverMenu::new(items, window, cx));
        let closing = cx.subscribe_in(&menu, window, |this, _, _: &gpui::DismissEvent, window, cx| {
            this.database_menu = None;
            this.focus(window, cx);
            cx.notify();
        });
        self.database_menu = Some((menu, position, closing));
        cx.notify();
    }

    fn database_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().query;
        let name = self.database.clone().unwrap_or_else(|| m.default_database.to_owned());
        let pickable = self.engine != Engine::Sqlite
            && !self.busy()
            && self.databases.as_ref().is_some_and(|databases| !databases.is_empty() || self.database.is_some());
        let picker = capsule(&theme)
            .id("database-picker")
            .max_w(px(280.))
            .text_color(if self.database.is_some() { theme.text } else { theme.text2 })
            .tooltip(Tooltip::text(m.database_to_query))
            .child(icon("database", theme.text3, 14.))
            .child(div().min_w_0().truncate().child(name))
            .when(self.engine != Engine::Sqlite, |picker| picker.child(icon("down", theme.text3, 12.)));
        if !pickable {
            return picker.into_any_element();
        }
        picker
            .cursor_pointer()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    if this.database_menu.is_none() {
                        this.open_database_menu(event.position + point(px(0.), px(12.)), window, cx);
                    }
                }),
            )
            .into_any_element()
    }

    /// Run, or Cancel while a run goes; both in one slot, so nothing moves.
    fn run_slot(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().query;
        let slot = div().min_w(px(132.)).flex().justify_end();
        match &self.running {
            Some(running) => {
                let label = if running.cancelling { m.cancelling } else { m.cancel };
                slot.child(
                    enabled(button(&theme, Some("close"), label), !running.cancelling)
                        .id("cancel-run")
                        .when(!running.cancelling, |cancel| {
                            cancel.on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
                        }),
                )
                .into_any_element()
            }
            None => slot
                .child(
                    enabled(div(), !self.explaining)
                        .id("run")
                        .h(px(32.))
                        .px(px(16.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(16.))
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(theme.accent_hi, 0.),
                            linear_color_stop(theme.accent, 1.),
                        ))
                        .border_1()
                        .border_color(color(0xFFFFFF47))
                        .shadow(shadow(20., 8., 0xD4532566))
                        .text_color(gpui::white())
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .when(!self.explaining, |run| {
                            run.on_click(cx.listener(|this, _, _, cx| {
                                let sql = this.text_to_run(cx);
                                this.run(sql, cx);
                            }))
                        })
                        .child(icon("play", gpui::white(), 14.))
                        .child(m.run)
                        .child(
                            div().font_weight(FontWeight::MEDIUM).text_color(color(0xFFFFFFCC)).child(shortcuts::RUN),
                        ),
                )
                .into_any_element(),
        }
    }

    /// Explain, or Explaining… while it goes; both the same width, so nothing moves.
    fn explain_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().query;
        let label = if self.explaining { m.explaining } else { m.explain };
        let on = !self.busy();
        group(
            &theme,
            vec![any(enabled(segment(&theme, Some("chart"), label, false).min_w(px(112.)).justify_center(), on)
                .id("explain")
                .when(on, |explain| {
                    explain.on_click(cx.listener(|this, _, _, cx| {
                        let sql = this.text_to_run(cx);
                        this.explain(sql, cx);
                    }))
                }))],
        )
        .into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().query;
        div()
            .h(px(34.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.database_picker(cx))
            .child(div().flex_1())
            // The cancel error keeps to one line, cut short, so nothing moves.
            .children(self.cancel_error.as_ref().map(|error| {
                div()
                    .id("cancel-error")
                    .min_w_0()
                    .max_w(px(360.))
                    .truncate()
                    .text_size(px(12.))
                    .text_color(theme.err)
                    .tooltip(Tooltip::text(error.clone()))
                    .child((m.could_not_cancel)(error))
            }))
            .child(group(
                &theme,
                vec![any(segment(&theme, Some("clock"), m.history, self.history.is_some())
                    .id("toggle-history")
                    .cursor_pointer()
                    .tooltip(Tooltip::text((m.history_title)(shortcuts::HISTORY)))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_history(window, cx))))],
            ))
            .child(
                div().id("analyze").tooltip(Tooltip::text(m.analyze_title)).child(
                    Checkbox::new("analyze-checkbox", m.analyze, self.analyze)
                        .on_change(cx.listener(|this, analyze: &bool, _, cx| this.set_analyze(*analyze, cx))),
                ),
            )
            .child(self.explain_button(cx))
            .child(self.run_slot(cx))
            .into_any_element()
    }
}

impl Render for QueryConsole {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Run, _, cx| {
                let sql = this.text_to_run(cx);
                this.run(sql, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleHistory, window, cx| this.toggle_history(window, cx)))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .child(self.toolbar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap(px(GAP))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(GAP))
                            .child(card(&theme).child(self.editor.clone()))
                            .child(card(&theme).child(self.results.clone())),
                    )
                    // A card fills the space left; the history's keeps its own width.
                    .children(
                        self.history.clone().map(|history| {
                            card(&theme).flex_none().flex_basis(px(crate::history::WIDTH)).child(history)
                        }),
                    ),
            )
            .children(self.database_menu.as_ref().map(|(menu, position, _)| popover(menu.clone(), *position)))
    }
}

#[cfg(test)]
mod tests {
    use dbforge_core::drivers::{Column, QueryOutcome, StatementResult};
    use gpui::TestAppContext;

    use super::*;
    use crate::backend::test_backend;
    use crate::i18n::{Language, Locale};
    use crate::theme::{Appearance, Theme};

    fn column(name: &str, data_type: &str) -> Column {
        serde_json::from_value(serde_json::json!({ "name": name, "dataType": data_type })).expect("a column")
    }

    #[gpui::test]
    fn draws_a_console_with_a_script_that_stopped_partway(cx: &mut TestAppContext) {
        let _dir = cx.update(|cx| {
            cx.set_global(Theme::new(Appearance::Dark));
            cx.set_global(Locale(Language::En));
            test_backend(cx)
        });
        let sql = "SELECT 1;\nSELECT id, name FROM users;\nSELECT nope";
        let (console, cx) = cx.add_window_view(|_, cx| {
            QueryConsole::new("query:1".into(), "gone".into(), Engine::Sqlite, None, Some(sql.into()), cx)
        });
        console.update_in(cx, |console, window, cx| {
            // Nothing selected: ⌘↵ and Run run the whole script.
            assert_eq!(console.text_to_run(cx), sql);
            let rows = (0..1000).map(|n| vec![Some(n.to_string()), (n % 7 != 0).then(|| format!("user {n}"))]);
            let outcome = QueryOutcome {
                results: vec![
                    StatementResult { columns: Vec::new(), rows: Vec::new(), truncated: false, rows_affected: 0 },
                    StatementResult {
                        columns: vec![column("id", "INTEGER"), column("name", "TEXT")],
                        rows: rows.collect(),
                        truncated: false,
                        rows_affected: 1000,
                    },
                ],
                elapsed_ms: 12,
                error: Some("no such column: nope".into()),
            };
            console.results.update(cx, |results, cx| results.show(LastRun::Outcome(outcome), cx));
            console.toggle_history(window, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        console.update_in(cx, |console, _, cx| {
            // The error that stopped the script shows first, after the results before it.
            console.results.update(cx, |results, cx| results.select(1, cx));
        });
        cx.update(|window, cx| window.draw(cx).clear());
        console.read_with(cx, |console, cx| {
            assert_eq!(console.results.read(cx).grid().read(cx).row_count(), 1000);
            assert!(console.history.is_some());
        });
    }

    #[gpui::test]
    fn draws_an_explained_statement_s_plan_starting_on_its_costliest_node(cx: &mut TestAppContext) {
        let _dir = cx.update(|cx| {
            cx.set_global(Theme::new(Appearance::Light));
            cx.set_global(Locale(Language::Es));
            test_backend(cx)
        });
        let (console, cx) = cx.add_window_view(|_, cx| {
            QueryConsole::new("query:1".into(), "gone".into(), Engine::Sqlite, None, Some("SELECT 1".into()), cx)
        });
        let plan = serde_json::json!([{
            "Plan": {
                "Node Type": "Hash Join", "Total Cost": 300, "Plan Rows": 500,
                "Actual Total Time": 30, "Actual Rows": 480, "Actual Loops": 1,
                "Plans": [
                    { "Node Type": "Seq Scan", "Relation Name": "orders", "Alias": "o", "Total Cost": 200,
                      "Plan Rows": 10, "Actual Total Time": 24, "Actual Rows": 480, "Actual Loops": 1,
                      "Filter": "(total > 100)", "Rows Removed by Filter": 50000 },
                    { "Node Type": "Index Scan", "Relation Name": "customers", "Index Name": "customers_pkey",
                      "Total Cost": 40, "Plan Rows": 1, "Actual Total Time": 0.002, "Actual Rows": 1,
                      "Actual Loops": 480 }
                ]
            },
            "Planning Time": 0.9, "Execution Time": 30.5
        }]);
        console.update(cx, |console, cx| {
            console.set_analyze(true, cx);
            console.results.update(cx, |results, cx| results.show(LastRun::Plan(Rc::new(plan)), cx));
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let view = console.read_with(cx, |console, cx| console.results.read(cx).plan().cloned().expect("a plan"));
        assert_eq!(view.read_with(cx, |view, _| view.selected()), 1, "the scan takes 80% of the time");
        view.update(cx, |view, cx| view.select(2, cx));
        cx.update(|window, cx| window.draw(cx).clear());
        // Nothing to explain while the editor is empty, and nothing runs twice.
        console.update(cx, |console, cx| {
            console.editor().update(cx, |editor, cx| editor.set_text("  ", cx));
            let sql = console.text_to_run(cx);
            console.explain(sql, cx);
            assert!(!console.is_explaining());
        });
    }
}
