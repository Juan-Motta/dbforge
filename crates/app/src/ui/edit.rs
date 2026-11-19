//! The Edit menu's commands (Undo, Redo, Cut, Copy, Paste, Select All),
//! carried out by whatever has the keyboard: a text field, the SQL editor,
//! a data grid. Each of those answers its own actions; these are handed to
//! the first one that can take them where the keyboard is, and do nothing
//! where none can (the explorer has nothing to paste into).

use gpui::{Action, App, InteractiveElement, Window};

use crate::ui::{code_editor, text_input};

gpui::actions!(edit, [Undo, Redo, Cut, Copy, Paste, SelectAll]);

/// The actions that carry out `action` in each kind of control, in the order they are tried.
fn handlers(action: &dyn Action) -> Vec<Box<dyn Action>> {
    let any = action.as_any();
    if any.is::<Undo>() {
        vec![Box::new(code_editor::Undo)]
    } else if any.is::<Redo>() {
        vec![Box::new(code_editor::Redo)]
    } else if any.is::<Cut>() {
        vec![Box::new(text_input::Cut), Box::new(code_editor::Cut)]
    } else if any.is::<Copy>() {
        vec![Box::new(text_input::Copy), Box::new(code_editor::Copy), Box::new(crate::grid::Copy)]
    } else if any.is::<Paste>() {
        vec![Box::new(text_input::Paste), Box::new(code_editor::Paste), Box::new(crate::table::Paste)]
    } else if any.is::<SelectAll>() {
        vec![Box::new(text_input::SelectAll), Box::new(code_editor::SelectAll), Box::new(crate::grid::SelectAll)]
    } else {
        Vec::new()
    }
}

/// Hands `action` to the control with the keyboard that can carry it out, if any.
pub fn forward(action: &dyn Action, window: &mut Window, cx: &mut App) {
    if let Some(handler) = handlers(action).into_iter().find(|handler| window.is_action_available(handler.as_ref(), cx))
    {
        window.dispatch_action(handler, cx);
    }
}

/// Answers the Edit commands on `element`, the window's root, for whatever has the keyboard inside it.
pub fn handle<E: InteractiveElement>(element: E) -> E {
    element
        .on_action(|action: &Undo, window, cx| forward(action, window, cx))
        .on_action(|action: &Redo, window, cx| forward(action, window, cx))
        .on_action(|action: &Cut, window, cx| forward(action, window, cx))
        .on_action(|action: &Copy, window, cx| forward(action, window, cx))
        .on_action(|action: &Paste, window, cx| forward(action, window, cx))
        .on_action(|action: &SelectAll, window, cx| forward(action, window, cx))
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, TestAppContext, VisualTestContext, div,
    };

    use super::*;
    use crate::ui::{CodeEditor, TextInput};

    struct Host {
        input: Entity<TextInput>,
        editor: Entity<CodeEditor>,
    }

    impl Render for Host {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            handle(div().size_full().flex().flex_col().child(self.input.clone()).child(self.editor.clone()))
        }
    }

    fn host(cx: &mut TestAppContext) -> (Entity<Host>, &mut VisualTestContext) {
        let _ = crate::test_support::setup(cx);
        cx.update(|cx| {
            cx.bind_keys(crate::ui::text_input::key_bindings());
            cx.bind_keys(crate::ui::code_editor::key_bindings());
        });
        let (host, cx) = cx.add_window_view(|_, cx| Host {
            input: cx.new(|cx| TextInput::new("", cx)),
            editor: cx.new(|cx| CodeEditor::new("", crate::sql::Syntax::Plain, cx)),
        });
        crate::test_support::draw(cx);
        (host, cx)
    }

    fn run(action: impl Action, cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.dispatch_action(action.boxed_clone(), cx));
        cx.run_until_parked();
    }

    #[gpui::test]
    fn the_edit_menu_copies_and_pastes_in_the_focused_text_field(cx: &mut TestAppContext) {
        let (host, cx) = host(cx);
        let input = host.read_with(cx, |host, _| host.input.clone());
        input.update_in(cx, |input, window, cx| {
            input.set_text("orders", cx);
            input.focus(window);
        });
        run(SelectAll, cx);
        run(Copy, cx);
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("orders"));
        run(Paste, cx);
        run(Paste, cx);
        assert_eq!(
            input.read_with(cx, |input, _| input.text().to_owned()),
            "ordersorders",
            "paste replaced the selection"
        );
    }

    #[gpui::test]
    fn the_edit_menu_undoes_in_the_focused_sql_editor(cx: &mut TestAppContext) {
        let (host, cx) = host(cx);
        let editor = host.read_with(cx, |host, _| host.editor.clone());
        cx.update(|window, cx| editor.read(cx).focus(window));
        cx.simulate_input("SELECT 1");
        assert_eq!(editor.read_with(cx, |editor, _| editor.text().to_owned()), "SELECT 1");
        run(Undo, cx);
        assert_ne!(editor.read_with(cx, |editor, _| editor.text().to_owned()), "SELECT 1", "undone");
        run(Redo, cx);
        assert_eq!(editor.read_with(cx, |editor, _| editor.text().to_owned()), "SELECT 1", "redone");
    }

    #[gpui::test]
    fn the_edit_menu_does_nothing_where_nothing_takes_it(cx: &mut TestAppContext) {
        let (_host, cx) = host(cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("kept".into()));
        cx.update(|window, _| window.blur());
        run(Copy, cx);
        run(Paste, cx);
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("kept"));
    }
}
