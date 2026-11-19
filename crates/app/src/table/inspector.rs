//! A side panel with the picked cell's whole value, in the editor its
//! column's type calls for: a list of values (a boolean, an enum), date and
//! time controls, or text, shown as text, as indented JSON or as rendered
//! Markdown. Save puts the value in the cell as a pending edit; ⌘S (Ctrl+S)
//! does too, and a second one applies every change. Its left edge resizes
//! it; the width is kept under `dbforge.inspectorWidth`, as the former web UI kept it.

use dbforge_core::drivers::Column;
use gpui::{
    AnyElement, App, AppContext, BorrowAppContext, Context, DragMoveEvent, Entity, EventEmitter, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};

use super::Apply;
use super::cell_text::{compact_json, is_json_document, json_error, pretty_json};
use super::choices::choices_of;
use super::editors::temporal::{TemporalEvent, TemporalFields};
use super::markdown::{Block, blocks};
use super::temporal::{TemporalKind, offset_space, temporal_kind_of};
use crate::i18n::{TableMessages, Translate, thousands};
use crate::preferences::{Preferences, keys};
use crate::theme::{ActiveTheme, MONO};
use crate::ui::field::Tooltip;
use crate::ui::{
    CodeEditor, CodeEditorEvent, Select, SelectEvent, SelectOption, button, enabled, icon, primary_button,
};

/// The panel's width when none was kept, and the range it may be dragged within.
const WIDTH: f32 = 380.;
const MIN_WIDTH: f32 = 260.;
const MAX_WIDTH: f32 = 900.;

/// The width kept under [`keys::INSPECTOR_WIDTH`], within its range; the default when none was.
pub fn width(saved: Option<&str>) -> f32 {
    match saved.and_then(|saved| saved.parse::<f32>().ok()) {
        Some(width) if width > 0. => width.clamp(MIN_WIDTH, MAX_WIDTH),
        _ => WIDTH,
    }
}

/// The picked cell as the panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectedCell {
    pub column: Column,
    pub value: Option<String>,
    /// A new row's column left to its DEFAULT.
    pub unset: bool,
    /// Why the value can't be changed here, or `None` when it can.
    pub locked: Option<&'static str>,
    pub nullable: bool,
    /// The values an enum column takes.
    pub options: Vec<String>,
}

/// What the panel tells the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectorEvent {
    /// Put this value in the cell as a pending edit; `None` is NULL.
    Save(Option<String>),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    Json,
    Markdown,
}

/// The editor the column's type calls for.
enum Body {
    Text { editor: Entity<CodeEditor>, mode: Mode, original: String, starts_as_json: bool },
    Choice { select: Entity<Select<Option<String>>>, draft: Option<String> },
    Temporal { fields: Entity<TemporalFields>, kind: TemporalKind },
}

/// Dragging the panel's left edge.
struct ResizeInspector;

struct NoPreview;

impl Render for NoPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub struct CellInspector {
    cell: Option<InspectedCell>,
    body: Option<Body>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<InspectorEvent> for CellInspector {}

impl CellInspector {
    pub fn new(cell: Option<InspectedCell>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut inspector = Self { cell: None, body: None, _subscriptions: Vec::new() };
        inspector.show(cell, window, cx);
        inspector
    }

    pub fn cell(&self) -> Option<&InspectedCell> {
        self.cell.as_ref()
    }

    /// Shows `cell`, its draft starting over.
    pub fn show(&mut self, cell: Option<InspectedCell>, window: &mut Window, cx: &mut Context<Self>) {
        self._subscriptions.clear();
        self.body = cell.as_ref().map(|cell| self.body_for(cell, window, cx));
        self.cell = cell;
        cx.notify();
    }

