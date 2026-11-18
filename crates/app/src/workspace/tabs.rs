//! The open tabs, apart from drawing them: which are open, which one shows,
//! and how opening and closing change that. Query consoles are numbered
//! "Query 1", "Query 2"… for as long as the app runs. A table opens once
//! (opening it again shows its tab), or filtered in a tab of its own (a
//! foreign key followed); a schema's diagram opens once too. Key browsers
//! join with their feature.

use dbforge_core::query::QueryScope;

/// What a tab shows.
#[derive(Debug, Clone, PartialEq)]
pub enum TabKind {
    /// A query console on `scope` (its database, the connection's own when
    /// it has none, and a Cosmos DB container), starting with `initial_sql`
    /// (a query reopened from the history).
    Query { scope: QueryScope, initial_sql: Option<String> },
    /// A table's or view's rows, opened with `filter` (a `WHERE` condition) when it has one.
    Table { table: TableName, filter: Option<String> },
    /// A schema's ER diagram.
    Diagram { database: String, schema: String },
}

/// A table as a tab names it: its database, schema and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableName {
    pub database: String,
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    /// Unique among the tabs; a console's id is also its core session's.
    pub id: String,
    pub title: String,
    pub connection_id: String,
    pub kind: TabKind,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tabs {
    tabs: Vec<Tab>,
    active: Option<String>,
    /// Numbers new query tabs; never goes back, so a title is never reused.
    queries_created: usize,
}

impl Tabs {
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn active(&self) -> Option<&Tab> {
        let active = self.active.as_deref()?;
        self.tabs.iter().find(|tab| tab.id == active)
    }

    /// Opens a new query console, titled by `title` from its number, and shows it.
    pub fn new_query(
        &mut self,
        title: fn(usize) -> String,
        connection_id: &str,
        scope: QueryScope,
        initial_sql: Option<String>,
    ) -> &Tab {
        self.queries_created += 1;
        let n = self.queries_created;
        let tab = Tab {
            id: format!("query:{n}"),
            title: title(n),
            connection_id: connection_id.to_owned(),
            kind: TabKind::Query { scope, initial_sql },
        };
        self.open(tab)
    }

    /// Opens a table, or shows its tab when it is open already. With a
    /// `filter` (following a foreign key) it opens in a tab of its own,
    /// titled with the filter.
    pub fn open_table(&mut self, connection_id: &str, table: TableName, filter: Option<String>) -> &Tab {
        let base = format!("table:{connection_id}:{}.{}.{}", table.database, table.schema, table.name);
        let (id, title) = match &filter {
            Some(filter) => (format!("{base}?{filter}"), format!("{} · {filter}", table.name)),
            None => (base, table.name.clone()),
        };
        self.open(Tab { id, title, connection_id: connection_id.to_owned(), kind: TabKind::Table { table, filter } })
    }

    /// Opens a schema's ER diagram, titled by `title` from the schema's
    /// name, or shows its tab when it is open already.
    pub fn open_diagram(
        &mut self,
        title: fn(&str) -> String,
        connection_id: &str,
        database: &str,
        schema: &str,
    ) -> &Tab {
        self.open(Tab {
            id: format!("diagram:{connection_id}:{database}.{schema}"),
            title: title(schema),
            connection_id: connection_id.to_owned(),
            kind: TabKind::Diagram { database: database.to_owned(), schema: schema.to_owned() },
        })
    }

    /// Shows `tab`, adding it unless a tab with its id is open already.
    fn open(&mut self, tab: Tab) -> &Tab {
        let index = match self.tabs.iter().position(|open| open.id == tab.id) {
            Some(index) => index,
            None => {
                self.tabs.push(tab);
                self.tabs.len() - 1
            }
        };
        self.active = Some(self.tabs[index].id.clone());
        &self.tabs[index]
    }

    /// Shows the tab `id`, if it is open.
    pub fn activate(&mut self, id: &str) {
        if self.tabs.iter().any(|tab| tab.id == id) {
            self.active = Some(id.to_owned());
        }
    }

    /// Closes the tab `id`; when it was showing, its right neighbour (or
    /// else its left one) shows instead. The closed tab.
    pub fn close(&mut self, id: &str) -> Option<Tab> {
        let index = self.tabs.iter().position(|tab| tab.id == id)?;
        let closed = self.tabs.remove(index);
        if self.active.as_deref() == Some(id) {
            self.active = self.tabs.get(index.min(self.tabs.len().saturating_sub(1))).map(|tab| tab.id.clone());
        }
        Some(closed)
    }

