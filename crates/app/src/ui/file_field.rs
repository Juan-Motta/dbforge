//! A path on this computer, typed or picked with the OS's own file dialog
//! (Browse…), on every platform GPUI runs on. Cancelling the dialog keeps
//! the current path. It re-emits its text field's [`TextInputEvent`]s, a
//! pick included as [`TextInputEvent::Changed`].

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, Focusable, InteractiveElement, IntoElement, ParentElement,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};

use super::button::{button, enabled};
use super::field::text_box;
use super::text_input::{TextInput, TextInputEvent};
use crate::theme::ActiveTheme;

pub struct FileField {
    input: Entity<TextInput>,
    browse_label: SharedString,
    invalid: bool,
    _forwarding: Subscription,
}

impl EventEmitter<TextInputEvent> for FileField {}

impl Focusable for FileField {
    fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl FileField {
    pub fn new(
        placeholder: impl Into<SharedString>,
        browse_label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = placeholder.into();
        let input = cx.new(|cx| TextInput::new(placeholder, cx));
        let forwarding = cx.subscribe(&input, |_, _, event: &TextInputEvent, cx| cx.emit(event.clone()));
        Self { input, browse_label: browse_label.into(), invalid: false, _forwarding: forwarding }
    }

    pub fn text<'a>(&self, cx: &'a App) -> &'a str {
        self.input.read(cx).text()
    }

    /// Sets the path without an event: the owner set it.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_text(text, cx));
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        let placeholder = placeholder.into();
        self.input.update(cx, |input, cx| input.set_placeholder(placeholder, cx));
    }

    pub fn set_browse_label(&mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.browse_label = label.into();
        cx.notify();
    }

    pub fn set_disabled(&mut self, disabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_disabled(disabled, window, cx));
    }

    pub fn set_invalid(&mut self, invalid: bool, cx: &mut Context<Self>) {
        if self.invalid != invalid {
            self.invalid = invalid;
            cx.notify();
        }
    }

    fn disabled(&self, cx: &App) -> bool {
        self.input.read(cx).is_disabled()
    }

    /// Opens the OS's file dialog; a chosen file replaces the path.
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        let input = self.input.clone();
        window.focus(&input.focus_handle(cx));
        cx.spawn(async move |this, cx| {
            // Cancelled, or the platform has no dialog: the path stays as it was.
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let path = path.to_string_lossy().into_owned();
            let _ = this.update(cx, |_, cx| {
                input.update(cx, |input, cx| input.set_text(&path, cx));
                cx.emit(TextInputEvent::Changed);
            });
        })
        .detach();
    }
}

impl Render for FileField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let disabled = self.disabled(cx);
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().min_w_0().child(text_box(&self.input, self.invalid, true, window, cx)))
            .child(
                enabled(button(&theme, None, &self.browse_label), !disabled).id("browse").when(!disabled, |browse| {
                    browse.on_click(cx.listener(|this, _, window, cx| this.browse(window, cx)))
                }),
            )
    }
}
