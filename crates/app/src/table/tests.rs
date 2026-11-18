//! A table's tab drawn headless: rows with pending edits, their editors,
//! menu and inspector, and the question a PROD connection asks first.

use dbforge_core::connections::model::{ConnectionInput, Engine};
use dbforge_core::drivers::{CellValue, Column, RowFilter, TableRef};
use gpui::{AnyView, AppContext, Entity, TestAppContext, VisualTestContext, point, px};

use super::documents::{DocumentId, DocumentState};
use super::edits::Counts;
use super::page::{TablePage, TablePageEvent};
use super::view::TableView;
use crate::backend::{Backend, test_backend};
use crate::grid::marks::RowState;
use crate::grid::selection::Cell;
use crate::i18n::{Language, Locale};
use crate::theme::{Appearance, Theme};

fn column(name: &str, data_type: &str, primary_key: bool) -> Column {
    Column { name: name.into(), data_type: data_type.into(), primary_key, generated: false, default: None }
}

fn orders() -> TableRef {
    TableRef { database: "main".into(), schema: "main".into(), name: "orders".into() }
}

/// A window with `orders` of `connection` open, showing 500 rows instead of reading them.
fn open<'a>(
    connection: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<TablePage>, Entity<TableView>, &'a mut VisualTestContext) {
    let connection = connection.to_owned();
    let (host, cx) = cx.add_window_view(|window, cx| {
        let page = cx.new(|cx| TablePage::new(connection, Engine::Sqlite, orders(), RowFilter::default(), window, cx));
        Host(page.into())
    });
    cx.run_until_parked();
    let page = host.read_with(cx, |host, _| host.0.clone().downcast::<TablePage>().expect("a table page"));
    let data = page.read_with(cx, |page, _| page.data().clone());
    data.update(cx, |data, cx| {
        let columns = vec![
            column("id", "INTEGER", true),
            column("customer", "TEXT", false),
            column("note", "TEXT", false),
            column("placed_at", "datetime", false),
        ];
        let rows = (1..=500)
            .map(|n| {
                vec![
                    Some(n.to_string()),
                    Some(format!("customer {n}")),
                    (n % 3 != 0).then(|| format!("note {n}")),
                    Some(format!("2026-11-{:02} 10:00:00", n % 28 + 1)),
                ]
            })
            .collect();
        data.show_rows(columns, rows, cx);
    });
    cx.update(|window, cx| window.draw(cx).clear());
    (page, data, cx)
}

fn setup(cx: &mut TestAppContext) -> tempfile::TempDir {
    cx.update(|cx| {
        cx.set_global(Theme::new(Appearance::Light));
        cx.set_global(Locale(Language::En));
        let dir = test_backend(cx);
        crate::preferences::test_preferences(cx, dir.path());
        dir
    })
}

#[gpui::test]
fn draws_pending_edits_without_moving_the_table(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (_page, data, cx) = open("gone", cx);
    let before = data.read_with(cx, |data, cx| data.grid().read(cx).cell_bounds(Cell::new(0, 1)).expect("drawn"));
    data.update(cx, |data, cx| {
        assert!(data.editable());
        data.set_cells(&[Cell::new(0, 1)], CellValue::Text("Ada".into()), cx);
        data.toggle_delete(2, cx);
        data.add_row(cx);
    });
    cx.update(|window, cx| window.draw(cx).clear());
    data.read_with(cx, |data, cx| {
        assert_eq!(data.edits().counts(), Counts { updated: 1, inserted: 1, deleted: 1 });
        let grid = data.grid().read(cx);
        let marks = grid.marks().expect("marks");
        assert_eq!(marks.rows[..4], [RowState::Inserted, RowState::Loaded, RowState::Loaded, RowState::Deleted]);
        assert!(marks.cell(Cell::new(1, 1)).edited);
        assert!(marks.cell(Cell::new(0, 2)).unset, "a new row's columns take their DEFAULT");
        let after = grid.cell_bounds(Cell::new(1, 1)).expect("drawn");
        // The edited cell moved down one row for the new row, and not an inch sideways.
        assert_eq!((after.left(), after.size.width), (before.left(), before.size.width));
        assert_eq!(after.top() - before.top(), px(crate::grid::ROW_HEIGHT));
    });

    // The text editor over a cell, its menu, the date controls and the inspector all draw.
    data.update_in(cx, |data, window, cx| data.open_editor(Cell::new(1, 2), window, cx));
    cx.update(|window, cx| window.draw(cx).clear());
    data.update_in(cx, |data, window, cx| {
        let bounds = data.grid().read(cx).cell_bounds(Cell::new(1, 2)).expect("drawn");
        data.open_menu(Cell::new(1, 2), bounds.origin + point(px(10.), px(10.)), window, cx);
    });
    cx.update(|window, cx| window.draw(cx).clear());
    data.update_in(cx, |data, window, cx| {
        data.cancel_editor(window, cx);
        data.open_editor(Cell::new(1, 3), window, cx);
        data.toggle_inspector(window, cx);
    });
    cx.update(|window, cx| window.draw(cx).clear());
    data.read_with(cx, |data, _| assert!(data.inspector().is_some()));
}

