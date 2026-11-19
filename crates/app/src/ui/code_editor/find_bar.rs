//! The find and replace bar (⌘F / Ctrl+F), as CodeMirror's search panel: a
//! query with match case, regular expression and whole word, the next and
//! previous match (Enter and ⇧Enter in the query, ⌘G and ⇧⌘G, F3 and ⇧F3),
//! every match selected at once, and a replacement for the match (Enter in
//! its field) or for all of them; Escape closes it. Every match is marked in
//! the text while it is open, the current one more strongly.
//!
//! It floats over the editor's top right rather than taking a row of its own
//! (CodeMirror's panel sits below the text and shortens it): opening and
//! closing it moves no line of text, and its size never changes while typing.

use gpui::{
    AnyElement, AppContext, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use super::document::{EditKind, Then};
use super::search::{self, SearchQuery};
use super::selection::{Change, Selection, Selections};
use super::{CloseFind, CodeEditor, FIND_CONTEXT, FindBar, FindNext, FindPrevious, OpenFind};
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::field::Tooltip;
use crate::ui::{TextInput, TextInputEvent, glass, icon};

/// The bar's width at most; narrower editors get a narrower bar.
const WIDTH: f32 = 440.;
const ROW_HEIGHT: f32 = 28.;
/// The most selections "All" makes, as CodeMirror's limit.
const MOST_SELECTED: usize = 1000;
/// Selected text longer than this doesn't become the query (CodeMirror's limit).
const LONGEST_SEEDED: usize = 100;

impl CodeEditor {
    /// The bar's fields, made the first time it opens.
    fn find_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) -> &mut FindBar {
        if self.find.is_none() {
            let m = &cx.messages().editor;
            let query = cx.new(|cx| TextInput::new(m.find, cx));
            let replacement = cx.new(|cx| TextInput::new(m.replace, cx));
            let subscriptions = vec![
                cx.subscribe_in(&query, window, |this, input, event, window, cx| match event {
                    TextInputEvent::Changed => {
                        this.query.text = input.read(cx).text().to_owned();
                        cx.notify();
                    }
                    TextInputEvent::Submitted => this.find_next(&FindNext, window, cx),
                    TextInputEvent::Cancelled => this.close_find(&CloseFind, window, cx),
                    _ => {}
                }),
                cx.subscribe_in(&replacement, window, |this, _, event, window, cx| match event {
                    TextInputEvent::Submitted => this.replace_next(cx),
                    TextInputEvent::Cancelled => this.close_find(&CloseFind, window, cx),
                    _ => {}
                }),
            ];
            self.find = Some(FindBar { query, replacement, open: false, _subscriptions: subscriptions });
        }
        self.find.as_mut().expect("made above")
    }

    pub(super) fn find_open(&self) -> bool {
        self.find.as_ref().is_some_and(|find| find.open)
    }

    /// ⌘F: opens the bar with the selected text (when short) as the query, and focuses it.
    pub(super) fn open_find(&mut self, _: &OpenFind, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.document.primary().range();
        let seed = (!selection.is_empty() && selection.len() <= LONGEST_SEEDED)
            .then(|| self.text()[selection].replace('\n', "\\n"));
        let find = self.find_fields(window, cx);
        find.open = true;
        let query = find.query.clone();
        if let Some(seed) = seed {
            query.update(cx, |input, cx| input.set_text(&seed, cx));
            self.query.text = seed;
        }
        query.update(cx, |input, cx| input.select_all(cx));
        query.read(cx).focus(window);
        cx.notify();
    }

    /// Escape: closes the bar and gives the keyboard back to the text.
    pub(super) fn close_find(&mut self, _: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = self.find.as_mut() {
            find.open = false;
        }
        self.focus(window);
        cx.notify();
    }

    /// The query's matches in the text, found again only when either changed.
    pub(super) fn matches(&mut self) -> &[std::ops::Range<usize>] {
        let key = Some((self.document.version(), self.query.clone()));
        if self.matches.key != key {
            self.matches.found =
                self.query.compile().map(|search| search.matches(self.document.text())).unwrap_or_default();
            self.matches.key = key;
        }
        &self.matches.found
    }

    /// Selects `range` alone and scrolls it into view.
    fn select_match(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) {
        self.document.select(range.start, range.end);
        self.completion = None;
        self.autoscroll = true;
        cx.notify();
    }

    /// The next match after the primary selection, wrapping round; with no
    /// query yet, the bar opens to ask for one.
    pub(super) fn find_next(&mut self, _: &FindNext, window: &mut Window, cx: &mut Context<Self>) {
        if self.query.compile().is_none() {
            return self.open_find(&OpenFind, window, cx);
        }
        let from = self.document.primary().range().end;
        if let Some(found) = search::next(self.matches(), from) {
            self.select_match(found, cx);
        }
    }

    pub(super) fn find_previous(&mut self, _: &FindPrevious, window: &mut Window, cx: &mut Context<Self>) {
        if self.query.compile().is_none() {
            return self.open_find(&OpenFind, window, cx);
        }
        let to = self.document.primary().range().start;
        if let Some(found) = search::previous(self.matches(), to) {
            self.select_match(found, cx);
        }
    }

    /// Selects every match at once, a caret at each (CodeMirror's "all").
    pub(super) fn select_all_matches(&mut self, cx: &mut Context<Self>) {
        let matches = self.matches().to_vec();
        if matches.is_empty() || matches.len() > MOST_SELECTED {
            return;
        }
        let all = matches.into_iter().map(|found| Selection::new(found.start, found.end)).collect();
        self.document.set_selections(Selections::new(all, 0));
        self.autoscroll = true;
        cx.notify();
    }

    /// What replaces the matches: the replacement field's text.
    fn replacement(&self, cx: &Context<Self>) -> String {
        self.find.as_ref().map(|find| find.replacement.read(cx).text().to_owned()).unwrap_or_default()
    }

    /// Replaces the selected match and selects the next one; with no match
    /// selected, selects the next one first (CodeMirror's `replaceNext`).
    pub(super) fn replace_next(&mut self, cx: &mut Context<Self>) {
        let Some(search) = self.query.compile().filter(|_| !self.read_only) else { return };
        let replace = self.replacement(cx);
        let selected = self.document.primary().range();
        let Some(found) = search::next(self.matches(), selected.start) else { return };
        if found != selected {
            return self.select_match(found, cx);
        }
        let insert = search.replacement(self.text(), found.clone(), &replace);
        let after = found.start + insert.len();
        let change = Change::new(found.clone(), insert.clone());
        let caret = Selections::single(Selection::caret(after));
        let changed = self.document.change(EditKind::Other, vec![change], Then::Select(caret));
        self.after_edit(changed, cx);
        if let Some(next) = search::next(self.matches(), after) {
            self.select_match(next, cx);
        }
    }

    /// Replaces every match at once, as one step to undo.
    pub(super) fn replace_all(&mut self, cx: &mut Context<Self>) {
        let Some(search) = self.query.compile().filter(|_| !self.read_only) else { return };
        let replace = self.replacement(cx);
        let changes: Vec<Change> = self
            .matches()
            .to_vec()
            .into_iter()
            .map(|found| Change::new(found.clone(), search.replacement(self.document.text(), found, &replace)))
            .collect();
        if changes.is_empty() {
            return;
        }
        let changed = self.document.change(EditKind::Other, changes, Then::Map { after: true });
        self.after_edit(changed, cx);
    }

    pub(super) fn toggle_option(&mut self, option: fn(&mut SearchQuery) -> &mut bool, cx: &mut Context<Self>) {
        let value = option(&mut self.query);
        *value = !*value;
        cx.notify();
    }

    /// The bar over the editor's top right, while it is open.
    pub(super) fn find_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.find_open() {
            return None;
        }
        let theme = *cx.theme();
        let m = &cx.messages().editor;
        let invalid = !self.query.text.is_empty() && self.matches().is_empty();
        let find = self.find.as_ref()?;
        let field = |input: &gpui::Entity<TextInput>, invalid: bool| {
            let focused = input.read(cx).is_focused(window);
            div()
                .h(px(ROW_HEIGHT))
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .px(px(8.))
                .rounded(px(7.))
                .border_1()
                .border_color(if invalid {
                    theme.err
                } else if focused {
                    theme.accent
                } else {
                    theme.line
                })
                .bg(theme.sel)
                .font_family(MONO)
                .text_size(px(12.5))
                .text_color(theme.text)
                .child(input.clone())
        };
        let square = |id: &'static str, label: &'static str, tip: &'static str, on: bool| {
            div()
                .id(id)
                .size(px(ROW_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .cursor_pointer()
                .font_family(MONO)
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if on { theme.text } else { theme.text2 })
                .when(on, |button| button.bg(theme.accent.opacity(0.22)))
                .hover(|button| button.bg(theme.sel))
                .tooltip(Tooltip::text(tip))
                .child(label)
        };
        let icon_square = |id: &'static str, name: &'static str, tip: &'static str| {
            div()
                .id(id)
                .size(px(ROW_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .cursor_pointer()
                .hover(|button| button.bg(theme.sel))
                .tooltip(Tooltip::text(tip))
                .child(icon(name, theme.text2, 13.))
        };
        let text_button = |id: &'static str, label: &'static str, tip: &'static str| {
            div()
                .id(id)
                .h(px(ROW_HEIGHT))
                .px(px(9.))
                .flex_none()
                .flex()
                .items_center()
                .rounded(px(7.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme.text2)
                .hover(|button| button.bg(theme.sel).text_color(theme.text))
                .tooltip(Tooltip::text(tip))
                .child(label)
        };
        let query_field = field(&find.query, invalid);
        let replace_field = field(&find.replacement, false);
        let row = || div().flex().items_center().gap(px(4.));
        let query_row = row()
            .child(query_field)
            .child(
                square("find-case", "Aa", m.match_case, self.query.case_sensitive)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_option(|query| &mut query.case_sensitive, cx))),
            )
            .child(
                square("find-word", "W", m.by_word, self.query.whole_word)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_option(|query| &mut query.whole_word, cx))),
            )
            .child(
                square("find-regex", ".*", m.regex, self.query.regex)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_option(|query| &mut query.regex, cx))),
            )
            .child(
                icon_square("find-previous", "up", m.previous)
                    .on_click(cx.listener(|this, _, window, cx| this.find_previous(&FindPrevious, window, cx))),
            )
            .child(
                icon_square("find-next", "down", m.next)
                    .on_click(cx.listener(|this, _, window, cx| this.find_next(&FindNext, window, cx))),
            )
            .child(
                text_button("find-all", m.all, m.all_hint)
                    .on_click(cx.listener(|this, _, _, cx| this.select_all_matches(cx))),
            )
            .child(
                icon_square("find-close", "close", m.close)
                    .on_click(cx.listener(|this, _, window, cx| this.close_find(&CloseFind, window, cx))),
            );
        let replace_row = (!self.read_only).then(|| {
            row()
                .child(replace_field)
                .child(
                    text_button("replace-next", m.replace, m.replace)
                        .on_click(cx.listener(|this, _, _, cx| this.replace_next(cx))),
                )
                .child(
                    text_button("replace-all", m.replace_all, m.replace_all)
                        .on_click(cx.listener(|this, _, _, cx| this.replace_all(cx))),
                )
        });
        let width = (self.layout.bounds.size.width - px(28.)).min(px(WIDTH)).max(px(160.));
        Some(
            glass(&theme)
                .id("find-bar")
                .key_context(FIND_CONTEXT)
                .occlude()
                .absolute()
                .top(px(6.))
                .right(px(14.))
                .w(width)
                .p(px(5.))
                .rounded(px(11.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .cursor_default()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(query_row)
                .children(replace_row)
                .into_any_element(),
        )
    }
}
