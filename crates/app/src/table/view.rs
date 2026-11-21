//! A table's rows, one page at a time, editable when the table has a
//! primary key. The filter bar narrows and orders them; the grid shows them
//! with their pending changes (new rows on top, deleted rows struck out,
//! edited cells marked); the footer counts them, pages through them and,
//! while changes are pending, offers Preview SQL, Discard and Apply. A new
//! page keeps the rows shown until it arrives, so nothing flickers. A
//! document database's rows also show as whole documents (see
//! [`super::documents`]), switched from the bar while nothing is pending.

use crate::ui::Ellipsis;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::{ApplySummary, CellValue, Column, RowFilter, TableRef, TableStructure};
use dbforge_core::edit::{apply_changes, preview_changes};
use dbforge_core::explorer::fetch_rows;
use gpui::{
    AnyElement, AnyView, AppContext, ClipboardItem, Context, DismissEvent, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};

use super::cell_actions::is_relational;
use super::dialogs::{ConfirmDialog, ConfirmEvent, PreviewDialog};
use super::documents::Documents;
use super::editors::OpenEditor;
use super::edits::{Edits, RowRef, key_of, row_keys};
use super::filter::{Direction, is_filtered, sort_by, sort_of};
use super::filter_bar::{ApplyFilter, FilterBar};
use super::inspector::{CellInspector, InspectedCell, InspectorEvent};
use super::links::link_filter;
use super::{Apply, Deselect, Inspect, Paste};
use crate::backend::Backend;
use crate::grid::marks::{CellMark, ColumnMark, GridMarks, RowState, SortMark};
use crate::grid::selection::Cell;
use crate::grid::{DataGrid, GridEvent};
use crate::i18n::Translate;
use crate::sql::Syntax;
use crate::structure::StructureSource;
use crate::structure::format::summary;
use crate::tags::confirming_tag_of;
use crate::theme::ActiveTheme;
use crate::ui::field::Tooltip;
use crate::ui::{
    PopoverMenu, Select, SelectEvent, SelectOption, button, enabled, ghost_button, icon, icon_button, pill, popover,
    primary_button,
};

/// Rows per page the footer offers; the core reads at most 1000.
pub const PAGE_SIZES: [u32; 5] = [50, 100, 200, 500, 1000];
pub const DEFAULT_PAGE_SIZE: u32 = 200;

const CONTEXT: &str = "TableView";

/// What a table's rows ask of the window.
#[derive(Debug, Clone)]
pub enum TableEvent {
    /// Open this table filtered to these rows (a foreign key followed).
    OpenTable { table: TableRef, filter: RowFilter },
    /// Show this dialog over the window, or close the one shown.
    Dialog(Option<AnyView>),
}

