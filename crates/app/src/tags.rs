//! Connection tags as the window reads them: which one asks before data
//! changes. Changing data on a connection whose tag asks first (PROD) waits
//! for the user to confirm.

use dbforge_core::connections::tags::Tag;

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

#[cfg(test)]
mod tests {
    use dbforge_core::connections::tags::TagColor;

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
}
