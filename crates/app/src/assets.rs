//! The icon set: stroked 16×16 SVGs served by name (`plus.svg`), which GPUI
//! tints with the text colour.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

/// Each icon's path, drawn with a round 1.5px stroke.
const STROKED: &[(&str, &str)] = &[
    ("plus", "M8 3v10M3 8h10"),
    ("refresh", "M13 8a5 5 0 1 1-1.5-3.5M13 3v2.5h-2.5"),
    ("search", "M11.2 7a4.2 4.2 0 1 1-8.4 0a4.2 4.2 0 1 1 8.4 0M10.5 10.5 14 14"),
    ("down", "M4 6l4 4 4-4"),
    ("right", "M6 4l4 4-4 4"),
    ("table", "M4.5 3h7a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2M2.5 6.5h11M6.5 6.5v6.5"),
    ("sidebar", "M4.5 3h7a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2M6.5 3v10"),
    ("panel", "M4.5 3h7a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2M2.5 9.5h11"),
    (
        "sliders",
        "M3 4.5h10M3 11.5h10M7.6 4.5a1.6 1.6 0 1 1-3.2 0a1.6 1.6 0 1 1 3.2 0M11.6 11.5a1.6 1.6 0 1 1-3.2 0a1.6 1.6 0 1 1 3.2 0",
    ),
    ("code", "M6 4 2.5 8 6 12M10 4l3.5 4L10 12"),
    ("clock", "M13.5 8a5.5 5.5 0 1 1-11 0a5.5 5.5 0 1 1 11 0M8 5v3l2 1.5"),
    ("chart", "M2.5 13.5h11M4.5 11V8M8 11V4.5M11.5 11V6.5"),
    ("export", "M8 2.5v8M4.5 7 8 10.5 11.5 7M3 13.5h10"),
    (
        "database",
        "M3 4c0-1 2.2-1.8 5-1.8s5 .8 5 1.8v8c0 1-2.2 1.8-5 1.8s-5-.8-5-1.8zM3 4c0 1 2.2 1.8 5 1.8s5-.8 5-1.8M3 8c0 1 2.2 1.8 5 1.8s5-.8 5-1.8",
    ),
];

/// Icons filled rather than stroked.
const FILLED: &[(&str, &str)] = &[("play", "M5 3.5v9l7-4.5z")];

/// The app's assets: the icons.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        let name = path.trim_end_matches(".svg");
        let find = |set: &[(&str, &'static str)]| set.iter().find(|(icon, _)| *icon == name).map(|(_, d)| *d);
        let svg = if let Some(d) = find(STROKED) {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="{d}" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>"#
            )
        } else if let Some(d) = find(FILLED) {
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="{d}" fill="black"/></svg>"#)
        } else {
            return Ok(None);
        };
        Ok(Some(Cow::Owned(svg.into_bytes())))
    }

    fn list(&self, _path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_each_icon_by_name_and_nothing_else() {
        let plus = Assets.load("plus.svg").unwrap().unwrap();
        assert!(std::str::from_utf8(&plus).unwrap().contains("M8 3v10M3 8h10"));
        assert!(Assets.load("play.svg").unwrap().is_some());
        assert!(Assets.load("nope.svg").unwrap().is_none());
    }
}
