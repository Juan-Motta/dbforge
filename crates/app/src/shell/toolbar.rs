//! The toolbar above the tabs: the connection in use (the one selected in
//! the explorer), whose chevron opens its details ([`ConnectionDetails`])
//! under it, search everywhere (the command palette) and the toggles of the
//! explorer and the statement console, and the settings. Each says its
//! shortcut in its tooltip.

use dbforge_core::connections::model::Connection;
use dbforge_core::connections::tags::Tag;
use gpui::{
    AnyElement, AppContext, Context, DismissEvent, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window, anchored, deferred,
    div, prelude::FluentBuilder, px,
};

use super::connection_details::{ConnectionDetails, ConnectionDetailsEvent};
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::field::Tooltip;
use crate::ui::{any, capsule, engine_dot, group, icon, segment, shortcut, tag_badge};

/// Room for the traffic lights on macOS, when the explorer that holds them is hidden.
const TRAFFIC_LIGHTS: f32 = 72.;

/// Space kept between the window's edges and the connection's details.
const MARGIN: f32 = 8.;

/// What the toolbar asks of the window.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolbarEvent {
    Search,
    ToggleExplorer,
    ToggleStatements,
    OpenSettings,
    /// The details' Edit connection.
    EditConnection(Box<Connection>),
}

/// The connection's details while they are open, and what closes them.
struct OpenDetails {
    details: Entity<ConnectionDetails>,
    _subscriptions: [Subscription; 2],
}

pub struct Toolbar {
    /// The connection in use and its tag, if it has one.
    connection: Option<(Connection, Option<Tag>)>,
    explorer_open: bool,
    statements_open: bool,
    details: Option<OpenDetails>,
}

impl EventEmitter<ToolbarEvent> for Toolbar {}

impl Toolbar {
    pub fn new(explorer_open: bool, statements_open: bool) -> Self {
        Self { connection: None, explorer_open, statements_open, details: None }
    }

    /// Shows `connection` (and its tag) as the one in use; details open on
    /// another connection (or on the same one before a change) close.
    pub fn show(&mut self, connection: Option<(Connection, Option<Tag>)>, cx: &mut Context<Self>) {
        let changed = self.connection.as_ref().map(|(connection, _)| connection)
            != connection.as_ref().map(|(connection, _)| connection);
        self.connection = connection;
        if changed {
            self.details = None;
        }
        cx.notify();
    }

    /// The connection's details are showing.
    pub fn details(&self) -> Option<&Entity<ConnectionDetails>> {
        self.details.as_ref().map(|open| &open.details)
    }

    /// Opens the details of the connection in use, or closes them when they are open.
    pub fn toggle_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.details.take().is_none() {
            self.open_details(window, cx);
        }
        cx.notify();
    }

    fn open_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((connection, tag)) = self.connection.clone() else { return };
        let details = cx.new(|cx| ConnectionDetails::new(connection, tag, window, cx));
        let edit = cx.subscribe(&details, |this, _, ConnectionDetailsEvent::Edit(connection), cx| {
            this.details = None;
            cx.emit(ToolbarEvent::EditConnection(Box::new(connection.clone())));
            cx.notify();
        });
        let close = cx.subscribe(&details, |this, _, _: &DismissEvent, cx| {
            this.details = None;
            cx.notify();
        });
        self.details = Some(OpenDetails { details, _subscriptions: [edit, close] });
    }

    /// Shows which panels are open.
    pub fn set_panels(&mut self, explorer_open: bool, statements_open: bool, cx: &mut Context<Self>) {
        self.explorer_open = explorer_open;
        self.statements_open = statements_open;
        cx.notify();
    }

    /// The connection capsule: its engine, name and tag, or that there is none.
    fn connection(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let capsule = capsule(theme).font_weight(FontWeight::MEDIUM).max_w(px(360.));
        match &self.connection {
            Some((connection, tag)) => div()
                .flex_none()
                .relative()
                .child(
                    capsule
                        .id("toolbar-connection")
                        .text_color(theme.text)
                        .cursor_pointer()
                        .when(self.details.is_some(), |capsule| capsule.bg(theme.glass_hi))
                        // On press, while open details' own click-outside is still
                        // closing them, so pressing again closes rather than reopens.
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| this.toggle_details(window, cx)),
                        )
                        .child(engine_dot(theme, connection.engine))
                        .child(div().min_w_0().truncate().child(connection.name.clone()))
                        .children(tag.as_ref().map(|tag| tag_badge(theme, tag)))
                        .child(icon("down", theme.text3, 12.)),
                )
                // Floating under the capsule: the toolbar never changes size.
                .children(self.details.as_ref().map(|open| {
                    div().absolute().top(px(40.)).left_0().child(
                        deferred(anchored().snap_to_window_with_margin(px(MARGIN)).child(open.details.clone()))
                            .with_priority(1),
                    )
                }))
                .into_any_element(),
            None => capsule
                .text_color(theme.text3)
                .child(icon("database", theme.text3, 14.))
                .child(cx.messages().no_connection)
                .child(icon("down", theme.text3, 12.))
                .into_any_element(),
        }
    }
}

