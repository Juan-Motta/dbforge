//! The Redis key browser: a database's keys found with `SCAN` (as a `:`
//! tree or a flat list), the selected key's value a page at a time with
//! its edits, the server's metrics and a redis-cli panel. A port of the former web
//! UI's `features/keyspace`.

mod browser;
pub mod cli;
mod cli_panel;
mod detail;
pub mod dialog;
pub mod format;
pub mod keys;
mod list;
pub mod msgpack;
#[cfg(test)]
mod tests;

use gpui::{Div, FontWeight, Hsla, ParentElement, Styled, div, px};

pub use browser::{KeyBrowser, KeyBrowserEvent};
pub use cli_panel::{CliEvent, CliPanel};
pub use detail::{KeyDetail, KeyDetailEvent};
pub use list::{KeyAction, KeyList, KeyListEvent, KeyView};

use crate::theme::{MONO, Theme};

/// The keys the browser's parts answer.
pub fn key_bindings() -> Vec<gpui::KeyBinding> {
    let mut bindings = dialog::key_bindings();
    bindings.extend(cli_panel::key_bindings());
    bindings
}

/// A type's colours, as the former web UI's `kind-*` classes: its text and its
/// badge's tint; module types share a neutral one.
fn kind_tint(theme: &Theme, kind: &str) -> (Hsla, Hsla) {
    match kind {
        "string" => (theme.str, theme.str),
        "hash" => (theme.kw, theme.kw),
        "list" => (theme.num, theme.num),
        "set" => (theme.func, theme.func),
        "zset" => (theme.accent_text, theme.accent),
        "stream" => (theme.accent_hi, theme.accent_hi),
        _ => (theme.text2, theme.text3),
    }
}

/// A key's type as a small badge in its colour: `HASH`, `ZSET`…
pub fn kind_badge(theme: &Theme, kind: &str, label: &str) -> Div {
    let (text, tint) = kind_tint(theme, kind);
    div()
        .h(px(18.))
        .px(px(6.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(tint.opacity(0.18))
        .text_color(text)
        .font_family(MONO)
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .child(label.to_owned())
}

/// A fact about a key in the detail's header: its TTL, length, size, encoding.
fn badge(theme: &Theme, tint: Option<Hsla>) -> Div {
    let tint = tint.unwrap_or(theme.text2);
    div()
        .h(px(22.))
        .px(px(9.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .rounded(px(11.))
        .bg(tint.opacity(0.14))
        .text_color(tint)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
}
