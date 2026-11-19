//! A multi-line code editor: SQL in the console, DDL, documents and mongosh.
//! Built on GPUI's input handler like the one-line [`super::TextInput`], so
//! the OS's input methods work (dead keys, ñ, emoji, CJK). Only the rows in
//! view are laid out and drawn, so a long script costs no more than a short
//! one. It does what the former web UI's CodeMirror (`basicSetup`) did: line
//! numbers, the current line marked, selections made with the keyboard or
//! the mouse, several at once (⌘-click, ⌘D, ⌘⌥↑/↓), the clipboard, undo and
//! redo, indentation, ⌘/ comments, brackets that close and match, folds in
//! the gutter, a find and replace bar (⌘F), soft wrap as an option, and
//! with a SQL dialect highlighting and completion.
//!
//! The owner reads [`CodeEditor::text`] and the caret, and binds its own
//! commands (Run) in a key context around the editor. With several
//! selections, the primary one (the last added) is the one the owner reads.

mod completion_list;
mod display_map;
pub mod document;
mod element;
mod find_bar;
mod search;
mod selection;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    Font, FontStyle, FontWeight, InteractiveElement, IntoElement, KeyBinding, LineFragment, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, ScrollWheelEvent, ShapedLine,
    Styled, Subscription, TextRun, UTF16Selection, Window, actions, div, font, point, px,
};

use self::display_map::DisplayMap;
use self::document::{BracketRules, Document, EditKind};
use self::element::EditorElement;
pub use self::element::PADDING_TOP;
use self::search::SearchQuery;
use self::selection::Selection;
use super::TextInput;
use crate::sql::Syntax;
use crate::sql::completion::{self, Completion, CompletionNames};
use crate::sql::highlight::{self, Kind, LineState};
use crate::sql::structure::Structure;
use crate::theme::MONO;
use crate::ui::editing::utf8_offset;

actions!(
    code_editor,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteToLineStart,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        LineStart,
        LineEnd,
        SelectLineStart,
        SelectLineEnd,
        DocumentStart,
        DocumentEnd,
        SelectDocumentStart,
        SelectDocumentEnd,
        PageUp,
        PageDown,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Newline,
        Indent,
        Outdent,
        ShowCompletions,
        Escape,
        ShowCharacterPalette,
        ToggleComment,
        SelectNextOccurrence,
        AddCursorAbove,
        AddCursorBelow,
        Fold,
        Unfold,
        FoldAll,
        UnfoldAll,
        OpenFind,
        FindNext,
        FindPrevious,
        CloseFind,
    ]
);

const CONTEXT: &str = "CodeEditor";
/// The find bar's own context, apart from the editor's so its fields' keys stay theirs.
const FIND_CONTEXT: &str = "CodeEditorFind";

/// The editor's text: its font size and line height.
pub const FONT_SIZE: f32 = 13.;
pub const LINE_HEIGHT: f32 = 20.;

