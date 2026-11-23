//! The main area: the tab strip and the open tab under it (a query console,
//! a table, a schema's diagram, a Redis database's keys). With no tab open,
//! it says how to open one. Tabs that aren't shown keep their state (a
//! console's text and results, a table's page and pending changes) until
//! they close. A dialog a tab opens (a table's PROD question) goes to the
//! window as a [`WorkspaceEvent`].
//!
//! Saved queries open and run here: in a console opened from the query (an
//! open one, else a new tab titled with its name) on the connection it was
//! saved for, or for one saved for any connection of its engine, on the
//! connection at hand. One with `:parameters` asks for their values first.
//!
//! Closing a console's tab with a transaction open asks first whether to
//! roll it back or commit it; the tab closes once that is done.

mod strip;
pub mod tabs;

use std::collections::{BTreeMap, HashMap};

use dbforge_core::connections::model::{Connection, Engine};
use dbforge_core::drivers::{RowFilter, TableRef};
use dbforge_core::query::{QueryScope, open_transactions};
use dbforge_core::saved_queries::params::parameters;
use dbforge_core::saved_queries::{ParamValue, SavedQuery};
use gpui::{
    AnyView, AppContext, Context, DismissEvent, Entity, EventEmitter, FontWeight, IntoElement, ParentElement, Render,
    Styled, Subscription, Window, div, px,
};

use self::tabs::{TabKind, TableName, Tabs};
use crate::backend::Backend;
use crate::diagram::DiagramView;
use crate::export::ExportSource;
use crate::i18n::Translate;
use crate::import::ImportTarget;
use crate::keyspace::{KeyBrowser, KeyBrowserEvent};
use crate::query::{ConsoleEvent, QueryConsole};
use crate::saved::rules::{Place, run_connection};
use crate::saved::store;
use crate::saved::{RunAction, RunDialog, RunDialogEvent, RunRequest};
use crate::shell::OpenObject;
use crate::shell::explorer::tree::NodeKind;
use crate::sql::snippets::table_ref;
use crate::table::{TablePage, TablePageEvent, TableTabView};
use crate::theme::ActiveTheme;
use crate::transactions::{LeaveDialog, LeaveEvent, Leaving, Pending};
use crate::ui::card;
use crate::ui::notice::{Tone, notify};

/// Space between the strip and the tab's content.
const GAP: f32 = 10.;

/// Whether `engine` has a query console: SQL, mongosh for MongoDB, SQL
/// over a container for Cosmos DB. Redis's key browser holds its CLI.
pub fn has_query_console(engine: Engine) -> bool {
    engine != Engine::Redis
}

/// Where "New query" on a connection selected at `path` queries: the
/// database selected (else the connection's own) and, for Cosmos DB, the
/// container selected, if any.
pub fn target_scope(connection: &Connection, path: &[String]) -> QueryScope {
    let own = Some(connection.database.clone()).filter(|database| !database.is_empty());
    match connection.engine {
        Engine::Cosmosdb => QueryScope { database: path.first().cloned().or(own), container: path.get(1).cloned() },
        _ => QueryScope { database: path.first().cloned(), container: None },
    }
}

/// The Redis database "New query" opens in the key browser: the one
/// selected, else the connection's own (`db0` when it has none).
pub fn keyspace_target(connection: &Connection, path: &[String]) -> String {
    path.first().cloned().unwrap_or_else(|| {
        let own = if connection.database.is_empty() { "0" } else { connection.database.as_str() };
        format!("db{own}")
    })
}

/// An open console and what the workspace hears from it.
struct OpenConsole {
    console: Entity<QueryConsole>,
    _events: Subscription,
}

/// An open table and what the workspace hears from it.
struct OpenTable {
    page: Entity<TablePage>,
    _events: Subscription,
}

/// The dialog asking for a saved query's parameters, and what the workspace hears from it.
struct OpenRunDialog {
    dialog: Entity<RunDialog>,
    _events: Vec<Subscription>,
}