#[gpui::test]
fn the_text_editor_keeps_a_change_and_drops_an_untouched_value(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (_page, data, cx) = open("gone", cx);
    data.update_in(cx, |data, window, cx| {
        // Opening a NULL cell and leaving it stores nothing.
        data.open_editor(Cell::new(2, 2), window, cx);
        data.finish_editor(window, cx);
        assert!(!data.edits().has_edits());
        data.open_editor(Cell::new(0, 1), window, cx);
        data.type_in_editor("Grace", cx);
        data.finish_editor(window, cx);
        assert_eq!(data.edits().counts().updated, 1);
    });
}

/// A saved SQLite connection tagged `environment`, for the core to find.
fn save_connection(environment: &str, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let input: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "Shop",
            "engine": "sqlite",
            "environment": environment,
            "host": "",
            "port": 0,
            "database": "/nowhere/shop.db",
            "username": "",
        }))
        .expect("an input");
        let parts = input.into_parts().expect("a valid connection");
        Backend::global(cx).core().connections().save(parts).expect("saved").id
    })
}

#[gpui::test]
fn asks_before_applying_on_a_connection_tagged_prod(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (page, data, cx) = open("gone", cx);
    let id = save_connection("production", cx);
    let dialogs: std::rc::Rc<std::cell::RefCell<Vec<Option<AnyView>>>> = Default::default();
    let seen = dialogs.clone();
    let _listening = cx.update(|_, cx| {
        cx.subscribe(&page, move |_, event: &TablePageEvent, _| {
            if let TablePageEvent::Dialog(dialog) = event {
                seen.borrow_mut().push(dialog.clone());
            }
        })
    });
    data.update_in(cx, |data, window, cx| {
        data.set_connection_for_test(id);
        data.set_cells(&[Cell::new(0, 1)], CellValue::Text("Ada".into()), cx);
        data.request_apply(window, cx);
    });
    cx.run_until_parked();
    {
        let shown = dialogs.borrow();
        let dialog = shown.last().cloned().flatten().expect("the PROD question is shown");
        assert!(dialog.downcast::<super::dialogs::ConfirmDialog>().is_ok());
    }
    // Drawn as the window's dialog layer would draw it.
    let dialog = dialogs.borrow().last().cloned().flatten().expect("a dialog");
    let (_host, cx2) = cx.add_window_view(|_, _| Host(dialog));
    cx2.update(|window, cx| window.draw(cx).clear());
    data.read_with(cx, |data, _| {
        assert!(data.edits().has_edits(), "nothing is applied until the user says so");
        assert!(!data.is_applying());
    });
}

#[gpui::test]
fn applies_without_asking_on_a_connection_whose_tag_does_not_ask(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (_page, data, cx) = open("gone", cx);
    let id = save_connection("development", cx);
    data.update_in(cx, |data, window, cx| {
        data.set_connection_for_test(id);
        data.set_cells(&[Cell::new(0, 1)], CellValue::Text("Ada".into()), cx);
        data.request_apply(window, cx);
        assert!(data.is_applying(), "applies at once");
    });
}