    fn body_for(&mut self, cell: &InspectedCell, window: &mut Window, cx: &mut Context<Self>) -> Body {
        if let Some(choices) = choices_of(&cell.column.data_type, &cell.options) {
            let mut options: Vec<SelectOption<Option<String>>> =
                choices.into_iter().map(|choice| SelectOption::new(Some(choice.value), choice.label)).collect();
            if cell.nullable || cell.value.is_none() {
                options.push(SelectOption::new(None, "NULL"));
            }
            let value = cell.value.clone();
            let select = cx.new(|cx| Select::new(options, Some(value), cx));
            select.update(cx, |select, cx| select.set_disabled(cell.locked.is_some(), cx));
            self._subscriptions.push(cx.subscribe(&select, |this, _, SelectEvent(value), cx| {
                if let Some(Body::Choice { draft, .. }) = this.body.as_mut() {
                    *draft = value.clone();
                }
                cx.notify();
            }));
            return Body::Choice { select, draft: cell.value.clone() };
        }
        if let Some(kind) = temporal_kind_of(&cell.column.data_type) {
            let space = offset_space(&cell.column.data_type);
            let fields = cx.new(|cx| TemporalFields::new(cell.value.as_deref(), kind, space, window, cx));
            self._subscriptions.push(cx.subscribe(&fields, |this, _, event: &TemporalEvent, cx| match event {
                TemporalEvent::Submitted => this.save(cx),
                _ => cx.notify(),
            }));
            return Body::Temporal { fields, kind };
        }
        let original = cell.value.clone().unwrap_or_default();
        let starts_as_json = is_json_document(&original);
        let text = if starts_as_json { pretty_json(&original) } else { original.clone() };
        let editor = cx.new(|cx| {
            let mut editor = CodeEditor::new(&text, None, cx);
            editor.set_read_only(cell.locked.is_some(), cx);
            editor
        });
        self._subscriptions.push(cx.subscribe(&editor, |_, _, _: &CodeEditorEvent, cx| cx.notify()));
        Body::Text { editor, mode: if starts_as_json { Mode::Json } else { Mode::Text }, original, starts_as_json }
    }

    /// What Save would put in the cell; `None` when nothing (an unchanged
    /// value, a locked cell, a value that isn't valid).
    fn savable(&self, cx: &App) -> Option<Option<String>> {
        let cell = self.cell.as_ref()?;
        if cell.locked.is_some() {
            return None;
        }
        match self.body.as_ref()? {
            Body::Choice { draft, .. } => (draft != &cell.value).then(|| draft.clone()),
            Body::Temporal { fields, .. } => {
                let draft = fields.read(cx).draft();
                let changed = draft.text != cell.value.clone().unwrap_or_default();
                (changed).then(|| draft.value()).flatten().map(Some)
            }
            Body::Text { editor, mode, original, starts_as_json } => {
                let draft = editor.read(cx).text();
                let both_json = *starts_as_json && is_json_document(draft);
                let changed = if both_json { compact_json(draft) != compact_json(original) } else { draft != original };
                if !changed || (*mode == Mode::Json && json_error(draft).is_some()) {
                    return None;
                }
                // A one-line JSON value stays on one line; one the database keeps laid out keeps its layout.
                Some(Some(if both_json && !original.contains('\n') { compact_json(draft) } else { draft.to_owned() }))
            }
        }
    }

    fn changed(&self, cx: &App) -> bool {
        let Some(cell) = self.cell.as_ref() else { return false };
        match self.body.as_ref() {
            Some(Body::Choice { draft, .. }) => draft != &cell.value,
            Some(Body::Temporal { fields, .. }) => {
                fields.read(cx).draft().text != cell.value.clone().unwrap_or_default()
            }
            Some(Body::Text { editor, original, starts_as_json, .. }) => {
                let draft = editor.read(cx).text();
                if *starts_as_json && is_json_document(draft) {
                    compact_json(draft) != compact_json(original)
                } else {
                    draft != original
                }
            }
            None => false,
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Some(value) = self.savable(cx) {
            cx.emit(InspectorEvent::Save(value));
        }
    }

    fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cell) = self.cell.clone() else { return };
        match self.body.as_mut() {
            Some(Body::Choice { select, draft }) => {
                *draft = cell.value.clone();
                select.update(cx, |select, cx| select.set_value(Some(cell.value.clone()), cx));
            }
            Some(Body::Temporal { fields, .. }) => {
                let text = cell.value.clone().unwrap_or_default();
                fields.update(cx, |fields, cx| fields.reset(&text, window, cx));
            }
            Some(Body::Text { editor, original, starts_as_json, .. }) => {
                let text = if *starts_as_json { pretty_json(original) } else { original.clone() };
                editor.update(cx, |editor, cx| editor.set_text(&text, cx));
            }
            None => {}
        }
        cx.notify();
    }