/// The dialog asking before a tab with a transaction open closes, and what the workspace hears from it.
struct OpenLeaveDialog {
    dialog: Entity<LeaveDialog>,
    _events: Vec<Subscription>,
}

/// An open key browser and what the workspace hears from it.
struct OpenKeyspace {
    browser: Entity<KeyBrowser>,
    _events: Subscription,
}

/// What the workspace asks of the window.
#[derive(Debug, Clone)]
pub enum WorkspaceEvent {
    /// Show this dialog over the window, or close the one shown.
    Dialog(Option<AnyView>),
    /// A tab asks to export these rows.
    Export(ExportSource),
    /// A table's tab asks to import a file's rows into it.
    Import(Box<ImportTarget>),
}

#[derive(Default)]
pub struct Workspace {
    tabs: Tabs,
    consoles: HashMap<String, OpenConsole>,
    tables: HashMap<String, OpenTable>,
    diagrams: HashMap<String, Entity<DiagramView>>,
    keyspaces: HashMap<String, OpenKeyspace>,
    /// The connection selected in the explorer, and where in it: what "New query" queries.
    target: Option<(Connection, Vec<String>)>,
    run_dialog: Option<OpenRunDialog>,
    leave_dialog: Option<OpenLeaveDialog>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Workspace {
    pub fn tabs(&self) -> &Tabs {
        &self.tabs
    }

    /// The console of tab `id`, if it is one.
    pub fn console(&self, id: &str) -> Option<&Entity<QueryConsole>> {
        self.consoles.get(id).map(|open| &open.console)
    }

    /// The connection (and where in it) a "New query" opens on.
    pub fn set_target(&mut self, target: Option<(Connection, Vec<String>)>, cx: &mut Context<Self>) {
        self.target = target;
        cx.notify();
    }

    /// Whether "New query" can open now: a connection is selected.
    pub fn can_query(&self) -> bool {
        self.target.is_some()
    }

    /// "New query" (the strip's +, the palette): a console on the selected
    /// connection and database (and Cosmos DB container); on Redis, which
    /// has no console, the selected database's key browser, which holds the CLI.
    pub fn new_query_on_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((connection, path)) = self.target.clone() else { return };
        if has_query_console(connection.engine) {
            self.new_query(&connection.id, target_scope(&connection, &path), None, window, cx);
        } else {
            self.open_keyspace(&connection.id, &keyspace_target(&connection, &path), window, cx);
        }
    }

