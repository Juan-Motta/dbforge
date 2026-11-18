//! A table's structure as people read it: the keys each column takes part
//! in, sizes in bytes, and the size summary the rows' footer shows ("~48,213
//! rows · 9.4 MB data · 3.1 MB indexes"; views have no size).

use std::collections::{BTreeSet, HashMap};

use dbforge_core::drivers::{ObjectKind, TableStructure};

use crate::i18n::{TableMessages, thousands};

/// A key a column takes part in; the order is strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyKind {
    Pk,
    Fk,
    Uni,
    Idx,
}

impl KeyKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pk => "PK",
            Self::Fk => "FK",
            Self::Uni => "UNI",
            Self::Idx => "IDX",
        }
    }
}

/// The keys each column takes part in, strongest first. Only plain column
/// indexes count; an index on an expression like `lower(email)` marks
/// nothing. A partial unique index only makes some rows unique, so it
/// counts as IDX.
pub fn column_keys(structure: &TableStructure) -> HashMap<String, Vec<KeyKind>> {
    let mut keys: HashMap<String, BTreeSet<KeyKind>> = HashMap::new();
    let mut mark = |column: String, kind: KeyKind| {
        keys.entry(column).or_default().insert(kind);
    };
    for index in &structure.indexes {
        let unique = index.unique && index.condition.is_none() && index.columns.len() == 1;
        let kind = if index.primary {
            KeyKind::Pk
        } else if unique {
            KeyKind::Uni
        } else {
            KeyKind::Idx
        };
        for column in &index.columns {
            mark(unquote(column), kind);
        }
    }
    for key in &structure.foreign_keys {
        for column in &key.columns {
            mark(column.clone(), KeyKind::Fk);
        }
    }
    keys.into_iter().map(|(column, kinds)| (column, kinds.into_iter().collect())).collect()
}

/// Index columns come back as SQL, so mixed-case names arrive quoted: `"Note"`.
fn unquote(column: &str) -> String {
    match column.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) {
        Some(inner) => inner.replace("\"\"", "\""),
        None => column.to_owned(),
    }
}

const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];

/// 0 B, 16 kB, 9.4 MB: one decimal below 10, whole numbers above.
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

/// "~48,213 rows · 9.4 MB data · 3.1 MB indexes", in the language of `m`; views have no size.
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
pub(crate) mod tests {
    use dbforge_core::drivers::{ForeignKey, IndexInfo};

    use super::*;
    use crate::i18n::Language;

    pub(crate) fn structure(kind: ObjectKind, rows: Option<i64>) -> TableStructure {
        TableStructure {
            kind,
            row_estimate: rows,
            data_bytes: (9.4 * 1024. * 1024.) as i64,
            index_bytes: (3.1 * 1024. * 1024.) as i64,
            comment: None,
            columns: Vec::new(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            triggers: Vec::new(),
            ddl: String::new(),
        }
    }

    pub(crate) fn index(
        name: &str,
        columns: &[&str],
        primary: bool,
        unique: bool,
        condition: Option<&str>,
    ) -> IndexInfo {
        IndexInfo {
            name: name.into(),
            method: "btree".into(),
            unique: unique || primary,
            primary,
            columns: columns.iter().map(|c| c.to_string()).collect(),
            condition: condition.map(str::to_owned),
            bytes: 0,
        }
    }

    pub(crate) fn foreign_key(name: &str, column: &str, table: &str) -> ForeignKey {
        ForeignKey {
            name: name.into(),
            columns: vec![column.into()],
            referenced_table: table.into(),
            referenced_columns: vec!["id".into()],
            on_update: "NO ACTION".into(),
            on_delete: "SET NULL".into(),
        }
    }

    #[test]
    fn marks_primary_foreign_unique_and_indexed_columns_strongest_first() {
        let mut table = structure(ObjectKind::Table, None);
        table.indexes = vec![
            index("pk", &["id"], true, false, None),
            index("uq_email", &["email"], false, true, None),
            index("ix_country_segment", &["country", "segment"], false, false, None),
            index("ix_referrer", &["referrer_id"], false, false, None),
        ];
        table.foreign_keys = vec![foreign_key("fk", "referrer_id", "customers")];
        let keys = column_keys(&table);
        use KeyKind::*;
        let expected: HashMap<String, Vec<KeyKind>> = [
            ("id", vec![Pk]),
            ("email", vec![Uni]),
            ("country", vec![Idx]),
            ("segment", vec![Idx]),
            ("referrer_id", vec![Fk, Idx]),
        ]
        .into_iter()
        .map(|(column, kinds)| (column.to_owned(), kinds))
        .collect();
        assert_eq!(keys, expected);
    }

    #[test]
    fn treats_a_multi_column_unique_index_as_a_plain_index() {
        let mut table = structure(ObjectKind::Table, None);
        table.indexes = vec![index("uq", &["a", "b"], false, true, None)];
        assert_eq!(column_keys(&table)["a"], [KeyKind::Idx]);
    }

    #[test]
    fn treats_a_partial_unique_index_as_a_plain_index() {
        let mut table = structure(ObjectKind::Table, None);
        table.indexes = vec![index("uq_open", &["kb_id"], false, true, Some("status = 'in_progress'"))];
        assert_eq!(column_keys(&table)["kb_id"], [KeyKind::Idx]);
    }

    #[test]
    fn unquotes_mixed_case_names_and_ignores_expressions() {
        let mut table = structure(ObjectKind::Table, None);
        table.indexes =
            vec![index("a", &["\"Note\""], false, false, None), index("b", &["lower(email)"], false, false, None)];
        let keys = column_keys(&table);
        assert_eq!(keys["Note"], [KeyKind::Idx]);
        assert!(!keys.contains_key("email"));
        assert_eq!(unquote("\"say \"\"hi\"\"\""), "say \"hi\"");
    }

    #[test]
    fn writes_sizes_in_the_largest_unit() {
        let cases = [
            (0, "0 B"),
            (512, "512 B"),
            (16 * 1024, "16 kB"),
            (1536, "1.5 kB"),
            ((9.44 * 1024. * 1024.) as i64, "9.4 MB"),
            (3 * 1024_i64.pow(4), "3.0 TB"),
        ];
        for (bytes, text) in cases {
            assert_eq!(format_bytes(bytes), text, "{bytes} bytes");
        }
    }

    #[test]
    fn sums_up_a_table_and_a_view_in_the_language_in_use() {
        let en = &Language::En.messages().table;
        let table = structure(ObjectKind::Table, Some(48_213));
        assert_eq!(summary(&table, en), "~48,213 rows · 9.4 MB data · 3.1 MB indexes");
        let es = &Language::Es.messages().table;
        assert_eq!(summary(&table, es), "~48,213 filas · 9.4 MB de datos · 3.1 MB de índices");
        assert_eq!(summary(&structure(ObjectKind::View, None), en), "", "a view never analyzed has nothing to say");
        assert_eq!(summary(&structure(ObjectKind::View, Some(3)), es), "~3 filas");
    }
}
