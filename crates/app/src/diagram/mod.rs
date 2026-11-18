//! ER diagrams: a schema's tables as cards with their columns and keys, and
//! the foreign keys between them as curves; panned by dragging, zoomed with
//! the wheel or the buttons, a card moved by dragging it. A schema's diagram
//! is a tab of its own; a table's tab shows the table with its neighbours.

mod canvas;
pub mod layout;
#[cfg(test)]
mod tests;
mod view;

pub use view::{Card, CardColumn, ColumnKey, DiagramEvent, DiagramView, Line, Scene};
