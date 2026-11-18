//! The main area: the tab strip and the open tab under it (a query console,
//! a table, a schema's diagram). With no tab open,
//! it says how to open one. Tabs that aren't shown keep their state (a
//! console's text and results, a table's page and pending changes) until
//! they close. A dialog a tab opens (a table's PROD question) goes to the
//! window as a [`WorkspaceEvent`].

mod strip;
pub mod tabs;

use std::collections::HashMap;

use dbforge_core::connections::model::{Connection, Engine};
use dbforge_core::drivers::{RowFilter, TableRef};
use gpui::{
    AnyView, AppContext, Context, Entity, EventEmitter, FontWeight, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, px,
};

use self::tabs::{TabKind, TableName, Tabs};
use crate::backend::Backend;
use crate::diagram::DiagramView;
use crate::i18n::Translate;
use crate::query::{ConsoleEvent, QueryConsole};
use crate::shell::OpenObject;
use crate::shell::explorer::tree::NodeKind;
use crate::sql::snippets::table_ref;
use crate::table::{TablePage, TablePageEvent, TableTabView};
use crate::theme::ActiveTheme;
use crate::ui::card;

/// Space between the strip and the tab's content.
const GAP: f32 = 10.;

/// Whether `engine` has a SQL console. MongoDB's and Cosmos DB's consoles
/// and Redis's key browser come with their own features.
pub fn has_sql_console(engine: Engine) -> bool {
    matches!(engine, Engine::Postgres | Engine::Mysql | Engine::Mariadb | Engine::Sqlite | Engine::Sqlserver)
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

/// What the workspace asks of the window.
#[derive(Debug, Clone)]
pub enum WorkspaceEvent {
    /// Show this dialog over the window, or close the one shown.
    Dialog(Option<AnyView>),
}

#[derive(Default)]
pub struct Workspace {
    tabs: Tabs,
    consoles: HashMap<String, OpenConsole>,
    tables: HashMap<String, OpenTable>,
    diagrams: HashMap<String, Entity<DiagramView>>,
    /// The connection selected in the explorer, and where in it: what "New query" queries.
    target: Option<(Connection, Vec<String>)>,
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

    /// Whether "New query" can open now: a SQL connection is selected.
    pub fn can_query(&self) -> bool {
        self.target.as_ref().is_some_and(|(connection, _)| has_sql_console(connection.engine))
    }

    /// "New query" from the strip's +: a console on the selected connection
    /// and database.
    pub fn new_query_on_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((connection, path)) = self.target.clone() else { return };
        self.new_query(&connection.id, path.first().cloned(), None, window, cx);
    }

    /// Opens a console on `connection_id` and `database`, its editor
    /// starting with `sql`, and gives it the keyboard. Nothing for an engine
    /// without a SQL console, or a connection that is gone.
    pub fn new_query(
        &mut self,
        connection_id: &str,
        database: Option<String>,
        sql: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<QueryConsole>> {
        let connection = Backend::global(cx).core().connections().list().into_iter().find(|c| c.id == connection_id)?;
        if !has_sql_console(connection.engine) {
            return None;
        }
        let tab = self.tabs.new_query(cx.messages().query.query_tab, connection_id, database, sql).clone();
        let TabKind::Query { database, initial_sql } = tab.kind else { return None };
        let console = cx.new(|cx| {
            QueryConsole::new(tab.id.clone(), tab.connection_id.clone(), connection.engine, database, initial_sql, cx)
        });
        let events = cx.subscribe_in(&console, window, |this, _, event: &ConsoleEvent, window, cx| match event {
            ConsoleEvent::OpenQuery(entry) => {
                this.new_query(&entry.connection_id, entry.database.clone(), Some(entry.sql.clone()), window, cx);
            }
        });
        self.consoles.insert(tab.id, OpenConsole { console: console.clone(), _events: events });
        console.read(cx).focus(window, cx);
        cx.notify();
        Some(console)
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

    /// Closes tab `id`; a console's session closes with it.
    pub fn close_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.close(id).is_some() {
            self.forget(id, cx);
            if let Some(next) = self.tabs.active_id().map(str::to_owned) {
                self.activate(&next, window, cx);
            }
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
        if let Some(open) = self.consoles.remove(id) {
            let session = open.console.read(cx).session_id().to_owned();
            QueryConsole::close_session(session, cx);
        }
    }

    /// Opens a table or view the explorer asked for, on its rows or on
    /// `view`. Redis databases open in the key browser, which comes with its
    /// feature; until then, nothing.
    pub fn open(&mut self, object: &OpenObject, window: &mut Window, cx: &mut Context<Self>) {
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
        let theme = *cx.theme();
        let content = match (console, table, diagram) {
            (Some(console), _, _) => div().flex_1().min_h_0().flex().flex_col().child(console).into_any_element(),
            (_, Some(table), _) => div().flex_1().min_h_0().flex().flex_col().child(table).into_any_element(),
            (_, _, Some(diagram)) => card(&theme).child(diagram).into_any_element(),
            _ => self.empty(cx).into_any_element(),
        };
        div().flex_1().min_h_0().flex().flex_col().gap(px(GAP)).child(self.strip(cx)).child(content)
    }
}
