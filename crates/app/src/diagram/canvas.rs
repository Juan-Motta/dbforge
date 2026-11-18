//! Draws an ER diagram's canvas: a dotted background, the relationships as
//! curves and the cards over them, at the view's scale and offset. Only the
//! cards in view are laid out, and text too small to read is left out, so
//! a schema of many tables pans and zooms smoothly. The pointer's presses,
//! drags and wheel go to the [`DiagramView`].

use gpui::{
    App, Bounds, BoxShadow, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, Entity, Font, FontWeight,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Path, PathBuilder, Pixels, ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun,
    TransformationMatrix, Window, fill, font, point, px, relative, size,
};

use super::layout::{CARD_WIDTH, HEADER_HEIGHT, Point, ROW_HEIGHT, Size, View};
use super::view::{Card, ColumnKey, DiagramView, Scene};
use crate::theme::{ActiveTheme, MONO, Theme, color};

/// The background's dots: this far apart, whatever the zoom.
const DOT_SPACING: f32 = 20.;
/// Text smaller than this on screen is not drawn: nobody can read it.
const MIN_TEXT: f32 = 3.;

pub struct DiagramCanvas {
    view: Entity<DiagramView>,
}

impl DiagramCanvas {
    pub fn new(view: Entity<DiagramView>) -> Self {
        Self { view }
    }
}

impl IntoElement for DiagramCanvas {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A line of text and where it goes, clipped to `clip`.
struct Text {
    line: ShapedLine,
    origin: gpui::Point<Pixels>,
    line_height: Pixels,
    clip: Bounds<Pixels>,
}

/// One card's drawing: its shadow, its quads, its texts and icon.
struct DrawnCard {
    shadow: (Bounds<Pixels>, Pixels, Hsla),
    quads: Vec<PaintQuad>,
    texts: Vec<Text>,
    icon: Option<(Bounds<Pixels>, Hsla)>,
}

pub struct Prepainted {
    hitbox: Hitbox,
    dots: Vec<PaintQuad>,
    lines: Vec<(Path<Pixels>, Hsla)>,
    cards: Vec<DrawnCard>,
}

fn run(text: &str, family: SharedString, weight: FontWeight, tint: Hsla) -> TextRun {
    TextRun {
        len: text.len(),
        font: Font { weight, ..font(family) },
        color: tint,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

impl Element for DiagramCanvas {
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
        // The canvas's size first: a pending fit needs it before anything is drawn.
        let viewport = Size { width: f32::from(bounds.size.width), height: f32::from(bounds.size.height) };
        let (scene, view, fitted) = self.view.update(cx, |diagram, _| {
            diagram.viewport = viewport;
            let fitted = diagram.fit_now();
            (diagram.scene(), diagram.view(), fitted)
        });
        if fitted {
            // Drawn fitted now; the zoom's percentage follows on the next frame.
            let diagram = self.view.clone();
            window.on_next_frame(move |_, cx| diagram.update(cx, |_, cx| cx.notify()));
        }
        let theme = *cx.theme();
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let dots = dots(bounds, &theme);
        let (Some(scene), Some(view)) = (scene, view) else {
            return Prepainted { hitbox, dots, lines: Vec::new(), cards: Vec::new() };
        };
        let at = |p: Point| {
            let p = view.to_viewport(p);
            bounds.origin + point(px(p.x), px(p.y))
        };
        let lines = scene
            .lines
            .iter()
            .filter_map(|line| {
                let highlighted =
                    scene.active.as_ref().is_some_and(|active| &line.table == active || &line.referenced == active);
                let dimmed = scene.active.is_some() && !highlighted;
                let (width, tint) = if highlighted { (2., theme.accent) } else { (1.5, theme.text3.opacity(0.75)) };
                let tint = if dimmed { tint.opacity(tint.a * 0.25) } else { tint };
                let mut path = PathBuilder::stroke(px(width * view.scale));
                path.move_to(at(line.curve.from));
                path.cubic_bezier_to(at(line.curve.to), at(line.curve.c1), at(line.curve.c2));
                path.build().ok().map(|path| (path, tint))
            })
            .collect();
        let cards = scene
            .cards
            .iter()
            .filter_map(|card| {
                let top_left = at(card.position);
                let card_size = size(px(CARD_WIDTH * view.scale), px(card.height * view.scale));
                let card_bounds = Bounds::new(top_left, card_size);
                // Out of view: nothing to draw.
                if !card_bounds.intersects(&bounds) {
                    return None;
                }
                Some(draw_card(card, card_bounds, &scene, view, &theme, window))
            })
            .collect();
        Prepainted { hitbox, dots, lines, cards }
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
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for dot in prepainted.dots.drain(..) {
                window.paint_quad(dot);
            }
            for (path, tint) in prepainted.lines.drain(..) {
                window.paint_path(path, tint);
            }
            for card in &mut prepainted.cards {
                let (shadow_bounds, radius, shadow) = card.shadow;
                window.paint_shadows(
                    shadow_bounds,
                    gpui::Corners::all(radius),
                    &[BoxShadow {
                        color: shadow,
                        offset: point(px(0.), radius / 2.),
                        blur_radius: radius * 1.75,
                        spread_radius: px(0.),
                    }],
                );
                for quad in card.quads.drain(..) {
                    window.paint_quad(quad);
                }
                if let Some((icon_bounds, tint)) = card.icon {
                    // An icon that can't be drawn is left out.
                    let _ = window.paint_svg(icon_bounds, "table.svg".into(), TransformationMatrix::unit(), tint, cx);
                }
                for text in &card.texts {
                    window.with_content_mask(Some(ContentMask { bounds: text.clip }), |window| {
                        // Painting fails only when glyphs can't be rasterized; nothing to show then.
                        let _ = text.line.paint(text.origin, text.line_height, window, cx);
                    });
                }
            }
        });

        let hitbox = prepainted.hitbox.clone();
        let dragging = self.view.read(cx).is_dragging();
        window.set_cursor_style(if dragging { CursorStyle::ClosedHand } else { CursorStyle::OpenHand }, &hitbox);
        let origin = bounds.origin;
        let local = move |position: gpui::Point<Pixels>| {
            Point::new(f32::from(position.x - origin.x), f32::from(position.y - origin.y))
        };
        let view = self.view.clone();
        let over = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left && over.is_hovered(window) {
                view.update(cx, |view, cx| view.press(local(event.position), cx));
            }
        });
        let view = self.view.clone();
        let over = hitbox.clone();
        // Moves and releases are heard anywhere, so a drag goes on past the canvas's edge.
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                let hovered = over.is_hovered(window);
                view.update(cx, |view, cx| view.pointer_moved(local(event.position), hovered, cx));
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                view.update(cx, |view, cx| {
                    if view.is_dragging() {
                        view.release(event.click_count, cx);
                    }
                });
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                let delta = event.delta.pixel_delta(px(16.));
                if delta.y != px(0.) {
                    view.update(cx, |view, cx| view.wheel(f32::from(delta.y), local(event.position), cx));
                }
            }
        });
    }
}

