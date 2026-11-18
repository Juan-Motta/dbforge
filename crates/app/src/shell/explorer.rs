//! The explorer pane: the saved connections. For now their names, read-only;
//! the tree of databases, schemas and tables grows here.

use dbforge_core::connections::model::Connection;
use gpui::{Context, FontWeight, IntoElement, ParentElement, Render, Styled, Window, div, px};

use crate::backend::Backend;
use crate::i18n::Translate;
use crate::preferences::{Preferences, keys};
use crate::theme::ActiveTheme;
use crate::ui::{any, dot, glass, group, icon, segment};

/// The explorer's width when none was kept, and the range it may be dragged within.
const WIDTH: f32 = 280.;
const MIN_WIDTH: f32 = 200.;
const MAX_WIDTH: f32 = 560.;

pub struct Explorer {
    connections: Vec<Connection>,
}

impl Explorer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // The store keeps its list in memory: read at once, so the first frame already shows it.
        Self { connections: Backend::global(cx).core().connections().list() }
    }
}

/// The width kept under [`keys::EXPLORER_WIDTH`], within its range; the default when none was.
pub fn width(saved: Option<&str>) -> f32 {
    match saved.and_then(|saved| saved.parse::<f32>().ok()) {
        Some(width) if width > 0. => width.clamp(MIN_WIDTH, MAX_WIDTH),
        _ => WIDTH,
    }
}

impl Render for Explorer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let m = cx.messages();
        let rows = self.connections.iter().map(|connection| {
            div()
                .h(px(28.))
                .flex()
                .items_center()
                .gap(px(7.))
                .px(px(10.))
                .rounded(px(9.))
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text2)
                .child(icon("right", theme.text3, 12.))
                .child(dot(theme.accent_text))
                .child(div().truncate().child(connection.name.clone()))
        });
        let list = if self.connections.is_empty() {
            div().px(px(22.)).py(px(6.)).text_size(px(13.)).text_color(theme.text3).child(m.no_connections)
        } else {
            div().px(px(8.)).flex().flex_col().gap(px(1.)).children(rows)
        };

        glass(theme)
            .w(px(width(Preferences::global(cx).get(keys::EXPLORER_WIDTH))))
            .flex_none()
            .rounded(px(26.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                // The top row leaves the traffic lights their corner on macOS.
                div().h(px(52.)).flex().items_center().justify_end().px(px(12.)).child(group(
                    theme,
                    vec![any(segment(theme, Some("plus"), "", false)), any(segment(theme, Some("refresh"), "", false))],
                )),
            )
            .child(
                div().px(px(12.)).pb(px(10.)).child(
                    div()
                        .h(px(32.))
                        .rounded(px(16.))
                        .bg(theme.sel)
                        .border_1()
                        .border_color(theme.rim)
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(11.))
                        .text_size(px(13.))
                        .text_color(theme.text3)
                        .child(icon("search", theme.text3, 15.))
                        .child(m.filter_placeholder),
                ),
            )
            .child(
                div()
                    .px(px(22.))
                    .h(px(24.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text3)
                    .child(m.connections.to_uppercase()),
            )
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_saved_width_within_its_range() {
        assert_eq!(width(None), 280.);
        assert_eq!(width(Some("320")), 320.);
        assert_eq!(width(Some("90")), 200.);
        assert_eq!(width(Some("4000")), 560.);
        assert_eq!(width(Some("0")), 280.);
        assert_eq!(width(Some("wide")), 280.);
    }
}