impl Render for Toolbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let m = cx.messages();
        let (explorer_open, statements_open) = (self.explorer_open, self.statements_open);
        let explorer_tip = if explorer_open { m.hide_explorer } else { m.show_explorer };
        let statements_tip = if statements_open { m.hide_statements } else { m.show_statements };
        let toggle = |id: &'static str, name: &'static str, on: bool, tip: String, event: ToolbarEvent| {
            any(segment(theme, Some(name), "", on)
                .id(id)
                .cursor_pointer()
                .tooltip(Tooltip::text(tip))
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(event.clone()))))
        };
        div()
            .h(px(42.))
            .flex()
            .items_center()
            .gap(px(8.))
            // The traffic lights sit in the explorer's corner; hidden, they need the toolbar's.
            .when(!explorer_open && cfg!(target_os = "macos"), |bar| bar.pl(px(TRAFFIC_LIGHTS)))
            .child(self.connection(cx))
            .child(div().flex_1())
            .child(
                capsule(theme)
                    .id("toolbar-search")
                    .w(px(300.))
                    .text_color(theme.text3)
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ToolbarEvent::Search)))
                    .child(icon("search", theme.text3, 16.))
                    .child(m.search_everywhere)
                    .child(div().flex_1())
                    .child(div().font_family(MONO).text_size(px(11.5)).child(shortcut("k"))),
            )
            .child(group(
                theme,
                vec![
                    toggle(
                        "toolbar-explorer",
                        "sidebar",
                        explorer_open,
                        explorer_tip(&shortcut("b")),
                        ToolbarEvent::ToggleExplorer,
                    ),
                    toggle(
                        "toolbar-statements",
                        "panel",
                        statements_open,
                        statements_tip(&shortcut("j")),
                        ToolbarEvent::ToggleStatements,
                    ),
                    toggle("toolbar-settings", "sliders", false, m.settings.to_owned(), ToolbarEvent::OpenSettings),
                ],
            ))
    }
}

#[cfg(test)]
mod tests {
    use dbforge_core::connections::model::{AgentAccess, Engine};
    use gpui::TestAppContext;

    use super::*;

    fn connection(id: &str) -> Connection {
        Connection {
            id: id.into(),
            name: id.into(),
            engine: Engine::Sqlite,
            environment: None,
            host: String::new(),
            port: 0,
            database: "/nowhere.db".into(),
            username: String::new(),
            ssl: Default::default(),
            ssh: Default::default(),
            srv: false,
            agent_access: AgentAccess::None,
        }
    }

    #[gpui::test]
    fn the_chevron_opens_the_details_under_it_and_edit_asks_the_window(cx: &mut TestAppContext) {
        let _dir = crate::test_support::setup(cx);
        let (toolbar, cx) = cx.add_window_view(|_, _| Toolbar::new(true, false));
        let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events = heard.clone();
        cx.update(|_, cx| {
            cx.subscribe(&toolbar, move |_, event: &ToolbarEvent, _| events.borrow_mut().push(event.clone())).detach();
        });
        toolbar.update_in(cx, |toolbar, window, cx| toolbar.toggle_details(window, cx));
        assert!(toolbar.read_with(cx, |toolbar, _| toolbar.details().is_none()), "no connection, no details");

        toolbar.update(cx, |toolbar, cx| toolbar.show(Some((connection("a"), None)), cx));
        toolbar.update_in(cx, |toolbar, window, cx| toolbar.toggle_details(window, cx));
        crate::test_support::draw(cx);
        let details = toolbar.read_with(cx, |toolbar, _| toolbar.details().cloned()).expect("open");
        assert_eq!(details.read_with(cx, |details, _| details.connection().id.clone()), "a");

        // Another connection selected: they close.
        toolbar.update(cx, |toolbar, cx| toolbar.show(Some((connection("b"), None)), cx));
        assert!(toolbar.read_with(cx, |toolbar, _| toolbar.details().is_none()));

        toolbar.update_in(cx, |toolbar, window, cx| toolbar.toggle_details(window, cx));
        let details = toolbar.read_with(cx, |toolbar, _| toolbar.details().cloned()).expect("open");
        details.update(cx, |_, cx| cx.emit(ConnectionDetailsEvent::Edit(connection("b"))));
        cx.run_until_parked();
        assert!(toolbar.read_with(cx, |toolbar, _| toolbar.details().is_none()), "Edit closes them");
        assert_eq!(*heard.borrow(), [ToolbarEvent::EditConnection(Box::new(connection("b")))]);
    }
}
