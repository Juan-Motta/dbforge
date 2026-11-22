//! Manual transactions end to end, over SQLite files in the test's directory:
//! a console's run opens one, its tab shows it, Commit and Rollback end it,
//! and closing its tab or quitting asks first.

use std::cell::RefCell;
use std::rc::Rc;

use dbforge_core::connections::model::{Connection, ConnectionInput};
use dbforge_core::query::{QueryScope, TransactionEnd, TransactionMode, close_session, run_query};
use gpui::{
    AnyView, AppContext, DismissEvent, Entity, IntoElement, ParentElement, Render, Styled, TestAppContext,
    VisualTestContext, Window, div,
};

use super::{LeaveDialog, LeaveEvent, Leaving, Pending};
use crate::backend::{Backend, runtime};
use crate::query::QueryConsole;
use crate::query::results::LastRun;
use crate::test_support::{draw, setup, wait_until};
use crate::ui::notice::Notices;
use crate::workspace::Workspace;

const INSERT: &str = "INSERT INTO people (name) VALUES ('Ada')";

/// A SQLite file `name`.db in `dir` with an empty table `people`, saved as a
/// connection tagged `environment` (the default tags: `production` asks first).
fn sqlite(dir: &std::path::Path, name: &str, environment: &str, cx: &mut VisualTestContext) -> Connection {
    let file = dir.join(format!("{name}.db"));
    // An empty file is an empty SQLite database; the driver opens, never creates.
    std::fs::File::create(&file).expect("the database file");
    let connection = cx.update(|_, cx| {
        let input: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": name, "engine": "sqlite", "environment": environment, "host": "", "port": 0,
            "database": file.display().to_string(), "username": "",
        }))
        .expect("an input");
        Backend::global(cx).core().connections().save(input.into_parts().expect("valid")).expect("saved")
    });
    let created = run(cx, &connection, "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT)");
    assert_eq!(created, None);
    connection
}

/// Runs `sql` in a console session of its own, then closes it: what another
/// tab would see. The first value of its last result, or its error.
fn run(cx: &mut VisualTestContext, connection: &Connection, sql: &str) -> Option<String> {
    let core = cx.update(|_, cx| Backend::global(cx).core().clone());
    let (store, sessions, history, log) =
        (core.connections(), core.query_sessions(), core.history(), core.statement_log());
    let session = format!("probe:{}", uuid::Uuid::new_v4());
    let outcome = runtime().block_on(run_query(
        store,
        sessions,
        history,
        log,
        session.clone(),
        connection.id.clone(),
        QueryScope::default(),
        sql.to_owned(),
    ));
    runtime().block_on(close_session(sessions, log, session)).expect("closed");
    let outcome = outcome.expect("ran");
    match outcome.error {
        Some(error) => Some(error),
        None => outcome.results.last().and_then(|result| result.rows.first()).and_then(|row| row[0].clone()),
    }
}

fn people(cx: &mut VisualTestContext, connection: &Connection) -> String {
    run(cx, connection, "SELECT count(*) FROM people").expect("a count")
}

/// A workspace filling a window, with a console on `connection` in Manual.
fn console_in_manual<'a>(
    connection: &Connection,
    cx: &'a mut TestAppContext,
) -> (Entity<Workspace>, Entity<QueryConsole>, &'a mut VisualTestContext) {
    let (workspace, cx) = cx.add_window_view(|_, _| Workspace::default());
    let console = workspace
        .update_in(cx, |workspace, window, cx| {
            workspace.new_query(&connection.id, QueryScope::default(), None, window, cx)
        })
        .expect("a console");
    console.update(cx, |console, cx| console.set_mode(TransactionMode::Manual, cx));
    (workspace, console, cx)
}

fn run_in(console: &Entity<QueryConsole>, sql: &str, cx: &mut VisualTestContext) {
    console.update(cx, |console, cx| console.run(sql.to_owned(), cx));
    wait_until(cx, "the run", |cx| console.read_with(cx, |console, _| !console.is_running()));
}

fn notice(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| Notices::global(cx).read(cx).current().map(|notice| notice.text.to_string()))
}

