//! What the window asks of the saved queries.

use std::collections::BTreeMap;

use super::{ParamValue, SavedQueries, SavedQuery, SavedQueryInput};
use crate::error::Result;

/// Every saved query, by folder and then by name, ignoring case.
pub fn list_saved_queries(store: &SavedQueries) -> Vec<SavedQuery> {
    store.list()
}

/// Creates a saved query (`id` is `None`) or changes saved query `id`.
pub fn save_query(store: &SavedQueries, id: Option<&str>, input: SavedQueryInput) -> Result<SavedQuery> {
    store.save(id, input)
}

/// A copy of saved query `id` named "<name> copy".
pub fn duplicate_saved_query(store: &SavedQueries, id: &str) -> Result<SavedQuery> {
    store.duplicate(id)
}

pub fn delete_saved_query(store: &SavedQueries, id: &str) -> Result<()> {
    store.remove(id)
}

/// Records that saved query `id` ran, remembering the parameters' `values`
/// when given ("Remember values" on).
pub fn mark_saved_query_run(
    store: &SavedQueries,
    id: &str,
    values: Option<BTreeMap<String, ParamValue>>,
) -> Result<SavedQuery> {
    store.mark_run(id, values)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::connections::model::Engine;
    use crate::error::AppError;
    use crate::saved_queries::{ParamKind, SavedScope};

    fn store() -> (tempfile::TempDir, PathBuf, SavedQueries) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("saved_queries.json");
        let store = SavedQueries::load(file.clone()).unwrap();
        (dir, file, store)
    }

    fn input(name: &str, folder: Option<&str>) -> SavedQueryInput {
        SavedQueryInput {
            name: name.into(),
            sql: "select * from orders where id = :id".into(),
            description: None,
            folder: folder.map(Into::into),
            scope: SavedScope::Connection { connection_id: "c1".into() },
            engine: Engine::Postgres,
        }
    }

    fn value(kind: ParamKind, value: &str) -> ParamValue {
        ParamValue { kind, value: value.into() }
    }

    #[test]
    fn creates_a_query_with_trimmed_fields_and_no_run() {
        let (_dir, _, store) = store();
        let saved = save_query(
            &store,
            None,
            SavedQueryInput {
                name: "  Orders  ".into(),
                description: Some("  by id ".into()),
                folder: Some("   ".into()),
                ..input("x", None)
            },
        )
        .unwrap();
        assert_eq!(saved.name, "Orders");
        assert_eq!(saved.description.as_deref(), Some("by id"));
        assert_eq!(saved.folder, None);
        assert_eq!(saved.created_at, saved.updated_at);
        assert_eq!(saved.last_run_at, None);
        assert!(saved.last_values.is_empty());
        assert!(!saved.id.is_empty());
        assert_eq!(list_saved_queries(&store), [saved]);
    }

    #[test]
    fn updating_keeps_the_id_creation_and_last_run() {
        let (_dir, _, store) = store();
        let created = save_query(&store, None, input("Orders", None)).unwrap();
        let values = BTreeMap::from([("id".to_string(), value(ParamKind::Number, "7"))]);
        let run = mark_saved_query_run(&store, &created.id, Some(values.clone())).unwrap();
        let updated = save_query(
            &store,
            Some(&created.id),
            SavedQueryInput { sql: "select 1".into(), scope: SavedScope::Engine, ..input("Renamed", Some("Ops")) },
        )
        .unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.sql, "select 1");
        assert_eq!(updated.folder.as_deref(), Some("Ops"));
        assert_eq!(updated.scope, SavedScope::Engine);
        assert_eq!(updated.created_at, created.created_at);
        assert!(updated.updated_at >= created.updated_at);
        assert_eq!(updated.last_run_at, run.last_run_at);
        assert_eq!(updated.last_values, values);
        assert_eq!(list_saved_queries(&store), [updated]);
    }

    #[test]
    fn duplicates_under_a_new_id_and_name() {
        let (_dir, _, store) = store();
        let original = save_query(&store, None, input("Orders", Some("Ops"))).unwrap();
        mark_saved_query_run(&store, &original.id, None).unwrap();
        let copy = duplicate_saved_query(&store, &original.id).unwrap();
        assert_ne!(copy.id, original.id);
        assert_eq!(copy.name, "Orders copy");
        assert_eq!((copy.sql.as_str(), copy.folder.as_deref()), (original.sql.as_str(), Some("Ops")));
        assert_eq!(copy.last_run_at, None);
        assert_eq!(list_saved_queries(&store).len(), 2);
    }

    #[test]
    fn deletes_only_the_query_asked() {
        let (_dir, _, store) = store();
        let kept = save_query(&store, None, input("Kept", None)).unwrap();
        let gone = save_query(&store, None, input("Gone", None)).unwrap();
        delete_saved_query(&store, &gone.id).unwrap();
        assert_eq!(list_saved_queries(&store), [kept]);
    }

    #[test]
    fn lists_by_folder_then_name_ignoring_case() {
        let (_dir, _, store) = store();
        for (name, folder) in [("b", Some("reports")), ("A", Some("Reports")), ("z", None), ("c", Some("ops"))] {
            save_query(&store, None, input(name, folder)).unwrap();
        }
        let order: Vec<_> = list_saved_queries(&store).into_iter().map(|query| query.name).collect();
        assert_eq!(order, ["z", "c", "A", "b"]);
    }

    #[test]
    fn refuses_a_blank_name_and_saves_nothing() {
        let (_dir, file, store) = store();
        let refused = save_query(&store, None, input("   ", None));
        assert!(matches!(refused, Err(AppError::Invalid(_))));
        assert!(list_saved_queries(&store).is_empty());
        assert!(!file.exists());
        let saved = save_query(&store, None, input("Orders", None)).unwrap();
        assert!(matches!(save_query(&store, Some(&saved.id), input("", None)), Err(AppError::Invalid(_))));
        assert_eq!(list_saved_queries(&store), [saved]);
    }

    #[test]
    fn unknown_ids_are_not_found() {
        let (_dir, _, store) = store();
        assert!(matches!(save_query(&store, Some("missing"), input("x", None)), Err(AppError::NotFound(_))));
        assert!(matches!(duplicate_saved_query(&store, "missing"), Err(AppError::NotFound(_))));
        assert!(matches!(delete_saved_query(&store, "missing"), Err(AppError::NotFound(_))));
        assert!(matches!(mark_saved_query_run(&store, "missing", None), Err(AppError::NotFound(_))));
        assert!(matches!(store.get("missing"), Err(AppError::NotFound(_))));
    }

    #[test]
    fn marking_a_run_replaces_values_only_when_given() {
        let (_dir, _, store) = store();
        let saved = save_query(&store, None, input("Orders", None)).unwrap();
        let first = BTreeMap::from([("id".to_string(), value(ParamKind::Number, "7"))]);
        let run = mark_saved_query_run(&store, &saved.id, Some(first.clone())).unwrap();
        assert!(run.last_run_at.is_some());
        assert_eq!(run.last_values, first);
        let again = mark_saved_query_run(&store, &saved.id, None).unwrap();
        assert_eq!(again.last_values, first, "Remember values off keeps the previous ones");
        let second = BTreeMap::from([("id".to_string(), value(ParamKind::Null, ""))]);
        assert_eq!(mark_saved_query_run(&store, &saved.id, Some(second.clone())).unwrap().last_values, second);
    }

    fn run_with(store: &SavedQueries, id: &str, pairs: &[(&str, ParamValue)]) -> SavedQuery {
        let values = pairs.iter().map(|(name, value)| (name.to_string(), value.clone())).collect();
        mark_saved_query_run(store, id, Some(values)).unwrap()
    }

    fn numbers(values: &[&str]) -> Vec<ParamValue> {
        values.iter().map(|text| value(ParamKind::Number, text)).collect()
    }

    #[test]
    fn recent_values_are_kept_newest_first() {
        let (_dir, _, store) = store();
        let saved = save_query(&store, None, input("Orders", None)).unwrap();
        assert!(saved.recent_values.is_empty());
        run_with(&store, &saved.id, &[("id", value(ParamKind::Number, "1"))]);
        let run = run_with(&store, &saved.id, &[("id", value(ParamKind::Number, "2"))]);
        assert_eq!(run.recent_values["id"], numbers(&["2", "1"]));
        assert_eq!(run.last_values["id"], value(ParamKind::Number, "2"));
    }

    #[test]
    fn reusing_a_recent_value_moves_it_to_the_front() {
        let (_dir, _, store) = store();
        let saved = save_query(&store, None, input("Orders", None)).unwrap();
        for text in ["1", "2", "3", "1"] {
            run_with(&store, &saved.id, &[("id", value(ParamKind::Number, text))]);
        }
        let run = run_with(&store, &saved.id, &[("id", value(ParamKind::Text, "3"))]);
        let mut expected = vec![value(ParamKind::Text, "3")];
        expected.extend(numbers(&["1", "3", "2"]));
        assert_eq!(run.recent_values["id"], expected, "the same text with another kind is another value");
    }

    #[test]
    fn recent_values_keep_at_most_five() {
        let (_dir, _, store) = store();
        let saved = save_query(&store, None, input("Orders", None)).unwrap();
        let mut run = saved.clone();
        for text in ["1", "2", "3", "4", "5", "6", "7"] {
            run = run_with(&store, &saved.id, &[("id", value(ParamKind::Number, text))]);
        }
        assert_eq!(crate::saved_queries::RECENT_VALUES, 5);
        assert_eq!(run.recent_values["id"], numbers(&["7", "6", "5", "4", "3"]));
    }

    #[test]
    fn each_parameter_keeps_its_own_recent_values() {
        let (_dir, _, store) = store();
        let saved = save_query(&store, None, input("Orders", None)).unwrap();
        run_with(
            &store,
            &saved.id,
            &[("id", value(ParamKind::Number, "1")), ("day", value(ParamKind::Date, "2024-01-05"))],
        );
        let run = run_with(&store, &saved.id, &[("id", value(ParamKind::Number, "2"))]);
        assert_eq!(run.recent_values["id"], numbers(&["2", "1"]));
        assert_eq!(run.recent_values["day"], [value(ParamKind::Date, "2024-01-05")]);
    }

    #[test]
    fn a_run_without_values_leaves_recent_values_untouched() {
        let (_dir, _, store) = store();
        let saved = save_query(&store, None, input("Orders", None)).unwrap();
        let run = run_with(&store, &saved.id, &[("id", value(ParamKind::Number, "1"))]);
        let again = mark_saved_query_run(&store, &saved.id, None).unwrap();
        assert_eq!(again.recent_values, run.recent_values);
        assert_eq!(again.last_values, run.last_values);
    }

    #[test]
    fn duplicating_keeps_recent_values() {
        let (_dir, _, store) = store();
        let saved = save_query(&store, None, input("Orders", None)).unwrap();
        let run = run_with(&store, &saved.id, &[("id", value(ParamKind::Number, "1"))]);
        let copy = duplicate_saved_query(&store, &saved.id).unwrap();
        assert_eq!(copy.recent_values, run.recent_values);
        assert_eq!(copy.last_values, run.last_values);
    }

    #[test]
    fn a_file_without_recent_values_loads_with_none() {
        let (_dir, file, _) = store();
        fs::write(
            &file,
            r#"[{"id":"q1","name":"Orders","sql":"select :id","scope":{"kind":"engine"},"engine":"postgres",
                "createdAt":1,"updatedAt":1,"lastValues":{"id":{"kind":"number","value":"7"}}}]"#,
        )
        .unwrap();
        let store = SavedQueries::load(file).unwrap();
        let loaded = store.get("q1").unwrap();
        assert!(loaded.recent_values.is_empty());
        assert_eq!(loaded.last_values["id"], value(ParamKind::Number, "7"));
        assert!(store.set_aside().is_none());
    }

    #[test]
    fn reloading_from_disk_keeps_everything() {
        let (_dir, file, store) = store();
        let saved = save_query(&store, None, input("Orders", Some("Ops"))).unwrap();
        let values = BTreeMap::from([("day".to_string(), value(ParamKind::Date, "2024-01-05"))]);
        let run = mark_saved_query_run(&store, &saved.id, Some(values)).unwrap();
        assert_eq!(list_saved_queries(&SavedQueries::load(file).unwrap()), [run]);
    }

    #[test]
    fn writes_camel_case_json() {
        let (_dir, file, store) = store();
        save_query(&store, None, input("Orders", None)).unwrap();
        let json = fs::read_to_string(file).unwrap();
        assert!(json.contains("\"createdAt\""), "{json}");
        assert!(json.contains("\"kind\": \"connection\"") && json.contains("\"connectionId\": \"c1\""), "{json}");
        assert!(json.contains("\"lastValues\""), "{json}");
        assert!(json.contains("\"recentValues\""), "{json}");
    }

    #[test]
    fn a_damaged_file_is_set_aside_and_the_store_starts_empty() {
        let (_dir, file, _) = store();
        fs::write(&file, "{ not json").unwrap();
        let store = SavedQueries::load(file.clone()).unwrap();
        assert!(list_saved_queries(&store).is_empty());
        let kept = store.set_aside().expect("the damaged file is kept aside");
        assert_eq!(fs::read_to_string(kept).unwrap(), "{ not json");
        assert!(!file.exists());
    }
}
