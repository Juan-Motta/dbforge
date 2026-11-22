//! The dialogs over a table's pending changes: the SQL they would run, and
//! the question a connection tagged PROD asks before they run. Each tells
//! its owner with a [`DismissEvent`] when it closes, and the PROD question
//! with [`ConfirmEvent::Apply`] when the user goes ahead.

use gpui::{
    AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled, Window, actions, div, px,
};

use crate::i18n::Translate;
use crate::sql::Syntax;
use crate::table::edits::Counts;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::{CodeEditor, button, glass, icon, primary_button};

actions!(table_dialog, [Dismiss]);

const CONTEXT: &str = "TableDialog";

/// Escape closes either dialog; in the PROD question it is Cancel, which has the focus.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("escape", Dismiss, Some(CONTEXT)), KeyBinding::new("enter", Dismiss, Some(CONTEXT))]
}

/// The pending changes as the SQL they would run, with how they run.
pub struct PreviewDialog {
    note: &'static str,
    code: Entity<CodeEditor>,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for PreviewDialog {}

impl Focusable for PreviewDialog {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PreviewDialog {
    /// `statements` as the core wrote them, each ended with `;`, highlighted as `syntax`.
    pub fn new(statements: &[String], syntax: Syntax, note: &'static str, cx: &mut Context<Self>) -> Self {
        let text: Vec<String> =
            statements.iter().map(|line| if line.ends_with(';') { line.clone() } else { format!("{line};") }).collect();
        let code = cx.new(|cx| {
            let mut editor = CodeEditor::new(&text.join("\n"), syntax, cx);
            editor.set_read_only(true, cx);
            editor
        });
        Self { note, code, focus: cx.focus_handle() }
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }
}

/// A dialog's title row with its close button.
fn title_row(title: String, close_label: &'static str, cx: &mut Context<impl EventEmitter<DismissEvent>>) -> gpui::Div {
    let theme = cx.theme();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(title))
        .child(
            div()
                .id("dialog-close")
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(13.))
                .cursor_pointer()
                .hover(|close| close.bg(theme.sel))
                .tooltip(crate::ui::field::Tooltip::text(close_label))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
                .child(icon("close", theme.text2, 13.)),
        )
}

impl Render for PreviewDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().table;
        let theme = *cx.theme();
        glass(&theme)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .bg(theme.dialog)
            .w(px(720.))
            .max_w_full()
            .rounded(px(22.))
            .p(px(22.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(title_row(m.preview_title.to_owned(), m.close, cx))
            .child(div().text_size(px(12.5)).text_color(theme.text2).child(self.note))
            .child(
                div()
                    .h(px(320.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(theme.line)
                    .overflow_hidden()
                    .flex()
                    .child(self.code.clone()),
            )
    }
}

/// What the PROD question tells its owner, besides closing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmEvent {
    Apply,
}

/// Asks before changing data on a connection whose tag asks first (PROD).
pub struct ConfirmDialog {
    tag: String,
    table: String,
    counts: Counts,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for ConfirmDialog {}
impl EventEmitter<ConfirmEvent> for ConfirmDialog {}

impl Focusable for ConfirmDialog {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ConfirmDialog {
    /// `table` as `schema.name`, the changes `counts`.
    pub fn new(tag: String, table: String, counts: Counts, cx: &mut Context<Self>) -> Self {
        Self { tag, table, counts, focus: cx.focus_handle() }
    }

    /// Cancel has the keyboard: Enter or Escape changes nothing.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        cx.emit(ConfirmEvent::Apply);
    }
}

impl Render for ConfirmDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().table;
        let theme = *cx.theme();
        let Counts { updated, inserted, deleted } = self.counts;
        let focused = self.focus.is_focused(window);
        glass(&theme)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .bg(theme.dialog)
            .w(px(460.))
            .rounded(px(22.))
            .p(px(22.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(title_row((m.confirm_title)(&self.tag), m.close, cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_baseline()
                    .gap_x(px(4.))
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(theme.text2)
                    .child(m.confirm_marked)
                    .child(div().font_weight(FontWeight::BOLD).text_color(theme.text).child(format!("{}.", self.tag)))
                    .child(m.confirm_change)
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(theme.text)
                            .child(format!("{}:", self.table)),
                    )
                    .child(format!("{}.", (m.confirm_counts)(updated, inserted, deleted))),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button(&theme, None, m.cancel)
                            .id("confirm-cancel")
                            .cursor_pointer()
                            .when_focused(focused, &theme)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        primary_button(&theme, &(m.apply_on)(&self.tag))
                            .id("confirm-apply")
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    ),
            )
    }
}

/// A ring on the button that has the keyboard.
trait FocusRing {
    fn when_focused(self, focused: bool, theme: &crate::theme::Theme) -> Self;
}

impl FocusRing for gpui::Stateful<gpui::Div> {
    fn when_focused(self, focused: bool, theme: &crate::theme::Theme) -> Self {
        if focused { self.border_color(theme.accent) } else { self }
    }
}
