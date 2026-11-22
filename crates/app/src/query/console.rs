//! A query console: the database it queries (and for Cosmos DB the
//! container), the editor, the results of its last run below it and, on
//! ⌘Y, the Library beside them (its history and the saved queries). Each console has its own core session (a
//! connection kept open between runs, so a transaction or a `SET` carries
//! over); closing its tab closes it. SQL engines' consoles complete the
//! database's names; MongoDB's takes mongosh, Cosmos DB's SQL over one
//! container, picked beside the database.
//!
//! ⌘↵ (Ctrl+Enter) and the Run button run the selection, or the whole
//! script, as the web console did. Explain shows the plan of the same text
//! instead; with Analyze it runs the statement to measure it, inside a
//! transaction the core rolls back.
//!
//! ⌘S saves the console as a saved query: the first time a form under the
//! toolbar asks for its name, folder and scope; once it is one (or when it
//! was opened from one) ⌘S updates it, its tab takes its name, and the tab
//! shows a dot while the text differs from the saved one. Running it with
//! `:parameters` asks for their values first.

use crate::ui::Ellipsis;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::{ObjectKind, StatementResult};
use dbforge_core::explorer::{completion_schema, list_objects};
use dbforge_core::history::HistoryEntry;
use dbforge_core::query::{QueryScope, cancel_query, close_session, explain_query, run_query};
use dbforge_core::saved_queries::params::parameters;
use dbforge_core::saved_queries::{SavedQuery, SavedQueryInput};
use gpui::{
    AnyElement, AnyView, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Pixels, Point, Render, StatefulInteractiveElement,
    Styled, Subscription, Window, actions, deferred, div, linear_color_stop, linear_gradient, point,
    prelude::FluentBuilder, px,
};

use super::plan::parse_plan;
use super::results::{LastRun, ResultsEvent, ResultsPanel};
use super::shortcuts;
use crate::backend::Backend;
use crate::connections::engine::is_relational;
use crate::i18n::Translate;
use crate::saved::rules::{Place, is_edited, run_connection};
use crate::saved::store::{self, SavedStore};
use crate::saved::{Draft, LibraryEvent, LibraryPanel, RunAction, SaveForm, SaveFormEvent};
use crate::sql::Syntax;
use crate::sql::completion::CompletionNames;
use crate::sql::statements::changes_structure;
use crate::theme::{ActiveTheme, color};
use crate::ui::field::Tooltip;
use crate::ui::notice::{Tone, notify};
use crate::ui::{
    Checkbox, CodeEditor, CodeEditorEvent, MenuItem, PopoverMenu, any, button, capsule, card, enabled, group, icon,
    popover, segment, shadow,
};

actions!(query_console, [Run, ToggleLibrary, SaveQuery]);

const CONTEXT: &str = "QueryConsole";

/// How often the running time is redrawn.
const TICK: Duration = Duration::from_millis(100);

/// Space between the console's cards.
const GAP: f32 = 10.;

pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![
        KeyBinding::new("secondary-enter", Run, context),
        KeyBinding::new("secondary-y", ToggleLibrary, context),
        KeyBinding::new("secondary-s", SaveQuery, context),
    ]
}

