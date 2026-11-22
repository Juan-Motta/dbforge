//! The rules the saved queries' views follow, apart from drawing them: which
//! queries a connection is offered, how the Library groups and searches
//! them, where one runs, when a console differs from the query it was opened
//! from, and how long ago one last ran. Pure, so they are tested without a window.

use std::collections::BTreeSet;

use dbforge_core::connections::model::Engine;
use dbforge_core::saved_queries::params::ParamError;
use dbforge_core::saved_queries::{ParamKind, ParamValue, SavedQuery, SavedScope};

use crate::i18n::SavedMessages;

/// A connection, as the rules need it: its id and engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place<'a> {
    pub connection_id: &'a str,
    pub engine: Engine,
}

/// Whether `query` is offered on `place` ("This connection"): it was saved
/// for that connection, or for any connection of its engine.
pub fn offered_on(query: &SavedQuery, place: Place) -> bool {
    match &query.scope {
        SavedScope::Connection { connection_id } => connection_id == place.connection_id,
        SavedScope::Engine => query.engine == place.engine,
    }
}

/// Whether `query` matches `search` (in its name, description or folder),
/// ignoring case; a blank search matches every query.
pub fn matches_search(query: &SavedQuery, search: &str) -> bool {
    let search = search.trim().to_lowercase();
    if search.is_empty() {
        return true;
    }
    [Some(query.name.as_str()), query.description.as_deref(), query.folder.as_deref()]
        .into_iter()
        .flatten()
        .any(|text| text.to_lowercase().contains(&search))
}

/// A folder of the Library and its queries; `name` is `None` for the queries in no folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub name: Option<String>,
    pub queries: Vec<SavedQuery>,
}

/// The queries the Library shows for `search`, kept to those offered on
/// `place` when given ("This connection"), grouped by folder: the queries in
/// no folder first, then each folder by name; in each, by name. Names are
/// compared ignoring case.
pub fn library(queries: &[SavedQuery], search: &str, place: Option<Place>) -> Vec<Folder> {
    let mut shown: Vec<&SavedQuery> = queries
        .iter()
        .filter(|query| place.is_none_or(|place| offered_on(query, place)) && matches_search(query, search))
        .collect();
    shown.sort_by_cached_key(|query| {
        (query.folder.as_deref().map(str::to_lowercase), query.name.to_lowercase(), query.id.clone())
    });
    let mut folders: Vec<Folder> = Vec::new();
    for query in shown {
        let key = query.folder.as_deref().map(str::to_lowercase);
        match folders.last_mut() {
            Some(folder) if folder.name.as_deref().map(str::to_lowercase) == key => folder.queries.push(query.clone()),
            _ => folders.push(Folder { name: query.folder.clone(), queries: vec![query.clone()] }),
        }
    }
    folders
}

/// The folders queries are saved in, once each (ignoring case), by name.
pub fn folder_names(queries: &[SavedQuery]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut names: Vec<String> = queries
        .iter()
        .filter_map(|query| query.folder.clone())
        .filter(|folder| seen.insert(folder.to_lowercase()))
        .collect();
    names.sort_by_cached_key(|name| name.to_lowercase());
    names
}

/// The connection `query` runs on: the one it was saved for, or for a
/// query of any connection of its engine, `place` when it is one; `None`
/// when no connection is at hand.
pub fn run_connection(query: &SavedQuery, place: Option<Place>) -> Option<String> {
    match &query.scope {
        SavedScope::Connection { connection_id } => Some(connection_id.clone()),
        SavedScope::Engine => {
            place.filter(|place| place.engine == query.engine).map(|place| place.connection_id.into())
        }
    }
}

/// Whether the palette offers `query` with `place` selected: any query
/// saved for a connection (it opens there), and those of any connection of
/// the selected one's engine.
pub fn offered_in_palette(query: &SavedQuery, place: Option<Place>) -> bool {
    run_connection(query, place).is_some()
}

/// Where `query` is offered, as a row says: this connection (`here`), another
/// one by its name (`name_of`), or any connection of its engine.
pub fn scope_label(
    query: &SavedQuery,
    here: Option<&str>,
    name_of: impl Fn(&str) -> Option<String>,
    m: &SavedMessages,
) -> String {
    match &query.scope {
        SavedScope::Engine => (m.any_engine_connection)(query.engine.label()),
        SavedScope::Connection { connection_id } if here == Some(connection_id.as_str()) => {
            m.this_connection.to_owned()
        }
        SavedScope::Connection { connection_id } => {
            name_of(connection_id).unwrap_or_else(|| m.deleted_connection.to_owned())
        }
    }
}

/// Whether a console's text differs from the saved query it was opened
/// from (its tab shows a dot); never for a console of no saved query.
pub fn is_edited(saved_sql: Option<&str>, text: &str) -> bool {
    saved_sql.is_some_and(|saved| saved != text)
}

