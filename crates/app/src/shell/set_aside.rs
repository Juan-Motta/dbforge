//! Tells the user, once at start, about settings files DBForge couldn't read:
//! each was moved aside and kept, and the app started without it.

use gpui::{
    Context, DismissEvent, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled, Window, actions, div, px,
};

use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::{glass, primary_button};

actions!(set_aside_dialog, [Confirm, Dismiss]);

const CONTEXT: &str = "SetAsideDialog";

/// Enter accepts the notice and Escape closes it, like its OK button.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("enter", Confirm, Some(CONTEXT)), KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

pub struct SetAsideDialog {
    files: Vec<String>,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for SetAsideDialog {}

impl Focusable for SetAsideDialog {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SetAsideDialog {
    pub fn new(files: Vec<String>, cx: &mut Context<Self>) -> Self {
        Self { files, focus: cx.focus_handle() }
    }

    /// Takes the keyboard, so Enter and Escape reach the dialog.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }
}

impl Render for SetAsideDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let m = cx.messages();
        glass(theme)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.close(cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.close(cx)))
            .bg(theme.dialog)
            .w(px(500.))
            .rounded(px(22.))
            .p(px(22.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(m.set_aside_title))
            .child(div().text_size(px(13.)).text_color(theme.text2).child(m.set_aside_note))
            .child(
                div()
                    .rounded(px(12.))
                    .bg(theme.sel)
                    .border_1()
                    .border_color(theme.line)
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .font_family(MONO)
                    .text_size(px(12.))
                    .text_color(theme.text2)
                    .children(self.files.iter().map(|file| div().child(file.clone()))),
            )
            .child(
                div().flex().justify_end().child(
                    primary_button(theme, m.set_aside_ok)
                        .id("set-aside-ok")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                ),
            )
    }
}
