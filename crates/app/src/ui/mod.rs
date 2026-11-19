//! The glass building blocks views share: panels, cards, capsules and the
//! segments inside them, pills and dots, buttons, and the form controls
//! (text, password and file fields, dropdowns, checkboxes, section tabs).
//! Each takes the theme and returns an element to finish with its own
//! content, or is an entity its owner keeps.

pub mod button;
pub mod checkbox;
pub mod code_editor;
pub mod editing;
pub mod field;
pub mod file_field;
pub mod menu;
pub mod select;
pub mod sheet;
pub mod tabs;
pub mod text_input;

use std::time::Duration;

use dbforge_core::connections::model::Engine;
use dbforge_core::connections::tags::Tag;
use gpui::{
    Animation, AnimationExt, AnyElement, BoxShadow, Div, ElementId, FontWeight, Hsla, IntoElement, ParentElement,
    Styled, Transformation, div, percentage, point, px, svg,
};

pub use button::{button, enabled, ghost_button, icon_button};
pub use checkbox::Checkbox;
pub use code_editor::{CodeEditor, CodeEditorEvent};
pub use file_field::FileField;
pub use menu::{MenuItem, PopoverMenu, Swatch, popover};
pub use select::{Select, SelectEvent, SelectOption};
pub use tabs::{SectionTabs, Tab};
pub use text_input::{TextInput, TextInputEvent};

use crate::theme::{Theme, color};

/// A soft drop shadow, `blur` wide and `y` down, in `0xRRGGBBAA`.
pub fn shadow(blur: f32, y: f32, rgba: u32) -> Vec<BoxShadow> {
    vec![BoxShadow { color: color(rgba), offset: point(px(0.), px(y)), blur_radius: px(blur), spread_radius: px(0.) }]
}

/// An icon from [`crate::assets`], tinted `tint`.
pub fn icon(name: &str, tint: Hsla, size: f32) -> impl IntoElement + use<> {
    svg().path(format!("{name}.svg")).size(px(size)).flex_none().text_color(tint)
}

/// A translucent panel with a light rim and a soft shadow.
pub fn glass(theme: &Theme) -> Div {
    div().bg(theme.glass).border_1().border_color(theme.rim).shadow(shadow(24., 10., 0x00000038))
}

/// A near-opaque card that fills the space left: an editor, a grid, an empty state.
pub fn card(theme: &Theme) -> Div {
    div()
        .bg(theme.card)
        .border_1()
        .border_color(theme.rim)
        .rounded(px(22.))
        .shadow(shadow(30., 14., 0x00000040))
        .overflow_hidden()
        .flex_1()
        .min_h_0()
}

/// A row of segments in one glass capsule.
pub fn group(theme: &Theme, segments: Vec<AnyElement>) -> Div {
    glass(theme).h(px(34.)).rounded(px(17.)).p(px(3.)).flex().gap(px(2.)).children(segments)
}

/// One segment of a [`group`]: an icon, a label or both; raised when `on`.
pub fn segment(theme: &Theme, icon_name: Option<&str>, label: &str, on: bool) -> Div {
    let tint = if on { theme.text } else { theme.text2 };
    let mut segment = div()
        .h(px(26.))
        .flex()
        .items_center()
        .gap(px(7.))
        .rounded(px(13.))
        .px(px(if label.is_empty() { 6. } else { 12. }))
        .text_color(tint)
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM);
    if on {
        segment = segment.bg(theme.glass_hi).border_1().border_color(theme.rim);
    }
    if let Some(name) = icon_name {
        segment = segment.child(icon(name, tint, 14.));
    }
    if !label.is_empty() {
        segment = segment.child(label.to_owned());
    }
    segment
}

/// A glass capsule for a toolbar control: a picker, a search field.
pub fn capsule(theme: &Theme) -> Div {
    glass(theme).h(px(34.)).rounded(px(17.)).px(px(14.)).flex().items_center().gap(px(8.)).text_size(px(13.))
}

/// A small coloured label: a connection's tag.
pub fn pill(label: &str, tint: Hsla) -> Div {
    div()
        .h(px(18.))
        .px(px(7.))
        .flex()
        .items_center()
        .rounded(px(9.))
        .bg(tint.opacity(0.2))
        .text_color(tint)
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .child(label.to_owned())
}

/// A coloured dot: a connection's colour.
pub fn dot(tint: Hsla) -> Div {
    div().size(px(8.)).rounded(px(4.)).flex_none().bg(tint)
}

/// A connection's engine, as a dot in its colour.
pub fn engine_dot(theme: &Theme, engine: Engine) -> Div {
    dot(theme.engine(engine))
}

/// A connection's tag (its environment: PROD, STAGE…) as a badge in the tag's colour.
pub fn tag_badge(theme: &Theme, tag: &Tag) -> Div {
    pill(&tag.name, theme.tag(tag.color)).flex_none()
}

/// A ring turning while something loads; the text beside it says what.
/// `id` must be unique among the spinners drawn at once.
pub fn spinner(id: impl Into<ElementId>, tint: Hsla, size: f32) -> impl IntoElement {
    svg().path("spinner.svg").size(px(size)).flex_none().text_color(tint).with_animation(
        id,
        Animation::new(Duration::from_millis(900)).repeat(),
        |ring, turned| ring.with_transformation(Transformation::rotate(percentage(turned))),
    )
}

/// The accent's capsule button: Run, a dialog's main action.
pub fn primary_button(theme: &Theme, label: &str) -> Div {
    div()
        .h(px(32.))
        .px(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .rounded(px(16.))
        .bg(gpui::linear_gradient(
            180.,
            gpui::linear_color_stop(theme.accent_hi, 0.),
            gpui::linear_color_stop(theme.accent, 1.),
        ))
        .border_1()
        .border_color(color(0xFFFFFF47))
        .shadow(shadow(20., 8., 0xD4532566))
        .text_color(gpui::white())
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label.to_owned())
}

/// A `secondary-<key>` shortcut as each OS writes it: `⌘K` on macOS, `Ctrl+K` elsewhere.
pub fn shortcut(key: &str) -> String {
    let key = key.to_uppercase();
    if cfg!(target_os = "macos") { format!("⌘{key}") } else { format!("Ctrl+{key}") }
}

/// Keeps `element` as an [`AnyElement`], for [`group`]'s segments.
pub fn any(element: impl IntoElement) -> AnyElement {
    element.into_any_element()
}
