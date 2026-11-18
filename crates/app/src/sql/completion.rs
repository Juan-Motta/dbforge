//! What the editor offers to complete at the caret: the dialect's keywords
//! and the database's schemas, tables and columns, as the web UI's editor
//! did. After `schema.` come its tables; after `table.` (or an alias the
//! statement gives it) its columns. Matches by prefix rank before a name
//! whose later word (after `_`) matches.

use std::ops::Range;
use std::sync::OnceLock;

use dbforge_core::drivers::{CompletionSchema, ObjectKind};
use regex::Regex;

use super::Dialect;
use super::highlight::{Kind, LineState, tokenize};
use super::statements::statement_at;

/// The most suggestions listed at once.
const LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ItemKind {
    Column,
    Table,
    View,
    Schema,
    Keyword,
}

/// One suggestion: the text it inserts, what it is, and a detail beside it
/// (a column's type, `table` or `view`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub kind: ItemKind,
    pub detail: String,
}

/// The suggestions at a caret, and the word they replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub range: Range<usize>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
struct Table {
    name: String,
    view: bool,
    columns: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
struct Schema {
    name: String,
    tables: Vec<Table>,
}

/// A database's names, read once for completion.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompletionNames {
    schemas: Vec<Schema>,
    /// The schema whose tables complete without its name.
    default_schema: Option<String>,
}

impl CompletionNames {
    /// The names in `schema`; tables complete without their schema in the
    /// dialect's default one, or (MySQL) the database, its only schema.
    pub fn new(schema: &CompletionSchema, dialect: Dialect) -> Self {
        let mut schemas: Vec<Schema> = Vec::new();
        for table in &schema.tables {
            let index = match schemas.iter().position(|s| s.name == table.schema) {
                Some(index) => index,
                None => {
                    schemas.push(Schema { name: table.schema.clone(), tables: Vec::new() });
                    schemas.len() - 1
                }
            };
            schemas[index].tables.push(Table {
                name: table.name.clone(),
                view: table.kind == ObjectKind::View,
                columns: table.columns.iter().map(|c| (c.name.clone(), c.data_type.clone())).collect(),
            });
        }
        let wanted = dialect.default_schema().unwrap_or(schema.database.as_str());
        let default_schema = schemas.iter().any(|s| s.name == wanted).then(|| wanted.to_owned());
        Self { schemas, default_schema }
    }

    fn schema(&self, name: &str) -> Option<&Schema> {
        self.schemas.iter().find(|schema| schema.name.eq_ignore_ascii_case(name))
    }

    /// The table `parts` names: `schema.table`, or a table of the default
    /// schema, or else of any schema.
    fn table(&self, parts: &[String]) -> Option<&Table> {
        fn find<'a>(schema: &'a Schema, name: &str) -> Option<&'a Table> {
            schema.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name))
        }
        match parts {
            [schema, name] => find(self.schema(schema)?, name),
            [name] => self
                .default_schema
                .as_deref()
                .and_then(|default| self.schema(default))
                .and_then(|schema| find(schema, name))
                .or_else(|| self.schemas.iter().find_map(|schema| find(schema, name))),
            _ => None,
        }
    }
}

fn table_item(table: &Table) -> Item {
    let (kind, detail) = if table.view { (ItemKind::View, "view") } else { (ItemKind::Table, "table") };
    Item { label: table.name.clone(), kind, detail: detail.into() }
}

fn column_items(table: &Table) -> impl Iterator<Item = Item> + '_ {
    table.columns.iter().map(|(name, data_type)| Item {
        label: name.clone(),
        kind: ItemKind::Column,
        detail: data_type.clone(),
    })
}

/// The suggestions at `caret` in `text`. Without `explicit` (Ctrl+Space),
/// only while a word or a `name.` is being typed; never inside a string or
/// a comment.
pub fn complete(
    text: &str,
    caret: usize,
    dialect: Dialect,
    names: Option<&CompletionNames>,
    explicit: bool,
) -> Option<Completion> {
    let caret = caret.min(text.len());
    if in_string_or_comment(text, caret, dialect) {
        return None;
    }
    let start = text[..caret].rfind(|c: char| !is_word(c)).map_or(0, |index| index + 1);
    let prefix = &text[start..caret];
    let qualifier = qualifier(&text[..start]);
    if !explicit && prefix.is_empty() && qualifier.is_empty() {
        return None;
    }

    let mut candidates: Vec<Item> = Vec::new();
    match (names, qualifier.as_slice()) {
        (_, []) => {
            if let Some(names) = names {
                for table in tables_in_statement(text, caret, dialect, names) {
                    candidates.extend(column_items(table));
                }
                if let Some(schema) = names.default_schema.as_deref().and_then(|name| names.schema(name)) {
                    candidates.extend(schema.tables.iter().map(table_item));
                }
                candidates.extend(names.schemas.iter().map(|schema| Item {
                    label: schema.name.clone(),
                    kind: ItemKind::Schema,
                    detail: "schema".into(),
                }));
            }
            candidates.extend(dialect.keywords().map(|keyword| Item {
                label: keyword.to_owned(),
                kind: ItemKind::Keyword,
                detail: "keyword".into(),
            }));
        }
        (None, _) => return None,
        (Some(names), [first]) => {
            if let Some(schema) = names.schema(first) {
                candidates.extend(schema.tables.iter().map(table_item));
            }
            let table = aliases(text, caret, dialect)
                .into_iter()
                .find(|(alias, _)| alias.eq_ignore_ascii_case(first))
                .and_then(|(_, parts)| names.table(&parts))
                .or_else(|| names.table(std::slice::from_ref(first)));
            if let Some(table) = table {
                candidates.extend(column_items(table));
            }
        }
        (Some(names), parts) => {
            if let Some(table) = names.table(&parts[parts.len() - 2..]) {
                candidates.extend(column_items(table));
            }
        }
    }
    let items = rank(candidates, prefix);
    (!items.is_empty()).then_some(Completion { range: start..caret, items })
}

