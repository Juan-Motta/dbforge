//! When the automatic check runs, as the former web UI's `schedule.ts`: once a day,
//! a moment after launch when a day has passed, and while the app stays
//! open. The last check is kept in the preferences, in milliseconds since
//! the epoch, under the former web UI's key.

use std::time::Duration;

/// How often the automatic check runs: once a day.
pub const CHECK_EVERY_MS: i64 = 24 * 60 * 60 * 1000;

/// How long after launch the first look waits, so it doesn't compete with the first screen.
pub const FIRST_LOOK: Duration = Duration::from_secs(5);

/// How often to see whether the daily check is due, while the app stays open.
pub const LOOK_EVERY: Duration = Duration::from_secs(60 * 60);

/// Whether an automatic check is due at `now`, the last one having run at
/// `last` (`None`: never). A clock set back past the last check checks again.
pub fn check_due(last: Option<i64>, now: i64) -> bool {
    match last {
        None => true,
        Some(last) => now - last >= CHECK_EVERY_MS || now < last,
    }
}

/// The saved time of the last check: a positive number, or none.
pub fn last_check(saved: Option<&str>) -> Option<i64> {
    let value = saved?.trim().parse::<f64>().ok()?;
    (value.is_finite() && value > 0.).then_some(value as i64)
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

    #[test]
    fn checks_the_first_time_and_once_a_day_after() {
        assert!(check_due(None, NOW));
        assert!(!check_due(Some(NOW - CHECK_EVERY_MS + 1), NOW));
        assert!(check_due(Some(NOW - CHECK_EVERY_MS), NOW));
    }

    #[test]
    fn checks_when_the_clock_went_back_past_the_last_check() {
        assert!(check_due(Some(NOW + 60_000), NOW));
    }

    #[test]
    fn reads_only_a_positive_saved_time() {
        assert_eq!(last_check(Some("1800000000000")), Some(NOW));
        assert_eq!(last_check(None), None);
        assert_eq!(last_check(Some("")), None);
        assert_eq!(last_check(Some("0")), None);
        assert_eq!(last_check(Some("-5")), None);
        assert_eq!(last_check(Some("soon")), None);
        assert_eq!(last_check(Some("inf")), None);
    }

    #[test]
    fn checks_by_itself_until_turned_off() {
        assert!(automatic(None));
        assert!(automatic(Some("true")));
        assert!(!automatic(Some("false")));
        assert!(!automatic(Some("anything else")), "as the former web UI read its flag");
    }
}