/// The background's dots, fixed to the canvas as the web UI's were.
fn dots(bounds: Bounds<Pixels>, theme: &Theme) -> Vec<PaintQuad> {
    let tint = theme.text3.opacity(0.3);
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let mut quads = Vec::new();
    let mut y = DOT_SPACING / 2.;
    while y < height {
        let mut x = DOT_SPACING / 2.;
        while x < width {
            let dot = Bounds::new(bounds.origin + point(px(x - 0.5), px(y - 0.5)), size(px(1.), px(1.)));
            quads.push(fill(dot, tint));
            x += DOT_SPACING;
        }
        y += DOT_SPACING;
    }
    quads
}

/// A card's quads and texts at the view's scale.
fn draw_card(
    card: &Card,
    bounds: Bounds<Pixels>,
    scene: &Scene,
    view: View,
    theme: &Theme,
    window: &Window,
) -> DrawnCard {
    let s = view.scale;
    let focused = scene.active.as_ref() == Some(&card.name);
    let pinned = scene.pinned.as_ref() == Some(&card.name);
    let dimmed = scene.active.is_some() && !scene.related.contains(&card.name);
    let fade = |tint: Hsla| if dimmed { tint.opacity(tint.a * 0.35) } else { tint };
    let radius = px(8. * s);
    let header_height = px(HEADER_HEIGHT * s);
    let mut quads = Vec::new();

    if focused {
        // The accent's soft ring around the card.
        let ring = Bounds::new(bounds.origin - point(px(3.), px(3.)), bounds.size + size(px(6.), px(6.)));
        quads.push(fill(ring, theme.accent.opacity(0.25)).corner_radii(radius + px(3.)));
    }
    // An opaque card, so the lines under it don't show through.
    let body = if theme.appearance == crate::theme::Appearance::Dark { color(0x2A292FFF) } else { color(0xFDFDFEFF) };
    let border = if focused { theme.accent } else { theme.rim };
    quads.push(fill(bounds, fade(body)).corner_radii(radius).border_widths(px(1.)).border_color(fade(border)));
    let header =
        Bounds::new(bounds.origin + point(px(1.), px(1.)), size(bounds.size.width - px(2.), header_height - px(1.)));
    let header_tint = if pinned { theme.accent.opacity(0.14) } else { theme.sel };
    quads.push(fill(header, fade(header_tint)).corner_radii(gpui::Corners {
        top_left: radius,
        top_right: radius,
        ..Default::default()
    }));
    quads.push(fill(
        Bounds::new(bounds.origin + point(px(0.), header_height), size(bounds.size.width, px(1.))),
        fade(theme.line),
    ));

    let mut texts = Vec::new();
    let mut icon = None;
    let ui_font = window.text_style().font_family.clone();
    let mono: SharedString = MONO.into();
    let shape = |text: &str, family: SharedString, weight: FontWeight, tint: Hsla, font_size: f32| {
        window.text_system().shape_line(
            SharedString::from(text.to_owned()),
            px(font_size),
            &[run(text, family, weight, tint)],
            None,
        )
    };
    let inner = Bounds::new(bounds.origin, bounds.size);
    if 12.5 * s >= MIN_TEXT {
        let pad = px(10. * s);
        let line_height = header_height;
        icon = Some((
            Bounds::new(bounds.origin + point(pad, (header_height - px(14. * s)) / 2.), size(px(14. * s), px(14. * s))),
            fade(theme.text3),
        ));
        let mut name_right = bounds.right() - pad;
        if let Some(rows) = &card.rows {
            let rows = shape(rows, ui_font.clone(), FontWeight::NORMAL, fade(theme.text3), 11. * s);
            name_right -= rows.width + px(8. * s);
            texts.push(Text {
                origin: point(bounds.right() - pad - rows.width, bounds.top()),
                line: rows,
                line_height,
                clip: inner,
            });
        }
        let name_left = bounds.left() + pad + px(22. * s);
        let name = shape(&card.name, ui_font.clone(), FontWeight::SEMIBOLD, fade(theme.text), 12.5 * s);
        texts.push(Text {
            line: name,
            origin: point(name_left, bounds.top()),
            line_height,
            clip: Bounds::from_corners(
                point(name_left, bounds.top()),
                point(name_right.max(name_left), bounds.top() + header_height),
            ),
        });
        let row_height = px(ROW_HEIGHT * s);
        for (index, column) in card.columns.iter().enumerate() {
            let top = bounds.top() + header_height + row_height * index as f32;
            let key_left = bounds.left() + pad;
            if let Some(key) = column.key {
                let (label, tint) = match key {
                    ColumnKey::Primary => ("PK", theme.warn),
                    ColumnKey::Foreign => ("FK", theme.func),
                };
                let key = shape(label, ui_font.clone(), FontWeight::SEMIBOLD, fade(tint), 9.5 * s);
                texts.push(Text { line: key, origin: point(key_left, top), line_height: row_height, clip: inner });
            }
            let data_type = shape(&column.data_type, mono.clone(), FontWeight::NORMAL, fade(theme.text3), 11.5 * s);
            let type_left = (bounds.right() - pad - data_type.width).max(key_left + px(24. * s));
            let name_left = key_left + px(24. * s);
            let name = shape(&column.name, mono.clone(), FontWeight::NORMAL, fade(theme.text), 11.5 * s);
            texts.push(Text {
                line: name,
                origin: point(name_left, top),
                line_height: row_height,
                clip: Bounds::from_corners(
                    point(name_left, top),
                    point((type_left - px(6. * s)).max(name_left), top + row_height),
                ),
            });
            texts.push(Text {
                line: data_type,
                origin: point(type_left, top),
                line_height: row_height,
                clip: Bounds::from_corners(point(name_left, top), point(bounds.right() - pad / 2., top + row_height)),
            });
        }
    }
    DrawnCard { shadow: (bounds, radius, fade(color(0x0000002E))), quads, texts, icon }
}
