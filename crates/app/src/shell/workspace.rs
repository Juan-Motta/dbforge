//! The tab strip and the main area under it. With no tab open, the main area
//! says how to open one.

use gpui::{Context, FontWeight, IntoElement, ParentElement, Render, Styled, Window, div, px};

use crate::i18n::Translate;
use crate::theme::ActiveTheme;
use crate::ui::{any, card, group, segment};

pub struct Workspace;

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let m = cx.messages();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .child(
                div()
                    .h(px(40.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(group(theme, vec![any(segment(theme, Some("plus"), m.new_query, false))])),
            )
            .child(
                card(theme).flex().items_center().justify_center().child(
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_center()
                        .px(px(24.))
                        .text_size(px(13.5))
                        .text_color(theme.text3)
                        .child(m.empty_before)
                        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme.text2).child(m.new_query))
                        .child(m.empty_after),
                ),
            )
    }
}

const GAP: f32 = super::GAP;
