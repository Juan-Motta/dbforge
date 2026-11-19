//! The editor in a headless window: typed through the input handler and
//! driven with its keys, as the user drives it.

// One selection or one match is a one-range list here, not a range to collect.
#![allow(clippy::single_range_in_vec_init)]

use dbforge_core::drivers::{CompletionColumn, CompletionSchema, CompletionTable, ObjectKind};
use gpui::{Modifiers, TestAppContext, VisualTestContext};

use super::*;
use crate::i18n::{Language, Locale};
use crate::sql::Dialect;
use crate::theme::{Appearance, Theme};

/// A window holding an editor with `text`, its keys bound, drawn once with the editor focused.
fn editor<'a>(
    text: &str,
    syntax: Syntax,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(Theme::new(Appearance::Dark));
        cx.set_global(Locale(Language::En));
        cx.bind_keys(key_bindings());
        cx.bind_keys(crate::ui::text_input::key_bindings());
    });
    let (editor, cx) = cx.add_window_view(|_, cx| CodeEditor::new(text, syntax, cx));
    editor.update_in(cx, |editor, window, _| editor.focus(window));
    cx.update(|window, cx| window.draw(cx).clear());
    (editor, cx)
}

/// The selections, as byte ranges.
fn selections(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Range<usize>> {
    editor.read_with(cx, |editor, _| editor.document.selections().all().iter().map(Selection::range).collect())
}

#[gpui::test]
fn types_through_the_input_handler_completes_and_draws(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_global(Theme::new(Appearance::Dark)));
    let (editor, cx) = cx.add_window_view(|_, cx| CodeEditor::new("", Dialect::Postgres, cx));
    editor.update_in(cx, |editor, window, cx| {
        let schema = CompletionSchema {
            database: "shop".into(),
            tables: vec![CompletionTable {
                schema: "public".into(),
                name: "orders".into(),
                kind: ObjectKind::Table,
                columns: vec![CompletionColumn { name: "id".into(), data_type: "integer".into() }],
            }],
        };
        editor.set_completion_names(Some(Rc::new(CompletionNames::new(&schema, Dialect::Postgres))), cx);
        editor.focus(window);
        for typed in ["SELECT * FROM ", "o", "r"] {
            editor.replace_text_in_range(None, typed, window, cx);
        }
        let offered: Vec<&str> = editor
            .completion
            .as_ref()
            .map(|open| open.completion.items.iter().map(|item| item.label.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(offered[0], "orders");
        editor.newline(&Newline, window, cx);
        assert_eq!(editor.text(), "SELECT * FROM orders", "Enter picks the suggestion");
        editor.newline(&Newline, window, cx);
        editor.replace_text_in_range(None, "-- ñ", window, cx);
        assert_eq!(editor.text(), "SELECT * FROM orders\n-- ñ");
        // A dead key composes, then commits.
        editor.replace_and_mark_text_in_range(None, "´", None, window, cx);
        editor.replace_text_in_range(None, "é", window, cx);
        assert!(editor.text().ends_with("ñé"));
    });
    cx.update(|window, cx| window.draw(cx).clear());
    editor.update_in(cx, |editor, window, cx| {
        assert!(editor.layout.caret.is_some(), "the caret was placed on screen");
        editor.undo(&Undo, window, cx);
        assert_eq!(editor.text(), "SELECT * FROM orders\n");
        editor.set_text(&"SELECT 1;\n".repeat(5000), cx);
        assert_eq!(editor.document.line_count(), 5001);
    });
    // A long script draws only the lines in view.
    cx.update(|window, cx| window.draw(cx).clear());
    editor.read_with(cx, |editor, _| assert!(editor.scroll.y > px(0.), "the caret at the end is in view"));
}

#[gpui::test]
fn finds_and_replaces_from_the_bar(cx: &mut TestAppContext) {
    let (editor, cx) = editor("SELECT id FROM t;\nselect Id, id_2 FROM u", Syntax::Sql(Dialect::Postgres), cx);
    cx.simulate_keystrokes("secondary-f");
    cx.simulate_input("id");
    editor.read_with(cx, |editor, _| {
        assert!(editor.find_open(), "⌘F opens the bar");
        assert_eq!(editor.query.text, "id", "typing goes to the query");
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(selections(&editor, cx), [7..9], "Enter: the first match after the caret, wrapping round");
    cx.simulate_keystrokes("enter");
    assert_eq!(selections(&editor, cx), [25..27], "case doesn't matter by default");
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(selections(&editor, cx), [7..9], "⇧Enter: the previous one");
    cx.simulate_keystrokes("secondary-g secondary-g");
    assert_eq!(selections(&editor, cx), [29..31], "⌘G finds the next one too");
    editor.update(cx, |editor, cx| {
        editor.toggle_option(|query| &mut query.case_sensitive, cx);
        editor.toggle_option(|query| &mut query.whole_word, cx);
        assert_eq!(editor.matches(), [7..9], "match case and whole word narrow the matches");
        editor.toggle_option(|query| &mut query.whole_word, cx);
        editor.toggle_option(|query| &mut query.regex, cx);
    });
    cx.simulate_keystrokes("backspace backspace");
    cx.simulate_input("i(d)");
    editor.update(cx, |editor, cx| {
        assert_eq!(editor.matches(), [7..9, 29..31], "a regular expression");
        let replacement = editor.find.as_ref().unwrap().replacement.clone();
        replacement.update(cx, |input, cx| input.set_text("[$1]", cx));
        editor.replace_all(cx);
        assert_eq!(editor.text(), "SELECT [d] FROM t;\nselect Id, [d]_2 FROM u", "groups in the replacement");
    });
    cx.simulate_keystrokes("escape");
    editor.update_in(cx, |editor, window, cx| {
        assert!(!editor.find_open(), "Escape closes the bar");
        assert!(editor.is_focused(window), "and gives the keyboard back to the text");
        editor.undo(&Undo, window, cx);
        assert_eq!(editor.text(), "SELECT id FROM t;\nselect Id, id_2 FROM u", "replace all is one step to undo");
    });
}

#[gpui::test]
fn replaces_one_match_at_a_time(cx: &mut TestAppContext) {
    let (editor, cx) = editor("a1 a2 a3", Syntax::Plain, cx);
    cx.simulate_keystrokes("secondary-f");
    cx.simulate_input("a");
    editor.update(cx, |editor, cx| {
        let replacement = editor.find.as_ref().unwrap().replacement.clone();
        replacement.update(cx, |input, cx| input.set_text("b", cx));
        editor.document.place(0, false);
        editor.replace_next(cx);
        assert_eq!(
            (editor.text(), editor.document.primary().range()),
            ("a1 a2 a3", 0..1),
            "first it selects the match"
        );
        editor.replace_next(cx);
        assert_eq!((editor.text(), editor.document.primary().range()), ("b1 a2 a3", 3..4), "then replaces it");
    });
}

#[gpui::test]
fn the_find_bar_opens_with_the_selection_and_moves_no_text(cx: &mut TestAppContext) {
    let (editor, cx) = editor("SELECT total FROM t", Syntax::Sql(Dialect::Postgres), cx);
    editor.update(cx, |editor, _| editor.document.select(7, 12));
    let before = editor.read_with(cx, |editor, _| (editor.layout.text_left, editor.layout.bounds));
    cx.simulate_keystrokes("secondary-f");
    cx.update(|window, cx| window.draw(cx).clear());
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.query.text, "total", "the selected text is the query");
        assert_eq!((editor.layout.text_left, editor.layout.bounds), before, "the text stays where it was");
    });
}

#[gpui::test]
fn toggles_comments_per_syntax(cx: &mut TestAppContext) {
    let (sql, cx) = editor("SELECT 1", Syntax::Sql(Dialect::Mysql), cx);
    cx.simulate_keystrokes("secondary-/");
    sql.read_with(cx, |editor, _| assert_eq!(editor.text(), "-- SELECT 1"));
    cx.simulate_keystrokes("secondary-/");
    sql.read_with(cx, |editor, _| assert_eq!(editor.text(), "SELECT 1"));
    for (syntax, expected) in [(Syntax::Mongosh, "// db.t.find()"), (Syntax::Json, "db.t.find()")] {
        sql.update_in(cx, |editor, window, cx| {
            editor.set_syntax(syntax, cx);
            editor.set_text("db.t.find()", cx);
            editor.toggle_comment(&ToggleComment, window, cx);
            assert_eq!(editor.text(), expected, "{syntax:?}");
        });
    }
}

#[gpui::test]
fn closes_brackets_as_typed(cx: &mut TestAppContext) {
    let (editor, cx) = editor("", Syntax::Sql(Dialect::Postgres), cx);
    cx.simulate_input("count(");
    editor.read_with(cx, |editor, _| assert_eq!((editor.text(), editor.caret()), ("count()", 6)));
    cx.simulate_input("'");
    cx.simulate_input("x");
    cx.simulate_input("'");
    cx.simulate_input(")");
    editor.read_with(cx, |editor, _| assert_eq!((editor.text(), editor.caret()), ("count('x')", 10)));
    cx.simulate_keystrokes("left left left backspace");
    editor
        .read_with(cx, |editor, _| assert_eq!(editor.text(), "count(x')", "a quote with no pair after it goes alone"));
    cx.simulate_keystrokes("left");
    cx.simulate_input("[");
    editor.read_with(cx, |editor, _| assert_eq!(editor.text(), "count[(x')", "no pair before a bracket"));
    cx.simulate_keystrokes("backspace end");
    cx.simulate_input("[");
    editor.read_with(cx, |editor, _| assert_eq!(editor.text(), "count(x')[]"));
    cx.simulate_keystrokes("backspace");
    editor.read_with(cx, |editor, _| assert_eq!(editor.text(), "count(x')", "Backspace deletes an empty pair"));
    editor.update_in(cx, |editor, window, cx| {
        editor.set_text("a 'it(' b", cx);
        editor.place_caret(6, cx);
        editor.replace_text_in_range(None, "'", window, cx);
        assert_eq!(editor.text(), "a 'it('' b", "inside a string a quote doesn't pair");
    });
}

#[gpui::test]
fn folds_from_the_keyboard_and_the_gutter(cx: &mut TestAppContext) {
    let text = "{\n  \"a\": [\n    1\n  ],\n  \"b\": 2\n}";
    let (editor, cx) = editor(text, Syntax::Json, cx);
    editor.update(cx, |editor, cx| editor.place_caret(0, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-alt-[" } else { "ctrl-shift-[" });
    cx.update(|window, cx| window.draw(cx).clear());
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document.folds(), [1..text.len() - 1], "the object folds inside its braces");
        assert_eq!(editor.display.row_count(), 1, "on one row");
        assert_eq!(editor.display.row_text(editor.text(), 0), "{…}");
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-alt-]" } else { "ctrl-shift-]" });
    editor.read_with(cx, |editor, _| assert!(editor.document.folds().is_empty(), "and unfolds"));
    // The gutter's marker on the second line folds its array.
    cx.update(|window, cx| window.draw(cx).clear());
    let (folds_left, top) = editor.read_with(cx, |editor, _| (editor.layout.folds_left, editor.layout.bounds.top()));
    let marker = point(folds_left + px(7.), top + px(PADDING_TOP + LINE_HEIGHT * 1.5));
    cx.simulate_click(marker, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        let array = text.find('[').unwrap() + 1..text.find(']').unwrap();
        assert_eq!(editor.document.folds(), [array]);
    });
    cx.simulate_click(marker, Modifiers::none());
    editor.read_with(cx, |editor, _| assert!(editor.document.folds().is_empty(), "a second click unfolds"));
}

#[gpui::test]
fn edits_at_every_caret(cx: &mut TestAppContext) {
    let (editor, cx) = editor("id = 1;\nid = 2;\nname = 3", Syntax::Sql(Dialect::Postgres), cx);
    editor.update(cx, |editor, cx| editor.place_caret(0, cx));
    cx.simulate_keystrokes("secondary-d secondary-d");
    assert_eq!(selections(&editor, cx), [0..2, 8..10], "⌘D twice: the word and its next match");
    cx.simulate_input("key");
    editor.read_with(cx, |editor, _| assert_eq!(editor.text(), "key = 1;\nkey = 2;\nname = 3"));
    cx.simulate_keystrokes("backspace");
    editor.read_with(cx, |editor, _| assert_eq!(editor.text(), "ke = 1;\nke = 2;\nname = 3"));
    cx.simulate_keystrokes("escape");
    assert_eq!(selections(&editor, cx), [2..2], "Escape leaves the primary caret, the first one");
    cx.simulate_keystrokes("secondary-alt-down");
    assert_eq!(selections(&editor, cx), [2..2, 10..10], "a caret added below");
    cx.simulate_keystrokes("secondary-alt-down secondary-alt-up");
    assert_eq!(selections(&editor, cx), [2..2, 10..10, 18..18], "none added where one is");
    cx.simulate_keystrokes("end");
    cx.simulate_input("!");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.text(), "ke = 1;!\nke = 2;!\nname = 3!");
        assert_eq!(editor.selected_text(), None);
    });
    // ⌘-click (Ctrl+click elsewhere) adds a caret; the primary one is the last added.
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| window.draw(cx).clear());
    let (left, top) = editor.read_with(cx, |editor, _| (editor.layout.text_left, editor.layout.bounds.top()));
    let adds = if cfg!(target_os = "macos") { Modifiers::command() } else { Modifiers::control() };
    cx.simulate_click(point(left + px(1.), top + px(PADDING_TOP + LINE_HEIGHT / 2.)), adds);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document.selections().count(), 2);
        assert_eq!(editor.caret(), 0, "Run reads the primary selection: the one clicked last");
    });
}

