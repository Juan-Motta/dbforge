//! The toolbar connection's details, opened from its chevron as the former
//! web UI's title-bar chip did: whether the server answers (asked each time
//! it opens, with the saved secrets), its version, where it is, its
//! database, SSL mode and SSH tunnel ([`connection_facts`]), why it doesn't
//! answer, and a button that edits the connection. It closes on Escape, a
//! click outside it or Edit.

use dbforge_core::connections::commands;
use dbforge_core::connections::model::{Connection, ConnectionInput};
use dbforge_core::connections::tags::Tag;
use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled, Task, Window, actions, div,
    prelude::FluentBuilder, px,
};

use super::connection_facts::connection_facts;
use crate::backend::Backend;
use crate::i18n::Translate;
use crate::theme::ActiveTheme;
use crate::ui::{button, dot, engine_dot, glass, tag_badge};

actions!(connection_details, [Dismiss]);

const CONTEXT: &str = "ConnectionDetails";

/// As wide as the former web UI's popover: long addresses wrap inside it.
pub const WIDTH: f32 = 340.;

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

/// What the details ask of the window, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone)]
pub enum ConnectionDetailsEvent {
    /// Edit connection: open the connection dialog on it.
    Edit(Connection),
}

/// What the server said when asked.
#[derive(Debug, Clone, PartialEq)]
pub enum Server {
    Checking,
    Reachable { version: String },
    NotReachable { error: String },
}

pub struct ConnectionDetails {
    connection: Connection,
    tag: Option<Tag>,
    server: Server,
    focus: FocusHandle,
    /// The test running; dropped with the details, so a late answer is ignored.
    _testing: Task<()>,
}

impl EventEmitter<ConnectionDetailsEvent> for ConnectionDetails {}
impl EventEmitter<DismissEvent> for ConnectionDetails {}

impl Focusable for ConnectionDetails {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ConnectionDetails {
    /// The details of `connection`, asking its server at once; they take the keyboard.
    pub fn new(connection: Connection, tag: Option<Tag>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let input = saved_input(&connection);
        let testing = Backend::global(cx).spawn(cx, move |core| async move {
            commands::test_connection(core.connections(), core.statement_log(), input)
                .await
                .map_err(|error| error.to_string())
        });
        let testing = cx.spawn(async move |this, cx| {
            let answer = testing.await;
            let _ = this.update(cx, |this, cx| {
                this.server = match answer {
                    Ok(version) => Server::Reachable { version },
                    Err(error) => Server::NotReachable { error },
                };
                cx.notify();
            });
        });
        Self { connection, tag, server: Server::Checking, focus, _testing: testing }
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    fn edit(&mut self, cx: &mut Context<Self>) {
        cx.emit(ConnectionDetailsEvent::Edit(self.connection.clone()));
    }
}

/// The saved `connection` as a test asks for it: its saved password and SSH secret.
fn saved_input(connection: &Connection) -> ConnectionInput {
    let connection = connection.clone();
    ConnectionInput {
        id: Some(connection.id),
        name: connection.name,
        engine: connection.engine,
        environment: connection.environment,
        host: connection.host,
        port: connection.port,
        database: connection.database,
        username: connection.username,
        password: None,
        ssl: connection.ssl,
        ssh: connection.ssh,
        ssh_secret: None,
        srv: connection.srv,
        agent_access: connection.agent_access,
    }
}

impl Render for ConnectionDetails {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = cx.messages();
        let shell = &m.shell;
        let (state_tint, state) = match &self.server {
            Server::Checking => (theme.text3, shell.checking),
            Server::Reachable { .. } => (theme.ok, shell.reachable),
            Server::NotReachable { .. } => (theme.err, shell.not_reachable),
        };
        let version = match &self.server {
            Server::Checking => None,
            Server::Reachable { version } => Some(version.as_str()),
            Server::NotReachable { .. } => Some("—"),
        };
        let facts = connection_facts(&self.connection, version, shell);
        glass(&theme)
            .bg(theme.card)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
            .w(px(WIDTH))
            .rounded(px(14.))
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .text_size(px(12.5))
            .text_color(theme.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(engine_dot(&theme, self.connection.engine).size(px(12.)).rounded(px(6.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.connection.name.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(12.))
                                    .text_color(theme.text2)
                                    .child(dot(state_tint).size(px(7.)))
                                    .child(state),
                            ),
                    )
                    .children(self.tag.as_ref().map(|tag| tag_badge(&theme, tag))),
            )
            .child(div().flex().flex_col().gap(px(6.)).children(facts.into_iter().map(|(label, value)| {
                div()
                    .flex()
                    .gap(px(10.))
                    .child(div().w(px(96.)).flex_none().text_color(theme.text3).child(label))
                    .child(div().flex_1().min_w_0().font_family(crate::theme::MONO).text_size(px(12.)).child(value))
            })))
            .when_some(
                match &self.server {
                    Server::NotReachable { error } => Some(error.clone()),
                    _ => None,
                },
                |details, error| {
                    details.child(
                        div()
                            .id("connection-details-error")
                            .max_h(px(120.))
                            .overflow_y_scroll()
                            .text_size(px(12.))
                            .text_color(theme.err)
                            .child(error),
                    )
                },
            )
            .child(
                div().flex().child(
                    button(&theme, Some("edit"), m.edit_connection)
                        .id("connection-details-edit")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.edit(cx))),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use dbforge_core::connections::model::{AgentAccess, Engine};
    use gpui::{Entity, TestAppContext};