/// Draws a view filling a window, as the workspace or the dialog layer would.
/// A structure with every section filled: keys, a partial index, a foreign key, a trigger.
fn orders_structure() -> dbforge_core::drivers::TableStructure {
    use crate::structure::format::tests::{foreign_key, index, structure};
    use dbforge_core::drivers::{ColumnInfo, ObjectKind, TriggerInfo};
    let column =
        |name: &str, data_type: &str, nullable: bool, default: Option<&str>, comment: Option<&str>| ColumnInfo {
            name: name.into(),
            data_type: data_type.into(),
            nullable,
            default: default.map(str::to_owned),
            comment: comment.map(str::to_owned),
            options: Vec::new(),
        };
    let mut orders = structure(ObjectKind::Table, Some(48_213));
    orders.columns = vec![
        column("id", "INTEGER", false, None, Some("Surrogate key")),
        column("customer", "TEXT", false, None, None),
        column("note", "TEXT", true, Some("''"), Some("Free text,\nas the customer wrote it")),
        column("placed_at", "datetime", false, Some("CURRENT_TIMESTAMP"), None),
    ];
    orders.indexes = vec![
        index("orders_pkey", &["id"], true, false, None),
        index("orders_open", &["customer"], false, true, Some("note IS NULL")),
    ];
    orders.foreign_keys = vec![foreign_key("orders_customer_fkey", "customer", "customers")];
    orders.triggers = vec![TriggerInfo {
        name: "orders_touch".into(),
        enabled: true,
        definition: "CREATE TRIGGER orders_touch AFTER UPDATE ON orders\nBEGIN\n  UPDATE orders SET placed_at = CURRENT_TIMESTAMP WHERE id = NEW.id;\nEND".into(),
    }];
    orders.ddl =
        "CREATE TABLE orders (\n  id INTEGER PRIMARY KEY,\n  customer TEXT NOT NULL REFERENCES customers (id)\n);"
            .into();
    orders
}

#[gpui::test]
fn draws_each_section_of_the_structure_and_keeps_the_rows_loaded(cx: &mut TestAppContext) {
    use super::page::TableTabView;
    use crate::structure::sections::Section;
    let _dir = setup(cx);
    let (page, data, cx) = open("gone", cx);
    let source = page.read_with(cx, |page, _| page.source().clone());
    source.update(cx, |source, cx| source.show(orders_structure(), cx));
    cx.update(|window, cx| window.draw(cx).clear());
    for section in Section::ALL {
        page.update_in(cx, |page, window, cx| page.set_view(TableTabView::Section(section), window, cx));
        cx.update(|window, cx| window.draw(cx).clear());
    }
    page.read_with(cx, |page, cx| {
        let structure = page.structure_view().expect("the structure's view").read(cx);
        assert_eq!(structure.section(), Section::Ddl);
        let ddl = structure.ddl().read(cx);
        assert!(ddl.text().starts_with("CREATE TABLE orders ("));
        assert_eq!(ddl.caret(), 0, "the DDL reads from its top");
    });
    // A reload keeps the structure shown until the next one arrives.
    source.update(cx, |source, cx| source.reload(cx));
    source.read_with(cx, |source, _| {
        assert!(source.is_loading());
        assert!(source.structure().is_some());
    });
    cx.update(|window, cx| window.draw(cx).clear());
    // The rows stayed loaded while the structure showed.
    page.update_in(cx, |page, window, cx| page.set_view(TableTabView::Data, window, cx));
    cx.update(|window, cx| window.draw(cx).clear());
    data.read_with(cx, |data, cx| assert_eq!(data.grid().read(cx).row_count(), 500));
    page.read_with(cx, |page, _| assert!(page.has_diagram(), "SQLite draws a table's diagram"));
}

struct Host(AnyView);

impl gpui::Render for Host {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        gpui::div().size_full().flex().flex_col().child(self.0.clone())
    }
}

/// A window with a MongoDB collection open as documents: three loaded, keyed by `_id`.
fn open_documents(cx: &mut TestAppContext) -> (Entity<TableView>, &mut VisualTestContext) {
    let (host, cx) = cx.add_window_view(|window, cx| {
        let orders = TableRef { database: "shop".into(), schema: "shop".into(), name: "orders".into() };
        let page =
            cx.new(|cx| TablePage::new("gone".into(), Engine::Mongodb, orders, RowFilter::default(), window, cx));
        Host(page.into())
    });
    cx.run_until_parked();
    let page = host.read_with(cx, |host, _| host.0.clone().downcast::<TablePage>().expect("a table page"));
    let data = page.read_with(cx, |page, _| page.data().clone());
    data.update(cx, |data, cx| {
        let columns = vec![column("_id", "objectId", true), column("total", "int", false)];
        let ids = ["65a0", "65a1", "65a2"];
        let rows = ids.iter().enumerate().map(|(n, id)| vec![Some(id.to_string()), Some(n.to_string())]).collect();
        let documents = ids
            .iter()
            .enumerate()
            .map(|(n, id)| format!("{{\n  \"_id\": ObjectId(\"{id}\"),\n  \"total\": {n}\n}}"))
            .collect();
        data.show_documents_page(columns, rows, documents, cx);
    });
    cx.update(|window, cx| window.draw(cx).clear());
    (data, cx)
}