#[gpui::test]
fn soft_wrap_moves_the_caret_by_rows_on_screen(cx: &mut TestAppContext) {
    let long = "word ".repeat(400);
    let text = format!("{long}\nnext");
    let (editor, cx) = editor(&text, Syntax::Plain, cx);
    editor.update(cx, |editor, cx| editor.place_caret(0, cx));
    cx.simulate_keystrokes("down");
    editor
        .read_with(cx, |editor, _| assert_eq!(editor.caret(), long.len() + 1, "unwrapped, Down goes to the next line"));
    editor.update(cx, |editor, cx| {
        editor.set_soft_wrap(true, cx);
        editor.place_caret(0, cx);
    });
    cx.update(|window, cx| window.draw(cx).clear());
    cx.simulate_keystrokes("down");
    let first_row = editor.read_with(cx, |editor, _| {
        assert!(editor.display.row_count() > 2, "the long line wraps over rows");
        let caret = editor.caret();
        assert!(caret > 0 && caret < long.len(), "Down stays on the long line, a row lower: {caret}");
        assert_eq!(caret, editor.display.row(1).start, "at the second row's start");
        editor.display.row(0)
    });
    assert!(first_row.end - first_row.start > 0 && text[first_row.start..first_row.end].ends_with(' '));
    cx.simulate_keystrokes("up");
    editor.read_with(cx, |editor, _| assert_eq!(editor.caret(), 0, "Up comes back"));
    editor.read_with(cx, |editor, _| assert_eq!(editor.scroll.x, px(0.), "nothing scrolls sideways"));
}
