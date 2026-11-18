//! Asks before removing a saved connection; its databases on the server are
//! untouched. The core forgets the connection and its saved secrets.

use dbforge_core::connections::commands;
use dbforge_core::connections::model::Connection;
use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled, Window, actions, div,
    prelude::FluentBuilder, px,
};

use crate::backend::Backend;
use crate::i18n::Translate;
use crate::theme::ActiveTheme;
use crate::ui::{button, enabled, glass, primary_button};

actions!(delete_connection_dialog, [Confirm, Dismiss]);

const CONTEXT: &str = "DeleteConnectionDialog";

/// Enter deletes, as the focused main button would; Escape closes.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("enter", Confirm, Some(CONTEXT)), KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

/// What the dialog tells the window, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteConnectionEvent {
    /// The connection with this id was deleted.
    Deleted(String),
}

pub struct DeleteConnectionDialog {
    connection: Connection,
    deleting: bool,
    error: Option<String>,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for DeleteConnectionDialog {}
impl EventEmitter<DeleteConnectionEvent> for DeleteConnectionDialog {}

impl Focusable for DeleteConnectionDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DeleteConnectionDialog {
    pub fn new(connection: Connection, cx: &mut Context<Self>) -> Self {
        Self { connection, deleting: false, error: None, focus: cx.focus_handle() }
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
        let id = self.connection.id.clone();
        let deleting = Backend::global(cx).spawn(cx, {
            let id = id.clone();
            move |core| async move {
                commands::delete_connection(core.connections(), id).await.map_err(|error| error.to_string())
            }
        });
        cx.spawn(async move |this, cx| {
            let deleted = deleting.await;
            let _ = this.update(cx, |this, cx| match deleted {
                Ok(()) => {
                    cx.emit(DeleteConnectionEvent::Deleted(id));
                    cx.emit(DismissEvent);
                }
                Err(message) => {
                    this.error = Some(message);
                    this.deleting = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for DeleteConnectionDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().connection;
        let deleting = self.deleting;
        glass(&theme)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.delete(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .bg(theme.card)
            .w(px(400.))
            .rounded(px(22.))
            .p(px(22.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child((m.delete_title)(&self.connection.name)),
            )
            .child(div().text_size(px(13.)).text_color(theme.text2).child(m.delete_note))
            // Room for an error is kept, so one appearing never moves the buttons.
            .child(div().min_h(px(30.)).text_size(px(12.5)).text_color(theme.err).children(self.error.clone()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button(&theme, None, m.cancel)
                            .id("delete-cancel")
                            .cursor_pointer()
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        enabled(
                            primary_button(&theme, if deleting { m.deleting } else { m.delete_connection }),
                            !deleting,
                        )
                        .id("delete-confirm")
                        .when(!deleting, |confirm| confirm.on_click(cx.listener(|this, _, _, cx| this.delete(cx)))),
                    ),
            )
    }
}
