//! A table's size as the rows' footer says it: "~48,213 rows · 9.4 MB data ·
//! 3.1 MB indexes". Views have no size.

use dbforge_core::drivers::{ObjectKind, TableStructure};

use crate::i18n::{TableMessages, thousands};

const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];

/// `bytes` in the largest unit under 1024, one decimal below 10.
pub fn format_bytes(bytes: i64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    let text = if unit > 0 && value < 10. { format!("{value:.1}") } else { format!("{}", value.round() as i64) };
    format!("{text} {}", UNITS[unit])
}

pub fn summary(structure: &TableStructure, m: &TableMessages) -> String {
    let mut parts = Vec::new();
    if let Some(rows) = structure.row_estimate {
        let rows = if rows < 0 { format!("-{}", thousands(rows.unsigned_abs())) } else { thousands(rows as u64) };
        parts.push((m.row_estimate)(&rows));
    }
    if structure.kind == ObjectKind::Table {
        parts.push((m.data_size)(&format_bytes(structure.data_bytes)));
        parts.push((m.index_size)(&format_bytes(structure.index_bytes)));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn structure(kind: ObjectKind, rows: Option<i64>) -> TableStructure {
        TableStructure {
            kind,
            row_estimate: rows,
            data_bytes: 9_856_000,
            index_bytes: 3_250_000,
            comment: None,
            columns: Vec::new(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            triggers: Vec::new(),
            ddl: String::new(),
        }
    }

    #[test]
    fn writes_sizes_in_the_largest_unit() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(9_856_000), "9.4 MB");
        assert_eq!(format_bytes(50 * 1024), "50 kB");
    }

    #[test]
    fn sums_up_a_table_and_a_view_in_the_language_in_use() {
        let en = &Language::En.messages().table;
        assert_eq!(
            summary(&structure(ObjectKind::Table, Some(48_213)), en),
            "~48,213 rows · 9.4 MB data · 3.1 MB indexes"
        );
        assert_eq!(summary(&structure(ObjectKind::View, None), en), "");
        let es = &Language::Es.messages().table;
        assert_eq!(summary(&structure(ObjectKind::View, Some(3)), es), "~3 filas");
    }
}
