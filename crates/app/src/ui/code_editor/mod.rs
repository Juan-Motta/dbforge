//! A multi-line code editor: SQL in the console, and later DDL and mongosh.
//! Built on GPUI's input handler like the one-line [`super::TextInput`], so
//! the OS's input methods work (dead keys, ñ, emoji, CJK). Only the lines in
//! view are laid out and drawn, so a long script costs no more than a short
//! one. It has line numbers, the current line marked, a selection made with
//! the keyboard or the mouse, the clipboard, undo and redo, indentation, and
//! with a SQL dialect highlighting and completion.
//!
//! The owner reads [`CodeEditor::text`] and the caret, and binds its own
//! commands (Run) in a key context around the editor.

mod completion_list;
pub mod document;
mod element;

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font,
    FontStyle, FontWeight, InteractiveElement, IntoElement, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Render, ScrollWheelEvent, ShapedLine, Styled, TextRun, UTF16Selection,
    Window, actions, div, font, point, px,
};

use self::document::{Document, EditKind};
use self::element::EditorElement;
use crate::sql::Dialect;
use crate::sql::completion::{self, Completion, CompletionNames};
use crate::sql::highlight::{self, LineState};
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
    ]
);

const CONTEXT: &str = "CodeEditor";

/// The editor's text: its font size and line height.
pub const FONT_SIZE: f32 = 13.;
pub const LINE_HEIGHT: f32 = 20.;

/// The keys the editor answers, with each OS's own modifiers: ⌥ moves by
/// word and ⌘ to the line's or the text's ends on macOS; Ctrl moves by word
/// and Ctrl+Home/End to the text's ends elsewhere.
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
        // Ctrl+Y stays the console's history, as in the web UI.
        KeyBinding::new("secondary-shift-z", Redo, context),
        KeyBinding::new("enter", Newline, context),
        KeyBinding::new("tab", Indent, context),
        KeyBinding::new("shift-tab", Outdent, context),
        KeyBinding::new("ctrl-space", ShowCompletions, context),
        KeyBinding::new("escape", Escape, context),
    ];
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
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("ctrl-home", DocumentStart, context),
            KeyBinding::new("ctrl-end", DocumentEnd, context),
            KeyBinding::new("ctrl-shift-home", SelectDocumentStart, context),
            KeyBinding::new("ctrl-shift-end", SelectDocumentEnd, context),
        ]);
    }
    bindings
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
    /// The caret's box in the window, for the completion list.
    caret: Option<Bounds<Pixels>>,
    /// Rows in view.
    rows: usize,
}

pub struct CodeEditor {
    document: Document,
    dialect: Option<Dialect>,
    /// The state each line starts in, for highlighting; kept with the text.
    line_states: Vec<LineState>,
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
}

impl EventEmitter<CodeEditorEvent> for CodeEditor {}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CodeEditor {
    /// An editor holding `text`, highlighted and completed in `dialect` when it has one.
    pub fn new(text: &str, dialect: Option<Dialect>, cx: &mut Context<Self>) -> Self {
        let mut editor = Self {
            document: Document::new(text),
            dialect,
            line_states: Vec::new(),
            names: None,
            completion: None,
            focus: cx.focus_handle().tab_stop(true),
            scroll: point(px(0.), px(0.)),
            autoscroll: true,
            layout: Layout::default(),
            selecting: false,
            read_only: false,
        };
        editor.document.caret(|state| state.end());
        editor.restyle();
        editor
    }

    pub fn text(&self) -> &str {
        self.document.text()
    }

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<&str> {
        let selection = self.document.state().selection();
        (!selection.is_empty()).then(|| &self.text()[selection])
    }

    /// Where the caret is, as a byte offset in [`CodeEditor::text`].
    pub fn caret(&self) -> usize {
        self.document.state().cursor()
    }

    pub fn dialect(&self) -> Option<Dialect> {
        self.dialect
    }

