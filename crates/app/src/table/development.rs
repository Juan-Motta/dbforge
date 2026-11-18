//! Development builds only: a table's tab set up for a screenshot without
//! clicking (`DBFORGE_OPEN=table:…#edits`), once its first rows are drawn.

use dbforge_core::drivers::CellValue;
use gpui::{Context, Window, point, px};

use super::view::TableView;
use crate::grid::selection::Cell;

impl TableView {
    /// What to show once the rows arrive: `edits`, `review`, `prod`,
    /// `inspect`, `menu`, `edit` or `date`; on a document database,
    /// `documents`, `document-edits`, `document-edit` or `document-menu`.
    pub fn seed_for_development(&mut self, seed: String) {
        self.seed = Some(seed);
    }

    /// Runs the seed once the grid has drawn the rows; until then, waits a frame.
    pub(super) fn run_seed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page.is_none() || self.is_loading() || self.seed.is_none() {
            return;
        }
        if self.seed.as_deref().is_some_and(|seed| seed.starts_with("document")) {
            let Some(seed) = self.seed.take() else { return };
            return self.seed_documents(&seed, window, cx);
        }
        if self.grid.read(cx).cell_bounds(Cell::new(0, 0)).is_none() {
            cx.notify();
            return;
        }
        let Some(seed) = self.seed.take() else { return };
        let columns = self.columns().to_vec();
        // The first column the user can change, and a date or time one.
        let plain = columns.iter().position(|c| !c.primary_key && !c.generated).unwrap_or(0);
        let date = columns.iter().position(|c| {
            super::temporal::temporal_kind_of(&c.data_type).is_some()
                || c.name.contains("date")
                || c.name.contains("_at")
        });
        let with_edits = matches!(seed.as_str(), "edits" | "review" | "prod" | "inspect");
        if with_edits {
            self.set_cells(&[Cell::new(0, plain)], CellValue::Text("Edited value".into()), cx);
            let nullable = columns.iter().rposition(|c| !c.primary_key && !c.generated).unwrap_or(plain);
            self.set_cells(&[Cell::new(1, nullable)], CellValue::Null, cx);
            self.toggle_delete(2, cx);
            self.add_row(cx);
            self.set_cells(&[Cell::new(0, plain)], CellValue::Text("New row".into()), cx);
        }
        let pick = |view: &mut Self, cell: Cell, cx: &mut Context<Self>| {
            view.grid.update(cx, |grid, cx| grid.pick(cell, cx));
        };
        match seed.as_str() {
            "review" => self.preview(window, cx),
            "prod" => self.request_apply(window, cx),
            "inspect" => {
                pick(self, Cell::new(2, plain), cx);
                self.toggle_inspector(window, cx);
            }
            "menu" => {
                let cell = Cell::new(1, plain);
                pick(self, cell, cx);
                if let Some(bounds) = self.grid.read(cx).cell_bounds(cell) {
                    self.open_menu(cell, bounds.origin + point(px(24.), px(14.)), window, cx);
                }
            }
            "edit" => {
                let cell = Cell::new(1, plain);
                pick(self, cell, cx);
                self.open_editor(cell, window, cx);
            }
            "date" => {
                let cell = Cell::new(1, date.unwrap_or(plain));
                pick(self, cell, cx);
                self.open_editor(cell, window, cx);
            }
            _ => {}
        }
    }
}
