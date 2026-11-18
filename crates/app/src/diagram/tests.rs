//! A diagram drawn headless: a schema of 50 tables fitted into the canvas,
//! panned, zoomed, a card dragged, pinned and brought close.

use dbforge_core::drivers::{DiagramColumn, DiagramTable, Relationship, SchemaDiagram};
use gpui::{AnyView, AppContext, Entity, TestAppContext, VisualTestContext};

use super::layout::{Point, View};
use super::view::DiagramView;
use crate::backend::test_backend;
use crate::i18n::{Language, Locale};
use crate::theme::{Appearance, Theme};

/// `count` tables in a chain of foreign keys, each with a key, a reference and a few columns.
fn schema(count: usize) -> SchemaDiagram {
    let name = |n: usize| format!("t{n:02}");
    let column = |name: &str, data_type: &str, primary_key: bool| DiagramColumn {
        name: name.into(),
        data_type: data_type.into(),
        nullable: !primary_key,
        primary_key,
    };
    let tables = (0..count)
        .map(|n| DiagramTable {
            name: name(n),
            row_estimate: Some((n as i64 + 1) * 1_234),
            columns: vec![
                column("id", "bigint", true),
                column("parent_id", "bigint", false),
                column("title", "character varying(120)", false),
                column("created_at", "timestamp with time zone", false),
            ],
        })
        .collect();
    let relationships = (1..count)
        .map(|n| Relationship {
            name: format!("{}_parent_fkey", name(n)),
            table: name(n),
            columns: vec!["parent_id".into()],
            referenced_schema: "public".into(),
            referenced_table: name(n / 2),
            referenced_columns: vec!["id".into()],
        })
        .collect();
    SchemaDiagram { tables, relationships }
}

struct Host(AnyView);

impl gpui::Render for Host {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        gpui::div().size_full().flex().child(gpui::div().flex_1().flex().child(self.0.clone()))
    }
}

/// A window with a diagram of `diagram`, a table's own when `focus` names one.
fn open<'a>(
    diagram: SchemaDiagram,
    focus: Option<&str>,
    cx: &'a mut TestAppContext,
) -> (Entity<DiagramView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(Theme::new(Appearance::Dark));
        cx.set_global(Locale(Language::En));
    });
    let focus = focus.map(str::to_owned);
    let (host, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| DiagramView::new("gone".into(), "main".into(), "public".into(), focus, window, cx));
        Host(view.into())
    });
    cx.run_until_parked();
    let view = host.read_with(cx, |host, _| host.0.clone().downcast::<DiagramView>().expect("a diagram"));
    view.update(cx, |view, _| view.show(diagram));
    cx.update(|window, cx| window.draw(cx).clear());
    (view, cx)
}

#[gpui::test]
fn draws_a_schema_of_fifty_tables_fitted_into_the_canvas(cx: &mut TestAppContext) {
    let _dir = cx.update(test_backend);
    let (view, cx) = open(schema(50), None, cx);
    let (scene, fitted) = view.read_with(cx, |view, _| (view.scene().expect("drawn"), view.view().expect("a view")));
    // Past 40 tables only the related ones start shown: here all are related.
    assert_eq!(scene.cards.len(), 50);
    assert_eq!(scene.lines.len(), 49);
    assert!(fitted.scale < 1., "fifty tables are fitted by zooming out, at {}", fitted.scale);
    assert!(view.read_with(cx, |view, _| !view.pending_fit));

    // Dragging the empty canvas pans it; the wheel zooms around the pointer.
    view.update(cx, |view, cx| {
        let empty = Point::new(5., 5.);
        view.press(empty, cx);
        view.pointer_moved(Point::new(105., 55.), true, cx);
        view.release(1, cx);
    });
    let panned = view.read_with(cx, |view, _| view.view().unwrap());
    assert_eq!((panned.x - fitted.x, panned.y - fitted.y), (100., 50.));
    view.update(cx, |view, cx| view.wheel(-300., Point::new(200., 200.), cx));
    let zoomed = view.read_with(cx, |view, _| view.view().unwrap());
    assert!(zoomed.scale < panned.scale, "scrolling down zooms out");
    let (after, before) = (zoomed.to_diagram(Point::new(200., 200.)), panned.to_diagram(Point::new(200., 200.)));
    assert!((after.x - before.x).abs() < 1e-3 && (after.y - before.y).abs() < 1e-3, "{after:?} ≠ {before:?}");
    cx.update(|window, cx| window.draw(cx).clear());
}