    fn show_mode(&mut self, next: Mode, cx: &mut Context<Self>) {
        if let Some(Body::Text { editor, mode, .. }) = self.body.as_mut() {
            let draft = editor.read(cx).text().to_owned();
            if next == Mode::Json && is_json_document(&draft) {
                editor.update(cx, |editor, cx| editor.set_text(&pretty_json(&draft), cx));
            }
            *mode = next;
        }
        cx.notify();
    }

    /// Keeps the width the user dragged the edge to.
    fn resize(&mut self, width: f32, cx: &mut Context<Self>) {
        let width = width.clamp(MIN_WIDTH, MAX_WIDTH).round().to_string();
        if Preferences::global(cx).get(keys::INSPECTOR_WIDTH) != Some(width.as_str()) {
            cx.update_global::<Preferences, _>(|preferences, _| preferences.set(keys::INSPECTOR_WIDTH, Some(width)));
            cx.notify();
        }
    }

    /// Back to the default width (double-click on the edge), kept as the former web UI kept it.
    fn reset_width(&mut self, cx: &mut Context<Self>) {
        let width = WIDTH.to_string();
        cx.update_global::<Preferences, _>(|preferences, _| preferences.set(keys::INSPECTOR_WIDTH, Some(width)));
        cx.notify();
    }

