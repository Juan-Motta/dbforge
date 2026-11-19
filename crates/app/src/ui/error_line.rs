//! A failure on one line, cut with an ellipsis so the layout never moves,
//! as the former web UI's error line: pressing it opens all of it above the
//! line, to read and copy; Escape, a click outside or pressing the line
//! again closes it. A new failure starts closed.

use gpui::{
    App, ClipboardItem, Context, Corner, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, actions, anchored,
    deferred, div, point, px,
};

use super::field::Tooltip;
use super::{glass, icon};
use crate::i18n::Translate;
use crate::theme::ActiveTheme;

actions!(error_line, [Dismiss]);

const CONTEXT: &str = "ErrorLine";

/// The opened text's width, kept inside the window.
const WIDTH: f32 = 460.;
const MARGIN: f32 = 8.;
/// The line's height, whatever its text.
const LINE_HEIGHT: f32 = 18.;
/// Space between the line and the text opened above it.
const GAP: f32 = 10.;

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

pub struct ErrorLine {
    text: SharedString,
    open: bool,
    /// Copy was pressed since it opened: its icon shows a check.
    copied: bool,
    focus: FocusHandle,
}

impl Focusable for ErrorLine {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ErrorLine {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { text: SharedString::default(), open: false, copied: false, focus: cx.focus_handle() }
    }

    /// Shows `text`; a different failure starts closed.
    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        if self.text != text {
            self.text = text;
            self.open = false;
            self.copied = false;
            cx.notify();
        }
    }

    pub fn text(&self) -> &SharedString {
        &self.text
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the whole text above the line, or closes it; open, it takes the keyboard.
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = !self.open;
        self.copied = false;
        if self.open {
            window.focus(&self.focus);
        }
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }

    /// Puts the whole text on the clipboard.
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.text.to_string()));
        self.copied = true;
        cx.notify();
    }
}

impl Render for ErrorLine {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().shell;
        div()
            .relative()
            .w_full()
            .min_w_0()
            .child(
                div()
                    .id("error-line")
                    .w_full()
                    .h(px(LINE_HEIGHT))
                    .text_color(theme.err)
                    .cursor_pointer()
                    .hover(|line| line.underline())
                    .tooltip(Tooltip::text(m.show_error))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.toggle(window, cx)))
                    // One wrapped line cut with an ellipsis: GPUI lays a wrapping text out
                    // again when its width changes, a `truncate()`d one only once.
                    .line_height(px(LINE_HEIGHT))
                    .overflow_hidden()
                    .text_ellipsis()
                    .line_clamp(1)
                    .child(self.text.clone()),
            )
            // From the line's top-left corner, above it; the line never moves.
            .children(self.open.then(|| {
                div().absolute().top_0().left_0().child(
                    deferred(
                        anchored()
                            .anchor(Corner::BottomLeft)
                            .offset(point(px(0.), px(-GAP)))
                            .snap_to_window_with_margin(px(MARGIN))
                            .child(
                                glass(&theme)
                                    .bg(theme.card)
                                    .track_focus(&self.focus)
                                    .key_context(CONTEXT)
                                    .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.close(cx)))
                                    // After the press is handled, so pressing the line again closes it.
                                    .on_mouse_down_out(
                                        cx.listener(|_, _, window, cx| {
                                            cx.defer_in(window, |this, _, cx| this.close(cx))
                                        }),
                                    )
                                    .w(px(WIDTH))
                                    .rounded(px(10.))
                                    .px(px(14.))
                                    .py(px(12.))
                                    .flex()
                                    .flex_col()
                                    .items_start()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .id("error-line-text")
                                            .w_full()
                                            .max_h(px(180.))
                                            .overflow_y_scroll()
                                            .text_size(px(12.))
                                            .line_height(px(18.))
                                            .text_color(theme.err)
                                            .child(self.text.clone()),
                                    )
                                    .child(
                                        div()
                                            .id("error-line-copy")
                                            .h(px(26.))
                                            .px(px(8.))
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .rounded(px(6.))
                                            .border_1()
                                            .border_color(theme.line)
                                            .text_size(px(12.))
                                            .text_color(theme.text2)
                                            .cursor_pointer()
                                            .hover(|button| button.text_color(theme.text))
                                            .on_click(cx.listener(|this, _, _, cx| this.copy(cx)))
                                            .child(icon(
                                                if self.copied { "check" } else { "duplicate" },
                                                theme.text2,
                                                12.,
                                            ))
                                            .child(m.copy),
                                    ),
                            ),
                    )
                    .with_priority(2),
                )
            }))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext, Entity, TestAppContext};

    use super::*;

    struct Host {
        line: Entity<ErrorLine>,
    }

    impl Render for Host {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().flex().items_end().p(px(40.)).child(div().w(px(200.)).child(self.line.clone()))
        }
    }

    #[gpui::test]
    fn opens_the_whole_error_copies_it_and_a_new_failure_starts_closed(cx: &mut TestAppContext) {
        let _dir = crate::test_support::setup(cx);
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let error = "connection refused: is the server running on db.internal (10.0.0.7) and accepting TCP/IP \
                     connections on port 5432?";
        let (host, cx) = cx.add_window_view(|_, cx| Host { line: cx.new(ErrorLine::new) });
        let line = host.read_with(cx, |host, _| host.line.clone());
        line.update(cx, |line, cx| line.set_text(error, cx));
        crate::test_support::draw(cx);

        line.update_in(cx, |line, window, cx| line.toggle(window, cx));
        crate::test_support::draw(cx);
        assert!(line.read_with(cx, |line, _| line.is_open()));
        line.update(cx, |line, cx| line.copy(cx));
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some(error), "all of it");

        cx.simulate_keystrokes("escape");
        assert!(!line.read_with(cx, |line, _| line.is_open()), "Escape closes it");

        line.update_in(cx, |line, window, cx| line.toggle(window, cx));
        line.update(cx, |line, cx| line.set_text("timed out", cx));
        assert!(!line.read_with(cx, |line, _| line.is_open()), "another failure starts closed");
        crate::test_support::draw(cx);
    }
}
