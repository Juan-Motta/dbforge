//! A read-only grid of rows: a query's result now, a table's rows and
//! documents later. Only the rows and columns in view are laid out and
//! drawn, so a big result scrolls as smoothly as a small one. It has a
//! header with each column's type, row numbers, columns that resize by
//! dragging their edge, NULLs set apart, numbers aligned right, a selection
//! of cells (click, ⌘/Ctrl-click, Shift-click or drag, the arrows) that
//! copies as tab-separated text, and scrolling both ways.

pub mod columns;
mod element;
pub mod selection;

use std::rc::Rc;

use dbforge_core::drivers::Column;
use gpui::{
    App, Bounds, ClipboardItem, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, ScrollWheelEvent,
    Styled, Window, actions, div, point, px,
};

use self::columns::{MIN_DRAG_WIDTH, is_numeric_type};
use self::element::GridElement;
use self::selection::{Cell, Mode, Selection};

actions!(
    grid,
    [
        Up,
        Down,
        Left,
        Right,
        SelectUp,
        SelectDown,
        SelectLeft,
        SelectRight,
        PageUp,
        PageDown,
        First,
        Last,
        SelectAll,
        Copy
    ]
);

const CONTEXT: &str = "DataGrid";

/// The header's and each row's height.
pub const HEADER_HEIGHT: f32 = 32.;
pub const ROW_HEIGHT: f32 = 28.;
/// How close to a column's edge in the header the pointer resizes it.
const RESIZE_REACH: f32 = 4.;

pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    let mut bindings = vec![
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("pageup", PageUp, context),
        KeyBinding::new("pagedown", PageDown, context),
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("secondary-c", Copy, context),
    ];
    if cfg!(target_os = "macos") {
        bindings.extend([KeyBinding::new("cmd-up", First, context), KeyBinding::new("cmd-down", Last, context)]);
    } else {
        bindings.extend([KeyBinding::new("ctrl-home", First, context), KeyBinding::new("ctrl-end", Last, context)]);
    }
    bindings
}

/// The rows a grid shows: values as text, `None` for NULL.
pub type Rows = Rc<Vec<Vec<Option<String>>>>;

/// What the mouse is dragging.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    /// A block of cells from the anchor.
    Selecting,
    /// A column's right edge.
    Resizing { column: usize, from_x: Pixels, from_width: f32 },
}

/// Where the last frame drew things, to map the mouse onto cells.
#[derive(Debug, Clone, Default)]
struct GridLayout {
    bounds: Bounds<Pixels>,
    /// The row numbers' right edge: cells start there.
    gutter: Pixels,
    /// Each column's left edge in the window, scrolled.
    lefts: Vec<Pixels>,
    /// Rows that fit in view.
    rows_in_view: usize,
}

pub struct DataGrid {
    columns: Vec<Column>,
    rows: Rows,
    numeric: Vec<bool>,
    /// Each column's width; set from the content on the first frame.
    widths: Vec<f32>,
    selection: Selection,
    focus: FocusHandle,
    scroll: Point<Pixels>,
    /// A cell to scroll into view on the next frame.
    reveal: Option<Cell>,
    layout: GridLayout,
    drag: Option<Drag>,
}

