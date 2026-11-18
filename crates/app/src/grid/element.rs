//! Draws the grid: the cells in view, the selection, the row numbers that
//! stay at the left edge, the header that stays at the top, and the
//! scrollbars. Only what is in view is laid out.

use gpui::{
    App, Bounds, ContentMask, CursorStyle, Element, ElementId, Entity, FontStyle, FontWeight, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, IntoElement, LayoutId, PaintQuad, Pixels, ShapedLine, SharedString, StrikethroughStyle,
    Style, TextRun, TransformationMatrix, Window, fill, font, point, px, relative, size,
};

use super::columns::{CELL_PADDING, MAX_WIDTH, initial_width, short_type_name};
use super::marks::{RowState, SortMark};
use super::selection::Cell;
use super::{DataGrid, GridLayout, HEADER_HEIGHT, LINK_MARGIN, LINK_SIZE, RESIZE_REACH, ROW_HEIGHT};
use crate::theme::{ActiveTheme, MONO};

const FONT_SIZE: f32 = 12.5;
const TYPE_FONT_SIZE: f32 = 10.;
/// Rows sampled for a column's first width.
const SAMPLE_ROWS: usize = 200;
/// The UI's font, as the rest of the window is drawn in.
const UI_FONT: &str = ".SystemUIFont";

pub struct GridElement {
    grid: Entity<DataGrid>,
}

impl GridElement {
    pub fn new(grid: Entity<DataGrid>) -> Self {
        Self { grid }
    }
}

impl IntoElement for GridElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// An icon and where it goes, clipped to `clip`.
struct Icon {
    path: &'static str,
    bounds: Bounds<Pixels>,
    color: Hsla,
    clip: Bounds<Pixels>,
}

/// Room a header's marks take besides its name and type: the key, the
/// computed badge and the sort arrow.
const KEY_ROOM: f32 = 18.;
const BADGE_ROOM: f32 = 34.;
const SORT_ROOM: f32 = 18.;

/// A line of text and where it goes, clipped to `clip`.
struct Text {
    line: ShapedLine,
    origin: gpui::Point<Pixels>,
    clip: Bounds<Pixels>,
}

pub struct Prepainted {
    /// Where cells are drawn: right of the row numbers, under the header.
    body_bounds: Bounds<Pixels>,
    lines: Vec<PaintQuad>,
    body: Vec<PaintQuad>,
    cells: Vec<Text>,
    gutter: Vec<PaintQuad>,
    numbers: Vec<Text>,
    header: Vec<PaintQuad>,
    headings: Vec<Text>,
    /// Links in the cells, under the body's clip.
    links: Vec<Icon>,
    /// The header's marks, drawn with its texts.
    header_icons: Vec<Icon>,
    bars: Vec<PaintQuad>,
    edges: Vec<Hitbox>,
}

fn run(len: usize, family: &'static str, weight: FontWeight, style: FontStyle, color: Hsla) -> TextRun {
    TextRun {
        len,
        font: gpui::Font { weight, style, ..font(family) },
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// A cell's value on one line, cut to what `max_chars` can show.
fn one_line(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c }).collect()
}

impl Element for GridElement {
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
        let shape = |text: String, size: f32, run: TextRun| {
            let run = TextRun { len: text.len(), ..run };
            text_system.shape_line(SharedString::from(text), px(size), &[run], None)
        };
        let plain = run(0, UI_FONT, FontWeight::NORMAL, FontStyle::Normal, theme.text);
        let char_width = f32::from(
            shape("0".into(), FONT_SIZE, run(0, MONO, FontWeight::NORMAL, FontStyle::Normal, theme.text)).width,
        );

