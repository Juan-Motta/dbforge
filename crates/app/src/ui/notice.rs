//! A short line at the bottom of the window that goes away by itself, as
//! the former web UI's notices: "Copied to the clipboard." after a copy, or
//! why something failed (in the error colour). A newer notice replaces the
//! one shown and restarts the wait. It floats over the window, so nothing
//! moves, and it takes no clicks.

use std::time::Duration;

use gpui::{App, AppContext, ClipboardItem, Entity, Global, IntoElement, ParentElement, SharedString, Styled, div, px};

use super::shadow;
use crate::i18n::Translate;
use crate::theme::Theme;

/// How long a notice stays.
pub const SHOWN_FOR: Duration = Duration::from_millis(2500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Changes with every notice, so the same text shown twice still restarts the wait.
    pub id: u64,
    pub text: SharedString,
    pub tone: Tone,
}

/// The notice shown, if any.
#[derive(Debug, Default)]
pub struct Notices {
    current: Option<Notice>,
    shown: u64,
}

impl Notices {
    /// Shows `text` in place of the notice shown; its id.
    pub fn show(&mut self, text: impl Into<SharedString>, tone: Tone) -> u64 {
        self.shown += 1;
        self.current = Some(Notice { id: self.shown, text: text.into(), tone });
        self.shown
    }

    /// Hides notice `id` if it is still the one shown; whether it was.
    pub fn expire(&mut self, id: u64) -> bool {
        let shown = self.current.as_ref().is_some_and(|notice| notice.id == id);
        if shown {
            self.current = None;
        }
        shown
    }

    pub fn current(&self) -> Option<&Notice> {
        self.current.as_ref()
    }

    /// The window's notices, created the first time they are asked for.
    pub fn global(cx: &mut App) -> Entity<Notices> {
        if let Some(global) = cx.try_global::<GlobalNotices>() {
            return global.0.clone();
        }
        let notices = cx.new(|_| Notices::default());
        cx.set_global(GlobalNotices(notices.clone()));
        notices
    }
}

struct GlobalNotices(Entity<Notices>);

impl Global for GlobalNotices {}

/// Shows `text` at the bottom of the window for a moment.
pub fn notify(text: impl Into<SharedString>, tone: Tone, cx: &mut App) {
    let notices = Notices::global(cx);
    let id = notices.update(cx, |notices, cx| {
        cx.notify();
        notices.show(text, tone)
    });
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SHOWN_FOR).await;
        let _ = notices.update(cx, |notices, cx| {
            if notices.expire(id) {
                cx.notify();
            }
        });
    })
    .detach();
}

/// Puts `text` on the clipboard and says so.
pub fn copy_to_clipboard(text: impl Into<String>, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
    notify(cx.messages().shell.copied, Tone::Info, cx);
}

/// `notice` floating near the bottom of the window, centred. Render it as
/// the window's last child, over everything; it never takes a click.
pub fn notice_layer(theme: &Theme, notice: &Notice) -> impl IntoElement {
    let tint = if notice.tone == Tone::Error { theme.err } else { theme.text };
    let border = if notice.tone == Tone::Error { theme.err } else { theme.rim };
    div().absolute().left_0().right_0().bottom(px(20.)).px(px(16.)).flex().justify_center().child(
        div()
            .max_w(px(560.))
            .px(px(14.))
            .py(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .bg(theme.dialog)
            .shadow(shadow(24., 10., 0x00000038))
            .text_size(px(12.5))
            .text_color(tint)
            .child(notice.text.clone()),
    )
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[test]
    fn a_newer_notice_replaces_the_shown_one_and_only_the_shown_one_expires() {
        let mut notices = Notices::default();
        let first = notices.show("Copied to the clipboard.", Tone::Info);
        let second = notices.show("Copied to the clipboard.", Tone::Info);
        assert_ne!(first, second, "the same text twice is a new notice");
        assert!(!notices.expire(first), "the replaced notice's wait ends without hiding the new one");
        assert_eq!(notices.current().map(|notice| notice.id), Some(second));
        assert!(notices.expire(second));
        assert_eq!(notices.current(), None);
    }

    #[gpui::test]
    fn a_copy_puts_the_text_on_the_clipboard_says_so_and_the_notice_goes_away(cx: &mut TestAppContext) {
        let _dir = crate::test_support::setup(cx);
        cx.update(|cx| copy_to_clipboard("SELECT 1", cx));
        let read = |cx: &mut TestAppContext| {
            cx.update(|cx| Notices::global(cx).read(cx).current().map(|notice| (notice.text.to_string(), notice.tone)))
        };
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("SELECT 1"));
        assert_eq!(read(cx), Some(("Copied to the clipboard.".to_owned(), Tone::Info)));
        cx.executor().advance_clock(SHOWN_FOR - Duration::from_millis(1));
        assert!(read(cx).is_some(), "it stays a moment");
        cx.executor().advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(read(cx), None);
    }
}