    fn close_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let m = &cx.messages().table;
        div()
            .id("inspector-close")
            .size(px(26.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(13.))
            .cursor_pointer()
            .hover(|close| close.bg(theme.sel))
            .tooltip(Tooltip::text(m.close))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(InspectorEvent::Close)))
            .child(icon("close", theme.text2, 13.))
            .into_any_element()
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().table;
        let bar = div()
            .h(px(40.))
            .flex_none()
            .pl(px(12.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(theme.line);
        let Some(cell) = &self.cell else {
            return bar
                .child(div().flex_1().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).child(m.value))
                .child(self.close_button(cx))
                .into_any_element();
        };
        let title = div()
            .id("inspector-title")
            .flex_1()
            .min_w_0()
            .truncate()
            .tooltip(Tooltip::text(cell.column.data_type.clone()))
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .child(cell.column.name.clone())
            .child(
                div()
                    .ml(px(6.))
                    .font_family(MONO)
                    .text_size(px(10.5))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(theme.text3)
                    .child(cell.column.data_type.clone()),
            );
        let mut bar = bar.child(title.flex().items_baseline());
        if let Some(Body::Text { editor, mode, .. }) = &self.body {
            let json_allowed = *mode == Mode::Json || is_json_document(editor.read(cx).text());
            let segment = |id: &'static str, label: &'static str, this: Mode, allowed: bool, cx: &mut Context<Self>| {
                let on = *mode == this;
                enabled(
                    div()
                        .h(px(22.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(px(5.))
                        .text_size(px(11.5))
                        .text_color(if on { theme.text } else { theme.text2 })
                        .when(on, |segment| segment.bg(theme.glass_hi).border_1().border_color(theme.rim))
                        .child(label),
                    allowed,
                )
                .id(id)
                .when(allowed, |segment| {
                    segment.on_click(cx.listener(move |this, _, _, cx| this.show_mode(mode_of(id), cx)))
                })
            };
            bar = bar.child(
                div()
                    .flex_none()
                    .flex()
                    .p(px(2.))
                    .gap(px(2.))
                    .rounded(px(6.))
                    .bg(theme.sel)
                    .child(segment("mode-text", m.text, Mode::Text, true, cx))
                    .child(
                        segment("mode-json", "JSON", Mode::Json, json_allowed, cx).tooltip(Tooltip::text(m.json_hint)),
                    )
                    .child(segment("mode-markdown", "Markdown", Mode::Markdown, true, cx)),
            );
        }
        bar.child(self.close_button(cx)).into_any_element()
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().table;
        let Some(cell) = &self.cell else {
            return div().p(px(14.)).text_size(px(12.5)).text_color(theme.text3).child(m.pick_cell).into_any_element();
        };
        let typed = || div().flex().flex_col().gap(px(10.)).p(px(12.));
        match &self.body {
            Some(Body::Choice { select, .. }) => {
                if cell.locked.is_some() {
                    typed().child(locked_value(cell, cx)).into_any_element()
                } else {
                    typed().child(select.clone()).into_any_element()
                }
            }
            Some(Body::Temporal { fields, kind }) => {
                if cell.locked.is_some() {
                    return typed().child(locked_value(cell, cx)).into_any_element();
                }
                let now = if *kind == TemporalKind::Date { m.today } else { m.now };
                let fields_entity = fields.clone();
                typed()
                    .child(fields.clone())
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(button(&theme, None, now).id("inspector-now").cursor_pointer().on_click(
                                move |_, window, cx| fields_entity.update(cx, |fields, cx| fields.now(window, cx)),
                            ))
                            .when(cell.nullable && cell.value.is_some(), |actions| {
                                actions.child(
                                    button(&theme, None, "NULL")
                                        .id("inspector-null")
                                        .cursor_pointer()
                                        .on_click(cx.listener(|_, _, _, cx| cx.emit(InspectorEvent::Save(None)))),
                                )
                            }),
                    )
                    .into_any_element()
            }
            Some(Body::Text { editor, mode: Mode::Markdown, .. }) => markdown(editor.read(cx).text(), m, cx),
            Some(Body::Text { editor, .. }) => div().size_full().flex().child(editor.clone()).into_any_element(),
            None => div().into_any_element(),
        }
    }

    fn status(&self, cx: &App) -> (String, bool) {
        let Some(cell) = &self.cell else { return (String::new(), false) };
        let m = &cx.messages().table;
        let changed = self.changed(cx);
        match &self.body {
            Some(Body::Choice { draft, .. }) => {
                (if cell.unset && !changed { "DEFAULT".into() } else { draft.clone().unwrap_or("NULL".into()) }, false)
            }
            Some(Body::Temporal { fields, .. }) => {
                let draft = fields.read(cx).draft();
                let text = if cell.unset && !changed {
                    "DEFAULT".into()
                } else if draft.text.is_empty() {
                    "NULL".into()
                } else {
                    draft.text.clone()
                };
                (text, !draft.is_valid())
            }
            Some(Body::Text { editor, mode, .. }) => {
                let draft = editor.read(cx).text();
                match json_error(draft).filter(|_| *mode == Mode::Json) {
                    Some(error) => (error, true),
                    None if cell.unset => ("DEFAULT".into(), false),
                    None if cell.value.is_none() && !changed => ("NULL".into(), false),
                    None => ((m.characters)(&thousands(draft.chars().count() as u64)), false),
                }
            }
            None => (String::new(), false),
        }
    }

    fn footer(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let cell = self.cell.as_ref()?;
        let theme = *cx.theme();
        let m = &cx.messages().table;
        let (status, error) = self.status(cx);
        let changed = self.changed(cx);
        let can_save = self.savable(cx).is_some();
        let save = enabled(primary_button(&theme, m.save), can_save).id("inspector-save");
        let save = match cell.locked {
            Some(reason) => save.tooltip(Tooltip::text(reason)),
            None => save,
        };
        Some(
            div()
                .h(px(48.))
                .flex_none()
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .border_t_1()
                .border_color(theme.line)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.5))
                        .text_color(if error { theme.err } else { theme.text3 })
                        .child(status),
                )
                .child(
                    enabled(button(&theme, None, m.revert), changed).id("inspector-revert").when(changed, |revert| {
                        revert.on_click(cx.listener(|this, _, window, cx| this.revert(window, cx)))
                    }),
                )
                .child(save.when(can_save, |save| save.on_click(cx.listener(|this, _, _, cx| this.save(cx)))))
                .into_any_element(),
        )
    }
}

