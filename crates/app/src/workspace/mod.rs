//! The main area: the tab strip and the open tab under it. With no tab open,
//! it says how to open one. Tabs that aren't shown keep their state (a
//! console's text and results) until they close.

mod strip;
pub mod tabs;

use std::collections::HashMap;

use dbforge_core::connections::model::{Connection, Engine};
use gpui::{
    AppContext, Context, Entity, FontWeight, IntoElement, ParentElement, Render, Styled, Subscription, Window, div, px,
};

use self::tabs::{TabKind, Tabs};
use crate::backend::Backend;
use crate::i18n::Translate;
use crate::query::{ConsoleEvent, QueryConsole};
use crate::shell::OpenObject;
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

#[derive(Default)]
pub struct Workspace {
    tabs: Tabs,
    consoles: HashMap<String, OpenConsole>,
    /// The connection selected in the explorer, and where in it: what "New query" queries.
    target: Option<(Connection, Vec<String>)>,
}

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
        let TabKind::Query { database, initial_sql } = tab.kind;
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

    /// Shows tab `id`, its console taking the keyboard.
    pub fn activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.activate(id);
        if let Some(console) = self.console(id) {
            console.read(cx).focus(window, cx);
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
        if let Some(open) = self.consoles.remove(id) {
            let session = open.console.read(cx).session_id().to_owned();
            QueryConsole::close_session(session, cx);
        }
    }

    /// Opens a table, view or Redis database the explorer asked for. The
    /// table and key-browser tabs come with their features; until then, nothing.
    pub fn open(&mut self, _object: &OpenObject, _window: &mut Window, _cx: &mut Context<Self>) {}

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
        let active = self.tabs.active_id().and_then(|id| self.console(id)).cloned();
        div().flex_1().min_h_0().flex().flex_col().gap(px(GAP)).child(self.strip(cx)).child(match active {
            Some(console) => div().flex_1().min_h_0().flex().flex_col().child(console).into_any_element(),
            None => self.empty(cx).into_any_element(),
        })
    }
}
