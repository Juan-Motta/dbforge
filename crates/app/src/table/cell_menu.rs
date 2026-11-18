//! A cell's right-click menu: copying, filtering and sorting always; the
//! changes its column and row allow on a table that can be edited. With
//! several cells selected, this one among them, a change reaches them all.
//! The web UI's submenus are groups here, in the same order: Edit, Copy,
//! Paste, Set, Filter, Row, Revert.

use dbforge_core::drivers::CellValue;
use gpui::{AppContext, Context, DismissEvent, Pixels, Point, SharedString, Window};

use super::cell_actions::{
    ValueTest, filters_by_value, narrowed_by, row_as_insert, row_as_json, sorted_by, value_condition,
};
use super::edits::RowRef;
use super::filter::Direction;
use super::temporal::{format_temporal, now, offset_space, temporal_kind_of};
use super::view::{TableView, column_names, copies_inserts};
use crate::grid::columns::is_uuid_type;
use crate::grid::selection::Cell;
use crate::i18n::Translate;
use crate::ui::{MenuItem, PopoverMenu};

/// What a menu item does to the table's rows.
type Action = Box<dyn Fn(&mut TableView, &mut Window, &mut Context<TableView>)>;

impl TableView {
    /// The row as it stands, edits included; DEFAULT reads as NULL.
    fn current_row(&self, row: RowRef) -> Vec<Option<String>> {
        (0..self.columns().len())
            .map(|column| self.edits.cell_at(self.rows(), self.columns(), row, column).and_then(|shown| shown.value))
            .collect()
    }

