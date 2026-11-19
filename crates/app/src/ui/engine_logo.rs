//! An engine's logo drawn pixel for pixel. GPUI's `img` rasterizes an SVG once
//! at twice its declared size and scales that bitmap to wherever it lands, so
//! small, detailed logos come out soft. This element rasterizes each logo at
//! the exact number of device pixels it covers (its size times the window's
//! scale factor), caches that bitmap, and paints it at a whole-pixel origin, so
//! every pixel of the logo is one pixel on screen.

use std::sync::Arc;

use dbforge_core::connections::model::Engine;
use gpui::{
    App, Bounds, Element, ElementId, Global, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    RenderImage, Style, Window, px, size,
};
use image::{Frame, RgbaImage};

use crate::assets::engine_logo_svg;

/// `engine`'s logo, `points` square, in its own colours.
pub fn engine_logo(engine: Engine, points: f32) -> EngineLogo {
    EngineLogo { engine, points }
}

pub struct EngineLogo {
    engine: Engine,
    points: f32,
}

impl IntoElement for EngineLogo {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EngineLogo {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style =
            Style { size: size(px(self.points).into(), px(self.points).into()), flex_shrink: 0., ..Style::default() };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let scale = window.scale_factor();
        let pixels = device_pixels(self.points, scale);
        let Some(image) = cx.default_global::<LogoCache>().get(self.engine, pixels) else {
            return;
        };
        // Exactly `pixels` device pixels wide; GPUI floors the origin to a whole pixel.
        let side = px(pixels as f32 / scale);
        let _ = window.paint_image(Bounds::new(bounds.origin, size(side, side)), Default::default(), image, 0, false);
    }
}

/// The device pixels a logo `points` wide covers at `scale`.
fn device_pixels(points: f32, scale: f32) -> u32 {
    (points * scale).round().max(1.) as u32
}

/// Every logo rasterized so far, by engine and pixel size: a handful at most,
/// so a list is enough.
#[derive(Default)]
struct LogoCache(Vec<(LogoKey, Option<Arc<RenderImage>>)>);

/// A logo's engine and its size in device pixels.
type LogoKey = (Engine, u32);

impl Global for LogoCache {}

impl LogoCache {
    fn get(&mut self, engine: Engine, pixels: u32) -> Option<Arc<RenderImage>> {
        if let Some((_, image)) = self.0.iter().find(|(key, _)| *key == (engine, pixels)) {
            return image.clone();
        }
        let image = rasterize(engine_logo_svg(engine), pixels).map(|raster| Arc::new(raster.into_render_image()));
        self.0.push(((engine, pixels), image.clone()));
        image
    }
}

/// A logo's pixels, in GPUI's image layout: BGRA with straight alpha.
struct Raster(RgbaImage);

impl Raster {
    fn into_render_image(self) -> RenderImage {
        RenderImage::new(vec![Frame::new(self.0)])
    }
}

/// Renders `svg` into a `pixels`-square bitmap, fitting its view box.
fn rasterize(svg: &[u8], pixels: u32) -> Option<Raster> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixels, pixels)?;
    let fit = pixels as f32 / tree.size().width().max(tree.size().height());
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(fit, fit), &mut pixmap.as_mut());
    let mut bytes = pixmap.take();
    for pixel in bytes.as_chunks_mut::<4>().0 {
        to_straight_bgra(pixel);
    }
    RgbaImage::from_raw(pixels, pixels, bytes).map(Raster)
}

/// One premultiplied RGBA pixel (resvg's) as straight-alpha BGRA (GPUI's).
fn to_straight_bgra(pixel: &mut [u8]) {
    pixel.swap(0, 2);
    let alpha = pixel[3];
    if alpha > 0 && alpha < 255 {
        for channel in &mut pixel[..3] {
            *channel = ((*channel as u32 * 255 + alpha as u32 / 2) / alpha as u32).min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Engine; 8] = [
        Engine::Postgres,
        Engine::Mysql,
        Engine::Mariadb,
        Engine::Sqlite,
        Engine::Sqlserver,
        Engine::Mongodb,
        Engine::Cosmosdb,
        Engine::Redis,
    ];

    #[test]
    fn covers_exactly_the_device_pixels_of_its_size() {
        assert_eq!(device_pixels(16., 2.), 32);
        assert_eq!(device_pixels(16., 1.), 16);
        assert_eq!(device_pixels(22., 1.5), 33);
        assert_eq!(device_pixels(16., 1.25), 20);
    }

    #[test]
    fn rasterizes_every_logo_at_the_asked_size_and_in_colour() {
        for engine in ALL {
            for pixels in [16, 32, 44] {
                let Raster(image) = rasterize(engine_logo_svg(engine), pixels).unwrap();
                assert_eq!(image.dimensions(), (pixels, pixels), "{engine:?}");
                let painted = image.pixels().filter(|pixel| pixel[3] > 0).count();
                assert!(painted > (pixels * pixels / 10) as usize, "{engine:?} at {pixels}px draws its logo");
                let coloured = image.pixels().any(|pixel| pixel[0] != pixel[1] || pixel[1] != pixel[2]);
                assert!(coloured, "{engine:?} keeps its colours");
            }
        }
    }

    #[test]
    fn turns_premultiplied_rgba_into_straight_bgra() {
        let mut opaque = [10, 20, 30, 255];
        to_straight_bgra(&mut opaque);
        assert_eq!(opaque, [30, 20, 10, 255]);
        let mut half = [0x33, 0x1A, 0x00, 0x80];
        to_straight_bgra(&mut half);
        assert_eq!(half, [0x00, 0x34, 0x66, 0x80]);
        let mut clear = [0, 0, 0, 0];
        to_straight_bgra(&mut clear);
        assert_eq!(clear, [0, 0, 0, 0]);
    }
}
