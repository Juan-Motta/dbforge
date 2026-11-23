//! A collection's or container's page as whole documents (MongoDB, Cosmos
//! DB), each in a card: new ones first, then the loaded ones numbered as the
//! grid numbers its rows. A click picks a card (⌘-click or Ctrl+click adds
//! or takes it away), a double click opens it in an editor sized once for
//! the text it opens with, so typing never moves the cards below. ⌘↵
//! (Ctrl+Enter) keeps the document once the database can read it (the core
//! checks it without changing anything), Escape drops the edit. A kept
//! document is a pending change like a cell's, applied or discarded with
//! the rest. Only the cards in view are drawn, so a page of a thousand
//! documents costs no more than a few.

use crate::ui::Ellipsis;
use std::collections::BTreeSet;
use std::ops::Range;
use std::rc::Rc;

use dbforge_core::drivers::RowChange;
use dbforge_core::edit::preview_changes;
use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, DismissEvent, Div, ElementId, Entity, FontWeight, HighlightStyle,
    InteractiveElement, IntoElement, KeyBinding, ListAlignment, ListOffset, ListState, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, SharedString, Stateful, StatefulInteractiveElement, StrikethroughStyle, Styled,
    StyledText, WeakEntity, Window, actions, div, list, prelude::FluentBuilder, px,
};

use super::cell_menu::Action;
use super::edits::Key;
use super::shortcuts::KEEP_DOCUMENT;
use super::view::TableView;
use crate::backend::Backend;
use crate::i18n::{TableMessages, Translate};
use crate::sql::Syntax;
use crate::sql::document::{TokenKind, document_tokens};
use crate::theme::{ActiveTheme, MONO, Theme, color};
use crate::ui::code_editor::{self, LINE_HEIGHT, PADDING_TOP};
use crate::ui::field::Tooltip;
use crate::ui::{CodeEditor, MenuItem, PopoverMenu, button, enabled, icon, primary_button};

actions!(table_documents, [KeepDocument]);

const EDITOR_CONTEXT: &str = "DocumentEditor";

/// ⌘↵ (Ctrl+Enter) keeps the document being edited; Escape, which the
/// editor passes on once it has nothing of its own to close, drops it.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("secondary-enter", KeepDocument, Some(EDITOR_CONTEXT))]
}

/// The text a new document starts with.
pub const NEW_DOCUMENT: &str = "{\n  \n}";
/// Where a new document's caret starts: on its empty line, inside the braces.
const NEW_DOCUMENT_CARET: usize = 4;
/// Rows an editor shows at most before it scrolls.
const MAX_LINES: usize = 24;
/// A card's text: its size and line height.
const CODE_SIZE: f32 = 12.;
const CODE_LINE: f32 = 18.;
/// Space around the cards and between them.
const LIST_PADDING: f32 = 10.;
const CARD_GAP: f32 = 8.;
/// How long a copy button shows its check.
const COPIED_FOR: std::time::Duration = std::time::Duration::from_millis(1500);
/// The group a document's line hovers as, so its copy button shows.
const LINE_GROUP: &str = "document-line";

/// A document among those shown: a loaded row's, by its key among the
/// edits, or a new one, by its number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DocumentId {
    New(u64),
    Loaded(String),
}

/// What is pending on a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentState {
    Edited,
    Deleted,
    New,
}

/// A document as its card shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentCard {
    pub id: DocumentId,
    /// Its number in the table (`#12`); empty for a new one.
    pub label: String,
    /// Its text as it stands, a rewrite included.
    pub text: SharedString,
    pub state: Option<DocumentState>,
}

/// The rows an editor opening on `text` shows: its lines and two more, at
/// most [`MAX_LINES`]. Fixed when it opens, so typing never moves the cards below.
pub fn editor_rows(text: &str) -> usize {
    (text.split('\n').count() + 2).min(MAX_LINES)
}

/// Documents copied together, as an array.
pub fn as_array(texts: &[SharedString]) -> String {
    let texts: Vec<&str> = texts.iter().map(SharedString::as_ref).collect();
    format!("[\n{}\n]", texts.join(",\n"))
}

/// A document's tokens line by line, each by its range in its line; a
/// token running over a line break is cut there.
pub fn line_tokens(text: &str) -> Vec<Vec<(Range<usize>, TokenKind)>> {
    let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(at, _)| at + 1)).collect();
    let mut lines: Vec<Vec<(Range<usize>, TokenKind)>> = vec![Vec::new(); starts.len()];
    let mut line = 0;
    for token in document_tokens(text) {
        let mut at = token.range.start;
        while at < token.range.end {
            while line + 1 < starts.len() && starts[line + 1] <= at {
                line += 1;
            }
            let line_end = starts.get(line + 1).map_or(text.len(), |next| next - 1);
            let end = token.range.end.min(line_end);
            if end > at {
                lines[line].push((at - starts[line]..end - starts[line], token.kind));
            }
            // Past the line break, if the token runs over it.
            at = if end < token.range.end { end + 1 } else { end };
        }
    }
    lines
}