    /// Replaces the whole text (undoably), the caret at its end.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.document.replace_all(text) {
            self.changed(cx);
        }
        self.completion = None;
        self.autoscroll = true;
        cx.notify();
    }

    /// Shows `text` from its top, the caret at its start: a read-only
    /// script (a table's DDL) read again.
    pub fn show_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.set_text(text, cx);
        self.document.caret(|state| state.move_to(0));
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
        self.line_states = match self.dialect {
            Some(dialect) => highlight::line_states(self.text(), dialect),
            None => Vec::new(),
        };
    }

    /// Runs an edit, keeping what to undo.
    fn edit(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Document) -> bool) {
        if self.read_only {
            return;
        }
        if edit(&mut self.document) {
            self.changed(cx);
        }
        self.autoscroll = true;
        cx.notify();
    }

    /// Runs a caret move; it closes the suggestions.
    fn caret_move(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Document)) {
        change(&mut self.document);
        self.completion = None;
        self.autoscroll = true;
        cx.notify();
    }

    // Completion

    /// Shows the suggestions at the caret, or closes them when there are none.
    fn complete(&mut self, explicit: bool, cx: &mut Context<Self>) {
        let Some(dialect) = self.dialect else { return };
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
        self.edit(cx, |document| document.edit(EditKind::Deleting, |state| state.backspace()));
        if self.completion.is_some() {
            self.complete(false, cx);
        }
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.completion = None;
        self.edit(cx, |document| document.edit(EditKind::Deleting, |state| state.delete()));
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.completion = None;
        self.edit(cx, |document| document.edit(EditKind::Other, |state| state.delete_word_left()));
    }

    fn delete_to_line_start(&mut self, _: &DeleteToLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.completion = None;
        self.edit(cx, |document| {
            if document.state().selection().is_empty() {
                let (row, _) = document.caret_position();
                let start = document.line_range(row).start;
                document.place(start, true);
            }
            document.edit(EditKind::Other, |state| state.replace(None, ""))
        });
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if !self.pick(-1, cx) {
            self.caret_move(cx, |document| document.move_vertically(-1, false));
        }
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if !self.pick(1, cx) {
            self.caret_move(cx, |document| document.move_vertically(1, false));
        }
    }

    fn page(&self) -> isize {
        self.layout.rows.saturating_sub(1).max(1) as isize
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.page();
        self.caret_move(cx, |document| document.move_vertically(-page, false));
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.page();
        self.caret_move(cx, |document| document.move_vertically(page, false));
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
            self.edit(cx, |document| document.edit(EditKind::Other, |state| state.replace(None, "")));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
        self.completion = None;
        self.edit(cx, |document| document.edit(EditKind::Other, |state| state.replace(None, &text)));
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

    /// Escape closes the suggestions; otherwise it goes on to the owner.
    fn escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        if self.completion.take().is_some() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    // The mouse

    /// Lays out line `row` as it is drawn, to find columns under the mouse.
    fn shape_row(&self, row: usize, window: &mut Window) -> ShapedLine {
        let text = self.document.line(row).to_owned();
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

    /// The offset in the text under `position`.
    fn offset_at(&self, position: Point<Pixels>, window: &mut Window) -> usize {
        let bounds = self.layout.bounds;
        let y = position.y - bounds.top() + self.scroll.y - px(element::PADDING_TOP);
        let row = (f32::from(y) / LINE_HEIGHT).floor().max(0.) as usize;
        if row >= self.document.line_count() {
            return self.text().len();
        }
        let column = self.shape_row(row, window).closest_index_for_x((position.x - self.layout.text_left).max(px(0.)));
        self.document.offset(row, column)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window);
        let offset = self.offset_at(event.position, window);
        self.completion = None;
        match event.click_count {
            2 => self.document.select_word(offset),
            3.. => {
                let row = self.document.row_of(offset);
                self.document.select_line(row);
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
        self.document.caret(|state| state.unmark());
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
        self.edit(cx, |document| document.edit(EditKind::Typing, |state| state.replace(range, new_text) || composing));
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
        let range = self.document.state().range_from_utf16(&range_utf16);
        let row = self.document.row_of(range.start);
        let line_start = self.document.line_range(row).start;
        let line = self.shape_row(row, window);
        let top = bounds.top() + px(element::PADDING_TOP) + px(LINE_HEIGHT * row as f32) - self.scroll.y;
        let end = range.end.min(self.document.line_range(row).end);
        Some(Bounds::from_corners(
            point(self.layout.text_left + line.x_for_index(range.start - line_start), top),
            point(self.layout.text_left + line.x_for_index(end - line_start), top + px(LINE_HEIGHT)),
        ))
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
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
            .on_action(cx.listener(|this, _: &Left, _, cx| this.caret_move(cx, |d| d.caret(|s| s.left()))))
            .on_action(cx.listener(|this, _: &Right, _, cx| this.caret_move(cx, |d| d.caret(|s| s.right()))))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.caret_move(cx, |d| d.caret(|s| s.select_left()))))
            .on_action(
                cx.listener(|this, _: &SelectRight, _, cx| this.caret_move(cx, |d| d.caret(|s| s.select_right()))),
            )
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.caret_move(cx, |d| d.move_vertically(-1, true))))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.caret_move(cx, |d| d.move_vertically(1, true))))
            .on_action(cx.listener(|this, _: &WordLeft, _, cx| this.caret_move(cx, |d| d.caret(|s| s.word_left()))))
            .on_action(cx.listener(|this, _: &WordRight, _, cx| this.caret_move(cx, |d| d.caret(|s| s.word_right()))))
            .on_action(
                cx.listener(|this, _: &SelectWordLeft, _, cx| {
                    this.caret_move(cx, |d| d.caret(|s| s.select_word_left()))
                }),
            )
            .on_action(cx.listener(|this, _: &SelectWordRight, _, cx| {
                this.caret_move(cx, |d| d.caret(|s| s.select_word_right()))
            }))
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
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.caret_move(cx, |d| d.caret(|s| s.select_all()))))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(|this, _: &ShowCompletions, _, cx| this.complete(true, cx)))
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(|_, _: &ShowCharacterPalette, window, _| window.show_character_palette()))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(EditorElement::new(cx.entity()))
            .children(self.completion_list(cx))
    }
}

