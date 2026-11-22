//! The saved queries as the window knows them: one list every view reads
//! (the Library, consoles opened from a query, the palette), read from the
//! core off the UI thread and read again after each change, so all of them
//! follow at once. Observe it to hear of a change.
//!
//! An agent's change through the MCP server reaches it as the core's
//! [`Event::SavedQueriesChanged`], and the list is read again.
//!
//! Changes run on the core's threads, so they may finish in any order: each
//! one changes and reads the list in turn, numbered, and only a list newer
//! than the one shown replaces it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use dbforge_core::Event;
use dbforge_core::error::AppError;
use dbforge_core::saved_queries::commands::{
    delete_saved_query, duplicate_saved_query, list_saved_queries, mark_saved_query_run, save_query,
};
use dbforge_core::saved_queries::{ParamValue, SavedQuery, SavedQueryInput};
use gpui::{App, AppContext, Entity, Global, Subscription, Task};

use crate::backend::Backend;
use crate::events::EventHub;
use crate::i18n::Translate;

/// Numbers each change and the list read after it, one change at a time.
static TURN: Mutex<u64> = Mutex::new(0);

/// The saved queries, once read.
#[derive(Debug, Default)]
pub struct SavedStore {
    queries: Vec<SavedQuery>,
    loaded: bool,
    /// The turn of the list shown.
    shown: u64,
}

struct GlobalSaved {
    store: Entity<SavedStore>,
    /// Reads the list again when an agent changes it.
    _agents: Option<Subscription>,
}

impl Global for GlobalSaved {}

impl SavedStore {
    /// The window's saved queries, read the first time they are asked for.
    pub fn global(cx: &mut App) -> Entity<SavedStore> {
        if let Some(global) = cx.try_global::<GlobalSaved>() {
            return global.store.clone();
        }
        let store = cx.new(|_| SavedStore::default());
        let agents_changes = EventHub::try_global(cx).map(|hub| {
            cx.subscribe(&hub, |_, event: &Event, cx| {
                if matches!(event, Event::SavedQueriesChanged) {
                    reload(cx);
                }
            })
        });
        cx.set_global(GlobalSaved { store: store.clone(), _agents: agents_changes });
        reload(cx);
        store
    }

    /// The window's saved queries when they were asked for already, from a
    /// context that can't create them.
    pub fn global_read(cx: &App) -> Option<&SavedStore> {
        cx.try_global::<GlobalSaved>().map(|global| global.store.read(cx))
    }

    /// Every saved query, by folder and then by name.
    pub fn queries(&self) -> &[SavedQuery] {
        &self.queries
    }

    /// Whether the list was read: until then, no query is known to be missing.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn get(&self, id: &str) -> Option<&SavedQuery> {
        self.queries.iter().find(|query| query.id == id)
    }

    /// Shows the list read at `turn`, unless a newer one is shown.
    fn show(&mut self, turn: u64, queries: Vec<SavedQuery>) -> bool {
        if turn <= self.shown {
            return false;
        }
        self.shown = turn;
        self.queries = queries;
        self.loaded = true;
        true
    }
}

/// Reads the list again.
pub fn reload(cx: &mut App) {
    change(cx, |_| Ok(())).detach();
}

/// Creates a saved query (`id` is `None`) or changes saved query `id`.
pub fn save(id: Option<String>, input: SavedQueryInput, cx: &mut App) -> Task<Result<SavedQuery, String>> {
    change(cx, move |core| save_query(core.saved_queries(), id.as_deref(), input))
}

/// A copy of saved query `id` named "<name> copy".
pub fn duplicate(id: String, cx: &mut App) -> Task<Result<SavedQuery, String>> {
    change(cx, move |core| duplicate_saved_query(core.saved_queries(), &id))
}

pub fn delete(id: String, cx: &mut App) -> Task<Result<(), String>> {
    change(cx, move |core| delete_saved_query(core.saved_queries(), &id))
}

/// Records that saved query `id` ran, remembering `values` when given.
pub fn mark_run(
    id: String,
    values: Option<BTreeMap<String, ParamValue>>,
    cx: &mut App,
) -> Task<Result<SavedQuery, String>> {
    change(cx, move |core| mark_saved_query_run(core.saved_queries(), &id, values))
}

/// Runs `call` on the core off the UI thread, then shows the list as it is
/// now; its error in the interface language.
fn change<T: Send + 'static>(
    cx: &mut App,
    call: impl FnOnce(&dbforge_core::Core) -> dbforge_core::Result<T> + Send + 'static,
) -> Task<Result<T, String>> {
    let store = SavedStore::global(cx);
    let not_found = cx.messages().saved.not_found;
    let running = Backend::global(cx).spawn_blocking(cx, move |core| {
        let mut turn = TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *turn += 1;
        let outcome = call(core);
        (outcome, *turn, list_saved_queries(core.saved_queries()))
    });
    cx.spawn(async move |cx| {
        let (outcome, turn, queries) = running.await;
        let _ = store.update(cx, |store, cx| {
            if store.show(turn, queries) {
                cx.notify();
            }
        });
        outcome.map_err(|error| match error {
            // The core names any missing item a connection.
            AppError::NotFound(_) => not_found.to_owned(),
            error => error.to_string(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> SavedQuery {
        serde_json::from_value(serde_json::json!({
            "id": name, "name": name, "sql": "select 1", "scope": { "kind": "engine" }, "engine": "postgres",
            "createdAt": 0, "updatedAt": 0,
        }))
        .expect("a saved query")
    }

    #[test]
    fn a_list_read_before_the_one_shown_never_replaces_it() {
        let mut store = SavedStore::default();
        assert!(!store.is_loaded());
        assert!(store.show(2, vec![named("new")]));
        assert!(!store.show(1, Vec::new()), "the list read before the last change");
        assert_eq!(store.queries(), [named("new")]);
        assert!(store.is_loaded());
        assert!(store.show(3, Vec::new()));
        assert!(store.queries().is_empty());
    }

    #[gpui::test]
    fn reads_the_list_again_when_an_agent_changes_it(cx: &mut gpui::TestAppContext) {
        use dbforge_core::connections::model::Engine;
        use dbforge_core::saved_queries::SavedScope;

        let _dir = crate::test_support::setup(cx);
        let store = cx.update(SavedStore::global);
        let loaded = |cx: &mut gpui::TestAppContext, count: usize| {
            let start = std::time::Instant::now();
            while cx.update(|cx| (store.read(cx).is_loaded(), store.read(cx).queries().len())) != (true, count) {
                assert!(start.elapsed() < std::time::Duration::from_secs(15), "timed out waiting for {count} queries");
                cx.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        loaded(cx, 0);
        // An agent saves one through the MCP server: the core, not this window, changes it.
        let core = cx.update(|cx| Backend::global(cx).core().clone());
        let input = SavedQueryInput {
            name: "Orders".into(),
            sql: "select 1".into(),
            description: None,
            folder: None,
            scope: SavedScope::Engine,
            engine: Engine::Postgres,
        };
        core.saved_queries().save(None, input).unwrap();
        cx.update(|cx| EventHub::global(cx).update(cx, |_, cx| cx.emit(Event::SavedQueriesChanged)));
        loaded(cx, 1);
    }
}
