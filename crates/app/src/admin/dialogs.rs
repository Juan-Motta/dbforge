//! The two admin dialogs. New database / role / user asks for what the
//! engine needs and creates it; Drop database asks for the database's name
//! again before dropping it, so a misplaced click can't lose it. Both stay
//! open showing the server's reason when it refuses, and close when done.
//! Enter submits once the form is complete; Escape closes.

use dbforge_core::admin::{self, CreateRequest, DropRequest};
use dbforge_core::connections::model::Connection;
use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};

use super::new_object::{
    ExtraField, FieldValues, NewObjectKind, create_label_of, fields_of, is_complete, label_of, note_of, placeholder,
    title_of, to_new_object,
};
use crate::backend::Backend;
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, Theme};
use crate::ui::field::{field, label, text_box};
use crate::ui::{TextInput, TextInputEvent, button, enabled, glass, primary_button};

/// What an admin dialog tells the window, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminEvent {
    /// A database, role or user now exists on connection `connection_id`'s server.
    Created { connection_id: String },
    /// `database` is gone from connection `connection_id`'s server.
    Dropped { connection_id: String, database: String },
}

/// A dialog's frame: its title, its body, room for an error and its buttons.
fn frame(theme: &Theme, title: String) -> gpui::Div {
    glass(theme)
        .bg(theme.card)
        .w(px(420.))
        .max_w_full()
        .rounded(px(22.))
        .p(px(22.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).truncate().child(title))
}

/// Room for an error, kept, so one appearing never moves the buttons.
fn error_line(theme: &Theme, error: Option<String>) -> impl IntoElement {
    div().min_h(px(20.)).text_size(px(12.5)).text_color(theme.err).children(error)
}

pub struct NewObjectDialog {
    connection: Connection,
    kind: NewObjectKind,
    fields: Vec<ExtraField>,
    name: Entity<TextInput>,
    extra: Vec<(ExtraField, Entity<TextInput>)>,
    creating: bool,
    error: Option<String>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for NewObjectDialog {}
impl EventEmitter<AdminEvent> for NewObjectDialog {}

impl Focusable for NewObjectDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl NewObjectDialog {
    pub fn new(connection: Connection, kind: NewObjectKind, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fields = fields_of(connection.engine, kind);
        let name = cx.new(|cx| TextInput::new("", cx));
        let extra: Vec<(ExtraField, Entity<TextInput>)> = fields
            .iter()
            .map(|&field| {
                let input = cx.new(|cx| {
                    let input = TextInput::new(placeholder(field).unwrap_or_default(), cx);
                    if field == ExtraField::Password { input.masked() } else { input }
                });
                (field, input)
            })
            .collect();
        let subscriptions = std::iter::once(&name)
            .chain(extra.iter().map(|(_, input)| input))
            .map(|input| cx.subscribe_in(input, window, Self::on_input))
            .collect();
        Self {
            connection,
            kind,
            fields,
            name,
            extra,
            creating: false,
            error: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The name field takes the keyboard.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.name.read(cx).focus(window);
    }

    fn values(&self, cx: &App) -> FieldValues {
        let mut values = FieldValues { name: self.name.read(cx).text().to_owned(), ..FieldValues::default() };
        for (field, input) in &self.extra {
            values.set(*field, input.read(cx).text().to_owned());
        }
        values
    }

    fn on_input(&mut self, _: &Entity<TextInput>, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Changed => {
                self.error = None;
                cx.notify();
            }
            TextInputEvent::Submitted => self.create(window, cx),
            TextInputEvent::Cancelled => cx.emit(DismissEvent),
            TextInputEvent::Pasted(_) => {}
        }
    }

