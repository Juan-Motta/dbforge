//! The toolbar above the tabs: the connection in use, search everywhere and
//! the panel toggles. Static for now; the features behind each come later.

use gpui::{Context, FontWeight, IntoElement, ParentElement, Render, Styled, Window, div, px};

use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::{any, capsule, group, icon, segment};

/// The command palette's shortcut, as each OS writes it.
const PALETTE_SHORTCUT: &str = if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl+K" };

pub struct Toolbar;

impl Render for Toolbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let m = cx.messages();
        div()
            .h(px(42.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                capsule(theme)
                    .text_color(theme.text3)
                    .font_weight(FontWeight::MEDIUM)
                    .child(icon("database", theme.text3, 14.))
                    .child(m.no_connection)
                    .child(icon("down", theme.text3, 12.)),
            )
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
