//! Draws the editor: the line numbers, the current line, the selection, the
//! lines in view (highlighted) and the caret, and registers the editor with
//! the OS's input methods. Only the rows in view are laid out.

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, ElementInputHandler, Entity, FontStyle, FontWeight, GlobalElementId,
    Hsla, IntoElement, LayoutId, PaintQuad, Pixels, ShapedLine, SharedString, Style, TextRun, UnderlineStyle, Window,
    fill, point, px, relative, size,
};

use super::{CodeEditor, FONT_SIZE, LINE_HEIGHT, Layout, editor_font, styled_font};
use crate::sql::highlight::{Kind, LineState, tokenize};
use crate::theme::{ActiveTheme, Theme};

/// Space above the first line and below the last.
pub const PADDING_TOP: f32 = 10.;
/// Space between the line numbers and the text.
const GUTTER_GAP: f32 = 14.;
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

pub struct Prepainted {
    /// Each row in view: its number, its laid-out text and where it is drawn.
    lines: Vec<(ShapedLine, ShapedLine, Pixels)>,
    quads: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
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
        let editor = self.editor.read(cx);
        let document = &editor.document;
        let state = document.state();
        let line_height = px(LINE_HEIGHT);
        let font_size = px(FONT_SIZE);
        let text_system = window.text_system().clone();
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

        // The gutter is as wide as the last line's number.
        let digits = document.line_count().to_string().len().max(2);
        let char_width = plain("0".into(), theme.text).width;
        let gutter_right = bounds.left() + char_width * digits as f32 + px(16.);
        let text_bounds =
            Bounds::from_corners(point(gutter_right + px(GUTTER_GAP), bounds.top()), bounds.bottom_right());

        // Keeps the caret in view after it moved, then keeps the scroll within the text.
        let (caret_row, caret_column) = document.caret_position();
        let caret_line = plain(document.line(caret_row).to_owned(), theme.text);
        let caret_x = caret_line.x_for_index(caret_column);
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
        let content_height = line_height * document.line_count() as f32 + px(PADDING_TOP * 2.);
        let content_width = char_width * document.widest_line() as f32 + px(RIGHT_ROOM);
        scroll.y = scroll.y.min((content_height - bounds.size.height).max(px(0.))).max(px(0.));
        scroll.x = scroll.x.min((content_width - text_bounds.size.width).max(px(0.))).max(px(0.));
        let text_left = text_bounds.left() - scroll.x;
        let top = bounds.top() + px(PADDING_TOP) - scroll.y;

        let first = (f32::from(scroll.y - px(PADDING_TOP)) / LINE_HEIGHT).floor().max(0.) as usize;
        let shown = (f32::from(bounds.size.height) / LINE_HEIGHT).ceil() as usize + 1;
        let rows = first..(first + shown).min(document.line_count());

        let selection = state.selection();
        let marked = state.marked();
        let mut quads = Vec::new();
        let mut lines = Vec::new();
        for row in rows.clone() {
            let range = document.line_range(row);
            let y = top + line_height * row as f32;
            let text = document.line(row).to_owned();
            let runs = runs(
                &text,
                editor.line_states.get(row),
                editor.dialect,
                &theme,
                marked.as_ref().map(|marked| {
                    marked.start.saturating_sub(range.start).min(text.len())
                        ..marked.end.saturating_sub(range.start).min(text.len())
                }),
            );
            let line = text_system.shape_line(SharedString::from(text), font_size, &runs, None);
            // The selection on this row, running past its end when it takes the line break too.
            if !selection.is_empty() && selection.start <= range.end && selection.end >= range.start {
                let from = selection.start.max(range.start) - range.start;
                let to = selection.end.min(range.end) - range.start;
                let past_end = if selection.end > range.end { char_width } else { px(0.) };
                let corners = Bounds::from_corners(
                    point(text_left + line.x_for_index(from), y),
                    point(text_left + line.x_for_index(to) + past_end, y + line_height),
                );
                quads.push(fill(corners, theme.accent.opacity(0.28)));
            }
            let color = if row == caret_row { theme.text2 } else { theme.text3 };
            let number = plain((row + 1).to_string(), color);
            lines.push((number, line, y));
        }

        let current_line = (selection.is_empty() && rows.contains(&caret_row)).then(|| {
            let y = top + line_height * caret_row as f32;
            fill(Bounds::from_corners(point(bounds.left(), y), point(bounds.right(), y + line_height)), theme.sel)
        });
        let caret = rows.contains(&caret_row).then(|| {
            let origin = point(text_left + caret_x - px(1.), top + line_height * caret_row as f32 + px(2.));
            fill(Bounds::new(origin, size(px(2.), line_height - px(4.))), theme.accent_hi)
        });
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
            editor.layout = Layout { bounds, text_left, caret: Some(caret_bounds), rows: visible_rows };
        });
        let mut all = Vec::new();
        all.extend(current_line);
        all.extend(quads);
        Prepainted { lines, quads: all, caret, gutter_right, text_bounds, text_left }
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
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepainted.quads.drain(..) {
                window.paint_quad(quad);
            }
            for (number, _, y) in &prepainted.lines {
                // Painting fails only when glyphs can't be rasterized; nothing to show then.
                let _ = number.paint(point(prepainted.gutter_right - number.width, *y), line_height, window, cx);
            }
            window.with_content_mask(Some(ContentMask { bounds: prepainted.text_bounds }), |window| {
                for (_, line, y) in &prepainted.lines {
                    let _ = line.paint(point(prepainted.text_left, *y), line_height, window, cx);
                }
                if focus.is_focused(window)
                    && let Some(caret) = prepainted.caret.take()
                {
                    window.paint_quad(caret);
                }
            });
        });
    }
}

/// The coloured runs of a line that starts in `state`; `marked` (relative to
/// the line) is underlined, being composed by an input method.
fn runs(
    text: &str,
    state: Option<&LineState>,
    dialect: Option<crate::sql::Dialect>,
    theme: &Theme,
    marked: Option<std::ops::Range<usize>>,
) -> Vec<TextRun> {
    let run = |len: usize, kind: Kind| {
        let (color, weight, style) = match kind {
            Kind::Keyword => (theme.kw, FontWeight::SEMIBOLD, FontStyle::Normal),
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
    };
    let mut runs: Vec<TextRun> = match (dialect, state) {
        (Some(dialect), Some(state)) => {
            tokenize(text, state.clone(), dialect).0.into_iter().map(|span| run(span.range.len(), span.kind)).collect()
        }
        _ => vec![run(text.len(), Kind::Plain)],
    };
    if text.is_empty() {
        return Vec::new();
    }
    if let Some(marked) = marked.filter(|marked| !marked.is_empty()) {
        runs = underline(runs, marked, theme.text);
    }
    runs
}

/// `runs` with `range` underlined, splitting the runs it cuts.
fn underline(runs: Vec<TextRun>, range: std::ops::Range<usize>, color: Hsla) -> Vec<TextRun> {
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
