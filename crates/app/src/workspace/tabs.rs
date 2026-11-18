//! The open tabs, apart from drawing them: which are open, which one shows,
//! and how opening and closing change that. Query consoles are numbered
//! "Query 1", "Query 2"… for as long as the app runs; tables, diagrams and
//! key browsers join as their features arrive.

/// What a tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabKind {
    /// A query console on `database` (the connection's own when `None`),
    /// starting with `initial_sql` (a query reopened from the history).
    Query { database: Option<String>, initial_sql: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    /// Unique among the tabs; a console's id is also its core session's.
    pub id: String,
    pub title: String,
    pub connection_id: String,
    pub kind: TabKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
        database: Option<String>,
        initial_sql: Option<String>,
    ) -> &Tab {
        self.queries_created += 1;
        let n = self.queries_created;
        let tab = Tab {
            id: format!("query:{n}"),
            title: title(n),
            connection_id: connection_id.to_owned(),
            kind: TabKind::Query { database, initial_sql },
        };
        self.open(tab)
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

    fn titles(tabs: &Tabs) -> Vec<&str> {
        tabs.tabs().iter().map(|tab| tab.title.as_str()).collect()
    }

    fn en(n: usize) -> String {
        (Language::En.messages().query.query_tab)(n)
    }

    #[test]
    fn numbers_query_tabs_and_shows_the_new_one() {
        let mut tabs = Tabs::default();
        tabs.new_query(en, "pg", Some("shop".into()), None);
        tabs.new_query(en, "pg", None, None);
        assert_eq!(titles(&tabs), ["Query 1", "Query 2"]);
        assert_eq!(tabs.active_id(), Some("query:2"));
    }

    #[test]
    fn titles_query_tabs_in_the_interface_language() {
        let mut tabs = Tabs::default();
        tabs.new_query(Language::Es.messages().query.query_tab, "pg", None, None);
        assert_eq!(titles(&tabs), ["Consulta 1"]);
    }

    #[test]
    fn can_prefill_a_query_tab_from_the_history() {
        let mut tabs = Tabs::default();
        let tab = tabs.new_query(en, "pg", Some("shop".into()), Some("SELECT 1".into()));
        assert_eq!(tab.kind, TabKind::Query { database: Some("shop".into()), initial_sql: Some("SELECT 1".into()) });
    }

    #[test]
    fn keeps_numbering_after_a_query_tab_closes() {
        let mut tabs = Tabs::default();
        tabs.new_query(en, "pg", None, None);
        tabs.close("query:1");
        tabs.new_query(en, "pg", None, None);
        assert_eq!(titles(&tabs), ["Query 2"]);
    }

    #[test]
    fn shows_the_right_neighbour_then_the_left_when_the_shown_tab_closes() {
        let mut tabs = Tabs::default();
        for _ in 0..3 {
            tabs.new_query(en, "pg", None, None);
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
        tabs.new_query(en, "pg", None, None);
        tabs.new_query(en, "pg", None, None);
        tabs.close("query:1");
        assert_eq!(tabs.active_id(), Some("query:2"));
        tabs.activate("nope");
        assert_eq!(tabs.active().map(|tab| tab.title.as_str()), Some("Query 2"));
    }

    #[test]
    fn closes_all_tabs_of_a_connection() {
        let mut tabs = Tabs::default();
        tabs.new_query(en, "pg", None, None);
        tabs.new_query(en, "other", None, None);
        tabs.new_query(en, "pg", None, None);
        let closed = tabs.close_connection("pg");
        assert_eq!(closed.len(), 2);
        assert_eq!(titles(&tabs), ["Query 2"]);
        assert_eq!(tabs.active_id(), Some("query:2"));
    }
}