/// The keys the editor answers, with each OS's own modifiers: ⌥ moves by
/// word and ⌘ to the line's or the text's ends on macOS; Ctrl moves by word
/// and Ctrl+Home/End to the text's ends elsewhere. The rest are
/// CodeMirror's: ⌘F (Ctrl+F) finds, ⌘G and F3 find the next match, ⌘/
/// comments, ⌘D selects the next match, ⌘⌥↑/↓ add a caret, ⌘⌥[ and ⌘⌥]
/// (Ctrl+Shift+[ and ]) fold and unfold, Ctrl+Alt+[ and ] fold and unfold all.
pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    let mac = cfg!(target_os = "macos");
    let word = if mac { "alt" } else { "ctrl" };
    let mut bindings = vec![
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("shift-backspace", Backspace, context),
        KeyBinding::new(&format!("{word}-backspace"), DeleteWordLeft, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new(&format!("{word}-left"), WordLeft, context),
        KeyBinding::new(&format!("{word}-right"), WordRight, context),
        KeyBinding::new(&format!("{word}-shift-left"), SelectWordLeft, context),
        KeyBinding::new(&format!("{word}-shift-right"), SelectWordRight, context),
        KeyBinding::new("home", LineStart, context),
        KeyBinding::new("end", LineEnd, context),
        KeyBinding::new("shift-home", SelectLineStart, context),
        KeyBinding::new("shift-end", SelectLineEnd, context),
        KeyBinding::new("pageup", PageUp, context),
        KeyBinding::new("pagedown", PageDown, context),
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("secondary-c", Copy, context),
        KeyBinding::new("secondary-x", Cut, context),
        KeyBinding::new("secondary-v", Paste, context),
        KeyBinding::new("secondary-z", Undo, context),
        // Ctrl+Y stays the console's history, as in the former web UI.
        KeyBinding::new("secondary-shift-z", Redo, context),
        KeyBinding::new("enter", Newline, context),
        KeyBinding::new("tab", Indent, context),
        KeyBinding::new("shift-tab", Outdent, context),
        KeyBinding::new("ctrl-space", ShowCompletions, context),
        KeyBinding::new("escape", Escape, context),
        KeyBinding::new("secondary-/", ToggleComment, context),
        KeyBinding::new("secondary-d", SelectNextOccurrence, context),
        KeyBinding::new("secondary-alt-up", AddCursorAbove, context),
        KeyBinding::new("secondary-alt-down", AddCursorBelow, context),
        KeyBinding::new("ctrl-alt-[", FoldAll, context),
        KeyBinding::new("ctrl-alt-]", UnfoldAll, context),
    ];
    // The find bar's keys work in the editor and in the bar's fields.
    for context in [Some(CONTEXT), Some(FIND_CONTEXT)] {
        bindings.extend([
            KeyBinding::new("secondary-f", OpenFind, context),
            KeyBinding::new("secondary-g", FindNext, context),
            KeyBinding::new("secondary-shift-g", FindPrevious, context),
            KeyBinding::new("f3", FindNext, context),
            KeyBinding::new("shift-f3", FindPrevious, context),
        ]);
    }
    bindings.extend([
        KeyBinding::new("shift-enter", FindPrevious, Some(FIND_CONTEXT)),
        KeyBinding::new("escape", CloseFind, Some(FIND_CONTEXT)),
    ]);
    if mac {
        bindings.extend([
            KeyBinding::new("cmd-left", LineStart, context),
            KeyBinding::new("cmd-right", LineEnd, context),
            KeyBinding::new("cmd-shift-left", SelectLineStart, context),
            KeyBinding::new("cmd-shift-right", SelectLineEnd, context),
            KeyBinding::new("cmd-up", DocumentStart, context),
            KeyBinding::new("cmd-down", DocumentEnd, context),
            KeyBinding::new("cmd-shift-up", SelectDocumentStart, context),
            KeyBinding::new("cmd-shift-down", SelectDocumentEnd, context),
            KeyBinding::new("cmd-backspace", DeleteToLineStart, context),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, context),
            KeyBinding::new("cmd-alt-[", Fold, context),
            KeyBinding::new("cmd-alt-]", Unfold, context),
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("ctrl-home", DocumentStart, context),
            KeyBinding::new("ctrl-end", DocumentEnd, context),
            KeyBinding::new("ctrl-shift-home", SelectDocumentStart, context),
            KeyBinding::new("ctrl-shift-end", SelectDocumentEnd, context),
            KeyBinding::new("ctrl-shift-[", Fold, context),
            KeyBinding::new("ctrl-shift-]", Unfold, context),
        ]);
    }
    bindings
}

/// The modifier that adds a caret with a click: ⌘ on macOS, Ctrl elsewhere (CodeMirror's).
fn adds_caret(modifiers: &Modifiers) -> bool {
    if cfg!(target_os = "macos") { modifiers.platform } else { modifiers.control }
}

/// What the editor tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeEditorEvent {
    /// The user changed the text.
    Changed,
}

/// The suggestions shown under the caret, and the one picked.
struct OpenCompletion {
    completion: Completion,
    selected: usize,
}

/// Where the last frame drew things, to map the mouse and the input
/// methods onto the text.
#[derive(Clone, Default)]
struct Layout {
    bounds: Bounds<Pixels>,
    /// Where column 0 of the text is, the horizontal scroll included.
    text_left: Pixels,
    /// The fold markers' column, between the line numbers and the text.
    folds_left: Pixels,
    /// How wide the text may run before it wraps.
    text_width: Pixels,
    /// The primary caret's box in the window, for the completion list.
    caret: Option<Bounds<Pixels>>,
    /// Rows in view.
    rows: usize,
}