/// A page of rows as loaded.
pub(super) struct Page {
    pub columns: Vec<Column>,
    pub rows: Rc<Vec<Vec<Option<String>>>>,
    pub has_more: bool,
    /// Each row's key among the edits.
    pub keys: Vec<String>,
    /// Document databases: each row's whole document, in the engine's own syntax.
    pub documents: Option<Rc<Vec<SharedString>>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Status {
    Idle,
    Applying,
    Applied(ApplySummary),
    Failed(String),
}

pub struct TableView {
    pub(super) connection_id: String,
    pub(super) engine: Engine,
    pub(super) table: TableRef,
    pub(super) filter: RowFilter,
    page_size: u32,
    pub(super) offset: u64,
    pub(super) page: Option<Page>,
    error: Option<String>,
    loading: bool,
    /// The newest page asked for; an older one arriving late is dropped.
    request: u64,
    pub(super) structure: Option<Rc<TableStructure>>,
    pub(super) edits: Edits,
    pub(super) status: Status,
    pub(super) grid: Entity<DataGrid>,
    filter_bar: Entity<FilterBar>,
    page_sizes: Entity<Select<u32>>,
    inspector: Option<Entity<CellInspector>>,
    pub(super) editor: Option<OpenEditor>,
    /// The rows as documents, when the engine has them.
    pub(super) documents: Documents,
    pub(super) menu: Option<(Entity<PopoverMenu>, Point<Pixels>, Subscription)>,
    dialog: Vec<Subscription>,
    pub(super) focus: FocusHandle,
    /// Development only: what to show once the first page arrives.
    #[cfg(debug_assertions)]
    pub(super) seed: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TableEvent> for TableView {}

impl Focusable for TableView {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.grid.focus_handle(cx)
    }
}

impl TableView {
    /// `table` of `connection_id` (an `engine` connection), shown with `filter`.
    pub fn new(
        connection_id: String,
        engine: Engine,
        table: TableRef,
        filter: RowFilter,
        structure: Entity<StructureSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let grid = cx.new(DataGrid::new);
        let filter_bar = cx.new(|cx| FilterBar::new(engine, filter.clone(), window, cx));
        let m = &cx.messages().table;
        let options = PAGE_SIZES.iter().map(|size| SelectOption::new(*size, (m.per_page)(*size))).collect();
        let page_sizes = cx.new(|cx| Select::new(options, Some(DEFAULT_PAGE_SIZE), cx));
        let subscriptions = vec![
            cx.observe(&structure, Self::follow_structure),
            cx.subscribe_in(&grid, window, Self::on_grid),
            // The footer and the inspector follow the selection.
            cx.observe_in(&grid, window, |this, _, window, cx| {
                this.follow_cursor(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&filter_bar, window, |this, _, ApplyFilter(filter), window, cx| {
                this.set_filter(filter.clone(), window, cx);
            }),
            cx.subscribe(&page_sizes, |this, _, SelectEvent(size): &SelectEvent<u32>, cx| {
                this.page_size = *size;
                this.offset = 0;
                this.load(false, cx);
            }),
        ];
        let mut view = Self {
            connection_id,
            engine,
            table,
            filter,
            page_size: DEFAULT_PAGE_SIZE,
            offset: 0,
            page: None,
            error: None,
            loading: false,
            request: 0,
            structure: None,
            edits: Edits::default(),
            status: Status::Idle,
            grid,
            filter_bar,
            page_sizes,
            inspector: None,
            editor: None,
            documents: Documents::default(),
            menu: None,
            dialog: Vec::new(),
            focus: cx.focus_handle(),
            #[cfg(debug_assertions)]
            seed: None,
            _subscriptions: subscriptions,
        };
        view.load(false, cx);
        view.follow_structure(structure, cx);
        view
    }

    pub fn grid(&self) -> &Entity<DataGrid> {
        &self.grid
    }

    pub fn edits(&self) -> &Edits {
        &self.edits
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// The grid takes the keyboard; the view itself while documents show.
    pub fn focus(&self, window: &mut Window, cx: &gpui::App) {
        if self.showing_documents() {
            window.focus(&self.focus);
        } else {
            self.grid.read(cx).focus(window);
        }
    }

    /// Rows can be told apart to change them: the table has a primary key.
    pub fn editable(&self) -> bool {
        self.page.as_ref().is_some_and(|page| page.columns.iter().any(|column| column.primary_key))
    }

    pub(super) fn applying(&self) -> bool {
        self.status == Status::Applying
    }

    pub(super) fn columns(&self) -> &[Column] {
        self.page.as_ref().map_or(&[], |page| page.columns.as_slice())
    }

    pub(super) fn rows(&self) -> &[Vec<Option<String>>] {
        self.page.as_ref().map_or(&[], |page| page.rows.as_slice())
    }

    // Loading

    /// Reads the page asked for; the rows shown stay until it arrives. A
    /// `reload` keeps the place in the grid; another page starts at its top.
    pub fn load(&mut self, reload: bool, cx: &mut Context<Self>) {
        self.request += 1;
        let request = self.request;
        self.loading = true;
        let (connection, table, filter) = (self.connection_id.clone(), self.table.clone(), self.filter.clone());
        let (offset, limit) = (self.offset, self.page_size);
        let reading = Backend::global(cx).spawn(cx, move |core| async move {
            fetch_rows(core.connections(), core.statement_log(), connection, table, Some(filter), offset, limit).await
        });
        cx.spawn(async move |this, cx| {
            let result = reading.await;
            let _ = this.update(cx, |this, cx| {
                if this.request != request {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(page) => {
                        this.error = None;
                        let keys = row_keys(&page.rows, &page.columns);
                        let same_columns = this.page.as_ref().is_some_and(|shown| shown.columns == page.columns);
                        this.page = Some(Page {
                            columns: page.columns,
                            rows: Rc::new(page.rows),
                            has_more: page.has_more,
                            keys,
                            documents: page
                                .documents
                                .map(|documents| Rc::new(documents.into_iter().map(SharedString::from).collect())),
                        });
                        this.show_page(same_columns, reload, cx);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The table's structure arrived (or was read again): which columns take
    /// NULL or have a DEFAULT, enum values, foreign keys and the size for the
    /// footer. Until it does, NULL is offered everywhere and the server has
    /// the last word.
    fn follow_structure(&mut self, source: Entity<StructureSource>, cx: &mut Context<Self>) {
        let structure = source.read(cx).structure().cloned();
        if structure.is_some() && structure != self.structure {
            self.structure = structure;
            self.refresh_grid(None, cx);
        }
    }

    /// A page arrived: the grid (or the documents) shows it, nothing picked,
    /// the editors closed.
    fn show_page(&mut self, same_columns: bool, reload: bool, cx: &mut Context<Self>) {
        self.editor = None;
        self.menu = None;
        self.reset_documents(reload);
        let columns = self.columns().to_vec();
        self.grid.update(cx, |grid, cx| {
            if !same_columns {
                grid.set_data(columns, Rc::default(), cx);
            }
            grid.selection_mut(cx).clear();
            if !reload {
                grid.scroll_to_top(cx);
            }
        });
        self.refresh_grid(None, cx);
    }

    /// The filter the rows are shown with.
    pub fn filter(&self) -> &RowFilter {
        &self.filter
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if !self.loading && !self.edits.has_edits() {
            self.load(true, cx);
        }
    }

    fn set_filter(&mut self, filter: RowFilter, window: &mut Window, cx: &mut Context<Self>) {
        if self.edits.has_edits() {
            return;
        }
        self.filter = filter.clone();
        self.offset = 0;
        self.filter_bar.update(cx, |bar, cx| bar.set_applied(filter, cx));
        self.load(false, cx);
        self.focus(window, cx);
    }

    pub(super) fn narrow(&mut self, filter: RowFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.set_filter(filter, window, cx);
    }

    fn page_by(&mut self, forward: bool, cx: &mut Context<Self>) {
        let size = u64::from(self.page_size);
        self.offset = if forward { self.offset + size } else { self.offset.saturating_sub(size) };
        self.load(false, cx);
    }

    // The rows as shown

    /// The rows as the grid shows them, new ones first.
    pub(super) fn row_refs(&self) -> Vec<RowRef> {
        let inserted = self.edits.inserts.iter().map(|row| RowRef::New(row.id));
        inserted.chain((0..self.rows().len()).map(RowRef::Loaded)).collect()
    }

    pub(super) fn row_ref(&self, shown: usize) -> Option<RowRef> {
        let inserts = self.edits.inserts.len();
        if shown < inserts {
            return Some(RowRef::New(self.edits.inserts[shown].id));
        }
        (shown - inserts < self.rows().len()).then_some(RowRef::Loaded(shown - inserts))
    }

    /// The key of a loaded row among the edits.
    pub(super) fn loaded_key(&self, index: usize) -> Option<&str> {
        self.page.as_ref()?.keys.get(index).map(String::as_str)
    }

    /// Shows the edits in the grid. `before` is the rows as shown before
    /// the edits changed, so the selection follows its rows.
    pub(super) fn refresh_grid(&mut self, before: Option<Vec<RowRef>>, cx: &mut Context<Self>) {
        let Some(page) = &self.page else { return };
        let refs = self.row_refs();
        let columns = &page.columns;
        let foreign_keys = self.structure.as_ref().map(|s| s.foreign_keys.as_slice()).unwrap_or_default();
        let key_of_column: Vec<Option<usize>> = columns
            .iter()
            .map(|column| foreign_keys.iter().position(|key| key.columns.contains(&column.name)))
            .collect();
        let mut rows = Vec::with_capacity(refs.len());
        let mut marks = GridMarks {
            computed_badge: cx.messages().table.computed_badge,
            sortable: !self.edits.has_edits(),
            columns: columns
                .iter()
                .map(|column| ColumnMark {
                    key: column.primary_key,
                    generated: column.generated,
                    sort: sort_of(self.engine, &column.name, &self.filter.order).map(|direction| match direction {
                        Direction::Asc => SortMark::Asc,
                        Direction::Desc => SortMark::Desc,
                    }),
                })
                .collect(),
            ..GridMarks::default()
        };
        for (shown, row) in refs.iter().enumerate() {
            match *row {
                RowRef::New(id) => {
                    let Some(new) = self.edits.inserts.iter().find(|new| new.id == id) else { continue };
                    rows.push(columns.iter().map(|column| new.values.get(&column.name).cloned().flatten()).collect());
                    marks.row_labels.push("+".into());
                    marks.rows.push(RowState::Inserted);
                    for (index, column) in columns.iter().enumerate() {
                        let set = new.values.contains_key(&column.name);
                        marks.cells.insert(Cell::new(shown, index), CellMark { edited: set, unset: !set });
                    }
                }
                RowRef::Loaded(index) => {
                    let loaded = &page.rows[index];
                    let id = &page.keys[index];
                    let deleted = self.edits.is_deleted(id);
                    let changed = self.edits.updates.get(id).filter(|_| !deleted);
                    let mut values = loaded.clone();
                    for (column_index, column) in columns.iter().enumerate() {
                        let cell = Cell::new(shown, column_index);
                        match changed.and_then(|update| update.values.get(&column.name)) {
                            Some(value) => {
                                values[column_index] = match value {
                                    CellValue::Text(text) => Some(text.clone()),
                                    CellValue::Null | CellValue::Default => None,
                                };
                                let unset = *value == CellValue::Default;
                                marks.cells.insert(cell, CellMark { edited: true, unset });
                            }
                            // Only loaded values link: an edited one may not exist in the referenced table yet.
                            None => {
                                let linked = key_of_column[column_index].is_some_and(|key| {
                                    let named: HashMap<&str, Option<&str>> = columns
                                        .iter()
                                        .zip(loaded)
                                        .map(|(column, value)| (column.name.as_str(), value.as_deref()))
                                        .collect();
                                    link_filter(self.engine, &foreign_keys[key], &named).is_some()
                                });
                                if linked {
                                    marks.links.insert(cell);
                                }
                            }
                        }
                    }
                    rows.push(values);
                    let number = self.offset + index as u64 + 1;
                    marks.row_labels.push(if deleted { "−".into() } else { number.to_string() });
                    marks.rows.push(if deleted { RowState::Deleted } else { RowState::Loaded });
                }
            }
        }
        // The gutter is as wide as the page's last number, whatever is pending, so nothing shifts.
        marks.gutter_chars = (self.offset + page.rows.len().max(1) as u64).to_string().len();
        let shown: HashMap<RowRef, usize> = refs.iter().enumerate().map(|(index, row)| (*row, index)).collect();
        self.grid.update(cx, |grid, cx| {
            if let Some(before) = before {
                grid.selection_mut(cx).remap(|index| before.get(index).and_then(|row| shown.get(row).copied()));
            }
            grid.update_rows(Rc::new(rows), Some(Rc::new(marks)), cx);
        });
        cx.notify();
    }

    // Changing cells

    /// The cells a change to `cell` reaches: every selected cell when it is
    /// one of several selected, else itself.
    pub(super) fn targets_of(&self, cell: Cell, cx: &gpui::App) -> Vec<Cell> {
        let selection = self.grid.read(cx).selection();
        if selection.cells().len() > 1 && selection.contains(cell) {
            selection.cells().iter().copied().collect()
        } else {
            vec![cell]
        }
    }

    /// Whether `cell` can change: an editable table, a column the database
    /// doesn't compute, a row not marked for deletion. `None` when it can,
    /// else why not.
    pub(super) fn locked(&self, cell: Cell, cx: &gpui::App) -> Option<&'static str> {
        let m = &cx.messages().table;
        let column = self.columns().get(cell.column)?;
        let row = self.row_ref(cell.row)?;
        if !self.editable() && matches!(row, RowRef::Loaded(_)) {
            return Some(m.no_key);
        }
        if self.applying() {
            return Some(m.applying_changes);
        }
        if column.generated {
            return Some(m.computed);
        }
        match row {
            RowRef::Loaded(index) if self.loaded_key(index).is_some_and(|id| self.edits.is_deleted(id)) => {
                Some(m.row_deleted)
            }
            _ => None,
        }
    }

    /// Puts `value` in each of `cells`; computed columns and rows marked for deletion keep theirs.
    pub(super) fn set_cells(&mut self, cells: &[Cell], value: CellValue, cx: &mut Context<Self>) {
        let before = self.row_refs();
        for cell in cells {
            let (Some(row), Some(column)) = (self.row_ref(cell.row), self.columns().get(cell.column).cloned()) else {
                continue;
            };
            if column.generated {
                continue;
            }
            match row {
                RowRef::New(id) => match &value {
                    CellValue::Default => self.edits.unset_new_cell(id, &column.name),
                    CellValue::Null => self.edits.edit_new_cell(id, &column.name, None),
                    CellValue::Text(text) => self.edits.edit_new_cell(id, &column.name, Some(text.clone())),
                },
                RowRef::Loaded(index) => {
                    let Some(page) = &self.page else { continue };
                    if self.edits.is_deleted(&page.keys[index]) {
                        continue;
                    }
                    let loaded = &page.rows[index];
                    let key = key_of(loaded, &page.columns);
                    let original = loaded.get(cell.column).cloned().flatten();
                    self.edits.edit_cell(&key, &column.name, value.clone(), original.as_deref());
                }
            }
        }
        self.changed(Some(before), cx);
    }

    /// The edits changed: the grid and the documents show them and an old outcome goes.
    pub(super) fn changed(&mut self, before: Option<Vec<RowRef>>, cx: &mut Context<Self>) {
        if self.status != Status::Idle && self.status != Status::Applying {
            self.status = Status::Idle;
        }
        self.remeasure_documents();
        self.refresh_grid(before, cx);
    }

    /// The bar's +: a new document while documents show, else a new row.
    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.showing_documents() {
            self.new_document(window, cx);
        } else {
            self.add_row(cx);
        }
    }

    pub(super) fn add_row(&mut self, cx: &mut Context<Self>) {
        if !self.editable() || self.applying() {
            return;
        }
        let before = self.row_refs();
        self.edits.add_row();
        self.changed(Some(before), cx);
    }

    /// A new row with a loaded row's values; it gets its own key and lets
    /// the database compute what it computes.
    pub(super) fn duplicate_row(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(page) = &self.page else { return };
        let Some(loaded) = page.rows.get(index) else { return };
        let values: BTreeMap<String, Option<String>> = page
            .columns
            .iter()
            .zip(loaded)
            .filter(|(column, _)| !column.primary_key && !column.generated)
            .map(|(column, value)| (column.name.clone(), value.clone()))
            .collect();
        let before = self.row_refs();
        self.edits.add_row_with(values);
        self.changed(Some(before), cx);
    }

    /// Marks a loaded row for deletion, or keeps it again.
    pub(super) fn toggle_delete(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(page) = &self.page else { return };
        let Some(loaded) = page.rows.get(index) else { return };
        let key = key_of(loaded, &page.columns);
        self.edits.toggle_delete(&key);
        self.changed(None, cx);
    }

    /// Marks the selected rows (or documents) for deletion, or unmarks
    /// them; new ones are simply removed.
    pub(super) fn delete_selected(&mut self, cx: &mut Context<Self>) {
        if !self.editable() || self.applying() {
            return;
        }
        if self.showing_documents() {
            return self.delete_selected_documents(cx);
        }
        let rows: Vec<RowRef> =
            self.grid.read(cx).selection().rows().into_iter().filter_map(|row| self.row_ref(row)).collect();
        let before = self.row_refs();
        for row in rows {
            match row {
                RowRef::New(id) => self.edits.remove_new_row(id),
                RowRef::Loaded(index) => {
                    if let Some(page) = &self.page {
                        let key = key_of(&page.rows[index], &page.columns);
                        self.edits.toggle_delete(&key);
                    }
                }
            }
        }
        self.grid.update(cx, |grid, cx| grid.selection_mut(cx).clear());
        self.changed(Some(before), cx);
    }

    /// ⌘V: the clipboard's text in the picked cell (and the others selected with it).
    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(cell) = self.grid.read(cx).selection().cursor() else { return };
        if self.locked(cell, cx).is_some() {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
        let targets = self.targets_of(cell, cx);
        self.set_cells(&targets, CellValue::Text(text), cx);
    }

    /// Escape: lets go of the picked cell and the selection.
    fn deselect(&mut self, cx: &mut Context<Self>) {
        self.grid.update(cx, |grid, cx| grid.selection_mut(cx).clear());
    }

    pub(super) fn copy(&self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    // Applying

    /// ⌘S or Apply: on a connection whose tag asks first, the question; else at once.
    pub fn request_apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.edits.has_edits() || self.applying() {
            return;
        }
        match confirming_tag_of(cx, &self.connection_id) {
            Some(tag) => self.confirm(tag, window, cx),
            None => self.apply(cx),
        }
    }

    fn confirm(&mut self, tag: String, window: &mut Window, cx: &mut Context<Self>) {
        let table = format!("{}.{}", self.table.schema, self.table.name);
        let counts = self.edits.counts();
        let dialog = cx.new(|cx| ConfirmDialog::new(tag, table, counts, cx));
        dialog.read(cx).focus(window);
        let apply = cx.subscribe_in(&dialog, window, |this, _, ConfirmEvent::Apply, window, cx| {
            this.close_dialog(window, cx);
            this.apply(cx);
        });
        let dismiss = cx.subscribe_in(&dialog, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_dialog(window, cx);
        });
        self.dialog = vec![apply, dismiss];
        cx.emit(TableEvent::Dialog(Some(dialog.into())));
    }

    fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog.clear();
        cx.emit(TableEvent::Dialog(None));
        self.focus(window, cx);
    }

    /// Applies every change in one transaction, or none.
    fn apply(&mut self, cx: &mut Context<Self>) {
        self.status = Status::Applying;
        self.refresh_grid(None, cx);
        let (connection, table, changes) = (self.connection_id.clone(), self.table.clone(), self.edits.to_changes());
        let applying = Backend::global(cx).spawn(cx, move |core| async move {
            apply_changes(core.connections(), core.statement_log(), connection, table, changes).await
        });
        cx.spawn(async move |this, cx| {
            let outcome = applying.await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(summary) => {
                        this.edits = Edits::default();
                        this.status = Status::Applied(summary);
                        this.load(true, cx);
                    }
                    Err(error) => this.status = Status::Failed(error.to_string()),
                }
                this.refresh_grid(None, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// The SQL the changes would run, in a dialog.
    pub fn preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (connection, table, changes) = (self.connection_id.clone(), self.table.clone(), self.edits.to_changes());
        let previewing = Backend::global(cx).spawn(cx, move |core| async move {
            preview_changes(core.connections(), core.statement_log(), connection, table, changes).await
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let outcome = previewing.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| match outcome {
                    Ok(statements) => this.show_preview(statements, window, cx),
                    Err(error) => {
                        this.status = Status::Failed(error.to_string());
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }

    fn show_preview(&mut self, statements: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let note = preview_note(self.engine, cx);
        // The changes as the engine's console writes them: mongosh for MongoDB.
        let syntax = Syntax::console(self.engine);
        let dialog = cx.new(|cx| PreviewDialog::new(&statements, syntax, note, cx));
        dialog.read(cx).focus(window);
        let dismiss = cx.subscribe_in(&dialog, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_dialog(window, cx);
        });
        self.dialog = vec![dismiss];
        cx.emit(TableEvent::Dialog(Some(dialog.into())));
    }

    pub fn discard(&mut self, cx: &mut Context<Self>) {
        let before = self.row_refs();
        self.edits = Edits::default();
        self.editor = None;
        self.documents.editor = None;
        self.documents.selected.clear();
        self.remeasure_documents();
        self.status = Status::Idle;
        self.grid.update(cx, |grid, cx| grid.selection_mut(cx).clear());
        self.refresh_grid(Some(before), cx);
    }

    // The grid's mouse

    fn on_grid(&mut self, _: &Entity<DataGrid>, event: &GridEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            GridEvent::Edit(cell) => self.open_editor(*cell, window, cx),
            GridEvent::Menu(cell, position) => self.open_menu(*cell, *position, window, cx),
            GridEvent::Sort(column) => {
                if self.edits.has_edits() {
                    return;
                }
                let Some(name) = self.columns().get(*column).map(|column| column.name.clone()) else { return };
                let filter = sort_by(self.engine, &name, &self.filter);
                self.set_filter(filter, window, cx);
            }
            GridEvent::Follow(cell) => self.follow(*cell, cx),
        }
    }

    /// Opens the row a loaded cell's foreign key points to.
    fn follow(&mut self, cell: Cell, cx: &mut Context<Self>) {
        let Some(RowRef::Loaded(index)) = self.row_ref(cell.row) else { return };
        let (Some(page), Some(structure)) = (&self.page, &self.structure) else { return };
        let Some(column) = page.columns.get(cell.column) else { return };
        let Some(key) = structure.foreign_keys.iter().find(|key| key.columns.contains(&column.name)) else { return };
        let named: HashMap<&str, Option<&str>> =
            page.columns.iter().zip(&page.rows[index]).map(|(c, v)| (c.name.as_str(), v.as_deref())).collect();
        if let Some(filter) = link_filter(self.engine, key, &named) {
            let table = super::links::referenced_table(&self.table, &key.referenced_table);
            cx.emit(TableEvent::OpenTable { table, filter });
        }
    }

    // The inspector

    pub fn toggle_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.inspector.take().is_some() {
            self.focus(window, cx);
        } else {
            let cell = self.inspected(cx);
            let inspector = cx.new(|cx| CellInspector::new(cell, window, cx));
            self._subscriptions.push(cx.subscribe_in(&inspector, window, Self::on_inspector));
            self.inspector = Some(inspector);
        }
        cx.notify();
    }

    pub fn inspector(&self) -> Option<&Entity<CellInspector>> {
        self.inspector.as_ref()
    }

    fn on_inspector(
        &mut self,
        _: &Entity<CellInspector>,
        event: &InspectorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InspectorEvent::Save(value) => {
                let Some(cell) = self.grid.read(cx).selection().cursor() else { return };
                let targets = self.targets_of(cell, cx);
                self.set_cells(&targets, value.clone().into(), cx);
            }
            InspectorEvent::Close => {
                self.inspector = None;
                self.focus(window, cx);
                cx.notify();
            }
        }
    }

    /// The picked cell as the inspector shows it.
    fn inspected(&self, cx: &gpui::App) -> Option<InspectedCell> {
        let cell = self.grid.read(cx).selection().cursor()?;
        let row = self.row_ref(cell.row)?;
        let column = self.columns().get(cell.column)?.clone();
        let shown = self.edits.cell_at(self.rows(), self.columns(), row, cell.column)?;
        let info = self.structure.as_ref().and_then(|s| s.columns.iter().find(|info| info.name == column.name));
        Some(InspectedCell {
            locked: self.locked(cell, cx),
            // Until the structure loads, NULL is offered; the server has the last word.
            nullable: info.is_none_or(|info| info.nullable),
            options: info.map(|info| info.options.clone()).unwrap_or_default(),
            value: shown.value,
            unset: shown.unset,
            column,
        })
    }

    /// Another cell picked, or its value changed: the inspector starts a fresh draft.
    fn follow_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inspector) = self.inspector.clone() else { return };
        let cell = self.inspected(cx);
        if inspector.read(cx).cell() != cell.as_ref() {
            inspector.update(cx, |inspector, cx| inspector.show(cell, window, cx));
        }
    }

    // Drawing

    /// The row actions, for the page's top bar: the documents' switch (on
    /// a document database), add, delete, reload, inspect.
    pub fn actions(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = *cx.theme();
        let m = &cx.messages().table;
        let editable = self.editable();
        let pending = self.edits.has_edits();
        let documents = self.showing_documents();
        let has_selection =
            if documents { !self.documents.selected.is_empty() } else { !self.grid.read(cx).selection().is_empty() };
        let action = |id: &'static str, name: &'static str, label: &'static str, on: bool, active: bool| {
            icon_button(&theme, id, name, on, active).tooltip(Tooltip::text(label))
        };
        let label = |text: &'static str| if editable { text } else { m.no_key };
        let switch = self.has_documents().then(|| {
            let label = if pending {
                m.switch_views_locked
            } else if documents {
                m.show_as_table
            } else {
                m.show_as_documents
            };
            action("toggle-documents", "code", label, !pending, documents)
                .when(!pending, |switch| {
                    switch.on_click(cx.listener(|this, _, window, cx| this.toggle_documents(window, cx)))
                })
                .into_any_element()
        });
        let add_label = label(if documents { m.add_document } else { m.add_row });
        let add = action("add-row", "plus", add_label, editable && !self.applying(), false);
        let delete = action(
            "delete-rows",
            "trash",
            label(m.delete_selected),
            editable && has_selection && !self.applying(),
            false,
        );
        let reload = action("reload-rows", "refresh", m.reload_rows, !self.loading && !pending, false);
        let inspect_label = if documents { m.inspect_in_table } else { m.inspect_cell };
        let inspect = action("inspect-cell", "view", inspect_label, !documents, self.inspector.is_some() && !documents);
        switch
            .into_iter()
            .chain([
                add.when(editable && !self.applying(), |add| {
                    add.on_click(cx.listener(|this, _, window, cx| this.add(window, cx)))
                })
                .into_any_element(),
                delete
                    .when(editable && has_selection && !self.applying(), |delete| {
                        delete.on_click(cx.listener(|this, _, _, cx| this.delete_selected(cx)))
                    })
                    .into_any_element(),
                reload
                    .when(!self.loading && !pending, |reload| {
                        reload.on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                    })
                    .into_any_element(),
                inspect
                    .when(!documents, |inspect| {
                        inspect.on_click(cx.listener(|this, _, window, cx| this.toggle_inspector(window, cx)))
                    })
                    .into_any_element(),
            ])
            .collect()
    }

    fn footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().table;
        let pending = self.edits.has_edits();
        let counts = self.edits.counts();
        let lock_paging = self.loading || pending;
        let text = |text: String, tint| div().min_w_0().ellipsis().text_size(px(12.)).text_color(tint).child(text);
        let mut footer = div()
            .h(px(46.))
            .flex_none()
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_t_1()
            .border_color(theme.line);
        if pending {
            footer = footer
                .child(pill(&(m.pending_changes)(counts.total()), theme.warn).flex_none())
                .child(text((m.change_counts)(counts.updated, counts.inserted, counts.deleted), theme.text2))
                .child(
                    enabled(ghost_button(&theme, m.preview_sql), !self.applying())
                        .id("preview-sql")
                        .when(!self.applying(), |preview| {
                            preview.on_click(cx.listener(|this, _, window, cx| this.preview(window, cx)))
                        }),
                );
        }
        match &self.status {
            Status::Applied(summary) if !pending => {
                footer = footer.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(theme.ok)
                        .child(icon("check", theme.ok, 13.))
                        .child((m.applied)(summary.updated, summary.inserted, summary.deleted)),
                );
            }
            Status::Failed(error) => {
                footer = footer.child(
                    div()
                        .id("apply-error")
                        .min_w_0()
                        .flex_shrink()
                        .ellipsis()
                        .text_size(px(12.))
                        .text_color(theme.err)
                        .tooltip(Tooltip::text(error.clone()))
                        .child(error.clone()),
                );
            }
            Status::Idle if !pending => {
                let summary = self.structure.as_ref().map(|s| summary(s, m)).unwrap_or_default();
                footer = footer.child(text(summary, theme.text3));
            }
            _ => {}
        }
        if self.showing_documents() {
            let selected = self.documents.selected.len();
            if selected > 1 {
                footer = footer.child(pill(&(m.documents_selected)(selected), theme.accent).flex_none());
            }
        } else {
            let selected = self.grid.read(cx).selection().cells().len();
            if selected > 1 {
                footer = footer.child(pill(&(m.selection_hint)(selected), theme.accent).flex_none());
            }
        }
        let rows = self.rows().len() as u64;
        let range = if self.loading {
            m.loading.to_owned()
        } else if rows == 0 {
            if is_filtered(&self.filter) { m.no_rows_match } else { m.no_rows }.to_owned()
        } else {
            (m.rows_range)(self.offset + 1, self.offset + rows, is_filtered(&self.filter))
        };
        self.page_sizes.update(cx, |select, cx| select.set_disabled(lock_paging, cx));
        let has_previous = self.offset > 0 && !lock_paging;
        let has_next = self.page.as_ref().is_some_and(|page| page.has_more) && !lock_paging;
        let pager = |id: &'static str, name: &'static str, label: &'static str, on: bool| {
            enabled(
                div().size(px(28.)).flex_none().flex().items_center().justify_center().rounded(px(14.)).child(icon(
                    name,
                    theme.text2,
                    14.,
                )),
                on,
            )
            .id(id)
            .tooltip(Tooltip::text(label))
            .when(on, |button| button.hover(|button| button.bg(theme.sel)))
        };
        footer =
            footer
                .child(div().flex_1())
                .child(text(range, theme.text3).flex_none())
                .child(div().w(px(130.)).flex_none().child(self.page_sizes.clone()))
                .child(pager("previous-page", "left", m.previous_page, has_previous).when(has_previous, |button| {
                    button.on_click(cx.listener(|this, _, _, cx| this.page_by(false, cx)))
                }))
                .child(
                    pager("next-page", "right", m.next_page, has_next)
                        .when(has_next, |button| button.on_click(cx.listener(|this, _, _, cx| this.page_by(true, cx)))),
                );
        if pending {
            let applying = self.applying();
            footer = footer
                .child(div().w(px(1.)).h(px(20.)).bg(theme.line))
                .child(
                    enabled(button(&theme, None, m.discard), !applying)
                        .id("discard")
                        .when(!applying, |discard| discard.on_click(cx.listener(|this, _, _, cx| this.discard(cx)))),
                )
                .child(
                    enabled(primary_button(&theme, if applying { m.applying } else { m.apply }), !applying)
                        .id("apply")
                        .child(div().text_color(crate::theme::color(0xFFFFFFCC)).child(super::shortcuts::APPLY))
                        .when(!applying, |apply| {
                            apply.on_click(cx.listener(|this, _, window, cx| this.request_apply(window, cx)))
                        }),
                );
        }
        footer.into_any_element()
    }

    fn content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        if let Some(error) = &self.error {
            return div()
                .id("rows-error")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(16.))
                .text_size(px(12.5))
                .text_color(theme.err)
                .child(error.clone())
                .into_any_element();
        }
        if self.showing_documents() {
            return self.render_documents(cx);
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when(self.page.is_some(), |content| content.child(self.grid.clone()))
            .into_any_element()
    }
}

#[cfg(test)]
impl TableView {
    /// Shows `rows` as if they had been read; a read still going is dropped.
    pub(super) fn show_rows(&mut self, columns: Vec<Column>, rows: Vec<Vec<Option<String>>>, cx: &mut Context<Self>) {
        self.request += 1;
        self.loading = false;
        self.error = None;
        let keys = row_keys(&rows, &columns);
        self.page = Some(Page { columns, rows: Rc::new(rows), has_more: true, keys, documents: None });
        self.show_page(false, false, cx);
    }