    /// Opens a console on `connection_id` and `scope`, its editor starting
    /// with `sql`, and gives it the keyboard. Nothing for an engine without
    /// a console, or a connection that is gone.
    pub fn new_query(
        &mut self,
        connection_id: &str,
        scope: QueryScope,
        sql: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<QueryConsole>> {
        let connection = Backend::global(cx).core().connections().list().into_iter().find(|c| c.id == connection_id)?;
        if !has_query_console(connection.engine) {
            return None;
        }
        let tab = self.tabs.new_query(cx.messages().query.query_tab, connection_id, scope, sql).clone();
        let TabKind::Query { scope, initial_sql } = tab.kind else { return None };
        let console = cx.new(|cx| {
            QueryConsole::new(tab.id.clone(), tab.connection_id.clone(), connection.engine, scope, initial_sql, cx)
        });
        let events = cx.subscribe_in(&console, window, Self::on_console);
        self.consoles.insert(tab.id, OpenConsole { console: console.clone(), _events: events });
        console.read(cx).focus(window, cx);
        cx.notify();
        Some(console)
    }

    fn on_console(
        &mut self,
        console: &Entity<QueryConsole>,
        event: &ConsoleEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (tab, connection_id, engine) = {
            let console = console.read(cx);
            (console.session_id().to_owned(), console.connection_id().to_owned(), console.engine())
        };
        match event {
            ConsoleEvent::OpenQuery(entry) => {
                let scope = QueryScope { database: entry.database.clone(), container: entry.container.clone() };
                self.new_query(&entry.connection_id, scope, Some(entry.sql.clone()), window, cx);
            }
            ConsoleEvent::ExportResult(result) => cx.emit(WorkspaceEvent::Export(ExportSource::Rows {
                name: "query_result".into(),
                columns: result.columns.clone(),
                rows: std::sync::Arc::new(result.rows.clone()),
            })),
            ConsoleEvent::Saved(query) => {
                self.tabs.set_title(&tab, query.name.clone());
                cx.notify();
            }
            ConsoleEvent::EditedChanged | ConsoleEvent::TransactionChanged => cx.notify(),
            ConsoleEvent::RunSaved(query) => self.run_saved(query.clone(), Some((connection_id, engine)), window, cx),
            ConsoleEvent::OpenSaved(query) => {
                self.open_saved(query.clone(), Some((connection_id, engine)), window, cx);
            }
            ConsoleEvent::AskParameters { query, sql, action } => {
                let connection_name = self.connection_name(&connection_id, cx);
                let request = RunRequest {
                    query: query.clone(),
                    sql: sql.clone(),
                    connection_id,
                    connection_name,
                    console: Some(tab),
                    action: *action,
                };
                self.ask_parameters(request, window, cx);
            }
            ConsoleEvent::Dialog(dialog) => cx.emit(WorkspaceEvent::Dialog(dialog.clone())),
        }
    }

    fn connection_name(&self, connection_id: &str, cx: &gpui::App) -> String {
        Backend::global(cx)
            .core()
            .connections()
            .list()
            .into_iter()
            .find(|connection| connection.id == connection_id)
            .map_or_else(|| connection_id.to_owned(), |connection| connection.name)
    }

    /// The connection saved query `query` opens or runs on, seen `from` a
    /// console's connection (or else the one selected); why none, otherwise.
    fn saved_connection(
        &self,
        query: &SavedQuery,
        from: Option<(String, Engine)>,
        cx: &gpui::App,
    ) -> Result<Connection, String> {
        let m = &cx.messages().saved;
        let from =
            from.or_else(|| self.target.as_ref().map(|(connection, _)| (connection.id.clone(), connection.engine)));
        let place = from.as_ref().map(|(connection_id, engine)| Place { connection_id, engine: *engine });
        let id = run_connection(query, place).ok_or_else(|| (m.no_connection_to_run)(query.engine.label()))?;
        Backend::global(cx)
            .core()
            .connections()
            .list()
            .into_iter()
            .find(|connection| connection.id == id)
            .ok_or_else(|| m.connection_gone.to_owned())
    }

    /// Opens saved query `query` in a new tab, titled with its name, on its
    /// connection (seen `from` a console's, or else the one selected).
    pub fn open_saved(
        &mut self,
        query: SavedQuery,
        from: Option<(String, Engine)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<QueryConsole>> {
        match self.saved_connection(&query, from, cx) {
            Ok(connection) => self.open_saved_on(query, &connection.id, window, cx),
            Err(message) => {
                notify(message, Tone::Error, cx);
                None
            }
        }
    }

    fn open_saved_on(
        &mut self,
        query: SavedQuery,
        connection_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<QueryConsole>> {
        let console = self.new_query(connection_id, QueryScope::default(), Some(query.sql.clone()), window, cx)?;
        console.update(cx, |console, cx| console.link_saved(query, cx));
        Some(console)
    }

    /// Runs saved query `query` on its connection (seen `from` a console's,
    /// or else the one selected), asking for its parameters first.
    pub fn run_saved(
        &mut self,
        query: SavedQuery,
        from: Option<(String, Engine)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection = match self.saved_connection(&query, from, cx) {
            Ok(connection) => connection,
            Err(message) => return notify(message, Tone::Error, cx),
        };
        let request = RunRequest {
            sql: query.sql.clone(),
            query,
            connection_id: connection.id,
            connection_name: connection.name,
            console: None,
            action: RunAction::Run,
        };
        if parameters(&request.sql, request.query.engine).is_empty() {
            let sql = request.sql.clone();
            self.finish_run(request, sql, None, window, cx);
        } else {
            self.ask_parameters(request, window, cx);
        }
    }

    /// The dialog asking for the parameters of `request`, if one is open.
    pub fn run_dialog(&self) -> Option<&Entity<RunDialog>> {
        self.run_dialog.as_ref().map(|open| &open.dialog)
    }

    /// Asks for the values of `request`'s parameters, then runs it.
    fn ask_parameters(&mut self, request: RunRequest, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| RunDialog::new(request, window, cx));
        dialog.read(cx).focus(window, cx);
        let ran = cx.subscribe_in(&dialog, window, |this, _, event: &RunDialogEvent, window, cx| {
            let RunDialogEvent::Run { request, sql, values, remember } = event;
            let remembered = remember.then(|| values.clone());
            this.finish_run(request.clone(), sql.clone(), remembered, window, cx);
        });
        let closed = cx.subscribe_in(&dialog, window, |this, _, _: &DismissEvent, window, cx| {
            this.run_dialog = None;
            cx.emit(WorkspaceEvent::Dialog(None));
            if let Some(id) = this.tabs.active_id().map(str::to_owned) {
                this.activate(&id, window, cx);
            }
        });
        self.run_dialog = Some(OpenRunDialog { dialog: dialog.clone(), _events: vec![ran, closed] });
        cx.emit(WorkspaceEvent::Dialog(Some(dialog.into())));
    }

    /// Runs `sql` (`request`'s query, its values written in) in the console
    /// that asked, else one opened from the query on its connection, else a
    /// new one; then records the run, remembering `values` when given.
    fn finish_run(
        &mut self,
        request: RunRequest,
        sql: String,
        values: Option<BTreeMap<String, ParamValue>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let console = request
            .console
            .as_deref()
            .and_then(|id| self.console(id).cloned())
            .or_else(|| self.saved_console(&request.query.id, &request.connection_id, cx))
            .or_else(|| self.open_saved_on(request.query.clone(), &request.connection_id, window, cx));
        let Some(console) = console else {
            return notify(cx.messages().saved.connection_gone, Tone::Error, cx);
        };
        let id = console.read(cx).session_id().to_owned();
        self.activate(&id, window, cx);
        match request.action {
            RunAction::Run => console.update(cx, |console, cx| console.run(sql, cx)),
            RunAction::Explain => console.update(cx, |console, cx| console.explain(sql, cx)),
        }
        store::mark_run(request.query.id, values, cx).detach();
    }

    /// The first console, in tab order, opened from saved query `id` on `connection_id`.
    fn saved_console(&self, id: &str, connection_id: &str, cx: &gpui::App) -> Option<Entity<QueryConsole>> {
        self.tabs.tabs().iter().find_map(|tab| {
            let console = self.console(&tab.id)?;
            let read = console.read(cx);
            let opened_from = read.saved_query().is_some_and(|query| query.id == id);
            (opened_from && read.connection_id() == connection_id).then(|| console.clone())
        })
    }

    /// The table page of tab `id`, if it is one.
    pub fn table(&self, id: &str) -> Option<&Entity<TablePage>> {
        self.tables.get(id).map(|open| &open.page)
    }

    /// Opens `table` of `connection_id` (filtered when following a foreign
    /// key), or shows its tab when it is open, on `view` when one is asked
    /// for (its structure, from the explorer); its rows take the keyboard.
    pub fn open_table(
        &mut self,
        connection_id: &str,
        table: TableRef,
        filter: Option<RowFilter>,
        view: Option<TableTabView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<TablePage>> {
        let connection = Backend::global(cx).core().connections().list().into_iter().find(|c| c.id == connection_id)?;
        let name =
            TableName { database: table.database.clone(), schema: table.schema.clone(), name: table.name.clone() };
        let condition = filter.as_ref().map(|filter| filter.filter.clone());
        let tab = self.tabs.open_table(connection_id, name, condition).clone();
        if !self.tables.contains_key(&tab.id) {
            let (id, engine) = (connection_id.to_owned(), connection.engine);
            let (saved, opened) = (connection.clone(), table.clone());
            let page = cx.new(|cx| TablePage::new(id, engine, table, filter.unwrap_or_default(), window, cx));
            let source = connection_id.to_owned();
            let events =
                cx.subscribe_in(&page, window, move |this, _, event: &TablePageEvent, window, cx| match event {
                    TablePageEvent::OpenTable { table, filter } => {
                        this.open_table(&source, table.clone(), Some(filter.clone()), None, window, cx);
                    }
                    TablePageEvent::OpenSchemaDiagram { database, schema } => {
                        this.open_diagram(&source, database, schema, window, cx);
                    }
                    TablePageEvent::Dialog(dialog) => cx.emit(WorkspaceEvent::Dialog(dialog.clone())),
                    TablePageEvent::Export(filter) => cx.emit(WorkspaceEvent::Export(ExportSource::Table {
                        connection_id: source.clone(),
                        table: opened.clone(),
                        filter: filter.clone(),
                    })),
                    TablePageEvent::Import(rows) => cx.emit(WorkspaceEvent::Import(Box::new(ImportTarget {
                        connection: saved.clone(),
                        table: opened.clone(),
                        rows: Some(rows.clone()),
                    }))),
                });
            self.tables.insert(tab.id.clone(), OpenTable { page, _events: events });
        }
        let page = self.table(&tab.id).cloned();
        if let Some(page) = &page {
            if let Some(view) = view {
                page.update(cx, |page, cx| page.set_view(view, window, cx));
            }
            page.read(cx).focus(window, cx);
        }
        cx.notify();
        page
    }

    /// The diagram of tab `id`, if it is one.
    pub fn diagram(&self, id: &str) -> Option<&Entity<DiagramView>> {
        self.diagrams.get(id)
    }

    /// Opens the ER diagram of `schema` in `database` of `connection_id`, or
    /// shows its tab when it is open.
    pub fn open_diagram(
        &mut self,
        connection_id: &str,
        database: &str,
        schema: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<DiagramView>> {
        let exists = Backend::global(cx).core().connections().list().iter().any(|c| c.id == connection_id);
        if !exists {
            return None;
        }
        let tab = self.tabs.open_diagram(cx.messages().diagram.tab, connection_id, database, schema).clone();
        if !self.diagrams.contains_key(&tab.id) {
            let (connection, database, schema) = (connection_id.to_owned(), database.to_owned(), schema.to_owned());
            let diagram = cx.new(|cx| DiagramView::new(connection, database, schema, None, window, cx));
            self.diagrams.insert(tab.id.clone(), diagram);
        }
        cx.notify();
        self.diagram(&tab.id).cloned()
    }

    /// The key browser of tab `id`, if it is one.
    pub fn keyspace(&self, id: &str) -> Option<&Entity<KeyBrowser>> {
        self.keyspaces.get(id).map(|open| &open.browser)
    }

    /// Opens Redis database `database` (`db0`) of `connection_id` in the
    /// key browser, or shows its tab when it is open. Nothing for a
    /// connection that is gone.
    pub fn open_keyspace(
        &mut self,
        connection_id: &str,
        database: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<KeyBrowser>> {
        let connection = Backend::global(cx).core().connections().list().into_iter().find(|c| c.id == connection_id)?;
        if connection.engine != Engine::Redis {
            return None;
        }
        let tab = self.tabs.open_keyspace(connection_id, &connection.name, database).clone();
        if !self.keyspaces.contains_key(&tab.id) {
            let browser = cx.new(|cx| KeyBrowser::new(&tab.id, connection_id.to_owned(), database, window, cx));
            let events = cx.subscribe(&browser, |_, _, KeyBrowserEvent::Dialog(dialog), cx| {
                cx.emit(WorkspaceEvent::Dialog(dialog.clone()));
            });
            self.keyspaces.insert(tab.id.clone(), OpenKeyspace { browser, _events: events });
        }
        cx.notify();
        self.keyspace(&tab.id).cloned()
    }

    /// Shows tab `id`, its console or its rows taking the keyboard.
    pub fn activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.activate(id);
        if let Some(console) = self.console(id) {
            console.read(cx).focus(window, cx);
        }
        if let Some(page) = self.table(id) {
            page.read(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// The title of tab `id`, if it is open.
    pub fn tab_title(&self, id: &str) -> Option<&str> {
        self.tabs.tabs().iter().find(|tab| tab.id == id).map(|tab| tab.title.as_str())
    }

    /// The dialog asking before a tab with a transaction open closes, if one is open.
    pub fn leave_dialog(&self) -> Option<&Entity<LeaveDialog>> {
        self.leave_dialog.as_ref().map(|open| &open.dialog)
    }

    /// Closes tab `id`; a console's session closes with it. A console with a
    /// transaction open asks first whether to roll it back or commit it.
    pub fn close_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let open = open_transactions(Backend::global(cx).core().query_sessions());
        match open.into_iter().find(|open| open.session_id == id) {
            Some(transaction) => self.ask_before_closing(id, transaction, window, cx),
            None => self.close_tab_now(id, window, cx),
        }
    }

    /// Asks whether to roll back or commit `transaction` before tab `id`
    /// closes; it closes once that is done, and stays when the user cancels.
    fn ask_before_closing(
        &mut self,
        id: &str,
        transaction: dbforge_core::query::ConsoleTransaction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.leave_dialog.is_some() {
            return;
        }
        let title = self.tab_title(id).unwrap_or_default().to_owned();
        let pending = vec![Pending { title: title.clone(), transaction }];
        let dialog = cx.new(|cx| LeaveDialog::new(Leaving::Tab(title), pending, cx));
        dialog.read(cx).focus(window);
        let tab = id.to_owned();
        let ended = cx.subscribe_in(&dialog, window, move |this, _, _: &LeaveEvent, window, cx| {
            this.close_leave_dialog(cx);
            this.close_tab_now(&tab, window, cx);
        });
        let dismissed = cx.subscribe_in(&dialog, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_leave_dialog(cx);
            if let Some(id) = this.tabs.active_id().map(str::to_owned) {
                this.activate(&id, window, cx);
            }
        });
        self.leave_dialog = Some(OpenLeaveDialog { dialog: dialog.clone(), _events: vec![ended, dismissed] });
        cx.emit(WorkspaceEvent::Dialog(Some(dialog.into())));
    }

    fn close_leave_dialog(&mut self, cx: &mut Context<Self>) {
        self.leave_dialog = None;
        cx.emit(WorkspaceEvent::Dialog(None));
    }

    /// Closes tab `id` without asking.
    fn close_tab_now(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.close(id).is_some() {
            self.forget(id, cx);
            if let Some(next) = self.tabs.active_id().map(str::to_owned) {
                self.activate(&next, window, cx);
            }
        }
        cx.notify();
    }

    /// Closes the tabs showing a database that was dropped; consoles stay.
    pub fn close_database(&mut self, connection_id: &str, database: &str, window: &mut Window, cx: &mut Context<Self>) {
        for tab in self.tabs.close_database(connection_id, database) {
            self.forget(&tab.id, cx);
        }
        if let Some(next) = self.tabs.active_id().map(str::to_owned) {
            self.activate(&next, window, cx);
        }
        cx.notify();
    }

    /// Closes every tab of a connection that was deleted.
    pub fn close_connection(&mut self, connection_id: &str, cx: &mut Context<Self>) {
        for tab in self.tabs.close_connection(connection_id) {
            self.forget(&tab.id, cx);
        }
        cx.notify();
    }

    fn forget(&mut self, id: &str, cx: &mut Context<Self>) {
        self.tables.remove(id);
        self.diagrams.remove(id);
        // The CLI's session keeps a connection until the tab closes.
        if let Some(open) = self.keyspaces.remove(id) {
            let session = open.browser.read(cx).session_id(cx);
            QueryConsole::close_session(session, cx);
        }
        if let Some(open) = self.consoles.remove(id) {
            let session = open.console.read(cx).session_id().to_owned();
            QueryConsole::close_session(session, cx);
        }
    }

    /// Opens a table or view the explorer asked for, on its rows or on
    /// `view`, or a Redis database in the key browser.
    pub fn open(&mut self, object: &OpenObject, window: &mut Window, cx: &mut Context<Self>) {
        if object.kind == NodeKind::Database {
            if let [database] = object.path.as_slice() {
                self.open_keyspace(&object.connection_id, database, window, cx);
            }
            return;
        }
        if !matches!(object.kind, NodeKind::Table | NodeKind::View) {
            return;
        }
        if let Some(table) = table_ref(&object.path) {
            self.open_table(&object.connection_id, table, None, object.view, window, cx);
        }
    }

    fn empty(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let m = cx.messages();
        card(theme).flex().items_center().justify_center().child(
            div()
                .flex()
                .flex_wrap()
                .justify_center()
                .px(px(24.))
                .text_size(px(13.5))
                .text_color(theme.text3)
                .child(m.empty_before)
                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme.text2).child(m.new_query))
                .child(m.empty_after),
        )
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.tabs.active_id();
        let console = id.and_then(|id| self.console(id)).cloned();
        let table = id.and_then(|id| self.table(id)).cloned();
        let diagram = id.and_then(|id| self.diagram(id)).cloned();
        let keyspace = id.and_then(|id| self.keyspace(id)).cloned();
        let theme = *cx.theme();
        let content = if let Some(console) = console {
            div().flex_1().min_h_0().flex().flex_col().child(console).into_any_element()
        } else if let Some(table) = table {
            div().flex_1().min_h_0().flex().flex_col().child(table).into_any_element()
        } else if let Some(diagram) = diagram {
            card(&theme).child(diagram).into_any_element()
        } else if let Some(keyspace) = keyspace {
            card(&theme).flex().child(keyspace).into_any_element()
        } else {
            self.empty(cx).into_any_element()
        };
        div().flex_1().min_h_0().flex().flex_col().gap(px(GAP)).child(self.strip(cx)).child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(engine: Engine, database: &str) -> Connection {
        serde_json::from_value(serde_json::json!({
            "id": "c", "name": "c", "engine": engine, "environment": null, "host": "h", "port": 1, "database": database, "username": "u",
        }))
        .expect("a connection")
    }

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn opens_a_redis_new_query_in_the_selected_database_s_key_browser() {
        assert_eq!(keyspace_target(&connection(Engine::Redis, "3"), &path(&["db5"])), "db5");
        assert_eq!(keyspace_target(&connection(Engine::Redis, "3"), &[]), "db3");
        assert_eq!(keyspace_target(&connection(Engine::Redis, ""), &[]), "db0");
    }

    #[test]
    fn opens_every_console_but_redis_s() {
        assert!([Engine::Postgres, Engine::Mongodb, Engine::Cosmosdb].into_iter().all(has_query_console));
        assert!(!has_query_console(Engine::Redis));
    }

    #[test]
    fn queries_the_selected_database_and_a_cosmos_db_container() {
        let cosmos = connection(Engine::Cosmosdb, "shop");
        assert_eq!(
            target_scope(&cosmos, &path(&["sales", "orders"])),
            QueryScope { database: Some("sales".into()), container: Some("orders".into()) }
        );
        assert_eq!(target_scope(&cosmos, &[]), QueryScope { database: Some("shop".into()), container: None });
        let mongo = connection(Engine::Mongodb, "shop");
        assert_eq!(target_scope(&mongo, &path(&["catalog", "items"])).container, None);
        assert_eq!(target_scope(&mongo, &[]).database, None, "the connection's own database, as before");
    }
}