/// What a console asks of the workspace.
#[derive(Debug, Clone)]
pub enum ConsoleEvent {
    /// Open this past run in a new console on its own connection.
    OpenQuery(HistoryEntry),
    /// Export this result's rows.
    ExportResult(StatementResult),
    /// The console is this saved query (saved, opened, renamed): its tab takes its name.
    Saved(SavedQuery),
    /// Whether its text differs from its saved query changed: its tab's dot.
    EditedChanged,
    /// Run this saved query, asking for its parameters first.
    RunSaved(SavedQuery),
    /// Open this saved query in a new tab.
    OpenSaved(SavedQuery),
    /// Ask for the values of `sql`'s parameters (the console's text, of its
    /// saved query), then run or explain it here.
    AskParameters { query: SavedQuery, sql: String, action: RunAction },
    /// Show this dialog over the window, or close the one shown.
    Dialog(Option<AnyView>),
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
    syntax: Syntax,
    /// The database queried; the connection's own when `None`.
    database: Option<String>,
    /// The databases the picker offers; `None` while they load.
    databases: Option<Vec<String>>,
    /// Cosmos DB: the database's containers; `None` while they load, empty
    /// without a database.
    containers: Option<Vec<String>>,
    /// Cosmos DB: the container picked; until one is, the database's first.
    picked_container: Option<String>,
    /// Counts container listings, so a stale one is dropped.
    containers_generation: u64,
    editor: Entity<CodeEditor>,
    results: Entity<ResultsPanel>,
    /// The Library beside the console, and what the console hears from it.
    library: Option<(Entity<LibraryPanel>, Subscription)>,
    /// The saved query the console is, once saved or when opened from one.
    saved: Option<SavedQuery>,
    /// Its text differs from its saved query's.
    edited: bool,
    /// The form saving it the first time, open.
    save_form: Option<(Entity<SaveForm>, [Subscription; 2])>,
    running: Option<Running>,
    /// An explain going now.
    explaining: bool,
    /// Explain runs the statement to measure it (in a transaction rolled back).
    analyze: bool,
    cancel_error: Option<String>,
    /// The database's or the container's picker, open.
    picker_menu: Option<(Entity<PopoverMenu>, Point<Pixels>, Subscription)>,
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
    /// A console `session_id` on `connection_id` (an `engine` connection)
    /// and `scope`, its editor starting with `sql`.
    pub fn new(
        session_id: String,
        connection_id: String,
        engine: Engine,
        scope: QueryScope,
        sql: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let syntax = Syntax::console(engine);
        let editor = cx.new(|cx| CodeEditor::new(sql.as_deref().unwrap_or(""), syntax, cx));
        let results = cx.new(ResultsPanel::new);
        let exporting = cx.subscribe(&results, |_, _, ResultsEvent::Export(result): &ResultsEvent, cx| {
            cx.emit(ConsoleEvent::ExportResult(result.clone()));
        });
        let typing = cx.subscribe(&editor, |this, _, CodeEditorEvent::Changed: &CodeEditorEvent, cx| {
            this.sync_edited(cx);
        });
        let store = SavedStore::global(cx);
        let following = cx.observe(&store, Self::follow_saved);
        let mut console = Self {
            session_id,
            connection_id,
            engine,
            syntax,
            database: scope.database,
            databases: None,
            containers: None,
            picked_container: scope.container,
            containers_generation: 0,
            editor,
            results,
            library: None,
            saved: None,
            edited: false,
            save_form: None,
            running: None,
            explaining: false,
            analyze: false,
            cancel_error: None,
            picker_menu: None,
            names_generation: 0,
            focus: cx.focus_handle(),
            _subscriptions: vec![exporting, typing, following],
        };
        console.load_databases(cx);
        console.load_containers(cx);
        console.load_names(cx);
        console
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    pub fn engine(&self) -> Engine {
        self.engine
    }

    /// The saved query the console is, if it is one.
    pub fn saved_query(&self) -> Option<&SavedQuery> {
        self.saved.as_ref()
    }

    /// Its text differs from its saved query's (its tab shows a dot).
    pub fn is_edited(&self) -> bool {
        self.edited
    }

    pub fn library(&self) -> Option<&Entity<LibraryPanel>> {
        self.library.as_ref().map(|(library, _)| library)
    }

    pub fn save_form(&self) -> Option<&Entity<SaveForm>> {
        self.save_form.as_ref().map(|(form, _)| form)
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

    /// Where the next run goes: the database and, for Cosmos DB, the container.
    pub fn scope(&self) -> QueryScope {
        QueryScope { database: self.database.clone(), container: self.container() }
    }

    /// Cosmos DB: the container queried, the picked one or else the database's first.
    pub fn container(&self) -> Option<String> {
        if self.engine != Engine::Cosmosdb {
            return None;
        }
        self.picked_container.clone().or_else(|| self.containers.as_ref()?.first().cloned())
    }

    /// Cosmos DB: the containers the picker offers; `None` while they load.
    pub fn containers(&self) -> Option<&[String]> {
        self.containers.as_deref()
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

    /// Cosmos DB: the containers of the database queried, for the picker.
    /// Without a database there is nothing to list; a failed listing offers none.
    fn load_containers(&mut self, cx: &mut Context<Self>) {
        if self.engine != Engine::Cosmosdb {
            return;
        }
        self.containers_generation += 1;
        let generation = self.containers_generation;
        let Some(database) = self.database.clone().filter(|database| !database.is_empty()) else {
            self.containers = Some(Vec::new());
            return;
        };
        self.containers = None;
        let connection = self.connection_id.clone();
        let listing = Backend::global(cx).spawn(cx, move |core| async move {
            list_objects(core.connections(), core.statement_log(), connection, vec![database]).await
        });
        cx.spawn(async move |this, cx| {
            let containers = listing.await.map(|objects| objects.into_iter().map(|o| o.name).collect());
            let _ = this.update(cx, |this, cx| {
                if this.containers_generation == generation {
                    this.containers = Some(containers.unwrap_or_default());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Reads the names the editor completes; a failure leaves keywords
    /// alone. Only SQL engines' tables have names to complete.
    fn load_names(&mut self, cx: &mut Context<Self>) {
        self.names_generation += 1;
        let Some(dialect) = self.syntax.dialect().filter(|_| is_relational(self.engine)) else { return };
        let generation = self.names_generation;
        let (connection, database) = (self.connection_id.clone(), self.database.clone());
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

    /// Queries `database` from the next run on; a container picked in the
    /// previous one is not in it.
    pub fn pick_database(&mut self, database: Option<String>, cx: &mut Context<Self>) {
        if self.database != database {
            self.database = database;
            self.picked_container = None;
            self.load_containers(cx);
            self.load_names(cx);
            cx.notify();
        }
    }

    /// Cosmos DB: queries `container` from the next run on.
    pub fn pick_container(&mut self, container: String, cx: &mut Context<Self>) {
        self.picked_container = Some(container);
        cx.notify();
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
                if let Some((library, _)) = &this.library {
                    library.update(cx, |library, cx| library.reload_history(cx));
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

    /// Shows or hides the Library beside the console.
    pub fn toggle_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.library.take().is_some() {
            self.focus(window, cx);
        } else {
            let (connection, engine) = (self.connection_id.clone(), self.engine);
            let library = cx.new(|cx| LibraryPanel::new(connection, engine, window, cx));
            let events = cx.subscribe_in(&library, window, Self::on_library);
            self.library = Some((library, events));
        }
        cx.notify();
    }

    fn on_library(
        &mut self,
        _: &Entity<LibraryPanel>,
        event: &LibraryEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            LibraryEvent::LoadRun(entry) => self.load(entry, window, cx),
            LibraryEvent::OpenRun(entry) => {
                // A run whose connection was deleted can't open; it loads here instead.
                let exists =
                    Backend::global(cx).core().connections().list().iter().any(|c| c.id == entry.connection_id);
                if exists {
                    cx.emit(ConsoleEvent::OpenQuery(entry.clone()));
                } else {
                    self.load(entry, window, cx);
                }
            }
            // A query of another connection opens on its own.
            LibraryEvent::LoadSaved(query) => {
                let place = Place { connection_id: &self.connection_id, engine: self.engine };
                if run_connection(query, Some(place)).as_deref() == Some(self.connection_id.as_str()) {
                    self.load_saved(query.clone(), window, cx);
                } else {
                    cx.emit(ConsoleEvent::OpenSaved(query.clone()));
                }
            }
            LibraryEvent::RunSaved(query) => cx.emit(ConsoleEvent::RunSaved(query.clone())),
            LibraryEvent::OpenSaved(query) => cx.emit(ConsoleEvent::OpenSaved(query.clone())),
            LibraryEvent::Dialog(dialog) => cx.emit(ConsoleEvent::Dialog(dialog.clone())),
            LibraryEvent::Close => {
                self.library = None;
                self.focus(window, cx);
                cx.notify();
            }
        }
    }

    /// Makes the console saved query `query`: ⌘S updates it from then on.
    pub fn link_saved(&mut self, query: SavedQuery, cx: &mut Context<Self>) {
        self.saved = Some(query.clone());
        self.sync_edited(cx);
        cx.emit(ConsoleEvent::Saved(query));
        cx.notify();
    }

    /// Puts saved query `query` in the editor, the console becoming it.
    pub fn load_saved(&mut self, query: SavedQuery, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| editor.set_text(&query.sql, cx));
        self.link_saved(query, cx);
        self.focus(window, cx);
    }

    /// Tells the tab when the text starts or stops differing from the saved query's.
    fn sync_edited(&mut self, cx: &mut Context<Self>) {
        let edited = is_edited(self.saved.as_ref().map(|query| query.sql.as_str()), self.editor.read(cx).text());
        if edited != self.edited {
            self.edited = edited;
            cx.emit(ConsoleEvent::EditedChanged);
            cx.notify();
        }
    }

    /// Follows the console's saved query as the list changes: renamed or
    /// changed elsewhere, its tab follows; deleted, the console is a plain one again.
    fn follow_saved(&mut self, store: Entity<SavedStore>, cx: &mut Context<Self>) {
        let Some(linked) = &self.saved else { return };
        let store = store.read(cx);
        if !store.is_loaded() {
            return;
        }
        match store.get(&linked.id).cloned() {
            Some(query) if query != *linked => {
                let renamed = query.name != linked.name;
                self.saved = Some(query.clone());
                self.sync_edited(cx);
                if renamed {
                    cx.emit(ConsoleEvent::Saved(query));
                }
            }
            Some(_) => {}
            None => {
                self.saved = None;
                self.sync_edited(cx);
            }
        }
    }

    /// ⌘S: updates the console's saved query with its text, or asks how to
    /// save it the first time.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.saved.clone() else {
            self.open_save_form(window, cx);
            return;
        };
        let input = SavedQueryInput {
            name: query.name,
            sql: self.editor.read(cx).text().to_owned(),
            description: query.description,
            folder: query.folder,
            scope: query.scope,
            engine: query.engine,
        };
        let saving = store::save(Some(query.id), input, cx);
        cx.spawn(async move |this, cx| match saving.await {
            Ok(query) => {
                let _ = this.update(cx, |this, cx| {
                    this.saved = Some(query);
                    this.sync_edited(cx);
                });
            }
            Err(error) => {
                let _ = cx.update(|cx| notify(error, Tone::Error, cx));
            }
        })
        .detach();
    }

    /// The form asking how to save the console, under its toolbar.
    fn open_save_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((form, _)) = &self.save_form {
            form.read(cx).focus(window, cx);
            return;
        }
        let connection_name = Backend::global(cx)
            .core()
            .connections()
            .list()
            .into_iter()
            .find(|connection| connection.id == self.connection_id)
            .map(|connection| connection.name)
            .unwrap_or_else(|| self.connection_id.clone());
        let draft = Draft {
            connection_id: self.connection_id.clone(),
            connection_name,
            engine: self.engine,
            sql: self.editor.read(cx).text().to_owned(),
        };
        let form = cx.new(|cx| SaveForm::new(draft, window, cx));
        form.read(cx).focus(window, cx);
        let saved = cx.subscribe(&form, |this, _, SaveFormEvent::Saved(query): &SaveFormEvent, cx| {
            this.link_saved(query.clone(), cx);
        });
        let closed = cx.subscribe_in(&form, window, |this, _, _: &DismissEvent, window, cx| {
            this.save_form = None;
            this.focus(window, cx);
            cx.notify();
        });
        self.save_form = Some((form, [saved, closed]));
        cx.notify();
    }

    /// ⌘↵ and Run: the selection, or the whole script. A saved query's
    /// `:parameters` are asked for first.
    pub fn run_requested(&mut self, cx: &mut Context<Self>) {
        let sql = self.text_to_run(cx);
        if self.asks_parameters(&sql, RunAction::Run, cx) {
            return;
        }
        if let Some(query) = &self.saved {
            store::mark_run(query.id.clone(), None, cx).detach();
        }
        self.run(sql, cx);
    }

    /// Explain: the selection, or the whole script, its `:parameters` asked for first like Run's.
    pub fn explain_requested(&mut self, cx: &mut Context<Self>) {
        let sql = self.text_to_run(cx);
        if !self.asks_parameters(&sql, RunAction::Explain, cx) {
            self.explain(sql, cx);
        }
    }

    /// Whether `sql` waits for its saved query's parameters before `action`:
    /// then the values are asked for and the workspace finishes it.
    fn asks_parameters(&self, sql: &str, action: RunAction, cx: &mut Context<Self>) -> bool {
        let Some(query) = self.saved.clone() else { return false };
        if sql.trim().is_empty() || self.busy() || parameters(sql, self.engine).is_empty() {
            return false;
        }
        cx.emit(ConsoleEvent::AskParameters { query, sql: sql.to_owned(), action });
        true
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
        self.open_picker(items, position, window, cx);
    }

    fn open_container_menu(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(containers) = self.containers.clone() else { return };
        let this = cx.entity().downgrade();
        let current = self.container();
        let items = containers
            .into_iter()
            .map(|name| {
                let this = this.clone();
                let checked = current.as_ref() == Some(&name);
                MenuItem::new(name.clone(), move |_, cx| {
                    let name = name.clone();
                    let _ = this.update(cx, |this, cx| this.pick_container(name, cx));
                })
                .checked(checked)
            })
            .collect();
        self.open_picker(items, position, window, cx);
    }

    /// Shows a picker's choices under it; closing it gives the editor the keyboard back.
    fn open_picker(
        &mut self,
        items: Vec<MenuItem>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = cx.new(|cx| PopoverMenu::new(items, window, cx));
        let closing = cx.subscribe_in(&menu, window, |this, _, _: &gpui::DismissEvent, window, cx| {
            this.picker_menu = None;
            this.focus(window, cx);
            cx.notify();
        });
        self.picker_menu = Some((menu, position, closing));
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
            .child(div().min_w_0().ellipsis().child(name))
            .when(self.engine != Engine::Sqlite, |picker| picker.child(icon("down", theme.text3, 12.)));
        if !pickable {
            return picker.into_any_element();
        }
        picker
            .cursor_pointer()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    if this.picker_menu.is_none() {
                        this.open_database_menu(event.position + point(px(0.), px(12.)), window, cx);
                    }
                }),
            )
            .into_any_element()
    }

    /// Cosmos DB: the container queried, picked from the database's.
    fn container_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.engine != Engine::Cosmosdb {
            return None;
        }
        let theme = *cx.theme();
        let m = &cx.messages().query;
        let containers = self.containers.as_deref();
        let none = containers.is_some_and(<[String]>::is_empty);
        let name = if none { m.no_containers.to_owned() } else { self.container().unwrap_or_default() };
        let pickable = !self.busy() && containers.is_some_and(|containers| !containers.is_empty());
        let picker = capsule(&theme)
            .id("container-picker")
            .max_w(px(240.))
            .text_color(if none { theme.text2 } else { theme.text })
            .tooltip(Tooltip::text(m.container_to_query))
            .child(icon("table", theme.text3, 14.))
            .child(div().min_w_0().ellipsis().child(name))
            .child(icon("down", theme.text3, 12.));
        if !pickable {
            return Some(picker.into_any_element());
        }
        Some(
            picker
                .cursor_pointer()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                        if this.picker_menu.is_none() {
                            this.open_container_menu(event.position + point(px(0.), px(12.)), window, cx);
                        }
                    }),
                )
                .into_any_element(),
        )
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
                            run.on_click(cx.listener(|this, _, _, cx| this.run_requested(cx)))
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
                .when(on, |explain| explain.on_click(cx.listener(|this, _, _, cx| this.explain_requested(cx)))))],
        )
        .into_any_element()
    }

    /// What Save does: save the console, or update the saved query it is.
    fn save_tooltip(&self, cx: &gpui::App) -> String {
        let m = &cx.messages().saved;
        let key = crate::ui::shortcut("s");
        match &self.saved {
            Some(query) => (m.save_changes_tooltip)(&query.name, &key),
            None => (m.save_tooltip)(&key),
        }
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
            .children(self.container_picker(cx))
            .child(div().flex_1())
            // The cancel error keeps to one line, cut short, so nothing moves.
            .children(self.cancel_error.as_ref().map(|error| {
                div()
                    .id("cancel-error")
                    .min_w_0()
                    .max_w(px(360.))
                    .ellipsis()
                    .text_size(px(12.))
                    .text_color(theme.err)
                    .tooltip(Tooltip::text(error.clone()))
                    .child((m.could_not_cancel)(error))
            }))
            .child(group(
                &theme,
                vec![
                    any(segment(&theme, None, cx.messages().saved.save, self.save_form.is_some())
                        .id("save-query")
                        .cursor_pointer()
                        .tooltip(Tooltip::text(self.save_tooltip(cx)))
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))),
                    any(segment(&theme, Some("clock"), m.library, self.library.is_some())
                        .id("toggle-library")
                        .cursor_pointer()
                        .tooltip(Tooltip::text((m.library_title)(shortcuts::LIBRARY)))
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_library(window, cx)))),
                ],
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
            .on_action(cx.listener(|this, _: &Run, _, cx| this.run_requested(cx)))
            .on_action(cx.listener(|this, _: &ToggleLibrary, window, cx| this.toggle_library(window, cx)))
            .on_action(cx.listener(|this, _: &SaveQuery, window, cx| this.save(window, cx)))
            .relative()
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
                    // A card fills the space left; the Library's keeps its own width.
                    .children(
                        self.library()
                            .cloned()
                            .map(|library| card(&theme).flex_none().flex_basis(px(crate::saved::WIDTH)).child(library)),
                    ),
            )
            .children(self.picker_menu.as_ref().map(|(menu, position, _)| popover(menu.clone(), *position)))
            // Under the toolbar's Save, over the editor.
            .children(
                self.save_form()
                    .cloned()
                    .map(|form| deferred(div().absolute().top(px(42.)).right(px(150.)).child(form)).with_priority(1)),
            )
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
            QueryConsole::new(
                "query:1".into(),
                "gone".into(),
                Engine::Sqlite,
                QueryScope::default(),
                Some(sql.into()),
                cx,
            )
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
            console.toggle_library(window, cx);
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
            assert!(console.library.is_some());
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
            QueryConsole::new(
                "query:1".into(),
                "gone".into(),
                Engine::Sqlite,
                QueryScope::default(),
                Some("SELECT 1".into()),
                cx,
            )
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

    #[gpui::test]
    fn queries_a_cosmos_db_container_picked_beside_the_database(cx: &mut TestAppContext) {
        let _dir = cx.update(|cx| {
            cx.set_global(Theme::new(Appearance::Dark));
            cx.set_global(Locale(Language::Es));
            test_backend(cx)
        });
        let scope = QueryScope { database: Some("shop".into()), container: Some("orders".into()) };
        let (console, cx) = cx.add_window_view(|_, cx| {
            QueryConsole::new("query:1".into(), "gone".into(), Engine::Cosmosdb, scope.clone(), None, cx)
        });
        console.read_with(cx, |console, cx| {
            assert_eq!(console.scope(), scope, "the container it was opened on");
            assert_eq!(console.editor().read(cx).syntax(), Syntax::Sql(crate::sql::Dialect::Cosmos));
        });
        cx.update(|window, cx| window.draw(cx).clear());
        // Another database: the container picked in the previous one is not in it.
        console.update(cx, |console, cx| console.pick_database(Some("sales".into()), cx));
        console.read_with(cx, |console, _| assert_eq!(console.scope().container, None));
        // Its listing fails (the connection is gone): no containers to pick.
        for _ in 0..400 {
            cx.run_until_parked();
            if console.read_with(cx, |console, _| console.containers().is_some()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        console.read_with(cx, |console, _| assert_eq!(console.containers(), Some(&[][..])));
        cx.update(|window, cx| window.draw(cx).clear());
        console.update(cx, |console, cx| console.pick_container("orders".into(), cx));
        console.read_with(cx, |console, _| assert_eq!(console.scope().container.as_deref(), Some("orders")));
    }

    #[gpui::test]
    fn writes_mongodb_s_console_in_mongosh(cx: &mut TestAppContext) {
        let _dir = cx.update(|cx| {
            cx.set_global(Theme::new(Appearance::Light));
            cx.set_global(Locale(Language::En));
            test_backend(cx)
        });
        let find = "db.getCollection(\"orders\").find({}).limit(100) // the first ones";
        let (console, cx) = cx.add_window_view(|_, cx| {
            QueryConsole::new(
                "query:1".into(),
                "gone".into(),
                Engine::Mongodb,
                QueryScope::default(),
                Some(find.into()),
                cx,
            )
        });
        cx.update(|window, cx| window.draw(cx).clear());
        console.read_with(cx, |console, cx| {
            assert_eq!(console.editor().read(cx).syntax(), Syntax::Mongosh);
            assert_eq!(console.scope(), QueryScope::default(), "no container outside Cosmos DB");
            assert_eq!(console.text_to_run(cx), find);
        });
    }
}
