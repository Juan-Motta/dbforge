//! Saved queries drawn headless, over a core in a temporary directory: the
//! Library's two views, saving a console with ⌘S, and running a saved
//! query with parameters.

use std::cell::RefCell;
use std::rc::Rc;

use dbforge_core::connections::model::{ConnectionInput, Engine};
use dbforge_core::query::QueryScope;
use dbforge_core::saved_queries::{ParamKind, SavedQuery, SavedQueryInput, SavedScope};
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};

use super::store::{self, SavedStore};
use super::{LibraryPanel, LibraryView};
use crate::backend::Backend;
use crate::preferences::{Preferences, keys};
use crate::test_support::{draw, setup, wait_until};
use crate::workspace::Workspace;

/// A saved PostgreSQL connection nothing answers on, for the core to find.
fn save_postgres(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let input: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "Vivi Dev", "engine": "postgres", "environment": null,
            "host": "127.0.0.1", "port": 1, "database": "vivi", "username": "u",
        }))
        .expect("an input");
        Backend::global(cx).core().connections().save(input.into_parts().expect("valid")).expect("saved").id
    })
}

/// Saves `input` through the window's store and waits for the list to show it.
fn save_query(input: SavedQueryInput, cx: &mut VisualTestContext) -> SavedQuery {
    let done: Rc<RefCell<Option<Result<SavedQuery, String>>>> = Rc::default();
    let into = done.clone();
    cx.update(|_, cx| {
        let saving = store::save(None, input, cx);
        cx.spawn(async move |_| *into.borrow_mut() = Some(saving.await)).detach();
    });
    wait_until(cx, "the query to be saved", |_| done.borrow().is_some());
    let saved = done.take().expect("an outcome").expect("saved");
    assert!(cx.update(|_, cx| SavedStore::global(cx).read(cx).get(&saved.id).is_some()), "the list shows it");
    saved
}

fn queries(cx: &mut VisualTestContext) -> Vec<SavedQuery> {
    cx.update(|_, cx| SavedStore::global(cx).read(cx).queries().to_vec())
}

#[gpui::test]
fn the_library_switches_between_history_and_saved_and_remembers_the_choice(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (library, cx) = cx.add_window_view(|window, cx| LibraryPanel::new("pg".into(), Engine::Postgres, window, cx));
    assert_eq!(library.read_with(cx, |library, _| library.view()), LibraryView::History, "History the first time");
    draw(cx);
    save_query(
        SavedQueryInput {
            name: "Orders by status".into(),
            sql: "select status, count(*) from orders where day >= :since group by 1".into(),
            description: Some("Count per status".into()),
            folder: Some("Reports".into()),
            scope: SavedScope::Engine,
            engine: Engine::Postgres,
        },
        cx,
    );
    library.update(cx, |library, cx| library.show(LibraryView::Saved, cx));
    draw(cx);
    library.read_with(cx, |library, cx| {
        assert_eq!(library.view(), LibraryView::Saved);
        let folders = library.saved().read(cx).folders(cx);
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name.as_deref(), Some("Reports"));
        assert_eq!(folders[0].queries[0].name, "Orders by status");
    });
    let saved = cx.update(|_, cx| Preferences::global(cx).get(keys::LIBRARY_VIEW).map(str::to_owned));
    assert_eq!(saved.as_deref(), Some("saved"));
    // The next Library opens on Saved; History still works there.
    let next = cx.update(|window, cx| cx.new(|cx| LibraryPanel::new("pg".into(), Engine::Postgres, window, cx)));
    assert_eq!(next.read_with(cx, |library, _| library.view()), LibraryView::Saved);
    library.update(cx, |library, cx| library.show(LibraryView::History, cx));
    draw(cx);
    let history = library.read_with(cx, |library, _| library.history().clone());
    wait_until(cx, "the history to load", |cx| history.read_with(cx, |history, _| !history.is_loading()));
    draw(cx);
}