    use super::*;

    fn sqlite(path: &str) -> Connection {
        Connection {
            id: "local".into(),
            name: "Local".into(),
            engine: Engine::Sqlite,
            environment: None,
            host: String::new(),
            port: 0,
            database: path.into(),
            username: String::new(),
            ssl: Default::default(),
            ssh: Default::default(),
            srv: false,
            agent_access: AgentAccess::None,
        }
    }

    fn open(
        connection: Connection,
        cx: &mut TestAppContext,
    ) -> (Entity<ConnectionDetails>, &mut gpui::VisualTestContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        cx.add_window_view(|window, cx| ConnectionDetails::new(connection, None, window, cx))
    }

    #[gpui::test]
    fn asks_the_server_when_it_opens_and_says_it_is_reachable(cx: &mut TestAppContext) {
        let dir = crate::test_support::setup(cx);
        let file = dir.path().join("shop.db");
        std::fs::write(&file, b"").unwrap();
        let (details, cx) = open(sqlite(file.to_str().unwrap()), cx);
        assert_eq!(details.read_with(cx, |details, _| details.server().clone()), Server::Checking);
        crate::test_support::draw(cx);
        crate::test_support::wait_until(cx, "the test", |cx| {
            details.read_with(cx, |details, _| details.server() != &Server::Checking)
        });
        match details.read_with(cx, |details, _| details.server().clone()) {
            Server::Reachable { version } => assert!(version.contains("SQLite"), "{version}"),
            other => panic!("expected the file to answer, got {other:?}"),
        }
        crate::test_support::draw(cx);
    }

    #[gpui::test]
    fn says_why_a_server_is_not_reachable(cx: &mut TestAppContext) {
        let dir = crate::test_support::setup(cx);
        let missing = dir.path().join("missing").join("gone.db");
        let (details, cx) = open(sqlite(missing.to_str().unwrap()), cx);
        crate::test_support::wait_until(cx, "the test", |cx| {
            details.read_with(cx, |details, _| details.server() != &Server::Checking)
        });
        assert!(matches!(details.read_with(cx, |details, _| details.server().clone()), Server::NotReachable { .. }));
        crate::test_support::draw(cx);
    }

    #[gpui::test]
    fn edit_asks_for_the_connection_and_escape_closes(cx: &mut TestAppContext) {
        let _dir = crate::test_support::setup(cx);
        let (details, cx) = open(sqlite("/nowhere.db"), cx);
        let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let (edits, closes) = (heard.clone(), heard.clone());
        cx.update(|_, cx| {
            cx.subscribe(&details, move |_, event: &ConnectionDetailsEvent, _| {
                let ConnectionDetailsEvent::Edit(connection) = event;
                edits.borrow_mut().push(format!("edit {}", connection.id));
            })
            .detach();
            cx.subscribe(&details, move |_, _: &DismissEvent, _| closes.borrow_mut().push("close".into())).detach();
        });
        details.update(cx, |details, cx| details.edit(cx));
        crate::test_support::draw(cx);
        cx.simulate_keystrokes("escape");
        assert_eq!(*heard.borrow(), ["edit local", "close"]);
    }
}