/// Each line's wrap breaks at one width, by the line's text, so an edit
/// re-wraps only the lines it changed.
#[derive(Default)]
struct WrapCache {
    width: Option<Pixels>,
    breaks: HashMap<u64, Vec<usize>>,
}

/// What the display map was built from, to know when to build it again.
#[derive(Clone, PartialEq)]
struct DisplayKey {
    version: u64,
    folds: Vec<Range<usize>>,
    wrap: Option<Pixels>,
}

/// The find bar: its two fields, and whether it shows.
struct FindBar {
    query: Entity<TextInput>,
    replacement: Entity<TextInput>,
    open: bool,
    _subscriptions: Vec<Subscription>,
}

/// The matches of the find bar's query, kept until the text or the query changes.
#[derive(Default)]
struct Matches {
    key: Option<(u64, SearchQuery)>,
    found: Vec<Range<usize>>,
}

pub struct CodeEditor {
    document: Document,
    syntax: Syntax,
    /// The state each line starts in, for highlighting; kept with the text.
    line_states: Vec<LineState>,
    /// The brackets, strings and what folds; kept with the text.
    structure: Structure,
    names: Option<Rc<CompletionNames>>,
    completion: Option<OpenCompletion>,
    focus: FocusHandle,
    /// How far the text is scrolled, right and down.
    scroll: Point<Pixels>,
    /// Scroll the caret into view on the next frame.
    autoscroll: bool,
    layout: Layout,
    selecting: bool,
    /// Shows its text, which can be selected and copied but not changed.
    read_only: bool,
    /// Long lines wrap instead of scrolling sideways.
    soft_wrap: bool,
    display: DisplayMap,
    display_key: Option<DisplayKey>,
    wraps: WrapCache,
    find: Option<FindBar>,
    query: SearchQuery,
    matches: Matches,
}

impl EventEmitter<CodeEditorEvent> for CodeEditor {}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CodeEditor {
    /// An editor holding `text`, highlighted (and completed, when it is SQL) as `syntax`.
    pub fn new(text: &str, syntax: impl Into<Syntax>, cx: &mut Context<Self>) -> Self {
        let mut editor = Self {
            document: Document::new(text),
            syntax: syntax.into(),
            line_states: Vec::new(),
            structure: Structure::default(),
            names: None,
            completion: None,
            focus: cx.focus_handle().tab_stop(true),
            scroll: point(px(0.), px(0.)),
            autoscroll: true,
            layout: Layout::default(),
            selecting: false,
            read_only: false,
            soft_wrap: false,
            display: DisplayMap::default(),
            display_key: None,
            wraps: WrapCache::default(),
            find: None,
            query: SearchQuery::default(),
            matches: Matches::default(),
        };
        let end = editor.text().len();
        editor.document.place(end, false);
        editor.restyle();
        editor.display = editor.document.lines_map();
        editor
    }

    pub fn text(&self) -> &str {
        self.document.text()
    }

    /// The primary selection's text, if any: what Run runs instead of the whole script.
    pub fn selected_text(&self) -> Option<&str> {
        let selection = self.document.primary().range();
        (!selection.is_empty()).then(|| &self.text()[selection])
    }

    /// Where the primary caret is, as a byte offset in [`CodeEditor::text`].
    pub fn caret(&self) -> usize {
        self.document.primary().head
    }

    pub fn syntax(&self) -> Syntax {
        self.syntax
    }

    /// Highlights, completes, comments and folds the text as `syntax` now (the
    /// cell inspector's Text and JSON modes).
    pub fn set_syntax(&mut self, syntax: impl Into<Syntax>, cx: &mut Context<Self>) {
        let syntax = syntax.into();
        if syntax != self.syntax {
            self.syntax = syntax;
            self.document.unfold(|_| true);
            self.restyle();
            cx.notify();
        }
    }