#[gpui::test]
fn secondary_s_asks_how_to_save_a_new_console_then_updates_the_saved_query(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    cx.update(|cx| {
        cx.bind_keys(crate::query::key_bindings());
        cx.bind_keys(crate::ui::text_input::key_bindings());
        cx.bind_keys(super::key_bindings());
    });
    let (workspace, cx) = cx.add_window_view(|_, _| Workspace::default());
    let connection = save_postgres(cx);
    let sql = "select * from orders where id = :id";
    let console: Entity<crate::query::QueryConsole> = workspace
        .update_in(cx, |workspace, window, cx| {
            workspace.new_query(&connection, QueryScope::default(), Some(sql.into()), window, cx)
        })
        .expect("a console");
    draw(cx);
    cx.simulate_keystrokes("secondary-s");
    draw(cx);
    let form = console.read_with(cx, |console, _| console.save_form().cloned()).expect("the save form");
    // Saving without a name asks for one and saves nothing.
    form.update_in(cx, |form, window, cx| form.save(window, cx));
    cx.run_until_parked();
    assert!(queries(cx).is_empty());
    form.update_in(cx, |form, window, cx| {
        form.set_name("Order by id", cx);
        form.save(window, cx);
    });
    wait_until(cx, "the console to become the saved query", |cx| {
        console.read_with(cx, |console, _| console.saved_query().is_some())
    });
    draw(cx);
    let saved = queries(cx);
    assert_eq!(saved.len(), 1);
    assert_eq!((saved[0].name.as_str(), saved[0].sql.as_str()), ("Order by id", sql));
    assert_eq!(saved[0].scope, SavedScope::Connection { connection_id: connection.clone() });
    console.read_with(cx, |console, _| {
        assert!(console.save_form().is_none(), "the form closed");
        assert!(!console.is_edited());
    });
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.tabs().active().map(|tab| tab.title.as_str()), Some("Order by id"));
    });
    // Edited, the tab shows a dot; ⌘S then updates the query without asking.
    console.update(cx, |console, cx| console.editor().update(cx, |editor, cx| editor.set_text("select 2", cx)));
    cx.run_until_parked();
    assert!(console.read_with(cx, |console, _| console.is_edited()));
    draw(cx);
    cx.simulate_keystrokes("secondary-s");
    assert!(console.read_with(cx, |console, _| console.save_form().is_none()));
    wait_until(cx, "the query to be updated", |cx| queries(cx).first().is_some_and(|query| query.sql == "select 2"));
    cx.run_until_parked();
    assert!(!console.read_with(cx, |console, _| console.is_edited()));
    assert_eq!(queries(cx).len(), 1);
}

#[gpui::test]
fn running_a_query_with_parameters_asks_for_valid_values_first(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (workspace, cx) = cx.add_window_view(|_, _| Workspace::default());
    let connection = save_postgres(cx);
    let query = save_query(
        SavedQueryInput {
            name: "Orders of a day".into(),
            sql: "select * from orders where id = :id and day = :day".into(),
            description: None,
            folder: None,
            scope: SavedScope::Connection { connection_id: connection.clone() },
            engine: Engine::Postgres,
        },
        cx,
    );
    workspace.update_in(cx, |workspace, window, cx| workspace.run_saved(query.clone(), None, window, cx));
    let dialog = workspace.read_with(cx, |workspace, _| workspace.run_dialog().cloned()).expect("the dialog");
    assert!(workspace.read_with(cx, |workspace, _| workspace.tabs().tabs().is_empty()), "nothing runs yet");
    draw(cx);
    dialog.update_in(cx, |dialog, window, cx| {
        dialog.set_value("id", ParamKind::Number, "abc", window, cx);
        dialog.set_value("day", ParamKind::Date, "2026-01-02", window, cx);
        assert!(!dialog.can_run(cx), "a number that isn't one");
        let error = dialog.outcome(cx).expect_err("invalid");
        assert_eq!(super::rules::param_of(&error), Some("id"));
        dialog.run(cx);
    });
    draw(cx);
    assert!(workspace.read_with(cx, |workspace, _| workspace.run_dialog().is_some()), "Run did nothing");
    dialog.update_in(cx, |dialog, window, cx| {
        dialog.set_value("id", ParamKind::Number, "7", window, cx);
        let preview = dialog.outcome(cx).expect("valid values");
        assert_eq!(preview.sql, "select * from orders where id = 7 and day = '2026-01-02'");
        assert_eq!(&preview.sql[preview.spans[0].clone()], "7");
        dialog.run(cx);
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        assert!(workspace.run_dialog().is_none(), "the dialog closed");
        let tab = workspace.tabs().active().expect("a console for the query");
        assert_eq!(tab.title, "Orders of a day");
        let console = workspace.console(&tab.id).expect("its console").read(cx);
        assert_eq!(console.saved_query().map(|saved| saved.id.as_str()), Some(query.id.as_str()));
    });
    wait_until(cx, "the run to be recorded", |cx| {
        queries(cx).first().is_some_and(|saved| saved.last_values.get("id").is_some_and(|value| value.value == "7"))
    });
    draw(cx);
}