    /// The fields take no typing while the server works.
    fn set_busy(&mut self, busy: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.creating = busy;
        for input in std::iter::once(&self.name).chain(self.extra.iter().map(|(_, input)| input)) {
            input.update(cx, |input, cx| input.set_disabled(busy, window, cx));
        }
        cx.notify();
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values = self.values(cx);
        if self.creating || !is_complete(&values, &self.fields) {
            return;
        }
        self.set_busy(true, window, cx);
        let connection_id = self.connection.id.clone();
        let request = CreateRequest::new(connection_id.clone(), to_new_object(self.kind, &self.fields, &values));
        let creating = Backend::global(cx).spawn(cx, move |core| async move {
            admin::create_object(core.connections(), core.statement_log(), request).await.map_err(|e| e.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let created = creating.await;
            let _ = this.update_in(cx, |this, window, cx| match created {
                Ok(()) => {
                    cx.emit(AdminEvent::Created { connection_id });
                    cx.emit(DismissEvent);
                }
                Err(message) => {
                    this.error = Some(message);
                    this.set_busy(false, window, cx);
                    this.focus(window, cx);
                }
            });
        })
        .detach();
    }
}

impl Render for NewObjectDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().admin;
        let complete = is_complete(&self.values(cx), &self.fields);
        let ready = complete && !self.creating;
        let inputs = std::iter::once((None, &self.name)).chain(self.extra.iter().map(|(f, input)| (Some(*f), input)));
        let fields: Vec<gpui::AnyElement> = inputs
            .enumerate()
            .map(|(index, (extra, input))| {
                let text = extra.map_or(m.name, |field| label_of(field, m));
                let mono = extra != Some(ExtraField::Password);
                field(
                    label(&theme, ("new-object-label", index), text, None),
                    text_box(input, false, mono, window, cx),
                    None,
                )
                .into_any_element()
            })
            .collect();
        frame(&theme, title_of(self.kind, &self.connection.name, m))
            .track_focus(&self.focus)
            .children(fields)
            .child(div().text_size(px(12.5)).text_color(theme.text3).child(note_of(
                self.connection.engine,
                self.kind,
                m,
            )))
            .child(error_line(&theme, self.error.clone()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button(&theme, None, m.cancel)
                            .id("new-object-cancel")
                            .cursor_pointer()
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        enabled(
                            primary_button(
                                &theme,
                                if self.creating { m.creating } else { create_label_of(self.kind, m) },
                            ),
                            ready,
                        )
                        .id("new-object-create")
                        .when(ready, |create| {
                            create.on_click(cx.listener(|this, _, window, cx| this.create(window, cx)))
                        }),
                    ),
            )
    }
}

pub struct DropDatabaseDialog {
    connection: Connection,
    database: String,
    typed: Entity<TextInput>,
    dropping: bool,
    error: Option<String>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for DropDatabaseDialog {}
impl EventEmitter<AdminEvent> for DropDatabaseDialog {}

impl Focusable for DropDatabaseDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DropDatabaseDialog {
    pub fn new(connection: Connection, database: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let typed = cx.new(|cx| TextInput::new("", cx));
        let subscriptions = vec![cx.subscribe_in(&typed, window, Self::on_input)];
        Self {
            connection,
            database,
            typed,
            dropping: false,
            error: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The confirmation field takes the keyboard.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.typed.read(cx).focus(window);
    }

    /// The name typed again, exactly.
    fn confirmed(&self, cx: &App) -> bool {
        self.typed.read(cx).text() == self.database
    }

    fn on_input(&mut self, _: &Entity<TextInput>, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Changed => {
                self.error = None;
                cx.notify();
            }
            TextInputEvent::Submitted => self.drop_database(window, cx),
            TextInputEvent::Cancelled => cx.emit(DismissEvent),
            TextInputEvent::Pasted(_) => {}
        }
    }

    /// The field takes no typing while the server works.
    fn set_busy(&mut self, busy: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.dropping = busy;
        self.typed.update(cx, |input, cx| input.set_disabled(busy, window, cx));
        cx.notify();
    }

    fn drop_database(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dropping || !self.confirmed(cx) {
            return;
        }
        self.set_busy(true, window, cx);
        let (connection_id, database) = (self.connection.id.clone(), self.database.clone());
        let request = DropRequest::new(connection_id.clone(), database.clone());
        let dropping = Backend::global(cx).spawn(cx, move |core| async move {
            admin::drop_database(core.connections(), core.statement_log(), request).await.map_err(|e| e.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let dropped = dropping.await;
            let _ = this.update_in(cx, |this, window, cx| match dropped {
                Ok(()) => {
                    cx.emit(AdminEvent::Dropped { connection_id, database });
                    cx.emit(DismissEvent);
                }
                Err(message) => {
                    this.error = Some(message);
                    this.set_busy(false, window, cx);
                    this.focus(window, cx);
                }
            });
        })
        .detach();
    }
}

impl Render for DropDatabaseDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().admin;
        let ready = self.confirmed(cx) && !self.dropping;
        frame(&theme, (m.drop_title)(&self.database))
            .track_focus(&self.focus)
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme.text2)
                    .child((m.drop_note)(&self.database, &self.connection.name)),
            )
            .child(field(
                label(&theme, "drop-database-label", (m.drop_confirm)(&self.database), None),
                text_box(&self.typed, false, true, window, cx),
                None,
            ))
            .child(error_line(&theme, self.error.clone()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button(&theme, None, m.cancel)
                            .id("drop-database-cancel")
                            .cursor_pointer()
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        enabled(
                            button(&theme, None, if self.dropping { m.dropping } else { m.drop_database })
                                .text_color(theme.err),
                            ready,
                        )
                        .id("drop-database-confirm")
                        .when(ready, |drop| {
                            drop.on_click(cx.listener(|this, _, window, cx| this.drop_database(window, cx)))
                        }),
                    ),
            )
    }
}
