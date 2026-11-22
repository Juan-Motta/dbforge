//! The Library's small dialogs: renaming a saved query, and asking before
//! deleting one. Both stay open showing why when the change fails, and
//! close when it is done. Enter applies; Escape closes.

use dbforge_core::saved_queries::{SavedQuery, SavedQueryInput};
use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::store;
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, Theme};
use crate::ui::field::{field, label, text_box};
use crate::ui::{Ellipsis, TextInput, TextInputEvent, button, enabled, glass, primary_button};

actions!(saved_dialog, [Confirm, Dismiss]);

const CONTEXT: &str = "SavedQueryDialog";

/// Enter applies, as the main button would; Escape closes.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("enter", Confirm, Some(CONTEXT)), KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

/// A small dialog's frame: its title, its body, then room for an error and the buttons.
fn frame(theme: &Theme, title: String) -> gpui::Div {
    glass(theme)
        .bg(theme.dialog)
        .w(px(400.))
        .max_w_full()
        .rounded(px(22.))
        .p(px(22.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).ellipsis().child(title))
}

/// Room for an error, kept, so one appearing never moves the buttons.
fn error_line(theme: &Theme, error: Option<String>) -> impl IntoElement {
    div().min_h(px(20.)).text_size(px(12.5)).text_color(theme.err).children(error)
}

/// Cancel and the main action, on the right.
fn buttons(cancel: impl IntoElement, confirm: impl IntoElement) -> impl IntoElement {
    div().flex().justify_end().gap(px(8.)).child(cancel).child(confirm)
}

/// Asks for a saved query's new name.
pub struct RenameDialog {
    query: SavedQuery,
    name: Entity<TextInput>,
    saving: bool,
    error: Option<String>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for RenameDialog {}

impl Focusable for RenameDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl RenameDialog {
    pub fn new(query: SavedQuery, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| {
            let mut input = TextInput::new("", cx);
            input.set_text(&query.name, cx);
            input.select_all(cx);
            input
        });
        let subscriptions =
            vec![cx.subscribe_in(&name, window, |this, _, event: &TextInputEvent, _, cx| match event {
                TextInputEvent::Changed => {
                    this.error = None;
                    cx.notify();
                }
                TextInputEvent::Submitted => this.rename(cx),
                TextInputEvent::Cancelled => cx.emit(DismissEvent),
                TextInputEvent::Pasted(_) => {}
            })];
        Self { query, name, saving: false, error: None, focus: cx.focus_handle(), _subscriptions: subscriptions }
    }

    /// The name field takes the keyboard.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.name.read(cx).focus(window);
    }

    fn ready(&self, cx: &App) -> bool {
        !self.saving && !self.name.read(cx).text().trim().is_empty()
    }

    fn rename(&mut self, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }
        self.saving = true;
        let query = &self.query;
        let input = SavedQueryInput {
            name: self.name.read(cx).text().to_owned(),
            sql: query.sql.clone(),
            description: query.description.clone(),
            folder: query.folder.clone(),
            scope: query.scope.clone(),
            engine: query.engine,
        };
        let saving = store::save(Some(query.id.clone()), input, cx);
        cx.spawn(async move |this, cx| {
            let saved = saving.await;
            let _ = this.update(cx, |this, cx| match saved {
                Ok(_) => cx.emit(DismissEvent),
                Err(error) => {
                    this.error = Some(error);
                    this.saving = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for RenameDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().saved;
        let ready = self.ready(cx);
        frame(&theme, m.rename_title.to_owned())
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.rename(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .child(field(
                label(&theme, "rename-name", m.name, None),
                text_box(&self.name, false, false, window, cx),
                None,
            ))
            .child(error_line(&theme, self.error.clone()))
            .child(buttons(
                button(&theme, None, m.cancel)
                    .id("rename-cancel")
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                enabled(primary_button(&theme, if self.saving { m.saving } else { m.save }), ready)
                    .id("rename-save")
                    .when(ready, |save| save.on_click(cx.listener(|this, _, _, cx| this.rename(cx)))),
            ))
    }
}

/// Asks before deleting a saved query.
pub struct DeleteSavedDialog {
    query: SavedQuery,
    deleting: bool,
    error: Option<String>,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for DeleteSavedDialog {}

impl Focusable for DeleteSavedDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DeleteSavedDialog {
    pub fn new(query: SavedQuery, cx: &mut Context<Self>) -> Self {
        Self { query, deleting: false, error: None, focus: cx.focus_handle() }
    }

    /// Takes the keyboard, so Enter and Escape reach the dialog.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if self.deleting {
            return;
        }
        self.deleting = true;
        self.error = None;
        let deleting = store::delete(self.query.id.clone(), cx);
        cx.spawn(async move |this, cx| {
            let deleted = deleting.await;
            let _ = this.update(cx, |this, cx| match deleted {
                Ok(()) => cx.emit(DismissEvent),
                Err(error) => {
                    this.error = Some(error);
                    this.deleting = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for DeleteSavedDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().saved;
        let deleting = self.deleting;
        frame(&theme, (m.delete_title)(&self.query.name))
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.delete(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .child(div().text_size(px(13.)).text_color(theme.text2).child(m.delete_note))
            .child(error_line(&theme, self.error.clone()))
            .child(buttons(
                button(&theme, None, m.cancel)
                    .id("delete-saved-cancel")
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                enabled(primary_button(&theme, if deleting { m.deleting } else { m.confirm_delete }), !deleting)
                    .id("delete-saved-confirm")
                    .when(!deleting, |confirm| confirm.on_click(cx.listener(|this, _, _, cx| this.delete(cx)))),
            ))
    }
}
