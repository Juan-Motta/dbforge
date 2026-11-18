//! A read-only grid of rows: a query's result now, a table's rows and
//! documents later. Only the rows and columns in view are laid out and
//! drawn, so a big result scrolls as smoothly as a small one. It has a
//! header with each column's type, row numbers, columns that resize by
//! dragging their edge, NULLs set apart, numbers aligned right, a selection
//! of cells (click, ⌘/Ctrl-click, Shift-click or drag, the arrows) that
//! copies as tab-separated text, and scrolling both ways. A table's tab
//! draws its pending changes over it ([`marks::GridMarks`]) and hears what
//! the mouse asks of a cell ([`GridEvent`]).

pub mod columns;
mod element;
pub mod marks;
pub mod selection;

use std::rc::Rc;

use dbforge_core::drivers::Column;
use gpui::{
    App, Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render,
    ScrollWheelEvent, Styled, Window, actions, div, point, px, size,
};

use self::columns::{MIN_DRAG_WIDTH, is_numeric_type};
use self::element::GridElement;
use self::marks::GridMarks;
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

/// What the mouse asks of the grid's owner. The cell is already picked.
#[derive(Debug, Clone, PartialEq)]
pub enum GridEvent {
    /// A double click on a cell: edit it.
    Edit(Cell),
    /// A right click on a cell, at this point in the window: its menu.
    Menu(Cell, Point<Pixels>),
    /// A click on a column's header: sort by it.
    Sort(usize),
    /// A click on a cell's link: open the row its foreign key points to.
    Follow(Cell),
}

/// A cell's link, at its right end.
const LINK_SIZE: f32 = 18.;
const LINK_MARGIN: f32 = 6.;

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
    marks: Option<Rc<GridMarks>>,
}

impl EventEmitter<GridEvent> for DataGrid {}

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
            marks: None,
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

    /// Shows `rows` in place of the rows shown, with `marks`: the same
    /// columns, their widths and the scroll kept (a reload, an edit). The
    /// selection is the owner's to keep, move ([`Selection::remap`]) or clear.
    pub fn update_rows(&mut self, rows: Rows, marks: Option<Rc<GridMarks>>, cx: &mut Context<Self>) {
        self.rows = rows;
        self.marks = marks;
        cx.notify();
    }

    /// Back to the first row and column.
    pub fn scroll_to_top(&mut self, cx: &mut Context<Self>) {
        self.scroll = point(px(0.), px(0.));
        self.reveal = None;
        cx.notify();
    }

    pub fn marks(&self) -> Option<&GridMarks> {
        self.marks.as_deref()
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn selection_mut(&mut self, cx: &mut Context<Self>) -> &mut Selection {
        cx.notify();
        &mut self.selection
    }

    /// Picks `cell` alone and scrolls it into view.
    pub fn pick(&mut self, cell: Cell, cx: &mut Context<Self>) {
        self.selection.select(cell, Mode::Single, self.columns.len(), false);
        self.reveal = Some(cell);
        cx.notify();
    }

    /// Where `cell` was last drawn, in window coordinates; `None` while it is out of view.
    pub fn cell_bounds(&self, cell: Cell) -> Option<Bounds<Pixels>> {
        let left = *self.layout.lefts.get(cell.column)?;
        let width = px(*self.widths.get(cell.column)?);
        let top = self.layout.bounds.top() + px(HEADER_HEIGHT) + px(ROW_HEIGHT * cell.row as f32) - self.scroll.y;
        let bounds = Bounds::new(point(left, top), size(width, px(ROW_HEIGHT)));
        let body = Bounds::from_corners(
            point(self.layout.gutter, self.layout.bounds.top() + px(HEADER_HEIGHT)),
            self.layout.bounds.bottom_right(),
        );
        (cell.row < self.rows.len() && bounds.intersects(&body)).then_some(bounds)
    }

    /// Where the grid was last drawn, in window coordinates.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.layout.bounds
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
        // One cell left to its DEFAULT copies as that, as its text says.
        let one = self.selection.cells().iter().next().copied().filter(|_| self.selection.cells().len() == 1);
        let text = match (one, self.marks()) {
            (Some(cell), Some(marks)) if marks.cell(cell).unset => "DEFAULT".to_owned(),
            _ => self.selection_as_tsv(),
        };
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
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

    /// The cell whose link is under `position`.
    fn link_at(&self, position: Point<Pixels>) -> Option<Cell> {
        let marks = self.marks.as_ref()?;
        let (row, Some(column)) = self.cell_at(position)? else { return None };
        let cell = Cell::new(row, column);
        let bounds = self.cell_bounds(cell)?;
        let link = bounds.right() - px(LINK_MARGIN);
        (marks.links.contains(&cell) && position.x <= link && position.x >= link - px(LINK_SIZE)).then_some(cell)
    }

    /// The column whose header is under `position`, edges aside.
    fn header_at(&self, position: Point<Pixels>) -> Option<usize> {
        let header = self.layout.bounds.top()..self.layout.bounds.top() + px(HEADER_HEIGHT);
        if !header.contains(&position.y) || position.x < self.layout.gutter || self.columns.is_empty() {
            return None;
        }
        let column = self.layout.lefts.iter().rposition(|left| *left <= position.x)?;
        let right = self.layout.lefts[column] + px(*self.widths.get(column)?);
        (position.x < right).then_some(column)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window);
        if let Some(column) = self.edge_at(event.position) {
            let from_width = self.widths[column];
            self.drag = Some(Drag::Resizing { column, from_x: event.position.x, from_width });
            return;
        }
        if event.position.y < self.layout.bounds.top() + px(HEADER_HEIGHT) {
            let sortable = self.marks.as_ref().is_some_and(|marks| marks.sortable);
            if let Some(column) = self.header_at(event.position).filter(|_| sortable) {
                cx.emit(GridEvent::Sort(column));
            }
            return;
        }
        if let Some(cell) = self.link_at(event.position) {
            self.pick(cell, cx);
            cx.emit(GridEvent::Follow(cell));
            return;
        }
        let Some((row, column)) = self.cell_at(event.position) else { return };
        // Only an editing owner (one that marks its rows) edits on a double click.
        if event.click_count == 2
            && self.marks.is_some()
            && let Some(column) = column
        {
            let cell = Cell::new(row, column);
            if !self.selection.contains(cell) {
                self.pick(cell, cx);
            }
            cx.emit(GridEvent::Edit(cell));
            return;
        }
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

    /// A right click picks the cell (keeping a selection it is part of) and asks for its menu.
    fn on_right_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // A read-only result has no cell menu yet.
        if self.marks.is_none() {
            return;
        }
        self.focus(window);
        if event.position.y < self.layout.bounds.top() + px(HEADER_HEIGHT) {
            return;
        }
        let Some((row, Some(column))) = self.cell_at(event.position) else { return };
        let cell = Cell::new(row, column);
        if !self.selection.contains(cell) {
            self.selection.select(cell, Mode::Single, self.columns.len(), false);
        }
        cx.emit(GridEvent::Menu(cell, event.position));
        cx.notify();
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
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_mouse_down))
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