/// The candidates that match `prefix`, best first: by prefix, then a name
/// whose later word matches (`total` in `order_total`); then columns,
/// tables, schemas and keywords; then shorter names.
/// A lone suggestion that is exactly what was typed is no suggestion.
fn rank(candidates: Vec<Item>, prefix: &str) -> Vec<Item> {
    let wanted = prefix.to_lowercase();
    let mut scored: Vec<(u8, Item)> = candidates
        .into_iter()
        .filter_map(|item| {
            let label = item.label.to_lowercase();
            let score = if label.starts_with(&wanted) {
                0
            } else if !wanted.is_empty() && item.kind != ItemKind::Keyword && label.contains(&format!("_{wanted}")) {
                1
            } else {
                return None;
            };
            Some((score, item))
        })
        .collect();
    scored.sort_by(|(a, x), (b, y)| {
        a.cmp(b).then(x.kind.cmp(&y.kind)).then(x.label.len().cmp(&y.label.len())).then(x.label.cmp(&y.label))
    });
    scored.dedup_by(|(_, a), (_, b)| a.label == b.label && a.kind == b.kind);
    let items: Vec<Item> = scored.into_iter().map(|(_, item)| item).take(LIMIT).collect();
    if items.len() == 1 && items[0].label.eq_ignore_ascii_case(prefix) {
        return Vec::new();
    }
    items
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The dotted names right before a word: `public.orders.` gives `public`,
/// `orders`; quotes are dropped.
fn qualifier(before: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = before;
    while let Some(head) = rest.strip_suffix('.') {
        let (start, name) = match head.chars().last() {
            Some(close @ ('"' | '`' | ']')) => {
                let open = if close == ']' { '[' } else { close };
                let inner = &head[..head.len() - 1];
                match inner.rfind(open) {
                    Some(at) => (at, inner[at + 1..].to_owned()),
                    None => break,
                }
            }
            _ => {
                let at = head.rfind(|c: char| !is_word(c)).map_or(0, |index| index + 1);
                (at, head[at..].to_owned())
            }
        };
        if name.is_empty() {
            break;
        }
        parts.push(name);
        rest = &head[..start];
    }
    parts.reverse();
    parts
}

/// Whether the caret is inside a string, a quoted name or a comment.
fn in_string_or_comment(text: &str, caret: usize, dialect: Dialect) -> bool {
    let line_start = text[..caret].rfind('\n').map_or(0, |index| index + 1);
    let mut state = LineState::Normal;
    let mut offset = 0;
    for line in text[..line_start].split_terminator('\n') {
        state = tokenize(line, state, dialect).1;
        offset += line.len() + 1;
    }
    debug_assert!(offset <= line_start + 1);
    let (spans, open) = tokenize(&text[line_start..caret], state, dialect);
    if open != LineState::Normal {
        return true;
    }
    spans.last().is_some_and(|span| span.kind == Kind::Comment)
}

/// A table named after FROM, JOIN, UPDATE or INTO, and the alias after it.
fn table_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        let name = r#"(?:[\w$]+|"[^"]+"|`[^`]+`|\[[^\]]+\])"#;
        Regex::new(&format!(r"(?i)\b(?:from|join|update|into)\s+({name}(?:\.{name})*)(?:\s+(?:as\s+)?([\w$]+))?"))
            .expect("a valid pattern")
    })
}

/// The tables the statement at the caret names, each with its alias (or its
/// own name), as name parts.
fn aliases(text: &str, caret: usize, dialect: Dialect) -> Vec<(String, Vec<String>)> {
    let Some(range) = statement_at(text, caret, dialect) else { return Vec::new() };
    table_pattern()
        .captures_iter(&text[range])
        .map(|captures| {
            let parts: Vec<String> = captures[1]
                .split('.')
                .map(|part| part.trim_matches(|c| matches!(c, '"' | '`' | '[' | ']')).to_owned())
                .collect();
            let alias = captures
                .get(2)
                .map(|alias| alias.as_str())
                .filter(|alias| !dialect.is_keyword(&alias.to_uppercase()))
                .map_or_else(|| parts.last().cloned().unwrap_or_default(), str::to_owned);
            (alias, parts)
        })
        .collect()
}