    /// Closes every tab of a connection (it was deleted); the tabs closed.
    pub fn close_connection(&mut self, connection_id: &str) -> Vec<Tab> {
        let ids: Vec<String> =
            self.tabs.iter().filter(|tab| tab.connection_id == connection_id).map(|tab| tab.id.clone()).collect();
        ids.iter().filter_map(|id| self.close(id)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn on(database: &str) -> QueryScope {
        QueryScope { database: Some(database.into()), container: None }
    }

    fn titles(tabs: &Tabs) -> Vec<&str> {
        tabs.tabs().iter().map(|tab| tab.title.as_str()).collect()
    }

    fn en(n: usize) -> String {
        (Language::En.messages().query.query_tab)(n)
    }

    #[test]
    fn numbers_query_tabs_and_shows_the_new_one() {
        let mut tabs = Tabs::default();
        tabs.new_query(en, "pg", on("shop"), None);
        tabs.new_query(en, "pg", QueryScope::default(), None);
        assert_eq!(titles(&tabs), ["Query 1", "Query 2"]);
        assert_eq!(tabs.active_id(), Some("query:2"));
    }

    #[test]
    fn titles_query_tabs_in_the_interface_language() {
        let mut tabs = Tabs::default();
        tabs.new_query(Language::Es.messages().query.query_tab, "pg", QueryScope::default(), None);
        assert_eq!(titles(&tabs), ["Consulta 1"]);
    }

    #[test]
    fn can_prefill_a_query_tab_from_the_history() {
        let mut tabs = Tabs::default();
        let tab = tabs.new_query(en, "pg", on("shop"), Some("SELECT 1".into()));
        assert_eq!(tab.kind, TabKind::Query { scope: on("shop"), initial_sql: Some("SELECT 1".into()) });
        let scope = QueryScope { database: Some("shop".into()), container: Some("orders".into()) };
        let tab = tabs.new_query(en, "cosmos", scope.clone(), None);
        assert_eq!(tab.kind, TabKind::Query { scope, initial_sql: None }, "a Cosmos DB console keeps its container");
    }

    #[test]
    fn keeps_numbering_after_a_query_tab_closes() {
        let mut tabs = Tabs::default();
        tabs.new_query(en, "pg", QueryScope::default(), None);
        tabs.close("query:1");
        tabs.new_query(en, "pg", QueryScope::default(), None);
        assert_eq!(titles(&tabs), ["Query 2"]);
    }

    #[test]
    fn shows_the_right_neighbour_then_the_left_when_the_shown_tab_closes() {
        let mut tabs = Tabs::default();
        for _ in 0..3 {
            tabs.new_query(en, "pg", QueryScope::default(), None);
        }
        tabs.activate("query:2");
        assert_eq!(tabs.close("query:2").map(|tab| tab.id), Some("query:2".into()));
        assert_eq!(tabs.active_id(), Some("query:3"));
        tabs.close("query:3");
        assert_eq!(tabs.active_id(), Some("query:1"));
        tabs.close("query:1");
        assert_eq!(tabs.active_id(), None);
        assert!(tabs.close("query:1").is_none(), "closing a closed tab does nothing");
    }

    #[test]
    fn keeps_the_shown_tab_when_another_one_closes() {
        let mut tabs = Tabs::default();
        tabs.new_query(en, "pg", QueryScope::default(), None);
        tabs.new_query(en, "pg", QueryScope::default(), None);
        tabs.close("query:1");
        assert_eq!(tabs.active_id(), Some("query:2"));
        tabs.activate("nope");
        assert_eq!(tabs.active().map(|tab| tab.title.as_str()), Some("Query 2"));
    }

    fn orders() -> TableName {
        TableName { database: "shop".into(), schema: "public".into(), name: "orders".into() }
    }

    #[test]
    fn opens_a_table_once_and_shows_its_tab_again() {
        let mut tabs = Tabs::default();
        tabs.open_table("pg", orders(), None);
        tabs.new_query(en, "pg", QueryScope::default(), None);
        let tab = tabs.open_table("pg", orders(), None).clone();
        assert_eq!((tab.id.as_str(), tab.title.as_str()), ("table:pg:shop.public.orders", "orders"));
        assert_eq!(titles(&tabs), ["orders", "Query 1"]);
        assert_eq!(tabs.active_id(), Some("table:pg:shop.public.orders"));
        tabs.open_table("other", orders(), None);
        assert_eq!(tabs.tabs().len(), 3, "the same table of another connection is another tab");
    }

    #[test]
    fn opens_a_filtered_table_in_a_tab_of_its_own() {
        let mut tabs = Tabs::default();
        tabs.open_table("pg", orders(), None);
        let tab = tabs.open_table("pg", orders(), Some("\"id\" = '7'".into())).clone();
        assert_eq!(tab.id, "table:pg:shop.public.orders?\"id\" = '7'");
        assert_eq!(tab.title, "orders · \"id\" = '7'");
        assert_eq!(tabs.tabs().len(), 2);
    }

    #[test]
    fn opens_a_schema_diagram_once_titled_in_the_interface_language() {
        let mut tabs = Tabs::default();
        let es = Language::Es.messages().diagram.tab;
        let tab = tabs.open_diagram(Language::En.messages().diagram.tab, "pg", "shop", "public").clone();
        assert_eq!((tab.id.as_str(), tab.title.as_str()), ("diagram:pg:shop.public", "public diagram"));
        assert_eq!(tab.kind, TabKind::Diagram { database: "shop".into(), schema: "public".into() });
        tabs.new_query(en, "pg", QueryScope::default(), None);
        tabs.open_diagram(es, "pg", "shop", "public");
        assert_eq!(titles(&tabs), ["public diagram", "Query 1"], "opening it again shows its tab");
        assert_eq!(tabs.active_id(), Some("diagram:pg:shop.public"));
        assert_eq!(tabs.open_diagram(es, "pg", "shop", "sales").title, "Diagrama de sales");
    }

    #[test]
    fn closes_all_tabs_of_a_connection() {
        let mut tabs = Tabs::default();
        tabs.new_query(en, "pg", QueryScope::default(), None);
        tabs.new_query(en, "other", QueryScope::default(), None);
        tabs.new_query(en, "pg", QueryScope::default(), None);
        let closed = tabs.close_connection("pg");
        assert_eq!(closed.len(), 2);
        assert_eq!(titles(&tabs), ["Query 2"]);
        assert_eq!(tabs.active_id(), Some("query:2"));
    }
}
