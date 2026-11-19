//! Connection tags as the window reads them, as the web UI's `tags.ts`:
//! which one a connection carries, which one asks before data changes
//! (changing data on a connection whose tag asks first, PROD, waits for the
//! user to confirm), and what a new tag starts as. The settings' list of
//! tags is [`section::TagsSection`].

pub mod section;

use dbforge_core::connections::tags::{Tag, TagColor};

pub use section::TagsSection;

/// The palette a tag's colour comes from, in the order the picker shows it.
pub const TAG_COLORS: [TagColor; 9] = [
    TagColor::Red,
    TagColor::Orange,
    TagColor::Amber,
    TagColor::Green,
    TagColor::Teal,
    TagColor::Blue,
    TagColor::Violet,
    TagColor::Pink,
    TagColor::Gray,
];

/// The longest name a badge has room for; the core refuses longer ones.
pub const MAX_TAG_NAME: usize = 16;

/// The tag a connection carries, if it still exists.
pub fn tag_of<'a>(tags: &'a [Tag], id: Option<&str>) -> Option<&'a Tag> {
    let id = id?;
    tags.iter().find(|tag| tag.id == id)
}

/// The name of the tag that makes changes on a connection tagged `id` ask
/// first; `None` when they don't.
pub fn confirming_tag(tags: &[Tag], id: Option<&str>) -> Option<String> {
    tag_of(tags, id).filter(|tag| tag.confirm).map(|tag| tag.name.clone())
}

/// A new tag named `name`, with the first colour no other tag uses yet.
pub fn new_tag(tags: &[Tag], name: impl Into<String>, id: impl Into<String>) -> Tag {
    let color =
        TAG_COLORS.into_iter().find(|color| !tags.iter().any(|tag| tag.color == *color)).unwrap_or(TagColor::Gray);
    Tag { id: id.into(), name: name.into(), color, confirm: false }
}

/// The name a new tag gets: `name(1)`, or the first `name(n)` no tag in
/// `taken` has, ignoring case.
pub fn new_tag_name(taken: &[String], name: fn(usize) -> String) -> String {
    let taken: Vec<String> = taken.iter().map(|name| name.to_lowercase()).collect();
    (1..).map(name).find(|candidate| !taken.contains(&candidate.to_lowercase())).unwrap_or_default()
}

/// `name` cut to [`MAX_TAG_NAME`] characters, as the web UI's field kept it.
pub fn clamp_tag_name(name: &str) -> &str {
    match name.char_indices().nth(MAX_TAG_NAME) {
        Some((end, _)) => &name[..end],
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(id: &str, name: &str, confirm: bool) -> Tag {
        Tag { id: id.into(), name: name.into(), color: TagColor::Red, confirm }
    }

    #[test]
    fn asks_only_for_a_tag_that_asks_first_by_its_current_name() {
        let tags = [tag("prod", "PROD", true), tag("dev", "DEV", false), tag("live", "Live", true)];
        assert_eq!(confirming_tag(&tags, Some("prod")).as_deref(), Some("PROD"));
        assert_eq!(confirming_tag(&tags, Some("live")).as_deref(), Some("Live"), "a renamed tag keeps its id");
        assert_eq!(confirming_tag(&tags, Some("dev")), None);
        assert_eq!(confirming_tag(&tags, Some("gone")), None, "a deleted tag asks nothing");
        assert_eq!(confirming_tag(&tags, None), None);
    }

    fn colored(id: &str, color: TagColor) -> Tag {
        Tag { color, ..tag(id, id, false) }
    }

    #[test]
    fn gives_a_new_tag_a_color_no_other_tag_uses() {
        let tags = [colored("production", TagColor::Red), colored("development", TagColor::Blue)];
        assert_eq!(
            new_tag(&tags, "UAT", "x"),
            Tag { id: "x".into(), name: "UAT".into(), color: TagColor::Orange, confirm: false }
        );
        let every: Vec<Tag> = TAG_COLORS.iter().map(|color| colored("t", *color)).collect();
        assert_eq!(new_tag(&every, "MORE", "y").color, TagColor::Gray, "gray once every colour is used");
    }

    #[test]
    fn names_a_new_tag_after_the_ones_taken() {
        let name = |n: usize| if n == 1 { "NEW".to_owned() } else { format!("NEW {n}") };
        assert_eq!(new_tag_name(&["PROD".into()], name), "NEW");
        assert_eq!(new_tag_name(&["new".into(), "NEW 2".into()], name), "NEW 3", "ignoring case");
    }

    #[test]
    fn keeps_a_name_within_what_a_badge_has_room_for() {
        assert_eq!(clamp_tag_name("PRODUCTION-EU-WEST-1"), "PRODUCTION-EU-WE");
        assert_eq!(clamp_tag_name("ÉTÉ"), "ÉTÉ");
        assert_eq!(clamp_tag_name("ñññññññññññññññññ").chars().count(), MAX_TAG_NAME);
    }
}