#[gpui::test]
fn moves_pins_and_brings_a_card_close(cx: &mut TestAppContext) {
    let _dir = cx.update(test_backend);
    let (view, cx) = open(schema(6), None, cx);
    view.update(cx, |view, _| view_at_100(view));
    let card = view.read_with(cx, |view, _| view.scene().unwrap().cards[0].clone());
    let inside = Point::new(card.position.x + 40. + 40., card.position.y + 40. + 10.);
    // A drag on a card moves it, by as much as the pointer at 100%.
    view.update(cx, |view, cx| {
        view.press(inside, cx);
        view.pointer_moved(Point::new(inside.x + 30., inside.y + 20.), true, cx);
        view.release(1, cx);
    });
    let moved = view.read_with(cx, |view, _| view.scene().unwrap().cards[0].position);
    assert_eq!(moved, Point::new(card.position.x + 30., card.position.y + 20.));
    let moved_inside = Point::new(inside.x + 30., inside.y + 20.);
    // A click pins it: its relations stand out and the rest dim.
    view.update(cx, |view, cx| {
        view.press(moved_inside, cx);
        view.release(1, cx);
    });
    let scene = view.read_with(cx, |view, _| view.scene().unwrap());
    assert_eq!(scene.pinned.as_deref(), Some(card.name.as_str()));
    assert!(scene.related.contains("t01"), "{:?}", scene.related);
    assert!(!scene.related.contains("t05"));
    cx.update(|window, cx| window.draw(cx).clear());
    // A double click centres it at 100%.
    view.update(cx, |view, cx| {
        view.press(moved_inside, cx);
        view.release(2, cx);
    });
    let centred = view.read_with(cx, |view, _| view.view().unwrap());
    assert_eq!(centred.scale, 1.);
    // Auto layout puts it back.
    view.update(cx, |view, cx| view.auto_layout(cx));
    assert_eq!(view.read_with(cx, |view, _| view.scene().unwrap().cards[0].position), card.position);
    // A click on the empty canvas unpins it.
    view.update(cx, |view, cx| {
        view_at_100(view);
        view.press(Point::new(-500., -500.), cx);
        view.release(1, cx);
    });
    assert_eq!(view.read_with(cx, |view, _| view.scene().unwrap().pinned.clone()), None);
}

/// The diagram at 100%, its origin 40 pixels in.
fn view_at_100(view: &mut DiagramView) {
    view.set_view_for_tests(View { x: 40., y: 40., scale: 1. });
}

#[gpui::test]
fn draws_a_table_s_own_diagram_with_its_reach(cx: &mut TestAppContext) {
    let _dir = cx.update(test_backend);
    let (view, cx) = open(schema(20), Some("t04"), cx);
    let names = |view: &Entity<DiagramView>, cx: &mut VisualTestContext| {
        let mut names: Vec<String> =
            view.read_with(cx, |view, _| view.scene().unwrap().cards.iter().map(|c| c.name.clone()).collect());
        names.sort();
        names
    };
    // t04 references t02 and is referenced by t08 and t09.
    assert_eq!(names(&view, cx), ["t02", "t04", "t08", "t09"]);
    assert_eq!(view.read_with(cx, |view, _| view.scene().unwrap().pinned.clone()).as_deref(), Some("t04"));
    view.update(cx, |view, cx| view.set_reach(None, cx));
    cx.update(|window, cx| window.draw(cx).clear());
    assert_eq!(names(&view, cx).len(), 20);
    view.update(cx, |view, cx| view.set_mode(super::layout::ColumnMode::Keys, cx));
    let columns = view.read_with(cx, |view, _| {
        view.scene().unwrap().cards.iter().find(|card| card.name == "t04").map(|card| card.columns.len())
    });
    assert_eq!(columns, Some(2), "only the key and the reference");
    cx.update(|window, cx| window.draw(cx).clear());
}
