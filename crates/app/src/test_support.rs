//! What headless view tests share: a window's globals over a core in a
//! temporary directory (secrets in its encrypted file, never the keychain),
//! the event hub views listen to, and running the app until something holds.

use std::time::{Duration, Instant};

use gpui::{TestAppContext, VisualTestContext};

use crate::backend::test_backend;
use crate::events::{EventHub, channel};
use crate::i18n::{Language, Locale};
use crate::theme::{Appearance, Theme};

/// The globals views read, over a core in the returned directory.
pub fn setup(cx: &mut TestAppContext) -> tempfile::TempDir {
    cx.update(|cx| {
        cx.set_global(Theme::new(Appearance::Dark));
        cx.set_global(Locale(Language::En));
        let dir = test_backend(cx);
        crate::preferences::test_preferences(cx, dir.path());
        // The core of the test backend sends no events; tests emit through the hub themselves.
        let (_core_side, received) = channel();
        EventHub::start(received, cx);
        dir
    })
}

/// Draws the window once.
pub fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
}

/// Runs the app until `done` holds, the core's calls answering on their own threads.
pub fn wait_until(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let start = Instant::now();
    while !done(cx) {
        assert!(start.elapsed() < Duration::from_secs(15), "timed out waiting for {what}");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
}