    /// Shows `rows` with their `documents` as if they had been read.
    pub(super) fn show_documents_page(
        &mut self,
        columns: Vec<Column>,
        rows: Vec<Vec<Option<String>>>,
        documents: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.request += 1;
        self.loading = false;
        self.error = None;
        let keys = row_keys(&rows, &columns);
        let documents = Some(Rc::new(documents.into_iter().map(SharedString::from).collect()));
        self.page = Some(Page { columns, rows: Rc::new(rows), has_more: false, keys, documents });
        self.show_page(false, false, cx);
    }

    pub(super) fn set_connection_for_test(&mut self, connection_id: String) {
        self.connection_id = connection_id;
    }

    pub(super) fn is_applying(&self) -> bool {
        self.applying()
    }

    /// Types `text` in the open text editor, as the keyboard would.
    pub(super) fn type_in_editor(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(super::editors::OpenEditor { kind: super::editors::EditorKind::Text(input), .. }) = &self.editor {
            input.update(cx, |input, cx| {
                input.select_all(cx);
                input.insert(text, cx);
            });
        }
    }
}

/// How the changes run: one transaction that applies all or nothing, except where noted.
fn preview_note(engine: Engine, cx: &gpui::App) -> &'static str {
    let m = &cx.messages().table;
    match engine {
        Engine::Mongodb => m.preview_note_mongodb,
        Engine::Cosmosdb => m.preview_note_cosmosdb,
        _ => m.preview_note,
    }
}

impl Render for TableView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.edits.has_edits();
        self.filter_bar.update(cx, |bar, cx| bar.set_locked(pending, window, cx));
        #[cfg(debug_assertions)]
        if self.seed.is_some() {
            cx.defer_in(window, |this, window, cx| this.run_seed(window, cx));
        }
        let editor = self.render_editor(window, cx);
        div()
            .id("table-view")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Apply, window, cx| {
                if this.editor.is_some() {
                    // Saves the cell first; a second ⌘S applies everything.
                    this.finish_editor(window, cx);
                } else if this.documents.editor.is_some() {
                    // Keeps the document first, once the database can read it.
                    this.keep_document(window, cx);
                } else {
                    this.request_apply(window, cx);
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .child(self.filter_bar.clone())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .on_action(cx.listener(|this, _: &Inspect, window, cx| this.toggle_inspector(window, cx)))
                            .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
                            .on_action(cx.listener(|this, _: &Deselect, _, cx| this.deselect(cx)))
                            .when(self.loading && self.page.is_some(), |content| content.opacity(0.75))
                            .child(self.content(cx)),
                    )
                    .children(self.inspector.clone().filter(|_| !self.showing_documents())),
            )
            .child(self.footer(cx))
            .children(editor)
            .children(self.menu.as_ref().map(|(menu, position, _)| popover(menu.clone(), *position)))
    }
}

/// The table's columns, by name, for tests and the menu.
pub(super) fn column_names(columns: &[Column]) -> Vec<String> {
    columns.iter().map(|column| column.name.clone()).collect()
}

/// Whether the engine's rows copy as an INSERT.
pub(super) fn copies_inserts(engine: Engine) -> bool {
    is_relational(engine)
}
