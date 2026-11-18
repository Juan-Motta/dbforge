//! The toolbar above the tabs: the connection in use (the one selected in the
//! explorer), search everywhere and the panel toggles. The features behind
//! search and the toggles come later.

use dbforge_core::connections::model::Connection;
use dbforge_core::connections::tags::Tag;
use gpui::{AnyElement, Context, FontWeight, IntoElement, ParentElement, Render, Styled, Window, div, px};

use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::{any, capsule, engine_dot, group, icon, segment, tag_badge};

/// The command palette's shortcut, as each OS writes it.
const PALETTE_SHORTCUT: &str = if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl+K" };

#[derive(Default)]
pub struct Toolbar {
    /// The connection in use and its tag, if it has one.
    connection: Option<(Connection, Option<Tag>)>,
}

impl Toolbar {
    /// Shows `connection` (and its tag) as the one in use.
    pub fn show(&mut self, connection: Option<(Connection, Option<Tag>)>, cx: &mut Context<Self>) {
        self.connection = connection;
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
        div()
            .h(px(42.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.connection(cx))
            .child(div().flex_1())
            .child(
                capsule(theme)
                    .w(px(300.))
                    .text_color(theme.text3)
                    .child(icon("search", theme.text3, 16.))
                    .child(m.search_everywhere)
                    .child(div().flex_1())
                    .child(div().font_family(MONO).text_size(px(11.5)).child(PALETTE_SHORTCUT)),
            )
            .child(group(
                theme,
                vec![
                    any(segment(theme, Some("sidebar"), "", true)),
                    any(segment(theme, Some("panel"), "", false)),
                    any(segment(theme, Some("sliders"), "", false)),
                ],
            ))
    }
}
