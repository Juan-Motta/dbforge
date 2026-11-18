//! One section of a table's structure, read-only: its columns with their
//! keys, its indexes, foreign keys and triggers as tables, or its DDL in a
//! highlighted editor that can be selected and copied. While the structure
//! is read again, the one shown stays.

use std::rc::Rc;

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::TableStructure;
use gpui::{
    AnyElement, AppContext, Context, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};

use super::format::{KeyKind, column_keys};
use super::sections::Section;
use super::source::StructureSource;
use crate::i18n::{StructureMessages, Translate};
use crate::sql::Dialect;
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::sheet::{one_line, sheet, text_width};
use crate::ui::{CodeEditor, icon, pill};

const ROW_HEIGHT: f32 = 30.;
const PADDING: f32 = 12.;
const CELL_SIZE: f32 = 12.;
const HEADER_SIZE: f32 = 11.5;
/// The row number's column.
const NUMBER_WIDTH: f32 = 36.;
/// Room a growing column keeps at least: a comment, a trigger's definition.
const GROW_MIN: f32 = 160.;

/// How a cell's text is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    /// Monospace, as a name or value.
    Plain,
    /// A row number: faint and right-aligned.
    Number,
    /// A data type or access method, in the keyword colour.
    Type,
    /// Nothing there: a faint dash.
    Empty,
    /// Words rather than code (a comment, Yes/No), in the window's font.
    Prose,
    /// What a foreign key points to, in the accent.
    Reference,
    /// A trigger's whole statement, wrapped.
    Definition,
    /// A size, right-aligned.
    Numeric,
}

enum Cell {
    Text(String, Tone),
    /// A tick when true: the column takes NULL.
    Check(bool),
    Keys(Vec<KeyKind>),
}

impl Cell {
    fn text(text: impl Into<String>, tone: Tone) -> Self {
        Self::Text(text.into(), tone)
    }

    /// `value`, or a faint dash when there is none.
    fn or_dash(value: Option<&String>, tone: Tone) -> Self {
        match value {
            Some(value) => Self::Text(value.clone(), tone),
            None => Self::Text("—".into(), Tone::Empty),
        }
    }
}

/// A column of a section's table.
struct Column {
    header: &'static str,
    right: bool,
    /// Takes the room left over; the others fit their widest value.
    grow: bool,
}

const fn column(header: &'static str) -> Column {
    Column { header, right: false, grow: false }
}

pub struct StructureView {
    source: Entity<StructureSource>,
    section: Section,
    ddl: Entity<CodeEditor>,
    /// The structure whose DDL the editor holds.
    ddl_of: Option<Rc<TableStructure>>,
    _subscriptions: Vec<Subscription>,
}

impl StructureView {
    /// The sections of `source`'s structure, on an `engine` connection.
    pub fn new(source: Entity<StructureSource>, engine: Engine, section: Section, cx: &mut Context<Self>) -> Self {
        let ddl = cx.new(|cx| {
            let mut editor = CodeEditor::new("", definition_dialect(engine), cx);
            editor.set_read_only(true, cx);
            editor
        });
        let subscriptions = vec![cx.observe(&source, |this, _, cx| this.follow(cx))];
        let mut view = Self { source, section, ddl, ddl_of: None, _subscriptions: subscriptions };
        view.follow(cx);
        view
    }

    pub fn section(&self) -> Section {
        self.section
    }

    pub fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    pub fn ddl(&self) -> &Entity<CodeEditor> {
        &self.ddl
    }

