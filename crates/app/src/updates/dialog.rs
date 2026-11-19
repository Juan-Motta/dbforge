//! What a check found, and the update's download and install, as the web
//! UI's `UpdateDialog.tsx`: DBForge's mark with the headline, the release
//! notes, a box with the download's progress or what went wrong, and the
//! buttons for the phase. Enter presses the main button (Install and
//! Restart, Try Again, OK); Escape is Later, except while installing, which
//! nothing interrupts.

use std::ops::Range;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, FontWeight, HighlightStyle, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, StyledText, Subscription,
    Window, actions, div, prelude::FluentBuilder, px,
};

use super::notes::{Block, Run, read_notes};
use super::{Phase, Stage, Updates};
use crate::i18n::{Translate, UpdateMessages};
use crate::theme::{ActiveTheme, Theme};
use crate::ui::callout::{Callout, Tone};
use crate::ui::modal::{modal, modal_body, modal_footer};
use crate::ui::{button, enabled, icon, primary_button};

actions!(update_dialog, [Dismiss, Confirm]);

const CONTEXT: &str = "UpdateDialog";

/// The notes' box grows with them up to this height, then scrolls.
const NOTES_MAX_HEIGHT: f32 = 220.;

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("escape", Dismiss, Some(CONTEXT)), KeyBinding::new("enter", Confirm, Some(CONTEXT))]
}

pub struct UpdateDialog {
    updates: Entity<Updates>,
    focus: FocusHandle,
    _observe: Subscription,
}

impl Focusable for UpdateDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Megabytes, with one decimal, as the web UI wrote them.
pub fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024. * 1024.))
}

/// `runs` as one text, with the bold runs' ranges.
fn styled_runs(runs: &[Run]) -> (String, Vec<(Range<usize>, HighlightStyle)>) {
    let mut text = String::new();
    let mut bold = Vec::new();
    for run in runs {
        let start = text.len();
        text.push_str(&run.text);
        if run.bold {
            bold.push((
                start..text.len(),
                HighlightStyle { font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() },
            ));
        }
    }
    (text, bold)
}

impl UpdateDialog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let updates = Updates::global(cx);
        let observe = cx.observe(&updates, |_, _, cx| cx.notify());
        Self { updates, focus: cx.focus_handle(), _observe: observe }
    }

    /// Takes the keyboard, so Enter and Escape reach it.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.updates.update(cx, |updates, cx| updates.dismiss(cx));
    }

    /// The main button: install what is offered (or try again), or close an answer.
    fn confirm(&mut self, cx: &mut Context<Self>) {
        self.updates.update(cx, |updates, cx| match updates.phase() {
            Phase::Available(_) | Phase::Failed { update: Some(_), .. } => updates.install(cx),
            Phase::Failed { update: None, .. } => updates.check(true, cx),
            Phase::UpToDate => updates.dismiss(cx),
            Phase::Idle | Phase::Checking | Phase::Installing { .. } => {}
        });
    }

    fn render_product(&self, theme: &Theme, phase: &Phase, m: &UpdateMessages) -> impl IntoElement {
        let (headline, detail): (Option<String>, Option<String>) = match phase {
            Phase::Checking => (Some(m.checking.into()), None),
            Phase::UpToDate => (Some(m.up_to_date.into()), Some((m.up_to_date_detail)(dbforge_core::VERSION))),
            Phase::Available(update)
            | Phase::Installing { update, .. }
            | Phase::Failed { update: Some(update), .. } => {
                (Some((m.available)(&update.version)), Some((m.current)(&update.current_version)))
            }
            Phase::Failed { update: None, .. } => (Some(m.failed.into()), None),
            Phase::Idle => (None, None),
        };
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(
                div()
                    .size(px(44.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(11.))
                    .bg(theme.accent)
                    .child(icon("database", gpui::white(), 22.)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(headline.map(|headline| {
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(headline)
                    }))
                    .children(detail.map(|detail| div().text_size(px(12.5)).text_color(theme.text3).child(detail))),
            )
    }

    fn render_notes(&self, theme: &Theme, markdown: &str, m: &UpdateMessages) -> gpui::AnyElement {
        let blocks = read_notes(markdown);
        let body = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .when(blocks.is_empty(), |body| body.child(div().text_color(theme.text3).child(m.no_notes)));
        let body = body.children(blocks.into_iter().enumerate().map(|(index, block)| {
            match block {
                Block::Heading(text) => div()
                    .when(index > 0, |heading| heading.mt(px(8.)))
                    .mb(px(2.))
                    .text_color(theme.text)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text)
                    .into_any_element(),
                Block::Item(runs) => {
                    let (text, bold) = styled_runs(&runs);
                    div()
                        .flex()
                        .gap(px(7.))
                        .child(div().flex_none().text_color(theme.text3).child("•"))
                        .child(div().min_w_0().child(StyledText::new(SharedString::from(text)).with_highlights(bold)))
                        .into_any_element()
                }
                Block::Text(runs) => {
                    let (text, bold) = styled_runs(&runs);
                    div().child(StyledText::new(SharedString::from(text)).with_highlights(bold)).into_any_element()
                }
            }
        }));
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(theme.text2).child(m.notes))
            .child(
                div()
                    .id("update-notes")
                    .max_h(px(NOTES_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.line)
                    .bg(theme.glass)
                    .text_size(px(12.5))
                    .line_height(px(19.))
                    .text_color(theme.text2)
                    .child(body),
            )
            .into_any_element()
    }

    fn render_status(&self, phase: &Phase, m: &UpdateMessages) -> Option<Callout> {
        match phase {
            Phase::Installing { stage: Stage::Downloading { downloaded, total }, .. } => {
                let detail = match total {
                    Some(total) => (m.downloaded)(&megabytes(*downloaded), &megabytes(*total)),
                    None => (m.downloaded_so_far)(&megabytes(*downloaded)),
                };
                Some(Callout::new("update-status", Tone::Progress, m.downloading).detail(detail))
            }
            Phase::Installing { stage: Stage::Installing, .. } => {
                Some(Callout::new("update-status", Tone::Progress, m.installing))
            }
            Phase::Failed { error, .. } => {
                Some(Callout::new("update-status", Tone::Failed, m.failed).detail(error.clone()).mono(true))
            }
            _ => None,
        }
    }

    fn render_footer(
        &self,
        theme: &Theme,
        phase: &Phase,
        m: &UpdateMessages,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let later = |cx: &mut Context<Self>| {
            button(theme, None, m.later)
                .id("update-later")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx)))
        };
        let main = |label: &str, on: bool, cx: &mut Context<Self>| {
            enabled(primary_button(theme, label), on)
                .id("update-main")
                .when(on, |main| main.on_click(cx.listener(|this, _, _, cx| this.confirm(cx))))
        };
        let footer = modal_footer(theme);
        match phase {
            Phase::Available(_) => footer.child(later(cx)).child(main(m.install, true, cx)),
            Phase::Installing { .. } => footer.child(main(m.install, false, cx)),
            Phase::Failed { .. } => footer.child(later(cx)).child(main(m.retry, true, cx)),
            Phase::Checking | Phase::UpToDate | Phase::Idle => {
                footer.child(main(m.ok, matches!(phase, Phase::UpToDate), cx))
            }
        }
    }
}