        // First widths from the content, once.
        if self.grid.read(cx).widths.is_empty() {
            self.grid.update(cx, |grid, _| {
                grid.widths = grid
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| {
                        let values =
                            grid.rows.iter().take(SAMPLE_ROWS).map(|row| row.get(index).and_then(|v| v.as_deref()));
                        let width = initial_width(&column.name, &column.data_type, values, char_width);
                        // The header's marks get room of their own, so the name isn't cut short by them.
                        let marks = grid.marks.as_ref().map_or(0., |marks| {
                            let mark = marks.column(index);
                            SORT_ROOM
                                + if mark.key { KEY_ROOM } else { 0. }
                                + if mark.generated { BADGE_ROOM } else { 0. }
                        });
                        (width + marks).min(MAX_WIDTH + marks)
                    })
                    .collect();
            });
        }

        let grid = self.grid.read(cx);
        let marks = grid.marks.clone();
        let rows = grid.rows.len();
        // The gutter fits the longest label, the same whatever is pending, so nothing shifts.
        let digits = match &marks {
            Some(marks) => marks
                .row_labels
                .iter()
                .map(|label| label.chars().count())
                .max()
                .unwrap_or(1)
                .max(marks.gutter_chars)
                .max(1),
            None => rows.max(1).to_string().len(),
        };
        let gutter_width = px(char_width * digits as f32 + 21.);
        let gutter = bounds.left() + gutter_width;
        let body = Bounds::from_corners(point(gutter, bounds.top() + px(HEADER_HEIGHT)), bounds.bottom_right());
        let content_width: f32 = grid.widths.iter().sum();
        let content_height = ROW_HEIGHT * rows as f32;

        // Scrolls to a cell asked for, then keeps the scroll within the content.
        let mut scroll = grid.scroll;
        if let Some(cell) = grid.reveal {
            let top = px(ROW_HEIGHT * cell.row as f32);
            if top < scroll.y {
                scroll.y = top;
            } else if top + px(ROW_HEIGHT) > scroll.y + body.size.height {
                scroll.y = top + px(ROW_HEIGHT) - body.size.height;
            }
            let left = px(grid.widths[..cell.column.min(grid.widths.len())].iter().sum::<f32>());
            let right = left + px(grid.widths.get(cell.column).copied().unwrap_or(0.));
            if left < scroll.x {
                scroll.x = left;
            } else if right > scroll.x + body.size.width {
                scroll.x = right - body.size.width;
            }
        }
        scroll.y = scroll.y.min((px(content_height) - body.size.height).max(px(0.))).max(px(0.));
        scroll.x = scroll.x.min((px(content_width) - body.size.width).max(px(0.))).max(px(0.));

        let mut lefts = Vec::with_capacity(grid.widths.len());
        let mut x = gutter - scroll.x;
        for width in &grid.widths {
            lefts.push(x);
            x += px(*width);
        }
        let visible_columns: Vec<usize> =
            (0..lefts.len()).filter(|&c| lefts[c] + px(grid.widths[c]) > gutter && lefts[c] < bounds.right()).collect();
        let first_row = (f32::from(scroll.y) / ROW_HEIGHT).floor() as usize;
        let row_count = (f32::from(body.size.height) / ROW_HEIGHT).ceil() as usize + 1;
        let visible_rows = first_row..(first_row + row_count).min(rows);
        let row_top = |row: usize| body.top() + px(ROW_HEIGHT * row as f32) - scroll.y;

        let focused = grid.focus.is_focused(window);
        let cursor = grid.selection.cursor();
        let mut body_quads = Vec::new();
        let mut cells = Vec::new();
        let mut lines = Vec::new();
        let mut gutter_quads = Vec::new();
        let mut numbers = Vec::new();
        let mut links = Vec::new();
        for row in visible_rows.clone() {
            let top = row_top(row);
            let row_bounds = Bounds::from_corners(point(gutter, top), point(bounds.right(), top + px(ROW_HEIGHT)));
            let state = marks.as_ref().map_or(RowState::Loaded, |marks| marks.row(row));
            let gutter_bounds =
                Bounds::from_corners(point(bounds.left(), top), point(gutter - px(1.), top + px(ROW_HEIGHT)));
            match state {
                RowState::Inserted => {
                    body_quads.push(fill(row_bounds, theme.ok.opacity(0.08)));
                    gutter_quads.push(fill(gutter_bounds, theme.ok.opacity(0.08)));
                }
                RowState::Deleted => {
                    body_quads.push(fill(row_bounds, theme.err.opacity(0.07)));
                    gutter_quads.push(fill(gutter_bounds, theme.err.opacity(0.07)));
                }
                RowState::Loaded => {}
            }
            if cursor.is_some_and(|cell| cell.row == row) {
                body_quads.push(fill(row_bounds, theme.sel));
            }
            lines.push(fill(
                Bounds::new(point(bounds.left(), top + px(ROW_HEIGHT - 1.)), size(bounds.size.width, px(1.))),
                theme.line,
            ));
            for &column in &visible_columns {
                let width = px(grid.widths[column]);
                let cell = Bounds::new(point(lefts[column], top), size(width, px(ROW_HEIGHT)));
                let mark = marks.as_ref().map(|marks| marks.cell(Cell::new(row, column))).unwrap_or_default();
                if mark.edited {
                    body_quads.push(fill(cell, theme.warn.opacity(0.14)));
                }
                if grid.selection.contains(Cell::new(row, column)) {
                    let mut quad = fill(cell, theme.accent.opacity(0.18));
                    if cursor == Some(Cell::new(row, column)) {
                        quad = quad.border_widths(px(1.)).border_color(if focused {
                            theme.accent_hi
                        } else {
                            theme.text3
                        });
                    }
                    body_quads.push(quad.corner_radii(px(4.)));
                }
                let max_chars = (f32::from(width) / (char_width * 0.5)).ceil() as usize + 1;
                let (text, mut style) = match grid.rows[row].get(column).and_then(|value| value.as_deref()) {
                    None if mark.unset => {
                        ("DEFAULT".to_owned(), run(0, UI_FONT, FontWeight::NORMAL, FontStyle::Italic, theme.text3))
                    }
                    None => ("NULL".to_owned(), run(0, UI_FONT, FontWeight::NORMAL, FontStyle::Italic, theme.text3)),
                    Some(value) if grid.numeric[column] => {
                        (one_line(value, max_chars), run(0, MONO, FontWeight::NORMAL, FontStyle::Normal, theme.text))
                    }
                    Some(value) => (one_line(value, max_chars), plain.clone()),
                };
                if state == RowState::Deleted {
                    style.color = theme.text3;
                    style.strikethrough = Some(StrikethroughStyle { thickness: px(1.), color: Some(theme.text3) });
                }
                let linked = marks.as_ref().is_some_and(|marks| marks.links.contains(&Cell::new(row, column)));
                let link_room = if linked { px(LINK_SIZE + LINK_MARGIN) } else { px(0.) };
                if linked {
                    let link = Bounds::new(
                        point(cell.right() - px(LINK_MARGIN + LINK_SIZE / 2. + 6.), top + px((ROW_HEIGHT - 12.) / 2.)),
                        size(px(12.), px(12.)),
                    );
                    links.push(Icon { path: "external.svg", bounds: link, color: theme.text3, clip: body });
                }
                let line = shape(text, FONT_SIZE, style);
                let inner = width - px(CELL_PADDING * 2.) - link_room;
                let x = if grid.numeric[column] && line.width < inner {
                    cell.right() - px(CELL_PADDING) - link_room - line.width
                } else {
                    cell.left() + px(CELL_PADDING)
                };
                let clip = Bounds::from_corners(
                    point((cell.left() + px(CELL_PADDING / 2.)).max(gutter), cell.top()),
                    point(cell.right() - px(CELL_PADDING / 2.) - link_room, cell.bottom()),
                );
                cells.push(Text { line, origin: point(x, top + px((ROW_HEIGHT - 18.) / 2.)), clip });
            }
            let label = marks
                .as_ref()
                .and_then(|marks| marks.row_labels.get(row).cloned())
                .unwrap_or_else(|| (row + 1).to_string());
            let number_style = match state {
                RowState::Loaded => run(0, MONO, FontWeight::NORMAL, FontStyle::Normal, theme.text3),
                RowState::Inserted => run(0, MONO, FontWeight::BOLD, FontStyle::Normal, theme.ok),
                RowState::Deleted => run(0, MONO, FontWeight::BOLD, FontStyle::Normal, theme.err),
            };
            let number = shape(label, FONT_SIZE - 1., number_style);
            let origin = point(gutter - px(10.) - number.width, top + px((ROW_HEIGHT - 18.) / 2.));
            numbers.push(Text {
                line: number,
                origin,
                clip: Bounds::from_corners(point(bounds.left(), body.top()), point(gutter, bounds.bottom())),
            });
        }
        gutter_quads
            .push(fill(Bounds::new(point(gutter - px(1.), body.top()), size(px(1.), body.size.height)), theme.line));

        // The header, over the rows: each column's name and short type, and its edge.
        let header = Bounds::new(bounds.origin, size(bounds.size.width, px(HEADER_HEIGHT)));
        let mut header_quads = vec![fill(
            Bounds::new(point(bounds.left(), header.bottom() - px(1.)), size(bounds.size.width, px(1.))),
            theme.line,
        )];
        let mut headings = Vec::new();
        let mut header_icons = Vec::new();
        let mut edges = Vec::new();
        let header_clip = Bounds::from_corners(point(gutter, header.top()), header.bottom_right());
        for &column in &visible_columns {
            let left = lefts[column];
            let width = px(grid.widths[column]);
            let right = left + width;
            header_quads.push(fill(
                Bounds::new(point(right - px(1.), header.top() + px(8.)), size(px(1.), px(HEADER_HEIGHT - 16.))),
                theme.line,
            ));
            let name = shape(
                grid.columns[column].name.clone(),
                FONT_SIZE,
                run(0, UI_FONT, FontWeight::SEMIBOLD, FontStyle::Normal, theme.text2),
            );
            let type_name = shape(
                short_type_name(&grid.columns[column].data_type),
                TYPE_FONT_SIZE,
                run(0, MONO, FontWeight::NORMAL, FontStyle::Normal, theme.text3),
            );
            let clip = Bounds::from_corners(
                point((left + px(CELL_PADDING / 2.)).max(gutter), header.top()),
                point(right - px(CELL_PADDING / 2.), header.bottom()),
            );
            let y = header.top() + px((HEADER_HEIGHT - 18.) / 2.);
            let icon_top = header.top() + px((HEADER_HEIGHT - 13.) / 2.);
            let mark = marks.as_ref().map(|marks| marks.column(column)).unwrap_or_default();
            let mut x = left + px(CELL_PADDING);
            if mark.key {
                let bounds = Bounds::new(point(x, icon_top), size(px(13.), px(13.)));
                header_icons.push(Icon { path: "key.svg", bounds, color: theme.num, clip });
                x += px(KEY_ROOM);
            }
            let name_width = name.width;
            headings.push(Text { line: name, origin: point(x, y), clip });
            x += name_width + px(6.);
            if mark.generated {
                let badge = shape(
                    marks.as_ref().map_or("", |marks| marks.computed_badge).to_uppercase(),
                    TYPE_FONT_SIZE - 0.5,
                    run(0, UI_FONT, FontWeight::SEMIBOLD, FontStyle::Normal, theme.text3),
                );
                let badge_bounds = Bounds::new(point(x, icon_top), size(badge.width + px(8.), px(14.)));
                header_quads.push(fill(badge_bounds, theme.sel).corner_radii(px(3.)));
                let badge_width = badge.width;
                headings.push(Text { line: badge, origin: point(x + px(4.), y), clip });
                x += badge_width + px(14.);
            }
            headings.push(Text { line: type_name, origin: point(x, y + px(2.)), clip });
            if marks.is_some() {
                // The arrow stays while unsorted, faint, so the column keeps its width.
                let (path, color) = match mark.sort {
                    Some(SortMark::Asc) => ("up.svg", theme.accent),
                    Some(SortMark::Desc) => ("down.svg", theme.accent),
                    None => ("sortable.svg", theme.text3.opacity(0.6)),
                };
                let bounds =
                    Bounds::new(point(right - px(CELL_PADDING + 12.), icon_top + px(0.5)), size(px(12.), px(12.)));
                header_icons.push(Icon { path, bounds, color, clip });
            }
            if right > gutter {
                let reach = Bounds::new(
                    point(right - px(RESIZE_REACH), header.top()),
                    size(px(RESIZE_REACH * 2.), px(HEADER_HEIGHT)),
                );
                edges.push(window.insert_hitbox(reach.intersect(&header_clip), HitboxBehavior::Normal));
            }
        }
        let hash = shape("#".into(), FONT_SIZE - 1., run(0, MONO, FontWeight::NORMAL, FontStyle::Normal, theme.text3));
        let hash_origin = point(gutter - px(10.) - hash.width, header.top() + px((HEADER_HEIGHT - 18.) / 2.));
        headings.push(Text { line: hash, origin: hash_origin, clip: header });

        // Scrollbars, while the content is bigger than the view.
        let mut bars = Vec::new();
        let thumb = theme.text3.opacity(0.35);
        if px(content_height) > body.size.height && content_height > 0. {
            let length = (body.size.height * (body.size.height / px(content_height))).max(px(24.));
            let travel = body.size.height - length;
            let offset = travel * (scroll.y / (px(content_height) - body.size.height));
            bars.push(
                fill(Bounds::new(point(bounds.right() - px(7.), body.top() + offset), size(px(4.), length)), thumb)
                    .corner_radii(px(2.)),
            );
        }
        if px(content_width) > body.size.width && content_width > 0. {
            let length = (body.size.width * (body.size.width / px(content_width))).max(px(24.));
            let travel = body.size.width - length;
            let offset = travel * (scroll.x / (px(content_width) - body.size.width));
            bars.push(
                fill(Bounds::new(point(gutter + offset, bounds.bottom() - px(7.)), size(length, px(4.))), thumb)
                    .corner_radii(px(2.)),
            );
        }

        let rows_in_view = (f32::from(body.size.height) / ROW_HEIGHT).floor() as usize;
        self.grid.update(cx, |grid, _| {
            grid.scroll = scroll;
            grid.reveal = None;
            grid.layout = GridLayout { bounds, gutter, lefts, rows_in_view };
        });
        Prepainted {
            body_bounds: body,
            lines,
            body: body_quads,
            cells,
            gutter: gutter_quads,
            numbers,
            header: header_quads,
            headings,
            links,
            header_icons,
            bars,
            edges,
        }
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
        let line_height = px(18.);
        let paint_icons = |icons: &[Icon], window: &mut Window, cx: &mut App| {
            for icon in icons {
                window.with_content_mask(Some(ContentMask { bounds: icon.clip }), |window| {
                    // An icon that can't be drawn is left out.
                    let _ =
                        window.paint_svg(icon.bounds, icon.path.into(), TransformationMatrix::unit(), icon.color, cx);
                });
            }
        };
        let paint_texts = |texts: &[Text], window: &mut Window, cx: &mut App| {
            for text in texts {
                window.with_content_mask(Some(ContentMask { bounds: text.clip }), |window| {
                    // Painting fails only when glyphs can't be rasterized; nothing to show then.
                    let _ = text.line.paint(text.origin, line_height, window, cx);
                });
            }
        };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.with_content_mask(Some(ContentMask { bounds: prepainted.body_bounds }), |window| {
                for quad in prepainted.body.drain(..) {
                    window.paint_quad(quad);
                }
            });
            for quad in prepainted.lines.drain(..) {
                window.paint_quad(quad);
            }
            paint_texts(&prepainted.cells, window, cx);
            paint_icons(&prepainted.links, window, cx);
            for quad in prepainted.gutter.drain(..) {
                window.paint_quad(quad);
            }
            paint_texts(&prepainted.numbers, window, cx);
            for quad in prepainted.header.drain(..) {
                window.paint_quad(quad);
            }
            paint_texts(&prepainted.headings, window, cx);
            paint_icons(&prepainted.header_icons, window, cx);
            for quad in prepainted.bars.drain(..) {
                window.paint_quad(quad);
            }
        });
        for edge in &prepainted.edges {
            window.set_cursor_style(CursorStyle::ResizeLeftRight, edge);
        }
    }
}
