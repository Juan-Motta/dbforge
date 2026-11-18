//! The parts a table's structure is shown in, each a view of the table's
//! tab: its columns, indexes, foreign keys, triggers and DDL.

use dbforge_core::drivers::TableStructure;

use crate::i18n::StructureMessages;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Columns,
    Indexes,
    ForeignKeys,
    Triggers,
    Ddl,
}

impl Section {
    /// Every section, in the order the table's tab lists them.
    pub const ALL: [Section; 5] = [Self::Columns, Self::Indexes, Self::ForeignKeys, Self::Triggers, Self::Ddl];

    /// The section's name in the language of `m`.
    pub fn label(self, m: &StructureMessages) -> &'static str {
        match self {
            Self::Columns => m.section_columns,
            Self::Indexes => m.section_indexes,
            Self::ForeignKeys => m.section_foreign_keys,
            Self::Triggers => m.section_triggers,
            Self::Ddl => m.section_ddl,
        }
    }

    /// How many items the section lists; DDL has no count.
    pub fn count(self, structure: &TableStructure) -> Option<usize> {
        match self {
            Self::Columns => Some(structure.columns.len()),
            Self::Indexes => Some(structure.indexes.len()),
            Self::ForeignKeys => Some(structure.foreign_keys.len()),
            Self::Triggers => Some(structure.triggers.len()),
            Self::Ddl => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use dbforge_core::drivers::ObjectKind;

    use super::*;
    use crate::i18n::Language;
    use crate::structure::format::tests::{foreign_key, index, structure};

    #[test]
    fn names_the_sections_in_order_in_the_language_in_use() {
        let en = &Language::En.messages().structure;
        let names: Vec<&str> = Section::ALL.iter().map(|section| section.label(en)).collect();
        assert_eq!(names, ["Columns", "Indexes", "Foreign keys", "Triggers", "DDL"]);
        let es = &Language::Es.messages().structure;
        assert_eq!(Section::ForeignKeys.label(es), "Claves foráneas");
        assert_eq!(Section::Triggers.label(es), "Disparadores");
    }

    #[test]
    fn counts_what_each_section_lists_but_the_ddl() {
        let mut table = structure(ObjectKind::Table, None);
        table.indexes = vec![index("pk", &["id"], true, false, None), index("ix", &["a"], false, false, None)];
        table.foreign_keys = vec![foreign_key("fk", "a", "b")];
        let counts: Vec<Option<usize>> = Section::ALL.iter().map(|section| section.count(&table)).collect();
        assert_eq!(counts, [Some(0), Some(2), Some(1), Some(0), None]);
    }
}
