//! An explained statement's plan: its totals on top (or that it is only an
//! estimate), the nodes as an indented tree with their rows, estimate, own
//! time or cost and share of the whole, and the selected node's fields
//! beside them. It starts on the costliest node. The plan is kept as the
//! server's JSON and read in the interface language each time it is drawn.

use std::rc::Rc;

use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px, relative,
};
use serde_json::Value;

use super::plan::{Plan, PlanNode, format_count, format_ms, parse_plan};
use crate::i18n::{PlanMessages, Translate};
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::icon;
use crate::ui::sheet::sheet;

const ROW_HEIGHT: f32 = 32.;
const HEADER_HEIGHT: f32 = 28.;
const INDENT: f32 = 18.;
/// The numbers' columns and the share's.
const NUMBER_WIDTH: f32 = 80.;
const SHARE_WIDTH: f32 = 120.;
const NAME_MIN: f32 = 220.;
const GAP: f32 = 12.;
/// Narrow windows scroll the tree sideways instead of squeezing the names.
const MIN_WIDTH: f32 = 640.;
const DETAILS_WIDTH: f32 = 300.;

pub struct PlanView {
    json: Rc<Value>,
    /// The node shown beside the tree.
    selected: usize,
}

impl PlanView {
    /// A plan as the core returned it; it was read once already, so it is readable.
    pub fn new(json: Rc<Value>, cx: &mut Context<Self>) -> Self {
        let selected = parse_plan(&json, &cx.messages().query.plan).map(|plan| plan.costliest()).unwrap_or_default();
        Self { json, selected }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        cx.notify();
    }

