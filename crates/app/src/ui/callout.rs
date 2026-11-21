//! The box under a long-running form (an export, an import, a dump) saying
//! how it goes or how it went: a spinner and the time so far while in
//! progress, then an icon and colour for done, warning, failed or stopped,
//! with a title and an optional detail. Its room is kept while there is
//! nothing to say ([`callout_space`]), so the form never jumps.

use crate::ui::Ellipsis;
use std::time::{Duration, Instant};

use gpui::{
    App, Context, Div, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use super::{icon, spinner};
use crate::theme::{ActiveTheme, MONO};

/// The room a callout takes at least.
const MIN_HEIGHT: f32 = 52.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Progress,
    Done,
    Warning,
    Failed,
    Stopped,
}

#[derive(IntoElement)]
pub struct Callout {
    id: ElementId,
    tone: Tone,
    title: SharedString,
    aside: Option<SharedString>,
    detail: Option<SharedString>,
    mono: bool,
    one_line: bool,
}

impl Callout {
    pub fn new(id: impl Into<ElementId>, tone: Tone, title: impl Into<SharedString>) -> Self {
        Self { id: id.into(), tone, title: title.into(), aside: None, detail: None, mono: false, one_line: false }
    }

    /// Shown at the end of the title, e.g. the time so far.
    pub fn aside(mut self, aside: impl Into<SharedString>) -> Self {
        self.aside = Some(aside.into());
        self
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn detail_opt(mut self, detail: Option<impl Into<SharedString>>) -> Self {
        self.detail = detail.map(Into::into);
        self
    }

    /// The detail is a tool's output or a path, in monospace.
    pub fn mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }

    /// The detail stays on one line, cut short: a line that keeps changing.
    pub fn one_line(mut self) -> Self {
        self.one_line = true;
        self
    }
}

fn frame() -> Div {
    div()
        .w_full()
        .min_h(px(MIN_HEIGHT))
        .flex()
        .items_start()
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .rounded(px(10.))
        .border_1()
}

impl RenderOnce for Callout {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let (border, background, mark) = match self.tone {
            Tone::Progress => (theme.line, theme.sel, None),
            Tone::Done => (theme.ok.opacity(0.35), theme.ok.opacity(0.09), Some(("check", theme.ok))),
            Tone::Warning => (theme.warn.opacity(0.45), theme.warn.opacity(0.1), Some(("warning", theme.warn))),
            Tone::Failed => (theme.err.opacity(0.4), theme.err.opacity(0.08), Some(("warning", theme.err))),
            Tone::Stopped => (theme.line, theme.sel, Some(("stop", theme.text2))),
        };
        let leading = match mark {
            Some((name, tint)) => div().mt(px(1.)).child(icon(name, tint, 15.)).into_any_element(),
            None => div().mt(px(1.)).child(spinner(self.id.clone(), theme.text3, 15.)).into_any_element(),
        };
        let detail = self.detail.map(|detail| {
            div()
                .id("callout-detail")
                .text_color(theme.text3)
                .when(self.mono, |detail| detail.font_family(MONO).text_size(px(12.)))
                .map(|text| {
                    if self.one_line {
                        text.ellipsis().child(detail)
                    } else {
                        text.max_h(px(120.)).overflow_y_scroll().child(detail)
                    }
                })
        });
        frame().border_color(border).bg(background).text_size(px(12.5)).text_color(theme.text2).child(leading).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .text_color(theme.text)
                        .font_weight(FontWeight::MEDIUM)
                        .child(div().min_w_0().child(self.title))
                        .children(self.aside.map(|aside| div().flex_none().text_color(theme.text3).child(aside))),
                )
                .children(detail),
        )
    }
}

/// The room a callout takes, kept while there is nothing to say.
pub fn callout_space() -> Div {
    frame().border_color(gpui::transparent_black())
}

/// Whole seconds since `started`, as `0:07` or `1:12`.
pub fn elapsed(started: Instant) -> String {
    format_elapsed(started.elapsed().as_secs())
}

pub fn format_elapsed(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Redraws `this` every half second while `running` says it still runs, so
/// an elapsed time counts up.
pub fn tick_while<T: 'static>(cx: &mut Context<T>, running: impl Fn(&T) -> bool + 'static) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let going = this.update(cx, |this, cx| {
                cx.notify();
                running(this)
            });
            if !matches!(going, Ok(true)) {
                break;
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_whole_minutes_and_seconds() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(7), "0:07");
        assert_eq!(format_elapsed(72), "1:12");
        assert_eq!(format_elapsed(3600), "60:00");
    }
}