    /// A structure arrived: the editor shows its DDL from the top.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let structure = self.source.read(cx).structure().cloned();
        let same = match (&structure, &self.ddl_of) {
            (Some(new), Some(shown)) => Rc::ptr_eq(new, shown),
            (None, _) => true,
            _ => false,
        };
        if !same && let Some(structure) = structure {
            self.ddl.update(cx, |editor, cx| editor.show_text(&structure.ddl, cx));
            self.ddl_of = Some(structure);
        }
        cx.notify();
    }

    fn columns(structure: &TableStructure, m: &StructureMessages) -> (Vec<Column>, Vec<Vec<Cell>>) {
        let keys = column_keys(structure);
        let columns = vec![
            Column { header: "#", right: true, grow: false },
            column(m.name),
            column(m.data_type),
            column(m.nullable),
            column(m.default),
            column(m.key),
            Column { grow: true, ..column(m.comment) },
        ];
        let rows = structure
            .columns
            .iter()
            .enumerate()
            .map(|(index, info)| {
                vec![
                    Cell::text((index + 1).to_string(), Tone::Number),
                    Cell::text(info.name.clone(), Tone::Plain),
                    Cell::text(info.data_type.clone(), Tone::Type),
                    Cell::Check(info.nullable),
                    Cell::or_dash(info.default.as_ref(), Tone::Plain),
                    Cell::Keys(keys.get(&info.name).cloned().unwrap_or_default()),
                    Cell::text(info.comment.clone().unwrap_or_default(), Tone::Prose),
                ]
            })
            .collect();
        (columns, rows)
    }

    fn indexes(structure: &TableStructure, m: &StructureMessages) -> (Vec<Column>, Vec<Vec<Cell>>) {
        let columns = vec![
            column(m.index),
            column(m.method),
            column(m.unique),
            Column { grow: true, ..column(m.columns) },
            column(m.condition),
            Column { header: m.size, right: true, grow: false },
        ];
        let rows = structure
            .indexes
            .iter()
            .map(|index| {
                let unique = if index.primary {
                    m.primary
                } else if index.unique {
                    m.yes
                } else {
                    m.no
                };
                vec![
                    Cell::text(index.name.clone(), Tone::Plain),
                    Cell::text(index.method.clone(), Tone::Type),
                    Cell::text(unique, Tone::Prose),
                    Cell::text(index.columns.join(", "), Tone::Plain),
                    Cell::or_dash(index.condition.as_ref(), Tone::Plain),
                    Cell::text(super::format::format_bytes(index.bytes), Tone::Numeric),
                ]
            })
            .collect();
        (columns, rows)
    }

    fn foreign_keys(structure: &TableStructure, m: &StructureMessages) -> (Vec<Column>, Vec<Vec<Cell>>) {
        let columns = vec![
            column(m.constraint),
            column(m.columns),
            Column { grow: true, ..column(m.references) },
            column(m.on_update),
            column(m.on_delete),
        ];
        let rows = structure
            .foreign_keys
            .iter()
            .map(|key| {
                let reference = format!("{}({})", key.referenced_table, key.referenced_columns.join(", "));
                vec![
                    Cell::text(key.name.clone(), Tone::Plain),
                    Cell::text(key.columns.join(", "), Tone::Plain),
                    Cell::text(reference, Tone::Reference),
                    Cell::text(key.on_update.clone(), Tone::Prose),
                    Cell::text(key.on_delete.clone(), Tone::Prose),
                ]
            })
            .collect();
        (columns, rows)
    }

    fn triggers(structure: &TableStructure, m: &StructureMessages) -> (Vec<Column>, Vec<Vec<Cell>>) {
        let columns = vec![column(m.trigger), column(m.enabled), Column { grow: true, ..column(m.definition) }];
        let rows = structure
            .triggers
            .iter()
            .map(|trigger| {
                vec![
                    Cell::text(trigger.name.clone(), Tone::Plain),
                    Cell::text(if trigger.enabled { m.yes } else { m.no }, Tone::Prose),
                    Cell::text(trigger.definition.clone(), Tone::Definition),
                ]
            })
            .collect();
        (columns, rows)
    }

    /// A section's table: every column as wide as its widest value, the
    /// growing one taking what is left.
    fn table(&self, columns: Vec<Column>, rows: Vec<Vec<Cell>>, window: &Window, theme: &Theme) -> AnyElement {
        let widths: Vec<f32> = columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                if column.header == "#" {
                    return NUMBER_WIDTH;
                }
                let header = text_width(column.header, false, FontWeight::SEMIBOLD, HEADER_SIZE, window);
                let widest = rows.iter().map(|row| cell_width(&row[index], window)).fold(header, f32::max);
                let width = widest + PADDING * 2.;
                if column.grow { width.max(GROW_MIN) } else { width }
            })
            .collect();
        let min_width = px(widths.iter().sum());
        let wraps = rows.iter().flatten().any(|cell| matches!(cell, Cell::Text(_, Tone::Definition)));
        let place = |content: gpui::Div, index: usize| {
            let column = &columns[index];
            let cell = content.px(px(PADDING)).flex().items_center().when(column.right, |cell| cell.justify_end());
            if column.grow { cell.flex_1().min_w(px(widths[index])) } else { cell.w(px(widths[index])).flex_none() }
        };
        let header = div()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .flex()
            .bg(theme.glass)
            .border_b_1()
            .border_color(theme.line)
            .text_size(px(HEADER_SIZE))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.text2)
            .children(columns.iter().enumerate().map(|(index, column)| {
                let tint = if column.header == "#" { theme.text3 } else { theme.text2 };
                place(div().text_color(tint).child(column.header), index)
            }));
        let body = div().flex().flex_col().children(rows.into_iter().enumerate().map(|(row_index, row)| {
            let wraps = row.iter().any(|cell| matches!(cell, Cell::Text(_, Tone::Definition)));
            div()
                .id(("structure-row", row_index))
                .min_h(px(ROW_HEIGHT))
                .when(!wraps, |row| row.h(px(ROW_HEIGHT)))
                .flex()
                .border_b_1()
                .border_color(theme.line)
                .text_size(px(CELL_SIZE))
                .hover(|row| row.bg(theme.sel))
                .children(
                    row.into_iter()
                        .enumerate()
                        .map(|(index, cell)| place(draw_cell(cell, theme), index).into_any_element()),
                )
        }));
        // A wrapping column takes the width left, so the table never scrolls sideways.
        if wraps {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(header)
                .child(div().id("rows").flex_1().min_h_0().overflow_y_scroll().child(body))
                .into_any_element()
        } else {
            sheet("structure-table", min_width, header, body).into_any_element()
        }
    }

    fn empty(title: &'static str, detail: Option<&'static str>, theme: &Theme) -> AnyElement {
        div()
            .flex_1()
            .p(px(48.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.5))
            .text_color(theme.text2)
            .child(div().font_weight(FontWeight::MEDIUM).text_color(theme.text).child(title))
            .children(detail)
            .into_any_element()
    }
}

