//! A single-line text field, built on GPUI's input handler so the OS's input
//! methods work: dead keys and accents, ñ, emoji and CJK composition. It
//! draws only the text, the caret and the selection, at a fixed height; the
//! caller frames it (a capsule, a form field) and listens to its
//! [`TextInputEvent`]s. A masked field (a password) draws a bullet for each
//! character and never puts its text on the clipboard; a disabled one takes
//! neither the keyboard nor the mouse.

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, InteractiveElement, IntoElement,
    KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, Pixels,
    Point, Render, ShapedLine, SharedString, Style, Styled, TextRun, UTF16Selection, UnderlineStyle, Window, actions,
    div, fill, point, prelude::FluentBuilder, px, relative, size,
};

use super::editing::{EditState, utf8_offset};
use crate::theme::ActiveTheme;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        Left,
        Right,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        Home,
        End,
        SelectHome,
        SelectEnd,
        Copy,
        Cut,
        Paste,
        Submit,
        Cancel,
        ShowCharacterPalette,
    ]
);

const CONTEXT: &str = "TextInput";

/// The field's height, whatever it holds: the layout never moves while typing.
pub const HEIGHT: f32 = 20.;

/// The keys every text field answers, with each OS's own modifiers: ⌥ moves
/// by word and ⌘ to the ends on macOS, Ctrl moves by word elsewhere.
pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    let word = if cfg!(target_os = "macos") { "alt" } else { "ctrl" };
    let mut bindings = vec![
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("shift-backspace", Backspace, context),
        KeyBinding::new(&format!("{word}-backspace"), DeleteWordLeft, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new(&format!("{word}-left"), WordLeft, context),
        KeyBinding::new(&format!("{word}-right"), WordRight, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new(&format!("{word}-shift-left"), SelectWordLeft, context),
        KeyBinding::new(&format!("{word}-shift-right"), SelectWordRight, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("shift-home", SelectHome, context),
        KeyBinding::new("shift-end", SelectEnd, context),
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("secondary-c", Copy, context),
        KeyBinding::new("secondary-x", Cut, context),
        KeyBinding::new("secondary-v", Paste, context),
        KeyBinding::new("enter", Submit, context),
        KeyBinding::new("escape", Cancel, context),
    ];
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("cmd-left", Home, context),
            KeyBinding::new("cmd-right", End, context),
            KeyBinding::new("cmd-shift-left", SelectHome, context),
            KeyBinding::new("cmd-shift-right", SelectEnd, context),
            KeyBinding::new("cmd-backspace", SelectHome, context),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, context),
        ]);
    }
    bindings
}

/// What a text field tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInputEvent {
    /// The user changed the text (typing, deleting, pasting, composing).
    Changed,
    /// Enter.
    Submitted,
    /// Escape.
    Cancelled,
    /// Pasting, on a field that hands pastes to its owner
    /// ([`TextInput::intercept_paste`]): nothing was inserted; the owner
    /// inserts the text ([`TextInput::insert`]) or uses it otherwise.
    Pasted(String),
}

/// What a masked field draws for each character.
const BULLET: char = '•';