fn loaded(data: &TableView, index: usize) -> DocumentId {
    data.document_cards().iter().filter(|card| matches!(card.id, DocumentId::Loaded(_))).nth(index).unwrap().id.clone()
}

#[gpui::test]
fn shows_a_collection_as_documents_and_edits_them_one_at_a_time(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (data, cx) = open_documents(cx);
    data.update_in(cx, |data, window, cx| {
        assert!(data.has_documents() && !data.showing_documents(), "the table shows first");
        data.toggle_documents(window, cx);
        assert!(data.showing_documents());
        let labels: Vec<String> = data.document_cards().into_iter().map(|card| card.label).collect();
        assert_eq!(labels, ["#1", "#2", "#3"]);
        // Written back as it was, a document is no change.
        let first = loaded(data, 0);
        data.open_document(first, window, cx);
        assert!(data.document_editor().is_some());
    });
    cx.update(|window, cx| window.draw(cx).clear());
    // ⌘↵ in the editor keeps it.
    cx.dispatch_action(super::documents::KeepDocument);
    data.update_in(cx, |data, window, cx| {
        assert!(data.document_editor().is_none() && !data.edits().has_edits());
        // Picked together, deleted together; the views stay as they are while it is pending.
        let (second, third) = (loaded(data, 1), loaded(data, 2));
        data.select_document(second, false, cx);
        data.select_document(third, true, cx);
        assert_eq!(data.selected_documents().len(), 2);
        data.delete_selected(cx);
        assert_eq!(data.edits().counts(), Counts { updated: 0, inserted: 0, deleted: 2 });
        let states: Vec<_> = data.document_cards().into_iter().map(|card| card.state).collect();
        assert_eq!(states, [None, Some(DocumentState::Deleted), Some(DocumentState::Deleted)]);
        data.toggle_documents(window, cx);
        assert!(data.showing_documents(), "apply or discard first");
        // A deleted document doesn't open.
        let second = loaded(data, 1);
        data.open_document(second, window, cx);
        assert!(data.document_editor().is_none());
    });
    cx.update(|window, cx| window.draw(cx).clear());
    data.update_in(cx, |data, window, cx| {
        data.discard(cx);
        // A new document opens on its empty line; cancelled before it was kept, it is gone.
        data.new_document(window, cx);
        let editor = data.document_editor().expect("open").editor.clone();
        assert_eq!(editor.read(cx).caret(), 4);
        assert_eq!(data.document_cards()[0].state, Some(DocumentState::New));
        assert_eq!(data.edits().counts().inserted, 1);
    });
    cx.update(|window, cx| window.draw(cx).clear());
    // Escape, which the editor passes on, drops it.
    cx.dispatch_action(crate::ui::code_editor::Escape);
    data.update_in(cx, |data, _, _| {
        assert!(data.document_editor().is_none());
        assert!(!data.edits().has_edits());
        assert_eq!(data.document_cards().len(), 3);
    });
}

#[gpui::test]
fn keeps_a_document_open_with_what_the_database_said_of_it(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (data, cx) = open_documents(cx);
    data.update_in(cx, |data, window, cx| {
        data.toggle_documents(window, cx);
        let first = loaded(data, 0);
        data.open_document(first, window, cx);
        let editor = data.document_editor().expect("open").editor.clone();
        editor.update(cx, |editor, cx| editor.set_text("{\"_id\": ObjectId(\"65a0\"), \"total\": 9}", cx));
        data.keep_document(window, cx);
        assert!(data.document_editor().is_some_and(|open| open.is_saving()));
    });
    // The check runs on the core's runtime; the connection is gone, so it fails.
    let mut problem = None;
    for _ in 0..400 {
        cx.run_until_parked();
        problem =
            data.read_with(cx, |data, _| data.document_editor().and_then(|open| open.problem()).map(str::to_owned));
        if problem.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(problem.is_some(), "the editor says why");
    cx.update(|window, cx| window.draw(cx).clear());
    data.read_with(cx, |data, _| {
        let open = data.document_editor().expect("still open, the draft kept");
        assert!(!open.is_saving());
        assert!(!data.edits().has_edits(), "nothing pending until the database can read it");
    });
    // A menu on a card that isn't picked picks it.
    data.update_in(cx, |data, window, cx| {
        let second = loaded(data, 1);
        data.open_document_menu(second.clone(), point(px(40.), px(80.)), window, cx);
        assert!(data.selected_documents().contains(&second));
    });
    cx.update(|window, cx| window.draw(cx).clear());
}
