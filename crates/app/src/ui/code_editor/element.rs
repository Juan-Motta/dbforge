//! Draws the editor: the line numbers and fold markers, the current lines,
//! the selections, the find bar's matches, the brackets beside each caret,
//! the rows in view (highlighted, folds shown as a placeholder) and the
//! carets, and registers the editor with the OS's input methods. Only the
//! rows in view are laid out.

use std::collections::HashMap;
use std::ops::Range;

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, ElementInputHandler, Entity, FontStyle, FontWeight, GlobalElementId,
    Hsla, IntoElement, LayoutId, PaintQuad, Pixels, ShapedLine, SharedString, Style, TextRun, TransformationMatrix,
    UnderlineStyle, Window, fill, point, px, relative, size,
};

use super::display_map::{PLACEHOLDER, Piece};
use super::{CodeEditor, FONT_SIZE, LINE_HEIGHT, Layout, editor_font, styled_font};
use crate::sql::highlight::Kind;
use crate::theme::{ActiveTheme, Theme};

/// Space above the first line and below the last.
pub const PADDING_TOP: f32 = 10.;
/// The fold markers' column, right of the line numbers.
pub const FOLD_WIDTH: f32 = 14.;
/// Space between the fold markers and the text.
const GUTTER_GAP: f32 = 10.;
/// Room kept right of the widest line when scrolled across.
const RIGHT_ROOM: f32 = 40.;

pub struct EditorElement {
    editor: Entity<CodeEditor>,
}