/// Where a line's value starts in it: past its key and colon, or at its
/// first character for an array's item. `None` for a blank line or one that
/// closes an object or array.
fn value_start(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start().len();
    let mut at = indent;
    if line[at..].starts_with('"') {
        let key_end = at + string_end(&line[at..])?;
        let rest = &line[key_end..];
        if let Some(after_colon) = rest.trim_start().strip_prefix(':') {
            at = line.len() - after_colon.trim_start().len();
        }
    }
    match line[at..].chars().next() {
        None | Some('}' | ']') => None,
        Some(_) => Some(at),
    }
}

/// The length of the string literal `text` starts with, its quotes included.
fn string_end(text: &str) -> Option<usize> {
    let mut escaped = false;
    for (at, char) in text.char_indices().skip(1) {
        match char {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return Some(at + 1),
            _ => {}
        }
    }
    None
}

/// Where the object or array opening at `start` closes, just past its bracket.
fn closing(text: &str, start: usize) -> Option<usize> {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for (at, char) in text[start..].char_indices() {
        if in_string {
            match char {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match char {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(start + at + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// A line holds a value its copy button can copy.
pub fn has_value(line: &str) -> bool {
    value_start(line).is_some()
}

/// The value on line `line` of a document, as it is copied: a string's
/// text without its quotes, an object or array whole (its lines no longer
/// indented as deep as the line it opens on), anything else as written.
pub fn line_value(text: &str, line: usize) -> Option<String> {
    let line_start: usize = text.split('\n').take(line).map(|line| line.len() + 1).sum();
    let line_text = text.split('\n').nth(line)?;
    let at = value_start(line_text)?;
    if line_text[at..].starts_with(['{', '[']) {
        let start = line_start + at;
        let whole = &text[start..closing(text, start)?];
        let indent = line_text.len() - line_text.trim_start().len();
        let lines: Vec<&str> = whole
            .split('\n')
            .map(|inner| {
                let spaces = inner.len() - inner.trim_start_matches(' ').len();
                &inner[spaces.min(indent)..]
            })
            .collect();
        return Some(lines.join("\n"));
    }
    let value = line_text[at..].trim_end();
    let value = value.strip_suffix(',').unwrap_or(value).trim_end();
    if value.starts_with('"')
        && let Ok(text) = serde_json::from_str::<String>(value)
    {
        return Some(text);
    }
    Some(value.to_owned())
}

/// A loaded document's key: each key column's value, in the columns' order.
type KeyValues = Vec<(String, Option<String>)>;

/// The page's rows as documents: whether they show, the list's scroll and
/// measured cards, the cards picked, and the one being edited.
pub(super) struct Documents {
    pub shown: bool,
    pub list: ListState,
    pub selected: BTreeSet<DocumentId>,
    pub editor: Option<DocumentEditor>,
    /// Counts the checks of edited documents, so a stale one is dropped.
    checks: u64,
    /// What was just copied: a document whole, or one of its lines' value.
    copied: Option<Copied>,
    /// Counts the copies, so only the last one's check goes away.
    copies: u64,
}

/// A document, or the value on one of its lines.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Copied {
    id: DocumentId,
    line: Option<usize>,
}

impl Default for Documents {
    fn default() -> Self {
        Self {
            shown: false,
            list: ListState::new(0, ListAlignment::Top, px(600.)),
            selected: BTreeSet::new(),
            editor: None,
            checks: 0,
            copied: None,
            copies: 0,
        }
    }
}

/// A document open in its editor.
pub struct DocumentEditor {
    pub id: DocumentId,
    pub editor: Entity<CodeEditor>,
    height: Pixels,
    /// What the database said of the draft the last time it was kept.
    problem: Option<String>,
    /// The draft is being checked.
    saving: bool,
}

impl DocumentEditor {
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }
}

impl TableView {
    /// The page has documents: a document database's.
    pub fn has_documents(&self) -> bool {
        self.page.as_ref().is_some_and(|page| page.documents.is_some())
    }

    pub fn showing_documents(&self) -> bool {
        self.documents.shown && self.has_documents()
    }

    pub fn document_editor(&self) -> Option<&DocumentEditor> {
        self.documents.editor.as_ref()
    }

    pub fn selected_documents(&self) -> &BTreeSet<DocumentId> {
        &self.documents.selected
    }

    /// Shows the rows as documents, or as a table again. Not while changes
    /// are pending: each view has its own kind of edits.
    pub fn toggle_documents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.edits.has_edits() || !self.has_documents() {
            return;
        }
        self.documents.shown = !self.documents.shown;
        self.documents.editor = None;
        self.documents.selected.clear();
        self.grid.update(cx, |grid, cx| grid.selection_mut(cx).clear());
        self.reset_documents(false);
        self.focus(window, cx);
        cx.notify();
    }

    /// Why documents can't be edited now, or `None` when they can.
    pub(super) fn document_locked(&self, cx: &App) -> Option<&'static str> {
        let m = &cx.messages().table;
        if !self.editable() {
            Some(m.no_key)
        } else if self.applying() {
            Some(m.applying_changes)
        } else {
            None
        }
    }

    /// The cards in order: new documents first, then the page's.
    pub fn document_cards(&self) -> Vec<DocumentCard> {
        let mut cards: Vec<DocumentCard> = self
            .edits
            .new_documents
            .iter()
            .map(|new| DocumentCard {
                id: DocumentId::New(new.id),
                label: String::new(),
                text: new.document.clone().into(),
                state: Some(DocumentState::New),
            })
            .collect();
        let Some(page) = &self.page else { return cards };
        let Some(documents) = &page.documents else { return cards };
        for (index, (text, id)) in documents.iter().zip(&page.keys).enumerate() {
            let replaced = self.edits.replacements.get(id);
            let state = if self.edits.is_deleted(id) {
                Some(DocumentState::Deleted)
            } else {
                replaced.map(|_| DocumentState::Edited)
            };
            cards.push(DocumentCard {
                id: DocumentId::Loaded(id.clone()),
                label: format!("#{}", self.offset + index as u64 + 1),
                text: replaced.map_or_else(|| text.clone(), |replaced| replaced.document.clone().into()),
                state,
            });
        }
        cards
    }

    fn document_count(&self) -> usize {
        let loaded = self.page.as_ref().and_then(|page| page.documents.as_ref()).map_or(0, |documents| documents.len());
        self.edits.new_documents.len() + loaded
    }

    /// A loaded document's key (by column, in the columns' order) and its text as loaded.
    fn loaded_document(&self, id: &str) -> Option<(KeyValues, SharedString)> {
        let page = self.page.as_ref()?;
        let index = page.keys.iter().position(|key| key == id)?;
        let row = page.rows.get(index)?;
        let key = page
            .columns
            .iter()
            .zip(row)
            .filter(|(column, _)| column.primary_key)
            .map(|(column, value)| (column.name.clone(), value.clone()))
            .collect();
        Some((key, page.documents.as_ref()?.get(index)?.clone()))
    }

    /// A card's copy button: the document whole, or with `line` the value
    /// on that line. The button shows a check for a moment.
    fn copy_from_document(&mut self, id: DocumentId, line: Option<usize>, cx: &mut Context<Self>) {
        let Some(card) = self.document_cards().into_iter().find(|card| card.id == id) else { return };
        let text = match line {
            None => card.text.to_string(),
            Some(line) => match line_value(&card.text, line) {
                Some(value) => value,
                None => return,
            },
        };
        self.copy(text, cx);
        self.documents.copied = Some(Copied { id, line });
        self.documents.copies += 1;
        let copy = self.documents.copies;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |this, cx| {
                if this.documents.copies == copy {
                    this.documents.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// The cards changed (an edit, an editor opened or closed): they are
    /// measured again where they are, the list keeping its place.
    pub(super) fn remeasure_documents(&self) {
        let list = &self.documents.list;
        let top = list.logical_scroll_top();
        let count = self.document_count();
        list.splice(0..list.item_count(), count);
        if count > 0 {
            list.scroll_to(ListOffset { item_ix: top.item_ix.min(count - 1), offset_in_item: top.offset_in_item });
        }
    }

    /// Another page arrived: a reload keeps the place, another page starts at its top.
    pub(super) fn reset_documents(&mut self, keep_place: bool) {
        self.documents.editor = None;
        self.documents.selected.clear();
        if keep_place {
            self.remeasure_documents();
        } else {
            self.documents.list.reset(self.document_count());
        }
    }

    /// A click picks the document, a ⌘-click (Ctrl+click) adds or takes it
    /// away; a double click opens it in its editor.
    fn click_document(&mut self, id: DocumentId, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.documents.editor.as_ref().is_some_and(|open| open.id == id) {
            return;
        }
        if event.click_count() >= 2 {
            self.open_document(id, window, cx);
        } else {
            self.select_document(id, event.modifiers().secondary(), cx);
        }
    }

    /// Picks `id`, or with `additive` adds it to the picked ones (or takes it away).
    pub fn select_document(&mut self, id: DocumentId, additive: bool, cx: &mut Context<Self>) {
        let selected = &mut self.documents.selected;
        if !additive {
            selected.clear();
            selected.insert(id);
        } else if !selected.remove(&id) {
            selected.insert(id);
        }
        cx.notify();
    }

    /// A new document on top, open in its editor.
    pub fn new_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.document_locked(cx).is_some() {
            return;
        }
        let id = self.edits.add_document(NEW_DOCUMENT);
        self.changed(None, cx);
        self.documents.list.scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) });
        self.open_document(DocumentId::New(id), window, cx);
    }

    /// Removes the picked new documents and marks the picked loaded ones for
    /// deletion (or keeps them again).
    pub(super) fn delete_selected_documents(&mut self, cx: &mut Context<Self>) {
        if self.document_locked(cx).is_some() {
            return;
        }
        for id in std::mem::take(&mut self.documents.selected) {
            match id {
                DocumentId::New(id) => self.edits.remove_new_document(id),
                DocumentId::Loaded(id) => self.toggle_delete_document(&id),
            }
        }
        self.close_gone_editor();
        self.changed(None, cx);
    }

    fn toggle_delete_document(&mut self, id: &str) {
        if let Some((key, _)) = self.loaded_document(id) {
            self.edits.toggle_delete(&Key::from_iter(key));
        }
    }

    /// An editor on a document that was removed, or marked for deletion, closes.
    fn close_gone_editor(&mut self) {
        let Some(open) = &self.documents.editor else { return };
        let gone = match &open.id {
            DocumentId::New(id) => self.edits.new_document(*id).is_none(),
            DocumentId::Loaded(id) => self.edits.is_deleted(id),
        };
        if gone {
            self.documents.editor = None;
        }
    }

    /// Opens `id` in its editor, which takes the keyboard; nothing for a
    /// document that can't change now.
    pub fn open_document(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) {
        if self.document_locked(cx).is_some() {
            return;
        }
        let Some(card) = self.document_cards().into_iter().find(|card| card.id == id) else { return };
        if card.state == Some(DocumentState::Deleted) {
            return;
        }
        let text = card.text.to_string();
        let caret = if text == NEW_DOCUMENT { NEW_DOCUMENT_CARET } else { 0 };
        let syntax = Syntax::documents(self.engine);
        let editor = cx.new(|cx| {
            let mut editor = CodeEditor::new(&text, syntax, cx);
            editor.place_caret(caret, cx);
            editor
        });
        editor.read(cx).focus(window);
        // A check of the document open before is no longer wanted.
        self.documents.checks += 1;
        let height = px(editor_rows(&text) as f32 * LINE_HEIGHT + 2. * PADDING_TOP);
        self.documents.editor = Some(DocumentEditor { id, editor, height, problem: None, saving: false });
        self.remeasure_documents();
        cx.notify();
    }

    /// ⌘↵ or Keep: the draft becomes a pending change once the database
    /// can read it; until then it stays open with what the database said.
    /// A loaded document written back as it was is no change at all.
    pub fn keep_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.documents.editor else { return };
        if open.saving {
            return;
        }
        let (id, draft) = (open.id.clone(), open.editor.read(cx).text().to_owned());
        let change = match &id {
            DocumentId::New(_) => RowChange::InsertDocument { document: draft.clone() },
            DocumentId::Loaded(row) => {
                let Some((key, original)) = self.loaded_document(row) else { return };
                if draft == original.as_ref() {
                    return self.keep_checked(id, draft, window, cx);
                }
                RowChange::Replace { key: Key::from_iter(key), document: draft.clone() }
            }
        };
        if let Some(open) = self.documents.editor.as_mut() {
            open.saving = true;
        }
        self.documents.checks += 1;
        let check = self.documents.checks;
        let (connection, table) = (self.connection_id.clone(), self.table.clone());
        let checking = Backend::global(cx).spawn(cx, move |core| async move {
            preview_changes(core.connections(), core.statement_log(), connection, table, vec![change]).await
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let outcome = checking.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    if this.documents.checks != check || this.documents.editor.as_ref().is_none_or(|o| o.id != id) {
                        return;
                    }
                    match outcome {
                        Ok(_) => this.keep_checked(id, draft, window, cx),
                        Err(error) => {
                            if let Some(open) = this.documents.editor.as_mut() {
                                open.saving = false;
                                open.problem = Some(error.to_string());
                            }
                            cx.notify();
                        }
                    }
                });
            });
        })
        .detach();
        cx.notify();
    }

    /// Keeps a draft the database can read, and closes its editor.
    fn keep_checked(&mut self, id: DocumentId, draft: String, window: &mut Window, cx: &mut Context<Self>) {
        match &id {
            DocumentId::New(id) => self.edits.edit_new_document(*id, &draft),
            DocumentId::Loaded(row) => {
                if let Some((key, original)) = self.loaded_document(row) {
                    self.edits.replace_document(&Key::from_iter(key), &draft, &original);
                }
            }
        }
        self.documents.editor = None;
        window.focus(&self.focus);
        self.changed(None, cx);
    }

    /// Escape or Cancel: drops the draft. A new document never kept is no document at all.
    pub fn cancel_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.documents.editor.take() else { return };
        self.documents.checks += 1;
        if let DocumentId::New(id) = open.id
            && self.edits.new_document(id) == Some(NEW_DOCUMENT)
        {
            self.edits.remove_new_document(id);
        }
        window.focus(&self.focus);
        self.changed(None, cx);
    }

    /// A card's right-click menu, acting on what was right-clicked and keeping a selection it belongs to.
    pub(super) fn open_document_menu(
        &mut self,
        id: DocumentId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.documents.editor.as_ref().is_some_and(|open| open.id == id) {
            return;
        }
        if !self.documents.selected.contains(&id) {
            self.select_document(id.clone(), false, cx);
        }
        let Some(items) = self.document_menu_items(&id, cx) else { return };
        let menu = cx.new(|cx| PopoverMenu::new(items, window, cx));
        let closing = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.menu = None;
            this.focus(window, cx);
            cx.notify();
        });
        self.menu = Some((menu, position, closing));
        cx.notify();
    }

    /// Edit, then Copy (the document, each of its key's values, and the
    /// picked documents together), then delete or keep it.
    fn document_menu_items(&self, id: &DocumentId, cx: &mut Context<Self>) -> Option<Vec<MenuItem>> {
        let m = &cx.messages().table;
        let cards = self.document_cards();
        let card = cards.iter().find(|card| &card.id == id)?;
        let selected = &self.documents.selected;
        let count = if selected.len() > 1 && selected.contains(id) { selected.len() } else { 0 };
        let locked = self.document_locked(cx).is_some();
        let this = cx.entity().downgrade();
        let on = |act: Action| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |view, cx| act(view, window, cx));
            }
        };
        let copy = |text: String| -> Action { Box::new(move |view, _, cx| view.copy(text.clone(), cx)) };

        let mut items = Vec::new();
        if !locked && card.state != Some(DocumentState::Deleted) {
            let id = id.clone();
            items.push(
                MenuItem::new(
                    m.edit_document,
                    on(Box::new(move |view, window, cx| view.open_document(id.clone(), window, cx))),
                )
                .icon("edit"),
            );
        }
        let group = Some(SharedString::from(m.copy_menu));
        items.push(MenuItem::new(m.copy_document, on(copy(card.text.to_string()))).group(group.clone()));
        if let DocumentId::Loaded(row) = id
            && let Some((key, _)) = self.loaded_document(row)
        {
            for (name, value) in key {
                let value = value.unwrap_or_else(|| "null".to_owned());
                items.push(MenuItem::new((m.copy_key_value)(&name), on(copy(value))).group(group.clone()));
            }
        }
        if count > 0 {
            let texts: Vec<SharedString> =
                cards.iter().filter(|card| selected.contains(&card.id)).map(|card| card.text.clone()).collect();
            items.push(MenuItem::new((m.copy_selected_documents)(count), on(copy(as_array(&texts)))).group(group));
        }
        if !locked {
            match id {
                DocumentId::New(new) => {
                    let new = *new;
                    items.push(
                        MenuItem::new(
                            m.remove_new_document,
                            on(Box::new(move |view, _, cx| {
                                view.edits.remove_new_document(new);
                                view.documents.selected.remove(&DocumentId::New(new));
                                view.close_gone_editor();
                                view.changed(None, cx);
                            })),
                        )
                        .icon("trash"),
                    );
                }
                DocumentId::Loaded(row) => {
                    let label =
                        if card.state == Some(DocumentState::Deleted) { m.keep_document } else { m.delete_document };
                    let row = row.clone();
                    items.push(
                        MenuItem::new(
                            label,
                            on(Box::new(move |view, _, cx| {
                                view.toggle_delete_document(&row);
                                view.close_gone_editor();
                                view.changed(None, cx);
                            })),
                        )
                        .icon("trash"),
                    );
                }
            }
        }
        Some(items)
    }

    /// The cards, only those in view drawn.
    pub(super) fn render_documents(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.documents.list.item_count() != self.document_count() {
            self.remeasure_documents();
        }
        let locked = self.document_locked(cx);
        let context = Rc::new(CardContext {
            theme: *cx.theme(),
            m: &cx.messages().table,
            cards: self.document_cards(),
            selected: self.documents.selected.clone(),
            editing: self.documents.editor.as_ref().map(|open| Editing {
                id: open.id.clone(),
                editor: open.editor.clone(),
                height: open.height,
                problem: open.problem.clone(),
                saving: open.saving,
            }),
            copied: self.documents.copied.clone(),
            view: cx.entity().downgrade(),
        });
        div()
            .id("documents")
            .size_full()
            .when_some(locked, |documents, locked| documents.tooltip(Tooltip::text(locked)))
            .child(list(self.documents.list.clone(), move |index, _, cx| card(index, &context, cx)).size_full())
            .into_any_element()
    }
}