/// Draws a dialog filling a window, as the window's dialog layer would.
struct Host(AnyView);

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

#[gpui::test]
fn a_manual_run_opens_a_transaction_that_commit_makes_visible_to_other_tabs(cx: &mut TestAppContext) {
    let dir = setup(cx);
    let (_, scratch) = cx.add_window_view(|_, _| gpui::Empty);
    let connection = sqlite(dir.path(), "shop", "development", scratch);
    let (workspace, console, cx) = console_in_manual(&connection, cx);
    run_in(&console, INSERT, cx);
    console.read_with(cx, |console, _| {
        let transaction = console.transaction().expect("a transaction is open");
        assert_eq!((transaction.changes, transaction.rows), (1, 1));
    });
    // The capsule in the toolbar and the amber dot on the tab.
    draw(cx);
    assert_eq!(people(cx, &connection), "0", "other tabs don't see it yet");
    console.update(cx, |console, cx| console.set_mode(TransactionMode::Auto, cx));
    assert_eq!(console.read_with(cx, |console, _| console.transaction_mode()), Some(TransactionMode::Manual));

    console.update(cx, |console, cx| console.end_transaction(TransactionEnd::Committed, cx));
    wait_until(cx, "the commit", |cx| console.read_with(cx, |console, _| console.ending().is_none()));
    console.read_with(cx, |console, _| assert!(console.transaction().is_none()));
    assert_eq!(notice(cx).as_deref(), Some("Committed 1 change (1 row)"));
    assert_eq!(people(cx, &connection), "1");
    draw(cx);
    // With nothing open, the tab closes without asking.
    let id = console.read_with(cx, |console, _| console.session_id().to_owned());
    workspace.update_in(cx, |workspace, window, cx| workspace.close_tab(&id, window, cx));
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.tabs().tabs().is_empty());
        assert!(workspace.leave_dialog().is_none());
    });
}

#[gpui::test]
fn closing_a_tab_with_a_transaction_open_asks_and_rolling_back_closes_it(cx: &mut TestAppContext) {
    let dir = setup(cx);
    let (_, scratch) = cx.add_window_view(|_, _| gpui::Empty);
    let connection = sqlite(dir.path(), "shop", "development", scratch);
    let (workspace, console, cx) = console_in_manual(&connection, cx);
    let dialogs: Rc<RefCell<Vec<Option<AnyView>>>> = Rc::default();
    let seen = dialogs.clone();
    let _listening = cx.update(|_, cx| {
        cx.subscribe(&workspace, move |_, event: &crate::workspace::WorkspaceEvent, _| {
            if let crate::workspace::WorkspaceEvent::Dialog(dialog) = event {
                seen.borrow_mut().push(dialog.clone());
            }
        })
    });
    run_in(&console, INSERT, cx);
    let id = console.read_with(cx, |console, _| console.session_id().to_owned());

    // Cancel keeps the tab and its transaction.
    workspace.update_in(cx, |workspace, window, cx| workspace.close_tab(&id, window, cx));
    let dialog = workspace.read_with(cx, |workspace, _| workspace.leave_dialog().cloned()).expect("it asks");
    assert!(dialogs.borrow().last().cloned().flatten().is_some(), "shown over the window");
    dialog.update(cx, |_, cx| cx.emit(DismissEvent));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.tabs().tabs().len(), 1);
        assert!(workspace.leave_dialog().is_none());
    });
    assert!(dialogs.borrow().last().cloned().flatten().is_none(), "and closed");

    workspace.update_in(cx, |workspace, window, cx| workspace.close_tab(&id, window, cx));
    let dialog = workspace.read_with(cx, |workspace, _| workspace.leave_dialog().cloned()).expect("it asks again");
    let (_host, host_cx) = cx.add_window_view({
        let view: AnyView = dialog.clone().into();
        move |_, _| Host(view)
    });
    draw(host_cx);
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(dialog.pending().len(), 1);
        assert_eq!(dialog.pending()[0].transaction.connection_name, "shop");
        assert!(dialog.can_commit());
    });
    dialog.update(cx, |dialog, cx| dialog.end(TransactionEnd::RolledBack, cx));
    wait_until(cx, "the tab to close", |cx| workspace.read_with(cx, |workspace, _| workspace.tabs().tabs().is_empty()));
    assert_eq!(notice(cx).as_deref(), Some("Rolled back 1 change (1 row)"));
    assert_eq!(people(cx, &connection), "0", "the change was discarded");
}