impl EditorElement {
    pub fn new(editor: Entity<CodeEditor>) -> Self {
        Self { editor }
    }
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A row in view: its number (on a line's first row), its laid-out text and where it is drawn.
struct ShownRow {
    number: Option<ShapedLine>,
    text: ShapedLine,
    y: Pixels,
}

pub struct Prepainted {
    rows: Vec<ShownRow>,
    quads: Vec<PaintQuad>,
    carets: Vec<PaintQuad>,
    /// The fold markers: an icon and where.
    markers: Vec<(&'static str, Bounds<Pixels>)>,
    gutter_right: Pixels,
    text_bounds: Bounds<Pixels>,
    text_left: Pixels,
}

impl Element for EditorElement {
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
        style.size.height = relative(1.).into();
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
        let text_system = window.text_system().clone();
        let font_size = px(FONT_SIZE);
        let plain = |text: String, color: Hsla| {
            let run = TextRun {
                len: text.len(),
                font: editor_font(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            text_system.shape_line(SharedString::from(text), font_size, &[run], None)
        };

        // The gutter is as wide as the last line's number, then the fold markers.
        let (digits, soft_wrap) = {
            let editor = self.editor.read(cx);
            (editor.document.line_count().to_string().len().max(2), editor.soft_wrap)
        };
        let char_width = plain("0".into(), theme.text).width;
        let gutter_right = bounds.left() + char_width * digits as f32 + px(16.);
        let folds_left = gutter_right + px(2.);
        let text_bounds =
            Bounds::from_corners(point(folds_left + px(FOLD_WIDTH + GUTTER_GAP), bounds.top()), bounds.bottom_right());
        let text_width = text_bounds.size.width - px(RIGHT_ROOM / 2.);
        let matches = self.editor.update(cx, |editor, _| {
            editor.layout.text_width = text_width;
            editor.sync_display(window);
            if editor.find_open() { editor.matches().to_vec() } else { Vec::new() }
        });

        let editor = self.editor.read(cx);
        let document = &editor.document;
        let display = &editor.display;
        let text = document.text();
        let line_height = px(LINE_HEIGHT);
        let row_text = |row: usize| display.row_text(text, row);

        // Keeps the primary caret in view after it moved, then keeps the scroll within the text.
        let primary = document.primary();
        let caret_row = display.row_of(primary.head);
        let caret_line = plain(row_text(caret_row), theme.text);
        let caret_x = caret_line.x_for_index(display.display_index(caret_row, primary.head));
        let caret_y = px(PADDING_TOP) + line_height * caret_row as f32;
        let mut scroll = editor.scroll;
        if editor.autoscroll {
            if caret_y < scroll.y {
                scroll.y = caret_y - px(PADDING_TOP);
            } else if caret_y + line_height > scroll.y + bounds.size.height {
                scroll.y = caret_y + line_height + px(PADDING_TOP) - bounds.size.height;
            }
            let width = text_bounds.size.width - px(RIGHT_ROOM / 2.);
            if caret_x < scroll.x {
                scroll.x = (caret_x - px(RIGHT_ROOM)).max(px(0.));
            } else if caret_x > scroll.x + width {
                scroll.x = caret_x - width;
            }
        }
        let content_height = line_height * display.row_count() as f32 + px(PADDING_TOP * 2.);
        let content_width =
            if soft_wrap { px(0.) } else { char_width * document.widest_line() as f32 + px(RIGHT_ROOM) };
        scroll.y = scroll.y.min((content_height - bounds.size.height).max(px(0.))).max(px(0.));
        scroll.x = scroll.x.min((content_width - text_bounds.size.width).max(px(0.))).max(px(0.));
        let text_left = text_bounds.left() - scroll.x;
        let top = bounds.top() + px(PADDING_TOP) - scroll.y;

        let first = (f32::from(scroll.y - px(PADDING_TOP)) / LINE_HEIGHT).floor().max(0.) as usize;
        let shown = (f32::from(bounds.size.height) / LINE_HEIGHT).ceil() as usize + 1;
        let rows = first..(first + shown).min(display.row_count());

        let selections = document.selections().all();
        let marked = document.state().marked();
        // The lines with an empty caret are marked as current.
        let caret_lines: Vec<usize> =
            selections.iter().filter(|s| s.is_empty()).map(|s| document.row_of(s.head)).collect();
        let mut backdrops = Vec::new();
        let mut quads = Vec::new();
        let mut shown_rows = Vec::new();
        let mut markers = Vec::new();
        let mut spans: HashMap<usize, Vec<(Range<usize>, Kind)>> = HashMap::new();
        let mut laid_out: HashMap<usize, ShapedLine> = HashMap::new();
        for index in rows.clone() {
            let row = display.row(index);
            let y = top + line_height * index as f32;
            let pieces = display.pieces(index);
            let mut runs = Vec::new();
            let mut shown_text = String::new();
            for piece in &pieces {
                match piece {
                    Piece::Text(range) => {
                        let line = document.row_of(range.start);
                        let line_start = document.line_range(line).start;
                        let line_spans = spans.entry(line).or_insert_with(|| editor.line_spans(line));
                        let local = range.start - line_start..range.end - line_start;
                        runs.extend(slice_runs(line_spans, local, &theme));
                        shown_text.push_str(&text[range.clone()]);
                    }
                    Piece::Fold(_) => {
                        runs.push(TextRun {
                            len: PLACEHOLDER.len(),
                            font: editor_font(),
                            color: theme.text2,
                            background_color: Some(theme.sel),
                            underline: None,
                            strikethrough: None,
                        });
                        shown_text.push_str(PLACEHOLDER);
                    }
                }
            }
            if let Some(marked) = marked.as_ref().filter(|marked| marked.start < row.end && marked.end > row.start) {
                let from = display.display_index(index, marked.start);
                let to = display.display_index(index, marked.end);
                runs = underline(runs, from..to, theme.text);
            }
            let line = text_system.shape_line(SharedString::from(shown_text), font_size, &runs, None);
            let x = |offset: usize| text_left + line.x_for_index(display.display_index(index, offset));

            if caret_lines.contains(&row.line) {
                backdrops.push(fill(
                    Bounds::from_corners(point(bounds.left(), y), point(bounds.right(), y + line_height)),
                    theme.sel,
                ));
            }
            // The find bar's matches, the current one stronger.
            for found in overlapping(&matches, row.start, row.end) {
                let current = selections.iter().any(|s| s.range() == *found);
                let tint = if current { theme.warn.opacity(0.45) } else { theme.warn.opacity(0.2) };
                let corners = Bounds::from_corners(
                    point(x(found.start.max(row.start)), y + px(1.)),
                    point(x(found.end.min(row.end)), y + line_height - px(1.)),
                );
                quads.push(fill(corners, tint).corner_radii(px(2.)));
            }
            // Each selection on this row, running past its end when it goes on.
            for selection in selections.iter().filter(|s| !s.is_empty()) {
                let range = selection.range();
                if range.end < row.start || range.start > row.end {
                    continue;
                }
                let past_end = if range.end > row.end { char_width } else { px(0.) };
                let corners = Bounds::from_corners(
                    point(x(range.start.max(row.start)), y),
                    point(x(range.end.min(row.end)) + past_end, y + line_height),
                );
                quads.push(fill(corners, theme.accent.opacity(0.28)));
            }
            if row.first {
                let color = if caret_lines.contains(&row.line) { theme.text2 } else { theme.text3 };
                let number = plain((row.line + 1).to_string(), color);
                let marker = if editor.folded_on_line(row.line) {
                    Some("right")
                } else {
                    editor.fold_on_line(row.line).map(|_| "down")
                };
                if let Some(name) = marker {
                    let side = px(10.);
                    let origin = point(folds_left + (px(FOLD_WIDTH) - side) / 2., y + (line_height - side) / 2.);
                    markers.push((name, Bounds::new(origin, size(side, side))));
                }
                shown_rows.push(ShownRow { number: Some(number), text: line.clone(), y });
            } else {
                shown_rows.push(ShownRow { number: None, text: line.clone(), y });
            }
            laid_out.insert(index, line);
        }

        // The brackets beside each caret: the pair marked, or one with no match in red.
        let mut mark_bracket = |at: usize, tint: Hsla| {
            let index = display.row_of(at);
            let Some(line) = laid_out.get(&index) else { return };
            let from = line.x_for_index(display.display_index(index, at));
            let to = line.x_for_index(display.display_index(index, at + 1));
            let y = top + line_height * index as f32;
            let corners = Bounds::from_corners(
                point(text_left + from, y + px(2.)),
                point(text_left + to, y + line_height - px(2.)),
            );
            quads.push(fill(corners, tint).corner_radii(px(2.)));
        };
        for selection in selections.iter().filter(|s| s.is_empty()) {
            if let Some(found) = editor.structure.matching(selection.head) {
                let tint = if found.matched { theme.accent.opacity(0.3) } else { theme.err.opacity(0.35) };
                mark_bracket(found.at, tint);
                if let Some(partner) = found.partner {
                    mark_bracket(partner, tint);
                }
            }
        }

        let mut carets = Vec::new();
        for selection in selections {
            let index = display.row_of(selection.head);
            let Some(line) = laid_out.get(&index) else { continue };
            let x = line.x_for_index(display.display_index(index, selection.head));
            let origin = point(text_left + x - px(1.), top + line_height * index as f32 + px(2.));
            carets.push(fill(Bounds::new(origin, size(px(2.), line_height - px(4.))), theme.accent_hi));
        }
        let caret_bounds =
            Bounds::new(point(text_left + caret_x, top + line_height * caret_row as f32), size(px(2.), line_height));
        let visible_rows = (f32::from(bounds.size.height) / LINE_HEIGHT).floor() as usize;

        // The scrollbar, while the text is taller than the editor.
        if content_height > bounds.size.height {
            let ratio = bounds.size.height / content_height;
            let thumb = (bounds.size.height * ratio).max(px(24.));
            let travel = bounds.size.height - thumb;
            let offset = travel * (scroll.y / (content_height - bounds.size.height));
            let thumb = Bounds::new(point(bounds.right() - px(7.), bounds.top() + offset), size(px(4.), thumb));
            quads.push(fill(thumb, theme.text3.opacity(0.35)).corner_radii(px(2.)));
        }

        self.editor.update(cx, |editor, _| {
            editor.scroll = scroll;
            editor.autoscroll = false;
            editor.layout =
                Layout { bounds, text_left, folds_left, text_width, caret: Some(caret_bounds), rows: visible_rows };
        });
        backdrops.extend(quads);
        Prepainted { rows: shown_rows, quads: backdrops, carets, markers, gutter_right, text_bounds, text_left }
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
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.editor.clone()), cx);
        let line_height = px(LINE_HEIGHT);
        let marker_tint = cx.theme().text3;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepainted.quads.drain(..) {
                window.paint_quad(quad);
            }
            for row in &prepainted.rows {
                if let Some(number) = &row.number {
                    // Painting fails only when glyphs can't be rasterized; nothing to show then.
                    let _ = number.paint(point(prepainted.gutter_right - number.width, row.y), line_height, window, cx);
                }
            }
            for (name, at) in &prepainted.markers {
                let path = SharedString::from(format!("{name}.svg"));
                let _ = window.paint_svg(*at, path, TransformationMatrix::unit(), marker_tint, cx);
            }
            window.with_content_mask(Some(ContentMask { bounds: prepainted.text_bounds }), |window| {
                for row in &prepainted.rows {
                    let _ = row.text.paint(point(prepainted.text_left, row.y), line_height, window, cx);
                }
                if focus.is_focused(window) {
                    for caret in prepainted.carets.drain(..) {
                        window.paint_quad(caret);
                    }
                }
            });
        });
    }
}

