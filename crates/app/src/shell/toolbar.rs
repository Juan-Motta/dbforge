//! The toolbar above the tabs: the connection in use (the one selected in
//! the explorer), search everywhere (the command palette) and the toggles
//! of the explorer and the statement console, and the settings. Each says
//! its shortcut in its tooltip.

use dbforge_core::connections::model::Connection;
use dbforge_core::connections::tags::Tag;
use gpui::{
    AnyElement, Context, EventEmitter, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::field::Tooltip;
use crate::ui::{any, capsule, engine_dot, group, icon, segment, shortcut, tag_badge};

/// Room for the traffic lights on macOS, when the explorer that holds them is hidden.
const TRAFFIC_LIGHTS: f32 = 72.;

/// What the toolbar asks of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarEvent {
    Search,
    ToggleExplorer,
    ToggleStatements,
    OpenSettings,
}

pub struct Toolbar {
    /// The connection in use and its tag, if it has one.
    connection: Option<(Connection, Option<Tag>)>,
    explorer_open: bool,
    statements_open: bool,
}

impl EventEmitter<ToolbarEvent> for Toolbar {}

impl Toolbar {
    pub fn new(explorer_open: bool, statements_open: bool) -> Self {
        Self { connection: None, explorer_open, statements_open }
    }

    /// Shows `connection` (and its tag) as the one in use.
    pub fn show(&mut self, connection: Option<(Connection, Option<Tag>)>, cx: &mut Context<Self>) {
        self.connection = connection;
        cx.notify();
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
            Some((connection, tag)) => capsule
                .text_color(theme.text)
                .child(engine_dot(theme, connection.engine))
                .child(div().min_w_0().truncate().child(connection.name.clone()))
                .children(tag.as_ref().map(|tag| tag_badge(theme, tag)))
                .child(icon("down", theme.text3, 12.))
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
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(event))))
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