#[gpui::test]
fn manual_refuses_a_script_that_ends_the_transaction_itself_and_runs_nothing(cx: &mut TestAppContext) {
    let dir = setup(cx);
    let (_, scratch) = cx.add_window_view(|_, _| gpui::Empty);
    // Tagged production, which asks first: its consoles start in Manual.
    let connection = sqlite(dir.path(), "shop", "production", scratch);
    let (workspace, cx) = scratch_workspace(cx);
    let console = workspace
        .update_in(cx, |workspace, window, cx| {
            workspace.new_query(&connection.id, QueryScope::default(), None, window, cx)
        })
        .expect("a console");
    assert_eq!(console.read_with(cx, |console, _| console.transaction_mode()), Some(TransactionMode::Manual));
    draw(cx);

    console.update(cx, |console, cx| console.run(format!("{INSERT};\n-- done\ncommit"), cx));
    console.read_with(cx, |console, cx| {
        assert!(!console.is_running(), "nothing ran");
        assert!(console.transaction().is_none());
        match console.results().read(cx).last() {
            Some(LastRun::Failed(error)) => assert!(error.starts_with("COMMIT can't run in Manual mode"), "{error}"),
            other => panic!("expected the refusal, got {other:?}"),
        }
    });
    assert_eq!(people(cx, &connection), "0");

    // Auto-commit runs it as written.
    console.update(cx, |console, cx| console.set_mode(TransactionMode::Auto, cx));
    run_in(&console, &format!("BEGIN; {INSERT}; COMMIT"), cx);
    console.read_with(cx, |console, _| assert!(console.transaction().is_none()));
    assert_eq!(people(cx, &connection), "1");
}

fn scratch_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.add_window_view(|_, _| Workspace::default())
}

#[gpui::test]
fn quitting_with_transactions_open_commits_them_all_when_asked(cx: &mut TestAppContext) {
    let dir = setup(cx);
    let (_, scratch) = cx.add_window_view(|_, _| gpui::Empty);
    let shop = sqlite(dir.path(), "shop", "development", scratch);
    let crm = sqlite(dir.path(), "crm", "development", scratch);
    let (workspace, cx) = scratch_workspace(cx);
    for connection in [&shop, &crm] {
        let console = workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.new_query(&connection.id, QueryScope::default(), None, window, cx)
            })
            .expect("a console");
        console.update(cx, |console, cx| console.set_mode(TransactionMode::Manual, cx));
        run_in(&console, INSERT, cx);
    }
    let open = cx.update(|_, cx| dbforge_core::query::open_transactions(Backend::global(cx).core().query_sessions()));
    assert_eq!(open.len(), 2);
    let pending = open.into_iter().map(|transaction| Pending { title: String::new(), transaction }).collect();
    let dialog = cx.update(|_, cx| cx.new(|cx| LeaveDialog::new(Leaving::Quit, pending, cx)));
    let ended: Rc<RefCell<Vec<LeaveEvent>>> = Rc::default();
    let heard = ended.clone();
    let _listening =
        cx.update(|_, cx| cx.subscribe(&dialog, move |_, event: &LeaveEvent, _| heard.borrow_mut().push(*event)));
    let (_host, host_cx) = cx.add_window_view({
        let view: AnyView = dialog.clone().into();
        move |_, _| Host(view)
    });
    draw(host_cx);

    dialog.update(cx, |dialog, cx| dialog.end(TransactionEnd::Committed, cx));
    wait_until(cx, "both to commit", |_| !ended.borrow().is_empty());
    assert_eq!(*ended.borrow(), [LeaveEvent::Ended(TransactionEnd::Committed)]);
    dialog.read_with(cx, |dialog, _| assert!(dialog.pending().is_empty() && dialog.error().is_none()));
    assert_eq!(people(cx, &shop), "1");
    assert_eq!(people(cx, &crm), "1");
}