fn mode_of(id: &str) -> Mode {
    match id {
        "mode-json" => Mode::Json,
        "mode-markdown" => Mode::Markdown,
        _ => Mode::Text,
    }
}

fn locked_value(cell: &InspectedCell, cx: &App) -> impl IntoElement + use<> {
    div()
        .text_size(px(12.5))
        .font_family(MONO)
        .text_color(cx.theme().text2)
        .child(cell.value.clone().unwrap_or("NULL".into()))
}

/// The text as Markdown blocks; nothing in it loads or opens.
fn markdown(text: &str, m: &TableMessages, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let blocks = blocks(text, m.image).into_iter().map(|block| match block {
        Block::Heading(level, text) => div()
            .text_size(px(match level {
                1 => 20.,
                2 => 17.,
                3 => 15.,
                _ => 13.5,
            }))
            .font_weight(FontWeight::SEMIBOLD)
            .child(text),
        Block::Paragraph(text) => div().child(text),
        Block::Item { marker, depth, text } => div()
            .pl(px(12. * depth as f32))
            .flex()
            .gap(px(6.))
            .child(div().flex_none().text_color(theme.text3).child(marker))
            .child(div().flex_1().min_w_0().child(text)),
        Block::Quote(text) => {
            div().pl(px(10.)).border_l_2().border_color(theme.line).text_color(theme.text2).child(text)
        }
        Block::Code(code) => div()
            .p(px(8.))
            .rounded(px(6.))
            .bg(theme.sel)
            .font_family(MONO)
            .text_size(px(12.))
            .children(code.lines().map(|line| div().child(line.to_owned())).collect::<Vec<_>>()),
        Block::Rule => div().h(px(1.)).bg(theme.line),
    });
    div()
        .id("inspector-markdown")
        .size_full()
        .overflow_y_scroll()
        .p(px(14.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_size(px(13.))
        .line_height(px(19.))
        .children(blocks.collect::<Vec<_>>())
        .into_any_element()
}

impl Render for CellInspector {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().table;
        let panel_width = width(Preferences::global(cx).get(keys::INSPECTOR_WIDTH));
        div()
            .id("cell-inspector")
            .relative()
            .w(px(panel_width))
            .flex_none()
            .min_h_0()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.line)
            // ⌘S puts the value in the cell first; with nothing to save it goes on to apply every change.
            .on_action(cx.listener(|this, _: &Apply, _, cx| {
                if this.savable(cx).is_some() {
                    this.save(cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<ResizeInspector>, _, cx| {
                let right: f32 = event.bounds.right().into();
                let pointer: f32 = event.event.position.x.into();
                this.resize(right - pointer, cx);
            }))
            .child(self.header(cx))
            .child(div().flex_1().min_h_0().child(self.body(cx)))
            .children(self.footer(cx))
            .child(
                div()
                    .id("inspector-edge")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(px(6.))
                    .cursor_col_resize()
                    .tooltip(Tooltip::text(m.resize_panel))
                    .on_drag(ResizeInspector, |_, _, _, cx| cx.new(|_| NoPreview))
                    .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            this.reset_width(cx);
                        }
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_saved_width_within_its_range() {
        assert_eq!(width(None), 380.);
        assert_eq!(width(Some("120")), 260.);
        assert_eq!(width(Some("1200")), 900.);
        assert_eq!(width(Some("500")), 500.);
        assert_eq!(width(Some("x")), 380.);
    }
}
