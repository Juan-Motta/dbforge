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
    ("up", "M4 10l4-4 4 4"),
    ("left", "M10 4 6 8l4 4"),
    ("right", "M6 4l4 4-4 4"),
    ("sortable", "M5 6.5 8 3.5l3 3M5 9.5l3 3 3-3"),
    ("key", "M8.2 10.5a2.7 2.7 0 1 1-5.4 0a2.7 2.7 0 1 1 5.4 0M7.4 8.6 13 3M10.8 5.2l1.7 1.7"),
    ("table", "M4.5 3h7a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2M2.5 6.5h11M6.5 6.5v6.5"),
    ("sidebar", "M4.5 3h7a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2M6.5 3v10"),
    ("panel", "M4.5 3h7a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2M2.5 9.5h11"),
    (
        "sliders",
        "M3 4.5h10M3 11.5h10M7.6 4.5a1.6 1.6 0 1 1-3.2 0a1.6 1.6 0 1 1 3.2 0M11.6 11.5a1.6 1.6 0 1 1-3.2 0a1.6 1.6 0 1 1 3.2 0",
    ),
    ("code", "M6 4 2.5 8 6 12M10 4l3.5 4L10 12"),
    ("schema", "M3 3.5h4v3.5H3zM9 9h4v3.5H9zM5 7v3.75h4"),
    ("folder", "M2.5 4.5a1 1 0 0 1 1-1h3l1.5 1.5h4.5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1z"),
    (
        "view",
        "M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8s-2.4 4.5-6.5 4.5S1.5 8 1.5 8M9.8 8a1.8 1.8 0 1 1-3.6 0a1.8 1.8 0 1 1 3.6 0",
    ),
    ("collapse", "M4.5 10.5 8 7l3.5 3.5M4.5 6 8 2.5 11.5 6M3 13.5h10"),
    ("copy", "M6 6h6.5v7.5H6zM10 6V3.5H3.5V11H6"),
    ("spinner", "M8 2a6 6 0 1 1-6 6"),
    ("clock", "M13.5 8a5.5 5.5 0 1 1-11 0a5.5 5.5 0 1 1 11 0M8 5v3l2 1.5"),
    ("chart", "M2.5 13.5h11M4.5 11V8M8 11V4.5M11.5 11V6.5"),
    ("export", "M8 2.5v8M4.5 7 8 10.5 11.5 7M3 13.5h10"),
    (
        "database",
        "M3 4c0-1 2.2-1.8 5-1.8s5 .8 5 1.8v8c0 1-2.2 1.8-5 1.8s-5-.8-5-1.8zM3 4c0 1 2.2 1.8 5 1.8s5-.8 5-1.8M3 8c0 1 2.2 1.8 5 1.8s5-.8 5-1.8",
    ),
    ("info", "M14 8a6 6 0 1 1-12 0a6 6 0 1 1 12 0M8 7.25v3.75M8 5h.01"),
    ("check", "M3.5 8.5l3 3 6-7"),
    ("close", "M4 4l8 8M12 4l-8 8"),
    ("pulse", "M2.5 8h3l2-4 2 8 2-4h2"),
    (
        "duplicate",
        "M7 5.5h5a1.5 1.5 0 0 1 1.5 1.5v5a1.5 1.5 0 0 1-1.5 1.5h-5a1.5 1.5 0 0 1-1.5-1.5v-5a1.5 1.5 0 0 1 1.5-1.5M10.5 3.5v-.5A1.5 1.5 0 0 0 9 1.5H4A1.5 1.5 0 0 0 2.5 3v5A1.5 1.5 0 0 0 4 9.5h.5",
    ),
    ("edit", "M10.5 3 13 5.5 6 12.5H3.5V10z"),
    ("trash", "M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9"),
    ("external", "M6.5 4h5.5v5.5M12 4l-7.5 7.5"),
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