pub struct TextInput {
    state: EditState,
    placeholder: SharedString,
    focus: FocusHandle,
    /// The last drawn line and where, to place the caret under the mouse.
    layout: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    /// How far the text is scrolled left so the caret stays in view.
    scroll: Pixels,
    selecting: bool,
    masked: bool,
    disabled: bool,
    intercept_paste: bool,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            state: EditState::default(),
            placeholder: placeholder.into(),
            focus: cx.focus_handle().tab_stop(true),
            layout: None,
            bounds: None,
            scroll: px(0.),
            selecting: false,
            masked: false,
            disabled: false,
            intercept_paste: false,
        }
    }

    /// Draws a bullet for each character, and never copies the text: a password.
    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    /// Hands each paste to the owner as [`TextInputEvent::Pasted`] instead of inserting it.
    pub fn intercept_paste(mut self) -> Self {
        self.intercept_paste = true;
        self
    }

    /// A disabled field shows its text dimmed and takes neither the keyboard
    /// (it leaves the tab order) nor the mouse.
    pub fn set_disabled(&mut self, disabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled == disabled {
            return;
        }
        self.disabled = disabled;
        self.focus = self.focus.clone().tab_stop(!disabled);
        if disabled && self.focus.is_focused(window) {
            window.blur();
        }
        cx.notify();
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Inserts `text` where the caret is, replacing the selection, as typing it would.
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        self.edit(cx, |state| state.replace(None, text));
    }

    pub fn text(&self) -> &str {
        self.state.text()
    }

    /// Replaces the text, caret at its end, without a [`TextInputEvent::Changed`]:
    /// the owner set it. The same text leaves the caret where it is.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.state.text() == text {
            return;
        }
        self.state = EditState::new(text);
        cx.notify();
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// Selects the whole text, so typing replaces it.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.state.select_all();
        cx.notify();
    }

    /// Takes the keyboard.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    /// Runs `edit` and tells the owner when the text changed.
    fn edit(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut EditState) -> bool) {
        if self.disabled {
            return;
        }
        let changed = edit(&mut self.state);
        if changed {
            cx.emit(TextInputEvent::Changed);
        }
        cx.notify();
    }

    /// Runs a caret move or selection change.
    fn caret(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut EditState)) {
        change(&mut self.state);
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.masked && !self.state.selection().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.state.selected_text().to_owned()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.masked && !self.state.selection().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.state.selected_text().to_owned()));
            self.edit(cx, |state| state.replace(None, ""));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
        if self.disabled {
            return;
        }
        if self.intercept_paste {
            cx.emit(TextInputEvent::Pasted(text));
        } else {
            self.edit(cx, |state| state.replace(None, &text));
        }
    }

    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (self.bounds.as_ref(), self.layout.as_ref()) else {
            return 0;
        };
        if self.state.text().is_empty() {
            return 0;
        }
        self.text_offset(line.closest_index_for_x(position.x - bounds.left() + self.scroll))
    }

    /// The text as drawn: the text, or a bullet for each of its characters.
    fn shown_text(&self) -> String {
        if self.masked {
            BULLET.to_string().repeat(self.state.text().chars().count())
        } else {
            self.state.text().into()
        }
    }

    /// Where `offset` in the text is in the text as drawn.
    fn shown_offset(&self, offset: usize) -> usize {
        if self.masked { masked_offset(self.state.text(), offset) } else { offset }
    }

    /// Where `offset` in the text as drawn is in the text.
    fn text_offset(&self, offset: usize) -> usize {
        if self.masked { unmasked_offset(self.state.text(), offset) } else { offset }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.focus(window);
        let index = self.index_for_position(event.position);
        if event.click_count >= 2 {
            self.caret(cx, EditState::select_all);
            return;
        }
        self.selecting = true;
        if event.modifiers.shift {
            self.caret(cx, |state| state.select_to(index));
        } else {
            self.caret(cx, |state| state.move_to(index));
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            let index = self.index_for_position(event.position);
            self.caret(cx, |state| state.select_to(index));
        }
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.state.range_from_utf16(&range_utf16);
        actual_range.replace(self.state.range_to_utf16(&range));
        let text = &self.state.text()[range];
        // A masked field never hands its text to the OS (input methods,
        // accessibility): one bullet per UTF-16 unit keeps the ranges valid.
        Some(if self.masked { "•".repeat(text.encode_utf16().count()) } else { text.to_owned() })
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.state.range_to_utf16(&self.state.selection()),
            reversed: self.state.is_reversed(),
        })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.state.marked().map(|range| self.state.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.state.unmark();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.state.range_from_utf16(&range));
        // Committing a composition changes the text even when it reads the same.
        let composing = self.state.marked().is_some();
        self.edit(cx, |state| state.replace(range, new_text) || composing);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.state.range_from_utf16(&range));
        // The selection is in UTF-16 units of the new text.
        let selected = new_selected_range_utf16
            .map(|selected| utf8_offset(new_text, selected.start)..utf8_offset(new_text, selected.end));
        self.edit(cx, |state| {
            state.replace_and_mark(range, new_text, selected);
            true
        });
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.layout.as_ref()?;
        let range = self.state.range_from_utf16(&range_utf16);
        let left = bounds.left() - self.scroll;
        Some(Bounds::from_corners(
            point(left + line.x_for_index(self.shown_offset(range.start)), bounds.top()),
            point(left + line.x_for_index(self.shown_offset(range.end)), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.bounds?.localize(&point)?;
        let line = self.layout.as_ref()?;
        let index = self.text_offset(line.index_for_x(local.x + self.scroll)?);
        Some(self.state.to_utf16(index))
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(|this, _: &Backspace, _, cx| this.edit(cx, EditState::backspace)))
            .on_action(cx.listener(|this, _: &Delete, _, cx| this.edit(cx, EditState::delete)))
            .on_action(cx.listener(|this, _: &DeleteWordLeft, _, cx| this.edit(cx, EditState::delete_word_left)))
            .on_action(cx.listener(|this, _: &Left, _, cx| this.caret(cx, EditState::left)))
            .on_action(cx.listener(|this, _: &Right, _, cx| this.caret(cx, EditState::right)))
            .on_action(cx.listener(|this, _: &WordLeft, _, cx| this.caret(cx, EditState::word_left)))
            .on_action(cx.listener(|this, _: &WordRight, _, cx| this.caret(cx, EditState::word_right)))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.caret(cx, EditState::select_left)))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.caret(cx, EditState::select_right)))
            .on_action(cx.listener(|this, _: &SelectWordLeft, _, cx| this.caret(cx, EditState::select_word_left)))
            .on_action(cx.listener(|this, _: &SelectWordRight, _, cx| this.caret(cx, EditState::select_word_right)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.caret(cx, EditState::select_all)))
            .on_action(cx.listener(|this, _: &Home, _, cx| this.caret(cx, EditState::home)))
            .on_action(cx.listener(|this, _: &End, _, cx| this.caret(cx, EditState::end)))
            .on_action(cx.listener(|this, _: &SelectHome, _, cx| this.caret(cx, EditState::select_home)))
            .on_action(cx.listener(|this, _: &SelectEnd, _, cx| this.caret(cx, EditState::select_end)))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(|_, _: &Submit, _, cx| cx.emit(TextInputEvent::Submitted)))
            .on_action(cx.listener(|_, _: &Cancel, _, cx| cx.emit(TextInputEvent::Cancelled)))
            .on_action(cx.listener(|_, _: &ShowCharacterPalette, window, _| window.show_character_palette()))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .flex_1()
            .min_w_0()
            .h(px(HEIGHT))
            .line_height(px(HEIGHT))
            .overflow_hidden()
            .when(self.disabled, |field| field.cursor_default().text_color(cx.theme().text3))
            .child(TextElement { input: cx.entity() })
    }
}

