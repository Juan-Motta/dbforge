//! The key browser drawn headless: its tab, its dialogs, and (with a live
//! server) the round trip of a key's edits.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dbforge_core::connections::model::ConnectionInput;
use gpui::{AnyView, Entity, TestAppContext, VisualTestContext};

use super::dialog::FormDialog;
use super::format::FieldName;
use super::{KeyBrowser, KeyBrowserEvent};
use crate::backend::{Backend, test_backend};
use crate::i18n::{Language, Locale};
use crate::theme::{Appearance, Theme};
use crate::workspace::Workspace;

fn setup(cx: &mut TestAppContext) -> tempfile::TempDir {
    cx.update(|cx| {
        cx.set_global(Theme::new(Appearance::Dark));
        cx.set_global(Locale(Language::En));
        let dir = test_backend(cx);
        crate::preferences::test_preferences(cx, dir.path());
        dir
    })
}

/// A saved Redis connection to `host:port`, for the core to find.
fn save_redis(host: &str, port: u16, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let input: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "cache", "engine": "redis", "environment": "production",
            "host": host, "port": port, "database": "", "username": "",
        }))
        .expect("an input");
        let parts = input.into_parts().expect("a valid connection");
        Backend::global(cx).core().connections().save(parts).expect("saved").id
    })
}

/// Runs the app until `done` holds, the core's calls answering on their own threads.
fn wait_until(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let start = Instant::now();
    while !done(cx) {
        assert!(start.elapsed() < Duration::from_secs(15), "timed out waiting for {what}");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The dialogs a browser sent to the window, in order (`None`: closed).
type Dialogs = Rc<RefCell<Vec<Option<AnyView>>>>;

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
}

#[gpui::test]
fn opens_one_key_browser_per_database_and_shows_why_the_server_is_unreachable(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (workspace, cx) = cx.add_window_view(|_, _| Workspace::default());
    // Nothing listens on port 1.
    let id = save_redis("127.0.0.1", 1, cx);
    let browser = workspace
        .update_in(cx, |workspace, window, cx| {
            workspace.open_keyspace(&id, "db0", window, cx);
            workspace.open_keyspace(&id, "db1", window, cx);
            workspace.open_keyspace(&id, "db0", window, cx)
        })
        .expect("a key browser");
    workspace.read_with(cx, |workspace, _| {
        let titles: Vec<&str> = workspace.tabs().tabs().iter().map(|tab| tab.title.as_str()).collect();
        assert_eq!(titles, ["cache · db0", "cache · db1"]);
        assert_eq!(workspace.tabs().active_id(), Some(format!("keyspace:{id}:db0").as_str()));
    });
    draw(cx);
    let list = browser.read_with(cx, |browser, _| browser.list().clone());
    wait_until(cx, "the scan to fail", |cx| list.read_with(cx, |list, _| !list.is_loading()));
    list.read_with(cx, |list, _| {
        assert!(list.error().is_some(), "the scan's error is shown");
        assert!(list.keys().is_empty());
    });
    draw(cx);
}

#[gpui::test]
fn opens_nothing_for_a_connection_that_is_not_redis(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (workspace, cx) = cx.add_window_view(|_, _| Workspace::default());
    let id = cx.update(|_, cx| {
        let input: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "Shop", "engine": "sqlite", "environment": null,
            "host": "", "port": 0, "database": "/nowhere/shop.db", "username": "",
        }))
        .expect("an input");
        Backend::global(cx).core().connections().save(input.into_parts().expect("valid")).expect("saved").id
    });
    let opened = workspace.update_in(cx, |workspace, window, cx| workspace.open_keyspace(&id, "main", window, cx));
    assert!(opened.is_none());
    workspace.read_with(cx, |workspace, _| assert!(workspace.tabs().tabs().is_empty()));
}

/// A browser on an unreachable server, and the dialogs it sends to the window.
fn browser_with_dialogs(cx: &mut TestAppContext) -> (Entity<KeyBrowser>, Dialogs, &mut VisualTestContext) {
    // Nothing listens on port 1.
    browser_with_dialogs_on(cx, "127.0.0.1", 1)
}