#[cfg(test)]
mod tests {
    use dbforge_core::drivers::{CompletionColumn, CompletionSchema, CompletionTable, ObjectKind};
    use gpui::TestAppContext;

    use super::*;
    use crate::theme::{Appearance, Theme};

    #[gpui::test]
    fn types_through_the_input_handler_completes_and_draws(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(Theme::new(Appearance::Dark)));
        let (editor, cx) = cx.add_window_view(|_, cx| CodeEditor::new("", Some(Dialect::Postgres), cx));
        editor.update_in(cx, |editor, window, cx| {
            let schema = CompletionSchema {
                database: "shop".into(),
                tables: vec![CompletionTable {
                    schema: "public".into(),
                    name: "orders".into(),
                    kind: ObjectKind::Table,
                    columns: vec![CompletionColumn { name: "id".into(), data_type: "integer".into() }],
                }],
            };
            editor.set_completion_names(Some(Rc::new(CompletionNames::new(&schema, Dialect::Postgres))), cx);
            editor.focus(window);
            for typed in ["SELECT * FROM ", "o", "r"] {
                editor.replace_text_in_range(None, typed, window, cx);
            }
            let offered: Vec<&str> = editor
                .completion
                .as_ref()
                .map(|open| open.completion.items.iter().map(|item| item.label.as_str()).collect())
                .unwrap_or_default();
            assert_eq!(offered[0], "orders");
            editor.newline(&Newline, window, cx);
            assert_eq!(editor.text(), "SELECT * FROM orders", "Enter picks the suggestion");
            editor.newline(&Newline, window, cx);
            editor.replace_text_in_range(None, "-- ñ", window, cx);
            assert_eq!(editor.text(), "SELECT * FROM orders\n-- ñ");
            // A dead key composes, then commits.
            editor.replace_and_mark_text_in_range(None, "´", None, window, cx);
            editor.replace_text_in_range(None, "é", window, cx);
            assert!(editor.text().ends_with("ñé"));
        });
        cx.update(|window, cx| window.draw(cx).clear());
        editor.update_in(cx, |editor, window, cx| {
            assert!(editor.layout.caret.is_some(), "the caret was placed on screen");
            editor.undo(&Undo, window, cx);
            assert_eq!(editor.text(), "SELECT * FROM orders\n");
            editor.set_text(&"SELECT 1;\n".repeat(5000), cx);
            assert_eq!(editor.document.line_count(), 5001);
        });
        // A long script draws only the lines in view.
        cx.update(|window, cx| window.draw(cx).clear());
        editor.read_with(cx, |editor, _| assert!(editor.scroll.y > px(0.), "the caret at the end is in view"));
    }
}
