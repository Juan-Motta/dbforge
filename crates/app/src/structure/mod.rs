//! A table's structure: its columns, indexes, foreign keys, triggers and the
//! DDL that rebuilds it, each a view of the table's tab. The structure is
//! read once per tab ([`StructureSource`]) and shared with the rows, whose
//! footer, editors and links need it too.

pub mod format;
pub mod sections;
mod source;
mod view;

pub use source::StructureSource;
pub use view::StructureView;