/// The ranges in `ranges` (in text order) that reach into `start..end`.
fn overlapping(ranges: &[Range<usize>], start: usize, end: usize) -> impl Iterator<Item = &Range<usize>> {
    let first = ranges.partition_point(|range| range.end < start);
    ranges[first..]
        .iter()
        .take_while(move |range| range.start <= end)
        .filter(move |range| range.end > start || range.start == start)
}

/// The coloured runs for `range` of a line, from the line's highlighted spans.
fn slice_runs(spans: &[(Range<usize>, Kind)], range: Range<usize>, theme: &Theme) -> Vec<TextRun> {
    spans
        .iter()
        .filter_map(|(span, kind)| {
            let from = span.start.max(range.start);
            let to = span.end.min(range.end);
            (from < to).then(|| run(to - from, *kind, theme))
        })
        .collect()
}

fn run(len: usize, kind: Kind, theme: &Theme) -> TextRun {
    let (color, weight, style) = match kind {
        Kind::Keyword => (theme.kw, FontWeight::SEMIBOLD, FontStyle::Normal),
        Kind::Key => (theme.kw, FontWeight::NORMAL, FontStyle::Normal),
        Kind::Type | Kind::Function => (theme.func, FontWeight::NORMAL, FontStyle::Normal),
        Kind::String => (theme.str, FontWeight::NORMAL, FontStyle::Normal),
        Kind::Number => (theme.num, FontWeight::NORMAL, FontStyle::Normal),
        Kind::Comment => (theme.com, FontWeight::NORMAL, FontStyle::Italic),
        Kind::Plain | Kind::Identifier => (theme.text, FontWeight::NORMAL, FontStyle::Normal),
    };
    TextRun {
        len,
        font: styled_font(weight, style),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// `runs` with `range` underlined, splitting the runs it cuts.
fn underline(runs: Vec<TextRun>, range: Range<usize>, color: Hsla) -> Vec<TextRun> {
    let mut out = Vec::new();
    let mut at = 0;
    for run in runs {
        let (start, end) = (at, at + run.len);
        at = end;
        let cuts = [start, range.start.clamp(start, end), range.end.clamp(start, end), end];
        for pair in cuts.windows(2) {
            let len = pair[1] - pair[0];
            if len == 0 {
                continue;
            }
            let inside = pair[0] >= range.start && pair[1] <= range.end;
            out.push(TextRun {
                len,
                underline: inside.then_some(UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false }),
                ..run.clone()
            });
        }
    }
    out
}
