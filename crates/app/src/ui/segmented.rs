//! A [`group`] of segments where one is chosen (sections, filters, result
//! tabs), its raise drawn as a single thumb that slides to the segment chosen
//! next instead of jumping there. While it travels, its leading edge moves
//! ahead of its trailing one, so the thumb stretches toward its target and
//! settles into it.
//!
//! The segments are laid out as usual; once they are placed, their bounds are
//! kept in the control's state, and the thumb is painted behind them in the
//! same frame, so it never shows a frame late or in the wrong place.

use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, BorderStyle, Bounds, Div, ElementId, Hsla, IntoElement, ParentElement, Pixels, RenderOnce, Styled,
    Window, canvas, div, point, prelude::FluentBuilder, px, quad, size,
};

use super::group;
use crate::theme::Theme;

/// How long the leading edge takes to reach the target.
const LEAD: Duration = Duration::from_millis(240);
/// How long the trailing edge waits before it follows.
const TRAIL_DELAY: Duration = Duration::from_millis(50);
/// How long the trailing edge then takes, so the whole slide lasts `LEAD`-ish plus a little.
const TRAIL: Duration = Duration::from_millis(300);

/// A glass group whose chosen segment (`selected`, if any) is raised by a
/// sliding thumb. Build the segments with [`super::segment_face`], which
/// leaves the raise to the thumb.
pub fn segmented(theme: &Theme, id: impl Into<ElementId>, selected: Option<usize>, segments: Vec<AnyElement>) -> Div {
    group(theme, vec![sliding(theme, id, selected, segments).shrinking().into_any_element()])
}

/// The row of segments with the sliding thumb, for a container of its own
/// (the tab strip scrolls); [`segmented`] puts it in a glass group. Children
/// after the chosen ones (a + button) can follow in `segments`.
pub fn sliding(
    theme: &Theme,
    id: impl Into<ElementId>,
    selected: Option<usize>,
    segments: Vec<AnyElement>,
) -> Segments {
    Segments { id: id.into(), selected, segments, fill: theme.glass_hi, rim: theme.rim, shrinks: false }
}

#[derive(IntoElement)]
pub struct Segments {
    id: ElementId,
    selected: Option<usize>,
    segments: Vec<AnyElement>,
    fill: Hsla,
    rim: Hsla,
    shrinks: bool,
}

impl Segments {
    /// Lets the row be narrower than its segments, for a group that shrinks and clips them.
    pub fn shrinking(mut self) -> Self {
        self.shrinks = true;
        self
    }
}

/// What the control remembers between frames, in coordinates relative to the segments' row.
#[derive(Default)]
struct Thumb {
    /// Each segment's bounds, as last laid out.
    places: Vec<Bounds<Pixels>>,
    /// The segment the thumb is on, or heading to.
    selected: Option<usize>,
    /// Where the thumb was painted last.
    shown: Option<Bounds<Pixels>>,
    /// Where the current slide started, and when; none once it has settled.
    slide: Option<(Bounds<Pixels>, Instant)>,
}