impl Focusable for DataGrid {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DataGrid {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            columns: Vec::new(),
            rows: Rc::default(),
            numeric: Vec::new(),
            widths: Vec::new(),
            selection: Selection::default(),
            focus: cx.focus_handle().tab_stop(true),
            scroll: point(px(0.), px(0.)),
            reveal: None,
            layout: GridLayout::default(),
            drag: None,
        }
    }

    /// Shows `rows` under `columns`, from the top, nothing selected.
    pub fn set_data(&mut self, columns: Vec<Column>, rows: Rows, cx: &mut Context<Self>) {
        self.numeric = columns.iter().map(|column| is_numeric_type(&column.data_type)).collect();
        self.columns = columns;
        self.rows = rows;
        self.widths.clear();
        self.selection.clear();
        self.scroll = point(px(0.), px(0.));
        self.reveal = None;
        cx.notify();
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn value(&self, cell: Cell) -> Option<&str> {
        self.rows.get(cell.row)?.get(cell.column)?.as_deref()
    }

    /// The selection as tab-separated text.
    pub fn selection_as_tsv(&self) -> String {
        self.selection.as_tsv(|cell| self.value(cell))
    }

    // Keys

    fn step(&mut self, rows: isize, columns: isize, extend: bool, cx: &mut Context<Self>) {
        self.reveal = self.selection.step(rows, columns, extend, self.rows.len(), self.columns.len());
        cx.notify();
    }

    fn page(&self) -> isize {
        self.layout.rows_in_view.saturating_sub(1).max(1) as isize
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selection_as_tsv()));
        }
    }

    // The mouse

    /// The cell under `position` (a row's number: column `None`), clamped to the grid.
    fn cell_at(&self, position: Point<Pixels>) -> Option<(usize, Option<usize>)> {
        if self.rows.is_empty() {
            return None;
        }
        let body_top = self.layout.bounds.top() + px(HEADER_HEIGHT);
        let y = position.y - body_top + self.scroll.y;
        let row = ((f32::from(y) / ROW_HEIGHT).floor().max(0.) as usize).min(self.rows.len() - 1);
        if position.x < self.layout.gutter {
            return Some((row, None));
        }
        let column = self.layout.lefts.iter().rposition(|left| *left <= position.x).unwrap_or(0);
        Some((row, Some(column.min(self.columns.len().saturating_sub(1)))))
    }

    /// The column whose right edge in the header is under `position`.
    fn edge_at(&self, position: Point<Pixels>) -> Option<usize> {
        let header = self.layout.bounds.top()..self.layout.bounds.top() + px(HEADER_HEIGHT);
        if !header.contains(&position.y) {
            return None;
        }
        self.layout.lefts.iter().enumerate().find_map(|(column, left)| {
            let edge = *left + px(*self.widths.get(column)?);
            ((position.x - edge).abs() <= px(RESIZE_REACH) && edge > self.layout.gutter).then_some(column)
        })
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window);
        if let Some(column) = self.edge_at(event.position) {
            let from_width = self.widths[column];
            self.drag = Some(Drag::Resizing { column, from_x: event.position.x, from_width });
            return;
        }
        if event.position.y < self.layout.bounds.top() + px(HEADER_HEIGHT) {
            return;
        }
        let Some((row, column)) = self.cell_at(event.position) else { return };
        let mode = Mode::of(event.modifiers.shift, event.modifiers.secondary());
        let width = self.columns.len();
        self.selection.select(Cell::new(row, column.unwrap_or(0)), mode, width, column.is_none());
        self.drag = (mode != Mode::Toggle && column.is_some()).then_some(Drag::Selecting);
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        match self.drag {
            Some(Drag::Resizing { column, from_x, from_width }) => {
                self.widths[column] = (from_width + f32::from(event.position.x - from_x)).max(MIN_DRAG_WIDTH);
                cx.notify();
            }
            Some(Drag::Selecting) => {
                if let Some((row, Some(column))) = self.cell_at(event.position) {
                    let target = Cell::new(row, column);
                    if self.selection.cursor() != Some(target) {
                        self.selection.select(target, Mode::Range, self.columns.len(), false);
                        self.reveal = Some(target);
                        cx.notify();
                    }
                }
            }
            None => {}
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.drag = None;
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(ROW_HEIGHT));
        // Shift turns a wheel's vertical turn sideways, where the OS doesn't.
        let delta = if event.modifiers.shift && delta.x == px(0.) { point(delta.y, px(0.)) } else { delta };
        self.scroll = point(self.scroll.x - delta.x, self.scroll.y - delta.y);
        cx.notify();
    }
}

impl Render for DataGrid {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("data-grid")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .on_action(cx.listener(|this, _: &Up, _, cx| this.step(-1, 0, false, cx)))
            .on_action(cx.listener(|this, _: &Down, _, cx| this.step(1, 0, false, cx)))
            .on_action(cx.listener(|this, _: &Left, _, cx| this.step(0, -1, false, cx)))
            .on_action(cx.listener(|this, _: &Right, _, cx| this.step(0, 1, false, cx)))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.step(-1, 0, true, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.step(1, 0, true, cx)))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.step(0, -1, true, cx)))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.step(0, 1, true, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.step(-this.page(), 0, false, cx)))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| this.step(this.page(), 0, false, cx)))
            .on_action(cx.listener(|this, _: &First, _, cx| this.step(-(this.rows.len() as isize), 0, false, cx)))
            .on_action(cx.listener(|this, _: &Last, _, cx| this.step(this.rows.len() as isize, 0, false, cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.selection.select_all(this.rows.len(), this.columns.len());
                cx.notify();
            }))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(GridElement::new(cx.entity()))
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::theme::{Appearance, Theme};

    fn column(name: &str, data_type: &str) -> Column {
        serde_json::from_value(serde_json::json!({ "name": name, "dataType": data_type })).expect("a column")
    }

    #[gpui::test]
    fn draws_a_big_result_and_scrolls_to_the_cell_picked(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(Theme::new(Appearance::Light)));
        let (grid, cx) = cx.add_window_view(|_, cx| DataGrid::new(cx));
        grid.update(cx, |grid, cx| {
            let rows = (0..100_000).map(|n| vec![Some(n.to_string()), None, Some("x".repeat(n % 40))]).collect();
            let columns = vec![column("id", "int8"), column("note", "text"), column("long", "character varying(80)")];
            grid.set_data(columns, Rc::new(rows), cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        grid.update(cx, |grid, cx| {
            assert_eq!(grid.widths.len(), 3, "widths come from the content on the first frame");
            assert!(grid.layout.rows_in_view > 0);
            assert!(grid.cell_at(grid.layout.bounds.center()).is_some());
            grid.step(1, 0, false, cx);
            grid.step(80_000, 2, true, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        grid.read_with(cx, |grid, _| {
            assert!(grid.scroll.y > px(ROW_HEIGHT * 70_000.), "scrolled down to the picked row");
            assert_eq!(grid.selection().cursor(), Some(Cell::new(80_000, 2)));
            assert_eq!(grid.selection().cells().len(), 80_001 * 3);
        });
        grid.update(cx, |grid, cx| {
            grid.selection.clear();
            grid.selection.select(Cell::new(0, 0), Mode::Single, 3, true);
            assert_eq!(grid.selection_as_tsv(), "0\tNULL\t");
            grid.set_data(Vec::new(), Rc::default(), cx);
            assert!(grid.selection().is_empty(), "a new result starts with nothing selected");
        });
    }
}