    fn stats(plan: &Plan, m: &PlanMessages, theme: &Theme) -> AnyElement {
        let stat = |label: &'static str, value: String, warn: bool| {
            div()
                .flex_1()
                .flex_basis(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .px(px(14.))
                .py(px(8.))
                .border_r_1()
                .border_color(theme.line)
                .when(warn, |stat| stat.bg(theme.warn.opacity(0.08)))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if warn { theme.warn } else { theme.text3 })
                        .child(label.to_uppercase()),
                )
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(value))
        };
        let mut stats = Vec::new();
        if plan.analyzed {
            if let Some(ms) = plan.execution_ms {
                stats.push(stat(m.execution, format_ms(ms), false));
            }
            // MySQL doesn't report planning time.
            if let Some(ms) = plan.planning_ms {
                stats.push(stat(m.planning, format_ms(ms), false));
            }
            stats.push(stat(m.rows_returned, format_count(plan.rows.unwrap_or(0.)), false));
        } else {
            stats.push(stat(m.estimate_only, m.turn_on_analyze.to_owned(), false));
        }
        let warnings = plan.warning_count();
        let value = if warnings == 0 { m.none.to_owned() } else { warnings.to_string() };
        stats.push(stat(m.warnings, value, warnings > 0));
        div().flex_none().flex().border_b_1().border_color(theme.line).children(stats).into_any_element()
    }

    fn row(&self, index: usize, node: &PlanNode, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let numeric = |text: String| {
            div().w(px(NUMBER_WIDTH)).flex_none().flex().justify_end().font_family(MONO).text_size(px(12.)).child(text)
        };
        let level = share_level(node.share, theme);
        let selected = index == self.selected;
        div()
            .id(("plan-node", index))
            .h(px(ROW_HEIGHT))
            .px(px(14.))
            .flex()
            .items_center()
            .gap(px(GAP))
            .border_b_1()
            .border_color(theme.line)
            .text_size(px(12.5))
            .cursor_pointer()
            .when(selected, |row| row.bg(theme.accent.opacity(0.16)))
            .when(!selected, |row| row.hover(|row| row.bg(theme.sel)))
            .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(NAME_MIN))
                    .pl(px(node.depth as f32 * INDENT))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .overflow_hidden()
                    .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).child(node.node_type.clone()))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(theme.text2)
                            .child(node.detail.clone()),
                    )
                    .when(!node.warnings.is_empty(), |name| name.child(icon("warning", theme.warn, 13.))),
            )
            .child(numeric(node.actual_rows.map(format_count).unwrap_or_default()))
            .child(numeric(node.estimated_rows.map(format_count).unwrap_or_else(|| "—".into())))
            .child(numeric(match node.self_ms {
                Some(ms) => format_ms(ms),
                None => format!("{:.1}", node.self_cost),
            }))
            .child(
                div()
                    .w(px(SHARE_WIDTH))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .h(px(6.))
                            .rounded(px(3.))
                            .bg(theme.sel)
                            .overflow_hidden()
                            .child(div().h_full().w(relative(node.share as f32 / 100.)).rounded(px(3.)).bg(level)),
                    )
                    .child(
                        div()
                            .w(px(34.))
                            .flex()
                            .justify_end()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .text_color(theme.text3)
                            .child(format!("{}%", node.share)),
                    ),
            )
            .into_any_element()
    }

    fn header(plan: &Plan, m: &PlanMessages, theme: &Theme) -> AnyElement {
        let numeric =
            |text: &'static str| div().w(px(NUMBER_WIDTH)).flex_none().flex().justify_end().child(text.to_uppercase());
        div()
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .px(px(14.))
            .flex()
            .items_center()
            .gap(px(GAP))
            .bg(theme.glass)
            .border_b_1()
            .border_color(theme.line)
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.text3)
            .child(div().flex_1().min_w(px(NAME_MIN)).child(m.node.to_uppercase()))
            .child(numeric(if plan.analyzed { m.rows_column } else { "" }))
            .child(numeric(m.estimated_rows))
            .child(numeric(if plan.analyzed { m.self_time } else { m.self_cost }))
            .child(div().w(px(SHARE_WIDTH)).flex_none().child(m.share.to_uppercase()))
            .into_any_element()
    }

    fn details(node: &PlanNode, theme: &Theme) -> AnyElement {
        div()
            .id("plan-details")
            .w(px(DETAILS_WIDTH))
            .flex_none()
            .overflow_y_scroll()
            .px(px(14.))
            .py(px(12.))
            .border_l_1()
            .border_color(theme.line)
            .bg(theme.glass)
            .child(div().mb(px(10.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(node.node_type.clone()))
            .children(node.warnings.iter().map(|warning| {
                div()
                    .mb(px(10.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.warn.opacity(0.35))
                    .bg(theme.warn.opacity(0.08))
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .child(warning.clone())
            }))
            .children(node.fields.iter().map(|(key, value)| {
                div()
                    .py(px(5.))
                    .flex()
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(theme.line)
                    .text_size(px(12.))
                    .child(
                        div().w(relative(0.45)).flex_none().min_w(px(80.)).text_color(theme.text3).child(key.clone()),
                    )
                    .child(div().flex_1().min_w_0().font_family(MONO).child(value.clone()))
            }))
            .into_any_element()
    }
}

impl Render for PlanView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().query.plan;
        let plan = match parse_plan(&self.json, m) {
            Ok(plan) => plan,
            Err(error) => return div().p(px(16.)).text_color(theme.err).child(error).into_any_element(),
        };
        let selected = self.selected.min(plan.nodes.len().saturating_sub(1));
        let rows: Vec<AnyElement> =
            plan.nodes.iter().enumerate().map(|(index, node)| self.row(index, node, &theme, cx)).collect();
        let tree = div().flex_1().min_w_0().child(sheet(
            "plan-nodes",
            px(MIN_WIDTH),
            Self::header(&plan, m, &theme),
            div().children(rows),
        ));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(Self::stats(&plan, m, &theme))
            .child(div().flex_1().min_h_0().flex().child(tree).child(Self::details(&plan.nodes[selected], &theme)))
            .into_any_element()
    }
}

/// A share bar's colour: green for small ones, amber for notable ones, red for the bulk.
fn share_level(share: u32, theme: &Theme) -> Hsla {
    if share > 40 {
        theme.err
    } else if share > 10 {
        theme.warn
    } else {
        theme.ok
    }
}