/// The dialect a definition is highlighted in: SQL, except Cosmos DB's,
/// which is JSON, and MongoDB's, which has none.
fn definition_dialect(engine: Engine) -> Option<Dialect> {
    match engine {
        Engine::Cosmosdb => None,
        engine => Dialect::of(engine),
    }
}

fn mono(tone: Tone) -> bool {
    !matches!(tone, Tone::Prose | Tone::Definition)
}

/// How wide a cell's content is drawn; a definition wraps, so it asks for nothing.
fn cell_width(cell: &Cell, window: &Window) -> f32 {
    match cell {
        Cell::Text(_, Tone::Definition) => 0.,
        Cell::Text(text, tone) => text_width(text, mono(*tone), FontWeight::NORMAL, CELL_SIZE, window),
        Cell::Check(_) => 13.,
        Cell::Keys(keys) => keys.len() as f32 * 36.,
    }
}

fn draw_cell(cell: Cell, theme: &Theme) -> gpui::Div {
    let cell_div = div().min_w_0();
    match cell {
        Cell::Text(text, tone) => {
            let tint: Hsla = match tone {
                Tone::Plain | Tone::Numeric => theme.text,
                Tone::Number | Tone::Empty => theme.text3,
                Tone::Type => theme.kw,
                Tone::Prose | Tone::Definition => theme.text2,
                Tone::Reference => theme.accent_text,
            };
            let cell_div = cell_div.text_color(tint);
            let cell_div = if mono(tone) { cell_div.font_family(MONO) } else { cell_div };
            if tone == Tone::Definition {
                // Wrapped as one paragraph: its line breaks fold into spaces.
                let folded = text.split_whitespace().collect::<Vec<_>>().join(" ");
                cell_div.py(px(7.)).child(div().w_full().line_height(px(17.)).child(folded))
            } else {
                cell_div.truncate().child(one_line(&text))
            }
        }
        Cell::Check(checked) => cell_div.when(checked, |cell| cell.child(icon("check", theme.text, 13.))),
        Cell::Keys(keys) => cell_div
            .flex()
            .items_center()
            .gap(px(4.))
            .children(keys.into_iter().map(|kind| pill(kind.label(), key_tint(kind, theme)).flex_none())),
    }
}

/// A key's badge colour: primary in amber, foreign in blue, unique in green, a plain index faint.
fn key_tint(kind: KeyKind, theme: &Theme) -> Hsla {
    match kind {
        KeyKind::Pk => theme.warn,
        KeyKind::Fk => theme.func,
        KeyKind::Uni => theme.ok,
        KeyKind::Idx => theme.text3,
    }
}

impl Render for StructureView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let messages = cx.messages();
        let m = &messages.structure;
        let source = self.source.read(cx);
        let content = if let Some(error) = source.error() {
            div().p(px(16.)).text_size(px(12.5)).text_color(theme.err).child(error.to_owned()).into_any_element()
        } else if let Some(structure) = source.structure().cloned() {
            let empty = |title, detail| Self::empty(title, detail, &theme);
            match self.section {
                Section::Ddl => self.ddl.clone().into_any_element(),
                Section::Indexes if structure.indexes.is_empty() => empty(m.no_indexes, None),
                Section::ForeignKeys if structure.foreign_keys.is_empty() => empty(m.no_foreign_keys, None),
                Section::Triggers if structure.triggers.is_empty() => empty(m.no_triggers, Some(m.triggers_help)),
                section => {
                    let (columns, rows) = match section {
                        Section::Columns => Self::columns(&structure, m),
                        Section::Indexes => Self::indexes(&structure, m),
                        Section::ForeignKeys => Self::foreign_keys(&structure, m),
                        _ => Self::triggers(&structure, m),
                    };
                    self.table(columns, rows, window, &theme)
                }
            }
        } else {
            div()
                .p(px(16.))
                .text_size(px(12.5))
                .text_color(theme.text3)
                .child(messages.table.loading)
                .into_any_element()
        };
        div().size_full().flex().flex_col().child(content)
    }
}