impl Render for UpdateDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().updates;
        let phase = self.updates.read(cx).phase().clone();
        let notes = match &phase {
            Phase::Available(update)
            | Phase::Installing { update, .. }
            | Phase::Failed { update: Some(update), .. } => Some(update.notes.clone().unwrap_or_default()),
            _ => None,
        };
        modal(&theme, 520., m.title)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.dismiss(cx)))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.confirm(cx)))
            .child(
                modal_body("update-body")
                    .child(self.render_product(&theme, &phase, m))
                    .children(notes.map(|notes| self.render_notes(&theme, &notes, m)))
                    .children(self.render_status(&phase, m)),
            )
            .child(self.render_footer(&theme, &phase, m, cx))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::TestAppContext;

    use super::*;
    use crate::test_support::{draw, setup};
    use crate::updates::Update;
    use crate::updates::config::Settings;
    use crate::updates::target::Installer;

    #[test]
    fn writes_megabytes_with_one_decimal() {
        assert_eq!(megabytes(0), "0.0 MB");
        assert_eq!(megabytes(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
    }

    #[test]
    fn bolds_the_runs_the_notes_bolded() {
        let runs = [Run { text: "app:".into(), bold: true }, Run { text: " native".into(), bold: false }];
        let (text, bold) = styled_runs(&runs);
        assert_eq!(text, "app: native");
        assert_eq!(bold.len(), 1);
        assert_eq!(bold[0].0, 0..4);
    }

    #[gpui::test]
    fn draws_every_phase(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        cx.update(|cx| Updates::start_with(Settings::current(), false, cx));
        let update = Arc::new(Update {
            version: "0.4.0".into(),
            current_version: "0.3.5".into(),
            notes: Some("## Features\n\n- **app:** native window\nplain".into()),
            url: "http://127.0.0.1:9/x".into(),
            signature: "s".into(),
            installer: Installer::App,
        });
        let (dialog, cx) = cx.add_window_view(|_, cx| UpdateDialog::new(cx));
        let phases = [
            Phase::Checking,
            Phase::UpToDate,
            Phase::Available(update.clone()),
            Phase::Installing {
                update: update.clone(),
                stage: Stage::Downloading { downloaded: 1024, total: Some(4096) },
            },
            Phase::Installing { update: update.clone(), stage: Stage::Downloading { downloaded: 1024, total: None } },
            Phase::Installing { update: update.clone(), stage: Stage::Installing },
            Phase::Failed { error: "offline".into(), update: None },
            Phase::Failed { error: "bad signature".into(), update: Some(update.clone()) },
        ];
        for phase in phases {
            dialog.update(cx, |dialog, cx| {
                dialog.updates.update(cx, |updates, cx| updates.show_for_development(phase, cx));
            });
            draw(cx);
        }
        // OK closes an answer.
        dialog.update(cx, |dialog, cx| {
            dialog.updates.update(cx, |updates, cx| updates.show_for_development(Phase::UpToDate, cx));
            dialog.confirm(cx);
            assert_eq!(*dialog.updates.read(cx).phase(), Phase::Idle);
        });
    }
}