    pub(super) fn open_menu(
        &mut self,
        cell: Cell,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(items) = self.menu_items(cell, cx) else { return };
        let menu = cx.new(|cx| PopoverMenu::new(items, window, cx));
        let closing = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.menu = None;
            this.focus(window, cx);
            cx.notify();
        });
        self.menu = Some((menu, position, closing));
        cx.notify();
    }

    fn menu_items(&self, cell: Cell, cx: &mut Context<Self>) -> Option<Vec<MenuItem>> {
        let m = &cx.messages().table;
        let row = self.row_ref(cell.row)?;
        let column = self.columns().get(cell.column)?.clone();
        let shown = self.edits.cell_at(self.rows(), self.columns(), row, cell.column)?;
        let engine = self.engine;
        let names = column_names(self.columns());
        let this = cx.entity().downgrade();
        // Runs `act` on the view when the item is chosen.
        let on = |act: Action| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut gpui::App| {
                let _ = this.update(cx, |view, cx| act(view, window, cx));
            }
        };
        let item = |label: String, group: Option<SharedString>, act: Action| MenuItem::new(label, on(act)).group(group);
        let copy = |text: String| -> Action { Box::new(move |view, _, cx| view.copy(text.clone(), cx)) };

        let selection = self.grid.read(cx).selection();
        let across = if selection.cells().len() > 1 && selection.contains(cell) { selection.cells().len() } else { 0 };
        let selected_rows: Vec<Vec<Option<String>>> =
            selection.rows().into_iter().filter_map(|row| self.row_ref(row)).map(|row| self.current_row(row)).collect();
        let for_all = |label: &str| if across > 0 { (m.across_cells)(label, across) } else { label.to_owned() };

        let mut copies = Vec::new();
        let group = Some(SharedString::from(m.copy_menu));
        if across > 0 {
            let tsv = self.grid.read(cx).selection_as_tsv();
            copies.push(item((m.copy_selected_cells)(across), group.clone(), copy(tsv)));
            let json = format!(
                "[{}]",
                selected_rows.iter().map(|values| row_as_json(&names, values)).collect::<Vec<_>>().join(",\n")
            );
            copies.push(item((m.copy_selected_json)(selected_rows.len()), group.clone(), copy(json)));
            if copies_inserts(engine) {
                let inserts: Vec<String> =
                    selected_rows.iter().map(|values| row_as_insert(engine, &self.table, &names, values)).collect();
                copies.push(item(
                    (m.copy_selected_insert)(selected_rows.len()),
                    group.clone(),
                    copy(inserts.join("\n")),
                ));
            }
        }
        let cell_text = if shown.unset { "DEFAULT".to_owned() } else { shown.value.clone().unwrap_or("NULL".into()) };
        copies.push(item(m.copy_value.into(), group.clone(), copy(cell_text)));
        copies.push(item(m.copy_column_name.into(), group.clone(), copy(column.name.clone())));
        if filters_by_value(engine) {
            let condition = value_condition(engine, &column.name, ValueTest::Equal, shown.value.as_deref());
            copies.push(item(m.copy_condition.into(), group.clone(), copy(condition)));
        }
        let current = self.current_row(row);
        copies.push(item(m.copy_row_json.into(), group.clone(), copy(row_as_json(&names, &current))));
        if copies_inserts(engine) {
            copies.push(item(
                m.copy_row_insert.into(),
                group,
                copy(row_as_insert(engine, &self.table, &names, &current)),
            ));
        }

        // Filtering and sorting would hide pending rows: not while changes are pending.
        let mut filters = Vec::new();
        if !self.edits.has_edits() {
            let group = Some(SharedString::from(m.filter_menu));
            if filters_by_value(engine) {
                for (label, test) in [
                    (m.filter_equal, ValueTest::Equal),
                    (m.filter_not_equal, ValueTest::NotEqual),
                    (m.filter_is_null, ValueTest::IsNull),
                    (m.filter_is_not_null, ValueTest::IsNotNull),
                ] {
                    let condition = value_condition(engine, &column.name, test, shown.value.as_deref());
                    filters.push(item(
                        label.into(),
                        group.clone(),
                        Box::new(move |view, window, cx| {
                            let filter = narrowed_by(&view.filter, &condition);
                            view.narrow(filter, window, cx);
                        }),
                    ));
                }
            }
            for (label, direction) in [(m.sort_ascending, Direction::Asc), (m.sort_descending, Direction::Desc)] {
                let name = column.name.clone();
                filters.push(item(
                    label.into(),
                    group.clone(),
                    Box::new(move |view, window, cx| {
                        let filter = sorted_by(view.engine, &name, direction, &view.filter);
                        view.narrow(filter, window, cx);
                    }),
                ));
            }
        }

        if self.locked(cell, cx).is_some() {
            return Some(copies.into_iter().chain(filters).collect());
        }

        let set_value = |value: CellValue| -> Action {
            Box::new(move |view, _, cx| {
                let targets = view.targets_of(cell, cx);
                view.set_cells(&targets, value.clone(), cx);
            })
        };
        let info = self.structure.as_ref().and_then(|s| s.columns.iter().find(|info| info.name == column.name));
        let set_group = Some(SharedString::from(for_all(m.set_menu)));
        let mut sets = Vec::new();
        // Until the structure loads, nothing is assumed about NULL; the server has the last word.
        if info.is_none_or(|info| info.nullable) {
            sets.push(item("NULL".into(), set_group.clone(), set_value(CellValue::Null)));
        }
        if info.is_some_and(|info| info.default.is_some()) {
            sets.push(item("DEFAULT".into(), set_group.clone(), set_value(CellValue::Default)).disabled(shown.unset));
        }
        if let Some(kind) = temporal_kind_of(&column.data_type) {
            let label = if kind == super::temporal::TemporalKind::Date { m.today } else { m.now };
            let space = offset_space(&column.data_type);
            sets.push(item(
                label.into(),
                set_group.clone(),
                Box::new(move |view, _, cx| {
                    let targets = view.targets_of(cell, cx);
                    view.set_cells(&targets, CellValue::Text(format_temporal(&now(), kind, space)), cx);
                }),
            ));
        }
        if is_uuid_type(&column.data_type) {
            sets.push(item(
                m.generate_uuid.into(),
                set_group.clone(),
                Box::new(move |view, _, cx| {
                    let targets = view.targets_of(cell, cx);
                    view.set_cells(&targets, CellValue::Text(uuid::Uuid::new_v4().to_string()), cx);
                }),
            ));
        }

        let mut rows = Vec::new();
        if let RowRef::Loaded(index) = row {
            let group = Some(SharedString::from(m.row_menu));
            rows.push(item(
                m.duplicate_row.into(),
                group.clone(),
                Box::new(move |view, _, cx| view.duplicate_row(index, cx)),
            ));
            let deleted = shown.deleted;
            let label = if deleted { m.restore_row } else { m.delete_row };
            rows.push(item(label.into(), group, Box::new(move |view, _, cx| view.toggle_delete(index, cx))));
        }

        let mut items =
            vec![item(for_all(m.edit), None, Box::new(move |view, window, cx| view.open_editor(cell, window, cx)))];
        items.extend(copies);
        items.push(item(
            for_all(m.paste),
            None,
            Box::new(move |view, _, cx| {
                let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
                let targets = view.targets_of(cell, cx);
                view.set_cells(&targets, CellValue::Text(text), cx);
            }),
        ));
        items.extend(sets);
        items.extend(filters);
        items.extend(rows);
        if let RowRef::Loaded(_) = row {
            let edited = self.grid.read(cx).marks().is_some_and(|marks| marks.cell(cell).edited);
            let original = match row {
                RowRef::Loaded(index) => self.rows().get(index).and_then(|row| row.get(cell.column).cloned()).flatten(),
                RowRef::New(_) => None,
            };
            // Setting the loaded value back drops the change.
            items.push(
                item(
                    m.revert.into(),
                    None,
                    Box::new(move |view, _, cx| {
                        let targets = view.targets_of(cell, cx);
                        view.set_cells(&targets, original.clone().into(), cx);
                    }),
                )
                .disabled(!edited),
            );
        }
        Some(items)
    }
}
