//! One line of text that ends in "…" when it doesn't fit.
//!
//! GPUI's `truncate()` keeps the text on one line with `nowrap`, and a text
//! element without wrapping keeps the size it measured first. Inside a flex
//! row that first measure has no width yet, so the whole text is kept, the
//! container clips it, and no "…" is drawn. Wrapping text is measured again
//! once its width is known, so `ellipsis()` lets it wrap, keeps one line and
//! cuts it there with "…".

use gpui::Styled;

/// The single-line, "…"-ended text style, for any styled element.
pub trait Ellipsis: Styled + Sized {
    /// Keeps the text to one line, ending in "…" when it doesn't fit.
    fn ellipsis(self) -> Self {
        self.overflow_hidden().line_clamp(1).text_ellipsis()
    }
}

impl<E: Styled> Ellipsis for E {}

#[cfg(test)]
mod tests {
    use gpui::{
        AvailableSpace, IntoElement, ParentElement, StyledText, TestAppContext, VisualTestContext, div, point, px, size,
    };

    use super::*;

    #[gpui::test]
    fn cuts_text_too_long_for_a_flex_row_with_an_ellipsis(cx: &mut TestAppContext) {
        let window = cx.add_empty_window();
        let cx: &mut VisualTestContext = window;
        let text = StyledText::new(
            "SELECT a.attname::text, quote_ident(a.attname), format_type(a.atttypid) FROM pg_attribute a",
        );
        let layout = text.layout().clone();
        cx.draw(point(px(0.), px(0.)), size(AvailableSpace::Definite(px(200.)), AvailableSpace::MinContent), |_, _| {
            div()
                .w(px(200.))
                .flex()
                .child(div().w(px(40.)).flex_none())
                .child(div().flex_1().min_w_0().ellipsis().child(text))
                .into_any_element()
        });
        let shown = layout.text();
        assert!(shown.ends_with('…'), "{shown:?}");
        assert!(shown.len() < 60, "{shown:?}");
    }

    #[gpui::test]
    fn keeps_text_that_fits_whole(cx: &mut TestAppContext) {
        let window = cx.add_empty_window();
        let cx: &mut VisualTestContext = window;
        let text = StyledText::new("SELECT 1");
        let layout = text.layout().clone();
        cx.draw(point(px(0.), px(0.)), size(AvailableSpace::Definite(px(200.)), AvailableSpace::MinContent), |_, _| {
            div().w(px(200.)).flex().child(div().flex_1().min_w_0().ellipsis().child(text)).into_any_element()
        });
        assert_eq!(layout.text(), "SELECT 1");
    }
}