fn last_dialog(dialogs: &RefCell<Vec<Option<AnyView>>>) -> Entity<FormDialog> {
    dialogs.borrow().last().cloned().flatten().expect("a dialog").downcast::<FormDialog>().expect("a form dialog")
}

#[gpui::test]
fn the_add_key_dialog_asks_for_what_the_type_picked_needs(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (browser, dialogs, cx) = browser_with_dialogs(cx);
    browser.update_in(cx, |browser, window, cx| browser.open_add_key(window, cx));
    let dialog = last_dialog(&dialogs);
    use FieldName::*;
    dialog.read_with(cx, |dialog, cx| assert_eq!(dialog.shown_fields(cx), [Name, Kind, Value, Ttl]));
    dialog.update(cx, |dialog, cx| dialog.fill(Kind, "zset", cx));
    dialog.read_with(cx, |dialog, cx| assert_eq!(dialog.shown_fields(cx), [Name, Kind, Value, Score, Ttl]));
    dialog.update(cx, |dialog, cx| dialog.fill(Kind, "hash", cx));
    dialog.read_with(cx, |dialog, cx| assert_eq!(dialog.shown_fields(cx), [Name, Kind, Field, Value, Ttl]));
    // Drawn as the window's dialog layer would draw it.
    let (_host, cx2) = cx.add_window_view(|_, _| Host(dialog.clone().into()));
    draw(cx2);
}

#[gpui::test]
fn a_dialog_refused_at_once_stays_open_with_its_error(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (browser, dialogs, cx) = browser_with_dialogs(cx);
    browser.update_in(cx, |browser, window, cx| browser.open_add_key(window, cx));
    let dialog = last_dialog(&dialogs);
    dialog.update(cx, |dialog, cx| {
        dialog.fill(FieldName::Name, "k", cx);
        dialog.fill(FieldName::Ttl, "-5", cx);
    });
    dialog.update(cx, |dialog, cx| dialog.run_submit(cx));
    cx.run_until_parked();
    dialog.read_with(cx, |dialog, _| assert_eq!(dialog.error(), Some("Enter a number of seconds greater than 0.")));
    assert!(dialogs.borrow().last().cloned().flatten().is_some(), "the dialog is still shown");
}

#[gpui::test]
fn a_failed_call_is_shown_in_the_dialog_which_stays_open(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (browser, dialogs, cx) = browser_with_dialogs(cx);
    browser.update_in(cx, |browser, window, cx| browser.open_add_key(window, cx));
    let dialog = last_dialog(&dialogs);
    dialog.update(cx, |dialog, cx| {
        dialog.fill(FieldName::Name, "k", cx);
        dialog.run_submit(cx);
    });
    wait_until(cx, "the create to fail", |cx| dialog.read_with(cx, |dialog, _| dialog.error().is_some()));
    assert!(dialogs.borrow().last().cloned().flatten().is_some(), "the dialog is still shown");
}

struct Host(gpui::AnyView);

impl gpui::Render for Host {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        gpui::div().size_full().child(self.0.clone())
    }
}