impl RenderOnce for Segments {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let thumb = window.use_keyed_state(self.id.clone(), cx, |_, _| Thumb::default());
        let (fill, rim, selected) = (self.fill, self.rim, self.selected);
        let painter = thumb.clone();
        let placed = thumb;
        div()
            .relative()
            .h_full()
            .when(self.shrinks, |row| row.min_w_0())
            .when(!self.shrinks, |row| row.flex_none())
            .flex()
            .gap(px(2.))
            // Behind the segments: the thumb, painted where they were placed this frame.
            .child(
                canvas(
                    |_, _, _| (),
                    move |row, _, window, cx| {
                        let now = Instant::now();
                        let (shown, moving) = painter.update(cx, |thumb, _| thumb.paint(selected, now));
                        if let Some(shown) = shown {
                            let bounds = Bounds { origin: row.origin + shown.origin, size: shown.size };
                            let radius = shown.size.height / 2.;
                            window.paint_quad(quad(bounds, radius, fill, px(1.), rim, BorderStyle::Solid));
                        }
                        if moving {
                            window.request_animation_frame();
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .children(self.segments)
            .on_children_prepainted(move |bounds, _, cx| {
                // The first child is the thumb's canvas, which spans the row: the segments follow it.
                let Some((row, segments)) = bounds.split_first() else {
                    return;
                };
                let places = segments
                    .iter()
                    .map(|segment| Bounds {
                        origin: point(segment.origin.x - row.origin.x, segment.origin.y - row.origin.y),
                        size: segment.size,
                    })
                    .collect();
                placed.update(cx, |thumb, _| thumb.places = places);
            })
    }
}

impl Thumb {
    /// Where to paint the thumb at `now` for `selected`, and whether it is still moving.
    fn paint(&mut self, selected: Option<usize>, now: Instant) -> (Option<Bounds<Pixels>>, bool) {
        if selected != self.selected {
            self.selected = selected;
            self.slide = self.shown.map(|from| (from, now));
        }
        let target = selected.and_then(|index| self.places.get(index).copied());
        let (shown, moving) = match (target, self.slide) {
            (Some(to), Some((from, started))) => {
                let elapsed = now.saturating_duration_since(started);
                (Some(slide(from, to, elapsed)), elapsed < TRAIL_DELAY + TRAIL)
            }
            (target, _) => (target, false),
        };
        if !moving {
            self.slide = None;
        }
        self.shown = shown;
        (shown, moving)
    }
}

/// The thumb `elapsed` into a slide from `from` to `to`: the edge on the side
/// it moves toward leads, the other follows, both easing out.
fn slide(from: Bounds<Pixels>, to: Bounds<Pixels>, elapsed: Duration) -> Bounds<Pixels> {
    let lead = ease_out(elapsed.as_secs_f32() / LEAD.as_secs_f32());
    let trail = ease_out(elapsed.saturating_sub(TRAIL_DELAY).as_secs_f32() / TRAIL.as_secs_f32());
    let forward = to.origin.x >= from.origin.x;
    let (left, right) = if forward { (trail, lead) } else { (lead, trail) };
    let mix = |a: Pixels, b: Pixels, t: f32| a + (b - a) * t;
    let x0 = mix(from.origin.x, to.origin.x, left);
    let x1 = mix(from.right(), to.right(), right);
    let y0 = mix(from.origin.y, to.origin.y, lead);
    let y1 = mix(from.bottom(), to.bottom(), lead);
    Bounds { origin: point(x0, y0), size: size(x1 - x0, y1 - y0) }
}

/// Cubic ease-out over `0..=1`, clamped.
fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    1. - (1. - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(x: f32, width: f32) -> Bounds<Pixels> {
        Bounds { origin: point(px(x), px(0.)), size: size(px(width), px(26.)) }
    }

    #[test]
    fn slides_from_one_segment_to_the_next_stretching_on_the_way() {
        let (from, to) = (place(0., 50.), place(100., 70.));
        assert_eq!(slide(from, to, Duration::ZERO), from);
        assert_eq!(slide(from, to, TRAIL_DELAY + TRAIL), to);
        let midway = slide(from, to, LEAD / 2);
        assert!(midway.size.width > from.size.width.max(to.size.width), "{midway:?}");
        assert!(midway.origin.x > from.origin.x && midway.right() < to.right(), "{midway:?}");
    }

    #[test]
    fn leads_with_the_left_edge_when_sliding_left() {
        let (from, to) = (place(100., 70.), place(0., 50.));
        let midway = slide(from, to, LEAD / 2);
        let left_done = (from.origin.x - midway.origin.x) / (from.origin.x - to.origin.x);
        let right_done = (from.right() - midway.right()) / (from.right() - to.right());
        assert!(left_done > right_done, "{midway:?}");
    }

    #[test]
    fn sits_on_the_chosen_segment_until_another_is_chosen() {
        let now = Instant::now();
        let mut thumb = Thumb { places: vec![place(0., 50.), place(52., 60.)], ..Thumb::default() };
        assert_eq!(thumb.paint(Some(0), now), (Some(place(0., 50.)), false), "the first time it just appears");
        assert_eq!(thumb.paint(Some(0), now + LEAD), (Some(place(0., 50.)), false));

        let (shown, moving) = thumb.paint(Some(1), now + LEAD);
        assert_eq!((shown, moving), (Some(place(0., 50.)), true), "a slide starts where it was");
        let (shown, moving) = thumb.paint(Some(1), now + LEAD + TRAIL_DELAY + TRAIL);
        assert_eq!((shown, moving), (Some(place(52., 60.)), false));
    }

    #[test]
    fn shows_no_thumb_when_nothing_is_chosen() {
        let mut thumb = Thumb { places: vec![place(0., 50.)], ..Thumb::default() };
        assert_eq!(thumb.paint(None, Instant::now()), (None, false));
    }
}
