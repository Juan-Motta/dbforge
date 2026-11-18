//! A console's last run: a tab per statement, the last one shown first, each
//! with its row count and how long the run took. A script that stopped
//! partway shows its error last, after the statements that ran (and took
//! effect) before it. While the next run goes, the last one stays, dimmed,
//! with the time running in place of the timing, so nothing jumps.

use std::rc::Rc;
use std::time::Instant;

use dbforge_core::drivers::{QueryOutcome, StatementResult};
use gpui::{
    AnyElement, AppContext, ClipboardItem, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use super::shortcuts;
use crate::grid::DataGrid;
use crate::history::format::format_duration;
use crate::i18n::{QueryMessages, Translate};
use crate::theme::{ActiveTheme, MONO};
use crate::ui::{any, group, icon, pill, segment, spinner};

/// The results bar's height.
const BAR_HEIGHT: f32 = 44.;

/// A finished run: what its statements returned, or why it failed before any ran.
#[derive(Debug, Clone, PartialEq)]
pub enum LastRun {
    Outcome(QueryOutcome),
    Failed(String),
}

pub struct ResultsPanel {
    last: Option<LastRun>,
    /// The tab shown: a statement's result, or (one past them) the error.
    selected: usize,
    grid: Entity<DataGrid>,
    /// When the run going now started.
    running: Option<Instant>,
    copied: bool,
}

/// One statement's row count, as its tab's badge says it.
pub fn summary(result: &StatementResult, m: &QueryMessages) -> String {
    if result.columns.is_empty() {
        (m.rows_affected)(result.rows_affected)
    } else if result.truncated {
        (m.rows_showing_first)(result.rows_affected, result.rows.len())
    } else {
        (m.rows)(result.rows_affected)
    }
}

impl ResultsPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { last: None, selected: 0, grid: cx.new(DataGrid::new), running: None, copied: false }
    }

    pub fn grid(&self) -> &Entity<DataGrid> {
        &self.grid
    }

    pub fn last(&self) -> Option<&LastRun> {
        self.last.as_ref()
    }

    /// A run started at `started`, or none is going.
    pub fn set_running(&mut self, started: Option<Instant>, cx: &mut Context<Self>) {
        self.running = started;
        cx.notify();
    }

    /// Shows a finished run, its last tab first.
    pub fn show(&mut self, run: LastRun, cx: &mut Context<Self>) {
        self.selected = match &run {
            LastRun::Outcome(outcome) => {
                (outcome.results.len() + usize::from(outcome.error.is_some())).saturating_sub(1)
            }
            LastRun::Failed(_) => 0,
        };
        self.last = Some(run);
        self.running = None;
        self.copied = false;
        self.load_grid(cx);
        cx.notify();
    }

    /// Shows tab `index`: a statement's result, or one past them the error.
    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.copied = false;
        self.load_grid(cx);
        cx.notify();
    }

    /// Puts the shown result's rows in the grid.
    fn load_grid(&mut self, cx: &mut Context<Self>) {
        let Some(LastRun::Outcome(outcome)) = &self.last else { return };
        let Some(result) = outcome.results.get(self.selected) else { return };
        let (columns, rows) = (result.columns.clone(), Rc::new(result.rows.clone()));
        self.grid.update(cx, |grid, cx| grid.set_data(columns, rows, cx));
    }

    fn bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().query;
        let mut bar = div()
            .h(px(BAR_HEIGHT))
            .flex_none()
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(theme.line)
            .text_size(px(12.));
        if let Some(LastRun::Outcome(outcome)) = &self.last {
            let tabs = outcome.results.len() + usize::from(outcome.error.is_some());
            if tabs > 1 {
                let mut segments: Vec<AnyElement> = (0..outcome.results.len())
                    .map(|index| {
                        any(segment(&theme, None, &(m.result)(index + 1), index == self.selected)
                            .id(("result-tab", index))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx))))
                    })
                    .collect();
                if outcome.error.is_some() {
                    let index = outcome.results.len();
                    segments.push(any(segment(&theme, None, m.stopped_tab, index == self.selected)
                        .id("result-error-tab")
                        .text_color(theme.err)
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx)))));
                }
                bar = bar.child(group(&theme, segments).flex_shrink().min_w_0().overflow_hidden());
            }
            if let Some(result) = outcome.results.get(self.selected) {
                bar = bar.child(pill(&summary(result, m), theme.ok).flex_none());
            }
        }
        // The timing keeps its room, whether a run is going or not.
        let timing: SharedString = match (self.running, &self.last) {
            (Some(started), _) => {
                format!("{} {}", m.running, format_duration(started.elapsed().as_millis() as u64)).into()
            }
            (None, Some(LastRun::Outcome(outcome))) => format!("{} ms", outcome.elapsed_ms).into(),
            _ => "".into(),
        };
        bar.child(
            div()
                .min_w(px(110.))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(theme.text3)
                .font_family(MONO)
                .text_size(px(11.5))
                .when(self.running.is_some(), |timing| timing.child(spinner("run-spinner", theme.text3, 12.)))
                .child(timing),
        )
        .into_any_element()
    }

    /// A failure in full, wrapped, with a button that copies it.
    fn error(&self, heading: Option<String>, text: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let copy = text.to_owned();
        div()
            .id("run-error")
            .size_full()
            .overflow_y_scroll()
            .p(px(16.))
            .flex()
            .gap(px(10.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.5))
                    .line_height(px(19.))
                    .text_color(theme.err)
                    .children(heading.map(|heading| div().text_color(theme.text2).mb(px(4.)).child(heading)))
                    .child(text.to_owned()),
            )
            .child(
                div()
                    .id("copy-error")
                    .flex_none()
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(13.))
                    .text_color(theme.text2)
                    .cursor_pointer()
                    .hover(|button| button.bg(theme.sel))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                        this.copied = true;
                        cx.notify();
                    }))
                    .child(if self.copied { icon("check", theme.ok, 13.) } else { icon("copy", theme.text2, 13.) }),
            )
            .into_any_element()
    }

    fn placeholder(text: String, cx: &Context<Self>) -> AnyElement {
        div().p(px(16.)).text_size(px(12.5)).text_color(cx.theme().text3).child(text).into_any_element()
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let m = &cx.messages().query;
        match &self.last {
            None => {
                let text = if self.running.is_some() { m.running.into() } else { (m.placeholder)(shortcuts::RUN) };
                Self::placeholder(text, cx)
            }
            Some(LastRun::Failed(error)) => self.error(None, error, cx),
            Some(LastRun::Outcome(outcome)) => match outcome.results.get(self.selected) {
                Some(result) if !result.columns.is_empty() => self.grid.clone().into_any_element(),
                Some(_) => Self::placeholder(m.statement_executed.into(), cx),
                None => match &outcome.error {
                    Some(error) => {
                        let heading = (m.stopped_after)(outcome.results.len());
                        self.error(Some(heading), error, cx)
                    }
                    None => Self::placeholder(m.no_statements.into(), cx),
                },
            },
        }
    }
}

impl Render for ResultsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.running.is_some() && self.last.is_some();
        div()
            .size_full()
            .flex()
            .flex_col()
            .font_weight(FontWeight::NORMAL)
            .child(self.bar(cx))
            .child(div().flex_1().min_h_0().when(busy, |body| body.opacity(0.6)).child(self.body(cx)))
    }
}