    /// Wraps long lines at the editor's width instead of scrolling sideways.
    pub fn set_soft_wrap(&mut self, soft_wrap: bool, cx: &mut Context<Self>) {
        if soft_wrap != self.soft_wrap {
            self.soft_wrap = soft_wrap;
            self.scroll.x = px(0.);
            self.autoscroll = true;
            cx.notify();
        }
    }

    pub fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }

    /// Puts the caret at byte `offset` (clamped to the text), nothing selected.
    pub fn place_caret(&mut self, offset: usize, cx: &mut Context<Self>) {
        let mut offset = offset.min(self.text().len());
        while !self.text().is_char_boundary(offset) {
            offset -= 1;
        }
        self.document.place(offset, false);
        self.autoscroll = true;
        cx.notify();
    }

    /// Replaces the whole text (undoably), the caret at its end.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let changed = self.document.replace_all(text);
        self.after_edit(changed, cx);
        self.completion = None;
    }

    /// Shows `text` from its top, the caret at its start: a read-only
    /// script (a table's DDL) read again.
    pub fn show_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.set_text(text, cx);
        self.document.place(0, false);
        self.scroll = point(px(0.), px(0.));
        self.autoscroll = false;
        cx.notify();
    }

    /// A read-only editor takes no typing, pasting or undo; the owner may still set its text.
    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        self.read_only = read_only;
        cx.notify();
    }

    /// The database's names to complete, besides keywords.
    pub fn set_completion_names(&mut self, names: Option<Rc<CompletionNames>>, cx: &mut Context<Self>) {
        self.names = names;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    /// The text changed: highlighting follows, and the owner hears of it.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.restyle();
        cx.emit(CodeEditorEvent::Changed);
    }

    fn restyle(&mut self) {
        self.line_states = match self.syntax.dialect() {
            Some(dialect) => highlight::line_states(self.text(), dialect),
            None => Vec::new(),
        };
        self.structure = Structure::analyze(self.text(), self.syntax);
    }

    /// After an edit: highlighting, the owner, and the caret in view.
    fn after_edit(&mut self, changed: bool, cx: &mut Context<Self>) {
        if changed {
            self.changed(cx);
        }
        self.autoscroll = true;
        cx.notify();
    }

    /// Runs an edit, keeping what to undo.
    fn edit(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Document) -> bool) {
        if self.read_only {
            return;
        }
        let changed = edit(&mut self.document);
        self.after_edit(changed, cx);
    }

    /// Runs a caret move; it closes the suggestions.
    fn caret_move(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Document)) {
        change(&mut self.document);
        self.completion = None;
        self.autoscroll = true;
        cx.notify();
    }

    /// Types `text` at every caret, closing brackets as the syntax does.
    fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let structure = &self.structure;
        let in_string = |at: usize| structure.in_string(at);
        let rules = BracketRules { brackets: self.syntax.closing_brackets(), in_string: &in_string };
        let changed = self.document.type_text(text, &rules);
        self.after_edit(changed, cx);
    }

    // The rows on screen

    /// Builds the rows on screen again when the text, the folds or the wrap width changed.
    fn sync_display(&mut self, window: &mut Window) {
        let wrap = (self.soft_wrap && self.layout.text_width > px(0.)).then_some(self.layout.text_width);
        let key = DisplayKey { version: self.document.version(), folds: self.document.folds().to_vec(), wrap };
        if self.display_key.as_ref() == Some(&key) {
            return;
        }
        let document = &self.document;
        self.display = match wrap {
            None => document.lines_map(),
            Some(width) => {
                if self.wraps.width != Some(width) || self.wraps.breaks.len() > 4 * document.line_count() + 1024 {
                    self.wraps = WrapCache { width: Some(width), breaks: HashMap::new() };
                }
                let mut wrapper = window.text_system().line_wrapper(editor_font(), px(FONT_SIZE));
                let cache = &mut self.wraps.breaks;
                let mut wrap = |line: &str| -> Vec<usize> {
                    if line.is_empty() {
                        return Vec::new();
                    }
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    line.hash(&mut hasher);
                    cache
                        .entry(hasher.finish())
                        .or_insert_with(|| {
                            let fragments = [LineFragment::text(line)];
                            wrapper.wrap_line(&fragments, width).map(|boundary| boundary.ix).collect()
                        })
                        .clone()
                };
                DisplayMap::build(document.text(), document.line_ranges(), document.folds(), Some(&mut wrap))
            }
        };
        self.display_key = Some(key);
    }

    // Completion

    /// Shows the suggestions at the caret, or closes them when there are none.
    fn complete(&mut self, explicit: bool, cx: &mut Context<Self>) {
        let Some(dialect) = self.syntax.dialect() else { return };
        if self.document.selections().count() > 1 {
            self.completion = None;
            return;
        }
        let found = completion::complete(self.text(), self.caret(), dialect, self.names.as_deref(), explicit);
        self.completion = found.map(|completion| OpenCompletion { completion, selected: 0 });
        cx.notify();
    }

    /// Inserts the picked suggestion over the word it completes.
    fn accept(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(open) = self.completion.take() else { return };
        let Some(item) = open.completion.items.get(index) else { return };
        let (range, label) = (open.completion.range.clone(), item.label.clone());
        self.edit(cx, |document| document.edit(EditKind::Other, |state| state.replace(Some(range), &label)));
    }

    /// Moves the picked suggestion; whether the list is open to take the key.
    fn pick(&mut self, step: isize, cx: &mut Context<Self>) -> bool {
        let Some(open) = self.completion.as_mut() else { return false };
        let count = open.completion.items.len() as isize;
        open.selected = (open.selected as isize + step).rem_euclid(count) as usize;
        cx.notify();
        true
    }

    // Keys

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let brackets = self.syntax.closing_brackets();
        self.edit(cx, |document| document.backspace(brackets));
        if self.completion.is_some() {
            self.complete(false, cx);
        }
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.completion = None;
        self.edit(cx, Document::delete);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.completion = None;
        self.edit(cx, Document::delete_word_left);
    }

    fn delete_to_line_start(&mut self, _: &DeleteToLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.completion = None;
        self.edit(cx, Document::delete_to_line_start);
    }

    /// Up or Down by `rows` rows on screen, every caret.
    fn move_rows(&mut self, rows: isize, select: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_display(window);
        let display = &self.display;
        self.document.move_vertically(rows, select, display);
        self.completion = None;
        self.autoscroll = true;
        cx.notify();
    }

    fn up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pick(-1, cx) {
            self.move_rows(-1, false, window, cx);
        }
    }

    fn down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pick(1, cx) {
            self.move_rows(1, false, window, cx);
        }
    }

    fn page(&self) -> isize {
        self.layout.rows.saturating_sub(1).max(1) as isize
    }

    fn page_up(&mut self, _: &PageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.move_rows(-self.page(), false, window, cx);
    }

    fn page_down(&mut self, _: &PageDown, window: &mut Window, cx: &mut Context<Self>) {
        self.move_rows(self.page(), false, window, cx);
    }

    fn add_cursor(&mut self, below: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_display(window);
        let display = &self.display;
        self.document.add_caret_vertically(below, display);
        self.completion = None;
        self.autoscroll = true;
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.document.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.document.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.edit(cx, Document::delete_selections);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
        self.completion = None;
        self.edit(cx, |document| document.paste(&text));
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.completion = None;
        self.edit(cx, Document::undo);
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.completion = None;
        self.edit(cx, Document::redo);
    }

    /// Enter: picks the suggestion when the list is open, else a new indented line.
    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = &self.completion {
            let selected = open.selected;
            return self.accept(selected, cx);
        }
        self.edit(cx, Document::newline);
    }

    /// Tab: picks the suggestion when the list is open, else indents.
    fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = &self.completion {
            let selected = open.selected;
            return self.accept(selected, cx);
        }
        self.edit(cx, Document::indent);
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.completion = None;
        self.edit(cx, Document::outdent);
    }

    /// ⌘/: comments the selected lines out or in; nothing in JSON or plain text.
    fn toggle_comment(&mut self, _: &ToggleComment, _: &mut Window, cx: &mut Context<Self>) {
        let Some(token) = self.syntax.line_comment() else { return };
        self.completion = None;
        self.edit(cx, |document| document.toggle_comment(token));
    }

    fn select_next_occurrence(&mut self, _: &SelectNextOccurrence, _: &mut Window, cx: &mut Context<Self>) {
        self.caret_move(cx, |document| {
            document.select_next_occurrence();
        });
    }

    /// Escape closes the suggestions, then the find bar, then leaves only
    /// the primary selection; otherwise it goes on to the owner.
    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.completion.take().is_some() {
            cx.notify();
        } else if self.find.as_ref().is_some_and(|find| find.open) {
            self.close_find(&CloseFind, window, cx);
        } else if self.document.single_selection() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    // Folds

    /// The line's foldable range: folded already, or what can fold on it.
    fn fold_on_line(&self, line: usize) -> Option<Range<usize>> {
        self.structure.fold_on_line(self.document.line_range(line))
    }

    /// Whether a fold starts on `line`.
    fn folded_on_line(&self, line: usize) -> bool {
        let range = self.document.line_range(line);
        self.document.folds().iter().any(|fold| range.start <= fold.start && fold.start <= range.end)
    }

    /// Folds what can fold on each caret's line (⌘⌥[).
    fn fold(&mut self, _: &Fold, _: &mut Window, cx: &mut Context<Self>) {
        let lines: Vec<usize> =
            self.document.selections().all().iter().map(|selection| self.document.row_of(selection.head)).collect();
        for line in lines {
            if let Some(range) = self.fold_on_line(line) {
                self.document.fold(range);
            }
        }
        cx.notify();
    }

    /// Unfolds what is folded on each caret's line (⌘⌥]).
    fn unfold(&mut self, _: &Unfold, _: &mut Window, cx: &mut Context<Self>) {
        let lines: Vec<Range<usize>> = self
            .document
            .selections()
            .all()
            .iter()
            .map(|selection| self.document.line_range(self.document.row_of(selection.head)))
            .collect();
        self.document.unfold(|fold| lines.iter().any(|line| line.start <= fold.start && fold.start <= line.end));
        cx.notify();
    }

    /// Folds every outermost range that can fold, as CodeMirror's `foldAll`.
    fn fold_all(&mut self, _: &FoldAll, _: &mut Window, cx: &mut Context<Self>) {
        let mut line = 0;
        while line < self.document.line_count() {
            match self.fold_on_line(line) {
                Some(range) => {
                    line = self.document.row_of(range.end) + 1;
                    self.document.fold(range);
                }
                None => line += 1,
            }
        }
        cx.notify();
    }

    fn unfold_all(&mut self, _: &UnfoldAll, _: &mut Window, cx: &mut Context<Self>) {
        self.document.unfold(|_| true);
        cx.notify();
    }

    /// The gutter's marker on `line`: folds what can fold there, or unfolds it.
    fn toggle_fold(&mut self, line: usize, cx: &mut Context<Self>) {
        if self.folded_on_line(line) {
            let range = self.document.line_range(line);
            self.document.unfold(|fold| range.start <= fold.start && fold.start <= range.end);
        } else if let Some(range) = self.fold_on_line(line) {
            self.document.fold(range);
        }
        cx.notify();
    }

    // The mouse

    /// Lays out `text` as the rows are drawn, to find columns under the mouse.
    fn shape(&self, text: String, window: &mut Window) -> ShapedLine {
        let run = TextRun {
            len: text.len(),
            font: editor_font(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window.text_system().shape_line(text.into(), px(FONT_SIZE), &[run], None)
    }

    /// The row on screen under `y`, if any.
    fn row_at(&self, y: Pixels) -> Option<usize> {
        let y = y - self.layout.bounds.top() + self.scroll.y - px(element::PADDING_TOP);
        let row = (f32::from(y) / LINE_HEIGHT).floor().max(0.) as usize;
        (row < self.display.row_count()).then_some(row)
    }

    /// The row under `position` and how far into its shown text (in bytes).
    fn hit(&self, position: Point<Pixels>, window: &mut Window) -> Option<(usize, usize)> {
        let row = self.row_at(position.y)?;
        let shown = self.display.row_text(self.text(), row);
        let x = (position.x - self.layout.text_left).max(px(0.));
        Some((row, self.shape(shown, window).closest_index_for_x(x)))
    }

    /// The offset in the text under `position`.
    fn offset_at(&mut self, position: Point<Pixels>, window: &mut Window) -> usize {
        self.sync_display(window);
        match self.hit(position, window) {
            Some((row, shown)) => self.display.offset_at(row, shown),
            None => self.text().len(),
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window);
        self.completion = None;
        self.sync_display(window);
        // The fold markers' column folds and unfolds the line.
        let in_folds = event.position.x >= self.layout.folds_left
            && event.position.x < self.layout.folds_left + px(element::FOLD_WIDTH);
        if in_folds && let Some(row) = self.row_at(event.position.y) {
            let row = self.display.row(row);
            if row.first {
                return self.toggle_fold(row.line, cx);
            }
        }
        // A fold's placeholder unfolds it.
        if let Some((row, shown)) = self.hit(event.position, window)
            && let Some(fold) = self.display.fold_at(row, shown)
        {
            self.document.unfold(|other| *other == fold);
            return cx.notify();
        }
        let offset = self.offset_at(event.position, window);
        match event.click_count {
            2 => self.document.select_word(offset),
            3.. => {
                let row = self.document.row_of(offset);
                self.document.select_line(row);
            }
            _ if adds_caret(&event.modifiers) => {
                if !self.document.remove_caret(offset) {
                    self.document.add_selection(Selection::caret(offset));
                }
            }
            _ => {
                self.selecting = true;
                self.document.place(offset, event.modifiers.shift);
            }
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            let offset = self.offset_at(event.position, window);
            self.document.place(offset, true);
            self.autoscroll = true;
            cx.notify();
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
        // Shift turns a wheel's vertical turn sideways, where the OS doesn't.
        let delta = if event.modifiers.shift && delta.x == px(0.) { point(delta.y, px(0.)) } else { delta };
        self.scroll = point(self.scroll.x - delta.x, self.scroll.y - delta.y);
        self.autoscroll = false;
        cx.notify();
    }

    /// A line's highlighted spans, by byte range in the line.
    fn line_spans(&self, line: usize) -> Vec<(Range<usize>, Kind)> {
        use crate::sql::document;
        let text = self.document.line(line);
        match (self.syntax, self.line_states.get(line)) {
            (Syntax::Sql(dialect), Some(state)) => highlight::tokenize(text, state.clone(), dialect)
                .0
                .into_iter()
                .map(|span| (span.range, span.kind))
                .collect(),
            (Syntax::Json | Syntax::Mongosh, _) => document::spans(text, self.syntax == Syntax::Mongosh)
                .into_iter()
                .map(|span| (span.range, span.kind))
                .collect(),
            _ => vec![(0..text.len(), Kind::Plain)],
        }
    }
}

/// The editor's font.
fn editor_font() -> Font {
    font(MONO)
}

/// The editor's font, bold or italic as a span is highlighted.
fn styled_font(weight: FontWeight, style: FontStyle) -> Font {
    Font { weight, style, ..editor_font() }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let state = self.document.state();
        let range = state.range_from_utf16(&range_utf16);
        actual_range.replace(state.range_to_utf16(&range));
        Some(self.text()[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let state = self.document.state();
        Some(UTF16Selection { range: state.range_to_utf16(&state.selection()), reversed: state.is_reversed() })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        let state = self.document.state();
        state.marked().map(|range| state.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.document.edit(EditKind::Typing, |state| {
            state.unmark();
            false
        });
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.document.state().range_from_utf16(&range));
        let composing = self.document.state().marked().is_some();
        if range.is_none() && !composing {
            // Plain typing goes to every caret, closing brackets.
            self.type_text(new_text, cx);
        } else {
            self.edit(cx, |document| {
                document.edit(EditKind::Typing, |state| state.replace(range, new_text) || composing)
            });
        }
        // Typing a name, or the dot after one, offers what can follow.
        let completes = new_text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') && !new_text.is_empty();
        if completes {
            self.complete(false, cx);
        } else {
            self.completion = None;
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.document.state().range_from_utf16(&range));
        let selected = new_selected_range_utf16
            .map(|selected| utf8_offset(new_text, selected.start)..utf8_offset(new_text, selected.end));
        self.edit(cx, |document| {
            document.edit(EditKind::Typing, |state| {
                state.replace_and_mark(range, new_text, selected);
                true
            })
        });
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.sync_display(window);
        let range = self.document.state().range_from_utf16(&range_utf16);
        let row = self.display.row_of(range.start);
        let line = self.shape(self.display.row_text(self.text(), row), window);
        let top = bounds.top() + px(element::PADDING_TOP) + px(LINE_HEIGHT * row as f32) - self.scroll.y;
        let x = |offset: usize| self.layout.text_left + line.x_for_index(self.display.display_index(row, offset));
        Some(Bounds::from_corners(point(x(range.start), top), point(x(range.end), top + px(LINE_HEIGHT))))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_at(point, window);
        Some(self.document.state().to_utf16(offset))
    }
}

impl Render for CodeEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = div()
            .id("code-editor")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .size_full()
            .relative()
            .overflow_hidden()
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(|this, _: &Left, _, cx| this.caret_move(cx, |d| d.left(false))))
            .on_action(cx.listener(|this, _: &Right, _, cx| this.caret_move(cx, |d| d.right(false))))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.caret_move(cx, |d| d.left(true))))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.caret_move(cx, |d| d.right(true))))
            .on_action(cx.listener(|this, _: &SelectUp, window, cx| this.move_rows(-1, true, window, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, window, cx| this.move_rows(1, true, window, cx)))
            .on_action(cx.listener(|this, _: &WordLeft, _, cx| this.caret_move(cx, |d| d.word_left(false))))
            .on_action(cx.listener(|this, _: &WordRight, _, cx| this.caret_move(cx, |d| d.word_right(false))))
            .on_action(cx.listener(|this, _: &SelectWordLeft, _, cx| this.caret_move(cx, |d| d.word_left(true))))
            .on_action(cx.listener(|this, _: &SelectWordRight, _, cx| this.caret_move(cx, |d| d.word_right(true))))
            .on_action(cx.listener(|this, _: &LineStart, _, cx| this.caret_move(cx, |d| d.line_home(false))))
            .on_action(cx.listener(|this, _: &LineEnd, _, cx| this.caret_move(cx, |d| d.line_end(false))))
            .on_action(cx.listener(|this, _: &SelectLineStart, _, cx| this.caret_move(cx, |d| d.line_home(true))))
            .on_action(cx.listener(|this, _: &SelectLineEnd, _, cx| this.caret_move(cx, |d| d.line_end(true))))
            .on_action(cx.listener(|this, _: &DocumentStart, _, cx| this.caret_move(cx, |d| d.place(0, false))))
            .on_action(
                cx.listener(|this, _: &DocumentEnd, _, cx| this.caret_move(cx, |d| d.place(d.text().len(), false))),
            )
            .on_action(cx.listener(|this, _: &SelectDocumentStart, _, cx| this.caret_move(cx, |d| d.place(0, true))))
            .on_action(
                cx.listener(|this, _: &SelectDocumentEnd, _, cx| {
                    this.caret_move(cx, |d| d.place(d.text().len(), true))
                }),
            )
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.caret_move(cx, Document::select_all)))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::toggle_comment))
            .on_action(cx.listener(Self::select_next_occurrence))
            .on_action(cx.listener(|this, _: &AddCursorAbove, window, cx| this.add_cursor(false, window, cx)))
            .on_action(cx.listener(|this, _: &AddCursorBelow, window, cx| this.add_cursor(true, window, cx)))
            .on_action(cx.listener(Self::fold))
            .on_action(cx.listener(Self::unfold))
            .on_action(cx.listener(Self::fold_all))
            .on_action(cx.listener(Self::unfold_all))
            .on_action(cx.listener(|this, _: &ShowCompletions, _, cx| this.complete(true, cx)))
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(|_, _: &ShowCharacterPalette, window, _| window.show_character_palette()))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(EditorElement::new(cx.entity()))
            .children(self.completion_list(cx));
        // The find bar floats over the editor's top right, so opening it moves no text.
        div()
            .size_full()
            .relative()
            .on_action(cx.listener(Self::open_find))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::close_find))
            .child(editor)
            .children(self.find_bar(window, cx))
    }
}

#[cfg(test)]
mod tests;
