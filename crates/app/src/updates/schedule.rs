//! When the automatic check runs and when it speaks. Looking is cheap (one
//! small manifest), so it looks a moment after launch and every hour while the
//! app stays open: a new release is noticed within the hour even if DBForge is
//! never quit. Speaking is rarer: a version is announced as soon as it is
//! first found, then, while it stays uninstalled, once a day. The last
//! announcement (version and time) is kept in the preferences.

use std::time::Duration;

/// How long after launch the first look waits, so it doesn't compete with the first screen.
pub const FIRST_LOOK: Duration = Duration::from_secs(5);

/// How often to look for a new version while the app stays open.
pub const LOOK_EVERY: Duration = Duration::from_secs(60 * 60);

/// How long a version already announced waits before it is announced again.
pub const REMIND_EVERY_MS: i64 = 24 * 60 * 60 * 1000;

/// The last version announced, and when (milliseconds since the epoch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announced {
    pub version: String,
    pub at: i64,
}

impl Announced {
    /// Reads the saved announcement, `<version>@<milliseconds>`; anything else is none.
    pub fn parse(saved: Option<&str>) -> Option<Self> {
        let (version, at) = saved?.trim().rsplit_once('@')?;
        let at = at.parse::<i64>().ok().filter(|at| *at > 0)?;
        (!version.is_empty()).then(|| Self { version: version.to_owned(), at })
    }

    /// The text kept in the preferences.
    pub fn saved(&self) -> String {
        format!("{}@{}", self.version, self.at)
    }
}

/// Whether an automatic check that found `version` at `now` says so: always
/// for a version not announced yet, else once `REMIND_EVERY_MS` has passed
/// since it was (or the clock went back past that).
pub fn announces(version: &str, last: Option<&Announced>, now: i64) -> bool {
    match last {
        Some(last) if last.version == version => now - last.at >= REMIND_EVERY_MS || now < last.at,
        _ => true,
    }
}

/// Whether the automatic check is on: on until the user turns it off, as
/// the former web UI's stored flag (`"true"` or `"false"`).
pub fn automatic(saved: Option<&str>) -> bool {
    saved.is_none_or(|saved| saved == "true")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn announced(version: &str, at: i64) -> Announced {
        Announced { version: version.to_owned(), at }
    }

    #[test]
    fn announces_a_new_version_as_soon_as_it_is_found() {
        assert!(announces("0.4.1", None, NOW));
        assert!(announces("0.4.2", Some(&announced("0.4.1", NOW - 60_000)), NOW), "a newer one than the last told");
    }

    #[test]
    fn reminds_of_a_version_already_announced_once_a_day() {
        let told = announced("0.4.1", NOW - REMIND_EVERY_MS + 1);
        assert!(!announces("0.4.1", Some(&told), NOW));
        let told = announced("0.4.1", NOW - REMIND_EVERY_MS);
        assert!(announces("0.4.1", Some(&told), NOW));
    }

    #[test]
    fn reminds_when_the_clock_went_back_past_the_announcement() {
        assert!(announces("0.4.1", Some(&announced("0.4.1", NOW + 60_000)), NOW));
    }

    #[test]
    fn looks_soon_after_launch_and_every_hour() {
        assert_eq!(FIRST_LOOK, Duration::from_secs(5));
        assert_eq!(LOOK_EVERY, Duration::from_secs(3600));
    }

    #[test]
    fn keeps_the_announcement_as_version_and_time() {
        let told = announced("0.4.1", NOW);
        assert_eq!(told.saved(), "0.4.1@1800000000000");
        assert_eq!(Announced::parse(Some(&told.saved())), Some(told));
        assert_eq!(Announced::parse(Some("1.0.0-rc.1@5")), Some(announced("1.0.0-rc.1", 5)));
        for broken in ["", "0.4.1", "@5", "0.4.1@", "0.4.1@0", "0.4.1@-5", "0.4.1@soon"] {
            assert_eq!(Announced::parse(Some(broken)), None, "{broken:?}");
        }
        assert_eq!(Announced::parse(None), None);
    }

    #[test]
    fn checks_by_itself_until_turned_off() {
        assert!(automatic(None));
        assert!(automatic(Some("true")));
        assert!(!automatic(Some("false")));
        assert!(!automatic(Some("anything else")), "as the former web UI read its flag");
    }
}