/// With `DBFORGE_TEST_REDIS=host:port` (a throwaway server: it writes keys
/// named `dbforge-test:*`), the browser lists, edits, renames and deletes a key.
#[gpui::test]
#[ignore = "needs a Redis server in DBFORGE_TEST_REDIS"]
fn edits_a_key_on_a_live_server(cx: &mut TestAppContext) {
    use super::detail::DialogKind;
    let Ok(address) = std::env::var("DBFORGE_TEST_REDIS") else { return };
    let (host, port) = address.rsplit_once(':').expect("host:port");
    let _dir = setup(cx);
    let (browser, dialogs, cx) = browser_with_dialogs_on(cx, host, port.parse().expect("a port"));
    let list = browser.read_with(cx, |browser, _| browser.list().clone());
    wait_until(cx, "the first scan", |cx| list.read_with(cx, |list, _| !list.is_loading()));
    list.update(cx, |list, cx| list.set_pattern("dbforge-test:*".into(), cx));

    // Add a hash with its first field, expiring.
    browser.update_in(cx, |browser, window, cx| browser.open_add_key(window, cx));
    let dialog = last_dialog(&dialogs);
    dialog.update(cx, |dialog, cx| {
        dialog.fill(FieldName::Name, "dbforge-test:user", cx);
        dialog.fill(FieldName::Kind, "hash", cx);
        dialog.fill(FieldName::Field, "name", cx);
        dialog.fill(FieldName::Value, "Ada", cx);
        dialog.fill(FieldName::Ttl, "120", cx);
        dialog.run_submit(cx);
    });
    wait_until(cx, "the new key to be listed and selected", |cx| {
        let listed = list.read_with(cx, |list, _| list.keys().iter().any(|key| key.name == "dbforge-test:user"));
        let shown = browser.read_with(cx, |browser, cx| {
            browser.detail().and_then(|detail| detail.read(cx).value().map(|value| value.name.clone()))
        });
        listed && shown.as_deref() == Some("dbforge-test:user")
    });
    let detail = browser.read_with(cx, |browser, _| browser.detail().cloned().expect("a detail"));
    detail.read_with(cx, |detail, _| {
        let value = detail.value().expect("read");
        assert_eq!((value.kind.as_str(), value.length), ("hash", 1));
        assert!(value.ttl_ms.is_some_and(|ttl| ttl > 100_000), "it expires");
    });

    // Remove its TTL.
    browser.update_in(cx, |browser, window, cx| browser.open_key_dialog(DialogKind::Ttl, window, cx));
    let dialog = last_dialog(&dialogs);
    dialog.read_with(cx, |dialog, cx| assert_eq!(dialog.shown_fields(cx), [FieldName::Ttl]));
    dialog.update(cx, |dialog, cx| dialog.run_secondary(cx));
    wait_until(cx, "the TTL to go", |cx| {
        detail.read_with(cx, |detail, _| detail.value().is_some_and(|value| value.ttl_ms.is_none()))
    });

    // Rename it, then delete it.
    browser.update_in(cx, |browser, window, cx| browser.open_key_dialog(DialogKind::Rename, window, cx));
    let dialog = last_dialog(&dialogs);
    dialog.update(cx, |dialog, cx| {
        dialog.fill(FieldName::Name, "dbforge-test:renamed", cx);
        dialog.run_submit(cx);
    });
    wait_until(cx, "the renamed key to be shown", |cx| {
        browser.read_with(cx, |browser, cx| {
            browser.detail().and_then(|detail| detail.read(cx).value().map(|value| value.name.clone()))
                == Some("dbforge-test:renamed".into())
        })
    });
    browser.update_in(cx, |browser, window, cx| browser.open_key_dialog(DialogKind::Delete, window, cx));
    last_dialog(&dialogs).update(cx, |dialog, cx| dialog.run_submit(cx));
    wait_until(cx, "the key to be gone", |cx| {
        let none = browser.read_with(cx, |browser, _| browser.detail().is_none());
        none && list.read_with(cx, |list, _| !list.is_loading() && list.keys().is_empty())
    });
}

fn browser_with_dialogs_on<'a>(
    cx: &'a mut TestAppContext,
    host: &str,
    port: u16,
) -> (Entity<KeyBrowser>, Dialogs, &'a mut VisualTestContext) {
    let (workspace, cx) = cx.add_window_view(|_, _| Workspace::default());
    let id = save_redis(host, port, cx);
    let browser = workspace
        .update_in(cx, |workspace, window, cx| workspace.open_keyspace(&id, "db0", window, cx))
        .expect("a key browser");
    let dialogs: Dialogs = Default::default();
    let seen = dialogs.clone();
    cx.update(|_, cx| {
        cx.subscribe(&browser, move |_, KeyBrowserEvent::Dialog(dialog), _| seen.borrow_mut().push(dialog.clone()))
            .detach();
    });
    (browser, dialogs, cx)
}