/// Where byte `offset` of `text` falls once each character is drawn as a bullet.
fn masked_offset(text: &str, offset: usize) -> usize {
    text[..offset].chars().count() * BULLET.len_utf8()
}

/// The byte of `text` at byte `offset` of its bullets.
fn unmasked_offset(text: &str, offset: usize) -> usize {
    text.char_indices().nth(offset / BULLET.len_utf8()).map_or(text.len(), |(index, _)| index)
}

/// Draws the input's line, selection and caret, and registers it with the
/// OS's input methods.
struct TextElement {
    input: Entity<TextInput>,
}

struct Prepainted {
    line: ShapedLine,
    caret: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(HEIGHT).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let theme = *cx.theme();
        let input = self.input.read(cx);
        let state = &input.state;
        let style = window.text_style();
        let (text, color) = if state.text().is_empty() {
            (input.placeholder.clone(), theme.text3)
        } else {
            (SharedString::from(input.shown_text()), style.color)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked = state.marked().map(|marked| input.shown_offset(marked.start)..input.shown_offset(marked.end));
        let runs = match marked.filter(|_| !state.text().is_empty()) {
            Some(marked) => [
                TextRun { len: marked.start, ..run.clone() },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false }),
                    ..run.clone()
                },
                TextRun { len: text.len() - marked.end, ..run },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(text, font_size, &runs, None);

        // Scrolls just enough to keep the caret inside the field.
        let caret_x =
            if state.text().is_empty() { px(0.) } else { line.x_for_index(input.shown_offset(state.cursor())) };
        let width = bounds.size.width - px(2.);
        let mut scroll = input.scroll;
        if caret_x < scroll {
            scroll = caret_x;
        } else if caret_x > scroll + width {
            scroll = caret_x - width;
        }
        scroll = scroll.min((line.width - width).max(px(0.))).max(px(0.));
        let left = bounds.left() - scroll;

        let selection = state.selection();
        let (selection, caret) = if selection.is_empty() || state.text().is_empty() {
            let caret =
                Bounds::new(point(left + caret_x, bounds.top() + px(2.)), size(px(1.5), bounds.size.height - px(4.)));
            (None, Some(fill(caret, theme.accent)))
        } else {
            let corners = Bounds::from_corners(
                point(left + line.x_for_index(input.shown_offset(selection.start)), bounds.top()),
                point(left + line.x_for_index(input.shown_offset(selection.end)), bounds.bottom()),
            );
            (Some(fill(corners, theme.accent.opacity(0.3))), None)
        };
        self.input.update(cx, |input, _| input.scroll = scroll);
        Prepainted { line, caret, selection }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepainted: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus, scroll) = {
            let input = self.input.read(cx);
            (input.focus.clone(), input.scroll)
        };
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.input.clone()), cx);
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            if let Some(selection) = prepainted.selection.take() {
                window.paint_quad(selection);
            }
            let origin = point(bounds.left() - scroll, bounds.top());
            // Painting a shaped line fails only when its glyphs can't be rasterized; nothing to show then.
            let _ = prepainted.line.paint(origin, px(HEIGHT), window, cx);
            if focus.is_focused(window)
                && let Some(caret) = prepainted.caret.take()
            {
                window.paint_quad(caret);
            }
        });
        let line = prepainted.line.clone();
        self.input.update(cx, |input, _| {
            input.layout = Some(line);
            input.bounds = Some(bounds);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_masked_field_maps_each_character_to_one_bullet() {
        let text = "pá🔑s";
        // p, á (2 bytes), 🔑 (4 bytes), s: four characters, four bullets.
        assert_eq!(masked_offset(text, 0), 0);
        assert_eq!(masked_offset(text, 1), 3);
        assert_eq!(masked_offset(text, 3), 6);
        assert_eq!(masked_offset(text, text.len()), 12);
        for (index, _) in text.char_indices() {
            assert_eq!(unmasked_offset(text, masked_offset(text, index)), index);
        }
        assert_eq!(unmasked_offset(text, 12), text.len());
    }
}
