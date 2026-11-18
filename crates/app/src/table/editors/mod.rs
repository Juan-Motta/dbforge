//! A cell being edited, its editor over the cell so the table doesn't move:
//! text in a field (Enter or leaving it keeps the text, Escape cancels), a
//! list of values for a boolean or an enum (a choice keeps it, Escape or a
//! click outside changes nothing), and date and time controls under the
//! cell (Enter or Apply keeps the value, Escape or Cancel doesn't).

pub mod temporal;

use dbforge_core::drivers::CellValue;
use gpui::{
    AnyElement, AppContext, Context, DismissEvent, Entity, Focusable, InteractiveElement, IntoElement, ParentElement,
    Point, StatefulInteractiveElement, Styled, Subscription, Window, anchored, div, point, prelude::FluentBuilder, px,
};

use self::temporal::{TemporalEvent, TemporalFields};
use super::choices::choices_of;
use super::temporal::{TemporalKind, offset_space, temporal_kind_of};
use super::view::TableView;
use crate::grid::selection::Cell;
use crate::i18n::Translate;
use crate::theme::ActiveTheme;
use crate::ui::{
    MenuItem, PopoverMenu, TextInput, TextInputEvent, button, enabled, ghost_button, glass, popover, primary_button,
};

/// Width of the date and time controls under a cell.
const PANEL_WIDTH: f32 = 340.;

pub enum EditorKind {
    Text(Entity<TextInput>),
    Choice(Entity<PopoverMenu>, Point<gpui::Pixels>),
    Temporal(Entity<TemporalFields>, TemporalKind),
}

/// The cell being edited and its editor.
pub struct OpenEditor {
    pub cell: Cell,
    /// What the cell held when the editor opened: untouched text is no change.
    pub initial: Option<String>,
    pub kind: EditorKind,
    nullable: bool,
    _subscriptions: Vec<Subscription>,
}