fn tables_in_statement<'a>(text: &str, caret: usize, dialect: Dialect, names: &'a CompletionNames) -> Vec<&'a Table> {
    let mut tables: Vec<&Table> = Vec::new();
    for (_, parts) in aliases(text, caret, dialect) {
        if let Some(table) = names.table(&parts)
            && !tables.iter().any(|known| std::ptr::eq(*known, table))
        {
            tables.push(table);
        }
    }
    tables
}

#[cfg(test)]
mod tests {
    use dbforge_core::drivers::{CompletionColumn, CompletionTable};

    use super::*;

    fn shop() -> CompletionNames {
        let table = |schema: &str, name: &str, kind: ObjectKind, columns: &[(&str, &str)]| CompletionTable {
            schema: schema.into(),
            name: name.into(),
            kind,
            columns: columns
                .iter()
                .map(|(name, data_type)| CompletionColumn { name: (*name).into(), data_type: (*data_type).into() })
                .collect(),
        };
        let schema = CompletionSchema {
            database: "shop".into(),
            tables: vec![
                table("public", "orders", ObjectKind::Table, &[("id", "integer"), ("order_total", "numeric")]),
                table("public", "open_orders", ObjectKind::View, &[("id", "integer")]),
                table("billing", "invoices", ObjectKind::Table, &[("total", "numeric(10,2)")]),
            ],
        };
        CompletionNames::new(&schema, Dialect::Postgres)
    }

    /// The labels offered with the caret at `|`.
    fn offered(text: &str, explicit: bool) -> Vec<String> {
        let caret = text.find('|').expect("a caret");
        let text = text.replacen('|', "", 1);
        complete(&text, caret, Dialect::Postgres, Some(&shop()), explicit)
            .map(|completion| completion.items.into_iter().map(|item| item.label).collect())
            .unwrap_or_default()
    }

    #[test]
    fn completes_keywords_tables_and_schemas_by_prefix_first() {
        assert_eq!(offered("SEL|", false), ["SELECT"]);
        assert_eq!(offered("SELECT * FROM or|", false), ["orders", "OR", "ORDER", "open_orders"]);
        let total = offered("SELECT * FROM o|", false);
        assert_eq!(&total[..3], ["orders", "open_orders", "ON"]);
        assert!(offered("SELECT * FROM bil|", false).contains(&"billing".to_owned()));
        assert_eq!(offered("SELECT * FROM rders|", false), Vec::<String>::new(), "not any letters inside");
        assert_eq!(offered("SELECT tot| FROM orders", false), ["order_total"], "a later word of a name");
    }

    #[test]
    fn completes_a_schema_s_tables_and_a_table_s_columns() {
        assert_eq!(offered("SELECT * FROM billing.|", false), ["invoices"]);
        assert_eq!(offered("SELECT orders.| FROM orders", false), ["id", "order_total"]);
        assert_eq!(offered("SELECT billing.invoices.t| FROM x", false), ["total"]);
        assert_eq!(offered("SELECT \"public\".\"orders\".o|", false), ["order_total"]);
    }

    #[test]
    fn completes_columns_through_an_alias_and_of_the_statement_s_tables() {
        assert_eq!(offered("SELECT o.| FROM orders AS o", false), ["id", "order_total"]);
        assert_eq!(offered("SELECT i.t| FROM billing.invoices i WHERE true", false), ["total"]);
        assert_eq!(offered("SELECT ord| FROM orders", false), ["order_total", "orders", "ORDER", "open_orders"]);
    }

    #[test]
    fn stays_quiet_in_strings_comments_and_on_nothing_typed() {
        assert!(offered("SELECT 'sel|", false).is_empty());
        assert!(offered("-- sel|", false).is_empty());
        assert!(offered("/* a\nsel|", false).is_empty());
        assert!(offered("SELECT |", false).is_empty());
        assert!(offered("SELECT |", true).len() > 10, "Ctrl+Space lists everything");
        assert!(offered("SELECT|", false).is_empty(), "a word typed in full needs nothing");
    }

    #[test]
    fn reads_the_default_schema_of_each_dialect() {
        let schema = CompletionSchema {
            database: "shop".into(),
            tables: vec![CompletionTable {
                schema: "shop".into(),
                name: "users".into(),
                kind: ObjectKind::Table,
                columns: Vec::new(),
            }],
        };
        let names = CompletionNames::new(&schema, Dialect::Mysql);
        assert_eq!(names.default_schema.as_deref(), Some("shop"), "MySQL's database is its schema");
        let names = CompletionNames::new(&schema, Dialect::Postgres);
        assert_eq!(names.default_schema, None, "no public schema in it");
        let items = complete(
            "SELECT * FROM us",
            16,
            Dialect::Mysql,
            Some(&CompletionNames::new(&schema, Dialect::Mysql)),
            false,
        );
        assert_eq!(items.unwrap().items[0].label, "users");
    }
}