/// Development builds only: documents set up for a screenshot.
#[cfg(debug_assertions)]
impl TableView {
    pub(super) fn seed_documents(&mut self, seed: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_documents() {
            return eprintln!("DBFORGE_OPEN: this table has no documents");
        }
        if !self.showing_documents() {
            self.toggle_documents(window, cx);
        }
        let loaded: Vec<String> = self.page.as_ref().map(|page| page.keys.clone()).unwrap_or_default();
        match seed {
            "document-edits" => {
                if let Some((key, text)) = loaded.first().and_then(|id| self.loaded_document(id)) {
                    let edited =
                        text.trim_end().trim_end_matches('}').trim_end().to_owned() + ",\n  \"note\": \"edited\"\n}";
                    self.edits.replace_document(&Key::from_iter(key), &edited, &text);
                }
                if let Some(id) = loaded.get(1) {
                    self.toggle_delete_document(id);
                }
                self.edits.add_document("{\n  \"total\": 42\n}");
                self.changed(None, cx);
            }
            "document-edit" => {
                if let Some(id) = loaded.first() {
                    self.open_document(DocumentId::Loaded(id.clone()), window, cx);
                }
            }
            "document-menu" => {
                let index = self.edits.new_documents.len() + 1;
                match (loaded.get(1), self.documents.list.bounds_for_item(index)) {
                    (Some(id), Some(bounds)) => {
                        let position = bounds.origin + gpui::point(px(120.), px(40.));
                        self.open_document_menu(DocumentId::Loaded(id.clone()), position, window, cx);
                    }
                    // Until the cards are laid out, waits a frame.
                    (Some(_), None) => {
                        self.seed = Some(seed.to_owned());
                        cx.notify();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// What every card is drawn with.
struct CardContext {
    theme: Theme,
    m: &'static TableMessages,
    cards: Vec<DocumentCard>,
    selected: BTreeSet<DocumentId>,
    editing: Option<Editing>,
    copied: Option<Copied>,
    view: WeakEntity<TableView>,
}

/// The document being edited, as its card draws it.
struct Editing {
    id: DocumentId,
    editor: Entity<CodeEditor>,
    height: Pixels,
    problem: Option<String>,
    saving: bool,
}

/// Card `index`, with the space around it.
fn card(index: usize, context: &CardContext, _cx: &mut App) -> AnyElement {
    let (theme, m) = (&context.theme, context.m);
    let card = &context.cards[index];
    let selected = context.selected.contains(&card.id);
    let editing = context.editing.as_ref().filter(|editing| editing.id == card.id);
    // The pending state shows over the selection, as in the former web UI.
    let (border, background) = match card.state {
        Some(DocumentState::Edited) => (theme.warn.opacity(0.6), theme.warn.opacity(0.07)),
        Some(DocumentState::New) => (theme.ok.opacity(0.6), theme.ok.opacity(0.06)),
        Some(DocumentState::Deleted) => (theme.err.opacity(0.5), theme.err.opacity(0.06)),
        None if selected => (theme.accent, theme.accent.opacity(0.05)),
        None => (theme.line, theme.card),
    };
    let state = card.state.map(|state| match state {
        DocumentState::Edited => (m.state_edited, theme.warn),
        DocumentState::Deleted => (m.state_deleted, theme.err),
        DocumentState::New => (m.state_new, theme.ok),
    });
    let header = div()
        .h(px(24.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(theme.line)
        .font_family(MONO)
        .text_size(px(11.))
        .text_color(theme.text3)
        .when(!card.label.is_empty(), |header| header.child(card.label.clone()))
        .when_some(state, |header, (label, tint)| {
            header.child(div().font_weight(FontWeight::SEMIBOLD).text_color(tint).child(label.to_uppercase()))
        })
        .child(div().flex_1())
        .child(
            copy_button(("copy-document", index), &card.id, None, context)
                .tooltip(Tooltip::text(m.copy_whole_document)),
        );
    let body = match editing {
        Some(editing) => editor_body(editing, context),
        None => code_body(index, card, context),
    };
    let (view, id) = (context.view.clone(), card.id.clone());
    let menu_view = context.view.clone();
    let menu_id = card.id.clone();
    let last = index + 1 == context.cards.len();
    div()
        .px(px(12.))
        .pt(px(if index == 0 { LIST_PADDING } else { CARD_GAP }))
        .when(last, |space| space.pb(px(LIST_PADDING)))
        .child(
            div()
                .id(("document", index))
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .bg(background)
                .overflow_hidden()
                .on_click(move |event, window, cx| {
                    let _ = view.update(cx, |view, cx| view.click_document(id.clone(), event, window, cx));
                })
                .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, window, cx| {
                    let _ = menu_view
                        .update(cx, |view, cx| view.open_document_menu(menu_id.clone(), event.position, window, cx));
                })
                .child(header)
                .child(body),
        )
        .into_any_element()
}

/// A document's text, highlighted, a long line wrapping under its own indent.
fn code_body(index: usize, card: &DocumentCard, context: &CardContext) -> AnyElement {
    let theme = &context.theme;
    let deleted = card.state == Some(DocumentState::Deleted);
    let text: &str = card.text.as_ref();
    let mut lines = Vec::new();
    for (number, (line, tokens)) in text.split('\n').zip(line_tokens(text)).enumerate() {
        let copyable = has_value(line);
        let highlights: Vec<(Range<usize>, HighlightStyle)> = if deleted {
            let struck = StrikethroughStyle { thickness: px(1.), color: Some(theme.text3) };
            vec![(0..line.len(), HighlightStyle { strikethrough: Some(struck), ..HighlightStyle::default() })]
        } else {
            tokens
                .into_iter()
                .filter_map(|(range, kind)| {
                    let tint = match kind {
                        TokenKind::Key => theme.kw,
                        TokenKind::String => theme.str,
                        TokenKind::Number | TokenKind::Literal => theme.num,
                        TokenKind::Call => theme.func,
                        TokenKind::Plain => return None,
                    };
                    Some((range, HighlightStyle { color: Some(tint), ..HighlightStyle::default() }))
                })
                .collect()
        };
        // The indent stays apart, so a long line wraps under its own start.
        let indent = line.len() - line.trim_start_matches(' ').len();
        let (lead, rest) = line.split_at(indent);
        let highlights: Vec<(Range<usize>, HighlightStyle)> = highlights
            .into_iter()
            .map(|(range, style)| (range.start.max(indent) - indent..range.end.max(indent) - indent, style))
            .filter(|(range, _)| !range.is_empty())
            .collect();
        // A line with a value has its copy button after it, there but
        // unseen until the line is hovered, so nothing moves.
        let copy = copyable.then(|| {
            let copied =
                context.copied.as_ref().is_some_and(|copied| copied.id == card.id && copied.line == Some(number));
            copy_button(("copy-value", number), &card.id, Some(number), context)
                .tooltip(Tooltip::text(context.m.copy_line_value))
                .when(!copied, |button| button.opacity(0.).group_hover(LINE_GROUP, |button| button.opacity(1.)))
        });
        lines.push(
            div()
                .group(LINE_GROUP)
                .min_h(px(CODE_LINE))
                .flex_none()
                .flex()
                .items_start()
                .gap(px(6.))
                .when(indent > 0, |row| row.child(div().flex_none().whitespace_nowrap().child(lead.to_owned())))
                .child(
                    div()
                        .min_w_0()
                        .child(StyledText::new(SharedString::from(rest.to_owned())).with_highlights(highlights)),
                )
                .children(copy),
        );
    }
    div()
        .id(("document-code", index))
        .px(px(12.))
        .py(px(8.))
        .flex()
        .flex_col()
        .font_family(MONO)
        .text_size(px(CODE_SIZE))
        .line_height(px(CODE_LINE))
        .text_color(if deleted { theme.text3 } else { theme.text2 })
        .children(lines)
        .into_any_element()
}

/// A copy button: the document `id` whole, or the value on `line`; a check
/// for a moment once it copied. Its clicks stay its own, so the card is
/// not picked or opened.
fn copy_button(
    element: impl Into<ElementId>,
    id: &DocumentId,
    line: Option<usize>,
    context: &CardContext,
) -> Stateful<Div> {
    let theme = &context.theme;
    let copied = context.copied.as_ref().is_some_and(|copied| &copied.id == id && copied.line == line);
    let (view, id) = (context.view.clone(), id.clone());
    div()
        .id(element)
        .flex_none()
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|button| button.bg(theme.sel))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            let _ = view.update(cx, |view, cx| view.copy_from_document(id.clone(), line, cx));
        })
        .child(if copied { icon("check", theme.ok, 12.) } else { icon("copy", theme.text3, 12.) })
}

/// The editor, its height fixed when it opened, and under it what the
/// database said of the draft, Cancel and Keep.
fn editor_body(editing: &Editing, context: &CardContext) -> AnyElement {
    let (theme, m) = (&context.theme, context.m);
    let (keep_view, cancel_view, escape_view, button_view) =
        (context.view.clone(), context.view.clone(), context.view.clone(), context.view.clone());
    let problem = editing.problem.clone().unwrap_or_default();
    let saving = editing.saving;
    div()
        .key_context(EDITOR_CONTEXT)
        .flex()
        .flex_col()
        .on_action(move |_: &KeepDocument, window, cx| {
            let _ = keep_view.update(cx, |view, cx| view.keep_document(window, cx));
        })
        .on_action(move |_: &code_editor::Escape, window, cx| {
            let _ = escape_view.update(cx, |view, cx| view.cancel_document(window, cx));
        })
        .child(div().h(editing.height).flex_none().child(editing.editor.clone()))
        .child(
            div()
                .h(px(44.))
                .flex_none()
                .pl(px(12.))
                .pr(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .border_t_1()
                .border_color(theme.line)
                // The problem keeps to one line, cut short, so nothing moves.
                .child(
                    div()
                        .id("document-problem")
                        .flex_1()
                        .min_w_0()
                        .ellipsis()
                        .text_size(px(12.))
                        .text_color(theme.err)
                        .when(!problem.is_empty(), |line| line.tooltip(Tooltip::text(problem.clone())))
                        .child(problem.clone()),
                )
                .child(button(theme, None, m.cancel).id("cancel-document").on_click(move |_, window, cx| {
                    let _ = cancel_view.update(cx, |view, cx| view.cancel_document(window, cx));
                }))
                .child(
                    enabled(primary_button(theme, m.keep), !saving)
                        .id("keep-document")
                        .child(div().text_color(color(0xFFFFFFCC)).child(KEEP_DOCUMENT))
                        .when(!saving, |keep| {
                            keep.on_click(move |_, window, cx| {
                                let _ = button_view.update(cx, |view, cx| view.keep_document(window, cx));
                            })
                        }),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_an_editor_for_the_text_it_opens_with_up_to_a_limit() {
        assert_eq!(editor_rows(NEW_DOCUMENT), 5);
        assert_eq!(editor_rows(&"x\n".repeat(40)), MAX_LINES);
        assert_eq!(&NEW_DOCUMENT[NEW_DOCUMENT_CARET - 1..NEW_DOCUMENT_CARET + 1], " \n", "on the empty line");
    }

    #[test]
    fn copies_documents_together_as_an_array() {
        let texts = [SharedString::from("{\"a\": 1}"), SharedString::from("{\"b\": 2}")];
        assert_eq!(as_array(&texts), "[\n{\"a\": 1},\n{\"b\": 2}\n]");
    }

    #[test]
    fn splits_a_document_s_tokens_by_line() {
        let text = "{\n  \"n\": 5\n}";
        let lines = line_tokens(text);
        assert_eq!(lines.len(), 3);
        let second: Vec<(&str, TokenKind)> =
            lines[1].iter().map(|(range, kind)| (&text.split('\n').nth(1).unwrap()[range.clone()], *kind)).collect();
        assert_eq!(
            second,
            [
                ("  ", TokenKind::Plain),
                ("\"n\"", TokenKind::Key),
                (":", TokenKind::Plain),
                (" ", TokenKind::Plain),
                ("5", TokenKind::Number)
            ]
        );
        assert_eq!(lines[2], [(0..1, TokenKind::Plain)]);
    }

    const DOCUMENT: &str = "{\n  \"id\": \"0dc8\",\n  \"note\": \"say \\\"hi\\\", bye\",\n  \"tokens\": 9471,\n  \"user_id\": null,\n  \"meta\": {\n    \"a\": [\n      1,\n      2\n    ]\n  },\n  \"empty\": {},\n  \"tags\": [\n    \"x\"\n  ]\n}";

    fn value(line: usize) -> Option<String> {
        line_value(DOCUMENT, line)
    }

    #[test]
    fn copies_a_line_s_value_strings_without_their_quotes() {
        assert_eq!(value(1).as_deref(), Some("0dc8"));
        assert_eq!(value(2).as_deref(), Some("say \"hi\", bye"));
        assert_eq!(value(3).as_deref(), Some("9471"));
        assert_eq!(value(4).as_deref(), Some("null"));
        assert_eq!(value(13).as_deref(), Some("x"), "an array's item");
    }

    #[test]
    fn copies_an_object_or_array_whole_without_the_line_s_indent() {
        assert_eq!(value(5).as_deref(), Some("{\n  \"a\": [\n    1,\n    2\n  ]\n}"));
        assert_eq!(value(6).as_deref(), Some("[\n  1,\n  2\n]"));
        assert_eq!(value(11).as_deref(), Some("{}"));
        assert_eq!(value(0).as_deref(), Some(DOCUMENT), "the document's opening line");
    }

    #[test]
    fn a_closing_line_has_no_value() {
        assert_eq!(value(9), None);
        assert_eq!(value(10), None);
        assert_eq!(value(15), None);
        assert_eq!(value(99), None);
        assert!(has_value(DOCUMENT.split('\n').nth(1).unwrap()));
        assert!(!has_value("  },"));
    }
}