impl TableView {
    /// Opens the editor its column's type calls for over `cell`; nothing for
    /// a cell that can't change.
    pub fn open_editor(&mut self, cell: Cell, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            self.finish_editor(window, cx);
        }
        if self.locked(cell, cx).is_some() {
            return;
        }
        let Some(row) = self.row_ref(cell.row) else { return };
        let Some(column) = self.columns().get(cell.column).cloned() else { return };
        let Some(shown) = self.edits.cell_at(self.rows(), self.columns(), row, cell.column) else { return };
        let info = self.structure.as_ref().and_then(|s| s.columns.iter().find(|info| info.name == column.name));
        let nullable = info.is_none_or(|info| info.nullable);
        let options = info.map(|info| info.options.clone()).unwrap_or_default();
        let initial = shown.value;
        let Some(bounds) = self.grid.read(cx).cell_bounds(cell) else { return };
        let mut subscriptions = Vec::new();
        let kind =
            if let Some(choices) = choices_of(&column.data_type, &options) {
                let this = cx.entity().downgrade();
                let mut values: Vec<(Option<String>, String)> =
                    choices.into_iter().map(|choice| (Some(choice.value), choice.label)).collect();
                if nullable {
                    values.push((None, "NULL".into()));
                }
                let items = values
                    .into_iter()
                    .map(|(value, label)| {
                        let detail = value.clone().filter(|value| *value != label);
                        let checked = value == initial;
                        let (this, chosen) = (this.clone(), value.clone());
                        MenuItem::new(label, move |window, cx| {
                            let chosen = chosen.clone();
                            let _ = this.update(cx, |this, cx| this.pick_choice(cell, chosen, window, cx));
                        })
                        .detail(detail)
                        .checked(checked)
                    })
                    .collect();
                let menu = cx.new(|cx| PopoverMenu::new(items, window, cx).with_width(bounds.size.width.max(px(160.))));
                subscriptions.push(cx.subscribe_in(&menu, window, move |this, _, _: &DismissEvent, window, cx| {
                    if this.editor.as_ref().is_some_and(|editor| editor.cell == cell) {
                        this.cancel_editor(window, cx);
                    }
                }));
                EditorKind::Choice(menu, bounds.bottom_left() + point(px(0.), px(2.)))
            } else if let Some(kind) = temporal_kind_of(&column.data_type) {
                let space = offset_space(&column.data_type);
                let fields = cx.new(|cx| TemporalFields::new(initial.as_deref(), kind, space, window, cx));
                subscriptions.push(cx.subscribe_in(&fields, window, |this, _, event: &TemporalEvent, window, cx| {
                    match event {
                        TemporalEvent::Submitted => this.finish_editor(window, cx),
                        TemporalEvent::Cancelled => this.cancel_editor(window, cx),
                        TemporalEvent::Changed => cx.notify(),
                    }
                }));
                EditorKind::Temporal(fields, kind)
            } else {
                let input = cx.new(|cx| {
                    let mut input = TextInput::new("", cx);
                    input.set_text(initial.as_deref().unwrap_or(""), cx);
                    // Selected, so typing replaces the value and the arrows still allow small fixes.
                    input.select_all(cx);
                    input
                });
                input.read(cx).focus(window);
                subscriptions.push(cx.subscribe_in(&input, window, |this, _, event: &TextInputEvent, window, cx| {
                    match event {
                        TextInputEvent::Submitted => this.finish_editor(window, cx),
                        TextInputEvent::Cancelled => this.cancel_editor(window, cx),
                        _ => {}
                    }
                }));
                // Leaving the field keeps the text, as Enter does.
                let focus = input.read(cx).focus_handle(cx);
                subscriptions.push(cx.on_blur(&focus, window, |this, window, cx| this.finish_editor(window, cx)));
                EditorKind::Text(input)
            };
        self.editor = Some(OpenEditor { cell, initial, kind, nullable, _subscriptions: subscriptions });
        cx.notify();
    }

    /// Keeps what the editor holds, when it is a change; an invalid date stays open.
    pub fn finish_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else { return };
        let initial = editor.initial.clone().unwrap_or_default();
        let value = match &editor.kind {
            EditorKind::Text(input) => {
                let text = input.read(cx).text().to_owned();
                // Untouched text is no change: just opening a NULL or DEFAULT cell must not store ''.
                (text != initial).then_some(CellValue::Text(text))
            }
            EditorKind::Temporal(fields, _) => {
                let draft = fields.read(cx).draft();
                if draft.text == initial {
                    None
                } else {
                    // Written back in the server's own form, whatever spacing was typed.
                    match draft.value() {
                        Some(value) => Some(CellValue::Text(value)),
                        None => return,
                    }
                }
            }
            EditorKind::Choice(..) => None,
        };
        let cell = editor.cell;
        match value {
            Some(value) => self.commit_editor(cell, value, window, cx),
            None => self.cancel_editor(window, cx),
        }
    }

    /// Closes the editor, the cell as it was.
    pub fn cancel_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.take().is_some() {
            self.focus(window, cx);
            cx.notify();
        }
    }

    /// Puts `value` in `cell` (and the cells selected with it) and closes the editor.
    fn commit_editor(&mut self, cell: Cell, value: CellValue, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        let targets = self.targets_of(cell, cx);
        self.set_cells(&targets, value, cx);
        self.focus(window, cx);
    }

    /// A value picked from a boolean's or an enum's list; the one it held changes nothing.
    fn pick_choice(&mut self, cell: Cell, value: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let initial =
            self.row_ref(cell.row).and_then(|row| self.edits.cell_at(self.rows(), self.columns(), row, cell.column));
        self.editor = None;
        if initial.is_some_and(|shown| shown.value == value) {
            self.focus(window, cx);
            cx.notify();
            return;
        }
        self.commit_editor(cell, value.into(), window, cx);
    }

    /// Sets the date or time editor to now (Now, Today).
    fn editor_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(OpenEditor { kind: EditorKind::Temporal(fields, _), .. }) = &self.editor {
            fields.update(cx, |fields, cx| fields.now(window, cx));
        }
    }

    /// The editor over its cell, drawn last so it is above the grid. Never
    /// deferred: the date controls' dropdowns are.
    pub(super) fn render_editor(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editor = self.editor.as_ref()?;
        let bounds = self.grid.read(cx).cell_bounds(editor.cell)?;
        let theme = *cx.theme();
        let m = &cx.messages().table;
        Some(match &editor.kind {
            EditorKind::Text(input) => anchored()
                .position(bounds.origin)
                .child(
                    div()
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        // The text stays where the cell drew it: its padding, less the border.
                        .px(px(crate::grid::columns::CELL_PADDING - 1.))
                        .flex()
                        .items_center()
                        .bg(theme.card)
                        .border_1()
                        .border_color(theme.accent)
                        .rounded(px(4.))
                        .text_size(px(12.5))
                        .text_color(theme.text)
                        .child(div().flex_1().min_w_0().child(input.clone())),
                )
                .into_any_element(),
            EditorKind::Choice(menu, position) => popover(menu.clone(), *position).into_any_element(),
            EditorKind::Temporal(fields, kind) => {
                let valid = fields.read(cx).draft().is_valid();
                let now = if *kind == TemporalKind::Date { m.today } else { m.now };
                let fields_entity = fields.clone();
                let panel = glass(&theme)
                    .id("temporal-editor")
                    .occlude()
                    .bg(theme.card)
                    .w(px(PANEL_WIDTH))
                    .rounded(px(12.))
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .on_mouse_down_out(cx.listener(move |this, _, window, cx| {
                        // A click outside keeps the value, as leaving the plain editor does;
                        // not while one of its dropdowns is open.
                        if !fields_entity.read(cx).is_choosing(cx) {
                            this.finish_editor(window, cx);
                        }
                    }))
                    .child(fields.clone())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                button(&theme, None, now)
                                    .id("temporal-now")
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| this.editor_now(window, cx))),
                            )
                            .when(editor.nullable, |actions| {
                                let cell = editor.cell;
                                actions.child(
                                    button(&theme, None, "NULL").id("temporal-null").cursor_pointer().on_click(
                                        cx.listener(move |this, _, window, cx| {
                                            this.commit_editor(cell, CellValue::Null, window, cx)
                                        }),
                                    ),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                ghost_button(&theme, m.cancel)
                                    .id("temporal-cancel")
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| this.cancel_editor(window, cx))),
                            )
                            .child(enabled(primary_button(&theme, m.apply), valid).id("temporal-apply").when(
                                valid,
                                |apply| {
                                    apply.on_click(cx.listener(|this, _, window, cx| this.finish_editor(window, cx)))
                                },
                            )),
                    );
                anchored()
                    .position(bounds.bottom_left() + point(px(0.), px(2.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(panel)
                    .into_any_element()
            }
        })
    }
}