/// The type a parameter's value starts as: the one it was last run with, else text.
pub fn default_kind(remembered: Option<&ParamValue>) -> ParamKind {
    remembered.map_or(ParamKind::Text, |value| value.kind)
}

/// How long ago `then` was at `now` (both in milliseconds since the epoch):
/// just now, minutes, hours, yesterday, days within a month, then the day.
pub fn ago(then: i64, now: i64, m: &SavedMessages) -> String {
    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let elapsed = (now - then).max(0);
    match elapsed {
        _ if elapsed < MINUTE => m.just_now.into(),
        _ if elapsed < HOUR => (m.minutes_ago)(elapsed / MINUTE),
        _ if elapsed < DAY => (m.hours_ago)(elapsed / HOUR),
        _ if elapsed < 2 * DAY => m.yesterday.into(),
        _ if elapsed < 30 * DAY => (m.days_ago)(elapsed / DAY),
        _ => {
            let day = chrono::DateTime::from_timestamp_millis(then)
                .map(|at| at.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
                .unwrap_or_default();
            (m.on_day)(&day)
        }
    }
}

/// When `query` last ran, or that it never did.
pub fn last_run(query: &SavedQuery, now: i64, m: &SavedMessages) -> String {
    query.last_run_at.map_or_else(|| m.never_run.into(), |then| ago(then, now, m))
}

/// Why a value can't be written, in the interface language.
pub fn param_error(error: &ParamError, m: &SavedMessages) -> String {
    match error {
        ParamError::Missing(name) => (m.missing_value)(name),
        ParamError::NotANumber(value) => (m.not_a_number)(value),
        ParamError::NotADate(value) => (m.not_a_date)(value),
        ParamError::NotADateTime(value) => (m.not_a_date_time)(value),
        ParamError::NotABoolean(value) => (m.not_a_boolean)(value),
        ParamError::Invalid { name, error } => (m.invalid_value)(name, &param_error(error, m)),
    }
}

/// The parameter whose value `error` is about, if it names one.
pub fn param_of(error: &ParamError) -> Option<&str> {
    match error {
        ParamError::Missing(name) | ParamError::Invalid { name, .. } => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::i18n::Language;

    fn m() -> &'static SavedMessages {
        &Language::En.messages().saved
    }

    fn query(id: &str, name: &str, folder: Option<&str>, scope: SavedScope, engine: Engine) -> SavedQuery {
        SavedQuery {
            id: id.into(),
            name: name.into(),
            sql: "select 1".into(),
            description: None,
            folder: folder.map(Into::into),
            scope,
            engine,
            created_at: 0,
            updated_at: 0,
            last_run_at: None,
            last_values: BTreeMap::new(),
            recent_values: BTreeMap::new(),
        }
    }

    fn on(connection_id: &str) -> SavedScope {
        SavedScope::Connection { connection_id: connection_id.into() }
    }

    const PG: Place = Place { connection_id: "pg", engine: Engine::Postgres };

    #[test]
    fn this_connection_offers_its_own_queries_and_those_of_any_connection_of_its_engine() {
        assert!(offered_on(&query("1", "a", None, on("pg"), Engine::Postgres), PG));
        assert!(!offered_on(&query("2", "a", None, on("other"), Engine::Postgres), PG));
        assert!(offered_on(&query("3", "a", None, SavedScope::Engine, Engine::Postgres), PG));
        assert!(!offered_on(&query("4", "a", None, SavedScope::Engine, Engine::Mysql), PG));
    }

    #[test]
    fn searches_the_name_description_and_folder_ignoring_case() {
        let mut found = query("1", "Orders by status", Some("Reports"), on("pg"), Engine::Postgres);
        found.description = Some("Count per STATUS".into());
        assert!(matches_search(&found, "  "));
        assert!(matches_search(&found, "ORDERS"));
        assert!(matches_search(&found, "count per"));
        assert!(matches_search(&found, "report"));
        assert!(!matches_search(&found, "select"), "the SQL is not searched");
    }

    #[test]
    fn groups_unfiled_queries_first_then_folders_by_name_each_by_name() {
        let queries = vec![
            query("1", "b", Some("reports"), on("pg"), Engine::Postgres),
            query("2", "A", Some("Reports"), on("pg"), Engine::Postgres),
            query("3", "z", None, on("pg"), Engine::Postgres),
            query("4", "c", Some("ops"), SavedScope::Engine, Engine::Postgres),
            query("5", "mine", None, on("other"), Engine::Postgres),
            query("6", "my", None, SavedScope::Engine, Engine::Mysql),
        ];
        let shape = |folders: Vec<Folder>| -> Vec<(Option<String>, Vec<String>)> {
            folders.into_iter().map(|f| (f.name, f.queries.into_iter().map(|q| q.name).collect())).collect()
        };
        assert_eq!(
            shape(library(&queries, "", Some(PG))),
            [
                (None, vec!["z".to_string()]),
                (Some("ops".into()), vec!["c".into()]),
                (Some("Reports".into()), vec!["A".into(), "b".into()]),
            ],
            "one folder whatever its case, named as its first query writes it; other connections' and \
             engines' queries left out"
        );
        let all = shape(library(&queries, "", None));
        assert_eq!(all[0], (None, vec!["mine".to_string(), "my".into(), "z".into()]), "All shows every query");
        assert_eq!(shape(library(&queries, "zz", None)), []);
        assert_eq!(shape(library(&queries, "rep", Some(PG))).len(), 1, "a search keeps the folders that match");
    }

    #[test]
    fn lists_each_folder_once_by_name() {
        let queries = vec![
            query("1", "a", Some("Support"), on("pg"), Engine::Postgres),
            query("2", "b", Some("reports"), on("pg"), Engine::Postgres),
            query("3", "c", Some("SUPPORT"), on("pg"), Engine::Postgres),
            query("4", "d", None, on("pg"), Engine::Postgres),
        ];
        assert_eq!(folder_names(&queries), ["reports", "Support"]);
    }

    #[test]
    fn runs_on_its_connection_or_on_the_one_at_hand_of_its_engine() {
        let mine = query("1", "a", None, on("prod"), Engine::Postgres);
        assert_eq!(run_connection(&mine, Some(PG)).as_deref(), Some("prod"), "a query saved for another connection");
        assert_eq!(run_connection(&mine, None).as_deref(), Some("prod"));
        let any = query("2", "a", None, SavedScope::Engine, Engine::Postgres);
        assert_eq!(run_connection(&any, Some(PG)).as_deref(), Some("pg"));
        assert_eq!(run_connection(&any, None), None);
        let mysql = Place { connection_id: "my", engine: Engine::Mysql };
        assert_eq!(run_connection(&any, Some(mysql)), None, "not on another engine's connection");
        assert!(offered_in_palette(&mine, Some(mysql)));
        assert!(!offered_in_palette(&any, Some(mysql)));
        assert!(offered_in_palette(&any, Some(PG)));
    }

    #[test]
    fn names_where_a_query_is_offered() {
        let name_of = |id: &str| (id == "prod").then(|| "Vivi Prod".to_string());
        let label = |query: &SavedQuery, here| scope_label(query, here, name_of, m());
        let here = query("1", "a", None, on("pg"), Engine::Postgres);
        assert_eq!(label(&here, Some("pg")), "This connection");
        assert_eq!(label(&query("2", "a", None, on("prod"), Engine::Postgres), Some("pg")), "Vivi Prod");
        assert_eq!(label(&here, None), "Deleted connection", "a connection no longer saved");
        assert_eq!(label(&query("3", "a", None, SavedScope::Engine, Engine::Mysql), None), "Any MySQL connection");
    }

    #[test]
    fn a_console_is_edited_while_its_text_differs_from_its_saved_query() {
        assert!(!is_edited(None, "select 2"), "a console of no saved query never is");
        assert!(!is_edited(Some("select 1"), "select 1"));
        assert!(is_edited(Some("select 1"), "select 1 "));
    }

    #[test]
    fn a_parameter_starts_as_the_type_it_was_last_run_with() {
        assert_eq!(default_kind(None), ParamKind::Text);
        let last = ParamValue { kind: ParamKind::Date, value: "2026-01-02".into() };
        assert_eq!(default_kind(Some(&last)), ParamKind::Date);
    }

    #[test]
    fn says_how_long_ago_a_query_ran() {
        const MIN: i64 = 60_000;
        let now = 1_000 * 24 * 60 * MIN;
        assert_eq!(ago(now - 5_000, now, m()), "Just now");
        assert_eq!(ago(now + 5_000, now, m()), "Just now", "a clock that went back");
        assert_eq!(ago(now - 12 * MIN, now, m()), "12 min ago");
        assert_eq!(ago(now - 125 * MIN, now, m()), "2 h ago");
        assert_eq!(ago(now - 30 * 60 * MIN, now, m()), "Yesterday");
        assert_eq!(ago(now - 3 * 24 * 60 * MIN, now, m()), "3 days ago");
        assert!(ago(now - 40 * 24 * 60 * MIN, now, m()).starts_with("On "));
        let never = query("1", "a", None, on("pg"), Engine::Postgres);
        assert_eq!(last_run(&never, now, m()), "Never run");
        assert_eq!(last_run(&SavedQuery { last_run_at: Some(now), ..never }, now, m()), "Just now");
    }

    #[test]
    fn writes_why_a_value_cant_be_used_in_the_interface_language() {
        let error = ParamError::Invalid { name: "id".into(), error: Box::new(ParamError::NotANumber("x".into())) };
        assert_eq!(param_error(&error, m()), ":id: “x” is not a number.");
        assert_eq!(param_error(&error, &Language::Es.messages().saved), ":id: “x” no es un número.");
        assert_eq!(param_of(&error), Some("id"));
        assert_eq!(param_of(&ParamError::Missing("day".into())), Some("day"));
        assert_eq!(param_of(&ParamError::NotADate("x".into())), None);
    }
}
