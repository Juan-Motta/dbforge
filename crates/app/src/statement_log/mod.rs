//! The statement console, as the web UI's `StatementConsole.tsx`: the
//! statements DBForge (and the agents using its MCP server) send to the
//! databases, newest at the bottom, under the workspace. [`log`] holds its
//! rules; [`StatementConsole`] draws it while it is open.

mod console;
pub mod log;

pub use console::{StatementConsole, StatementConsoleEvent};

/// The console's height when none was kept, and the range it may be dragged within.
const HEIGHT: f32 = 220.;
const MIN_HEIGHT: f32 = 120.;
const MAX_HEIGHT: f32 = 640.;

/// The height kept under [`crate::preferences::keys::STATEMENTS_HEIGHT`],
/// within its range; the default when none was.
pub fn height(saved: Option<&str>) -> f32 {
    match saved.and_then(|saved| saved.parse::<f32>().ok()) {
        Some(height) if height > 0. => height.clamp(MIN_HEIGHT, MAX_HEIGHT),
        _ => HEIGHT,
    }
}

/// Whether the console was left open, as the web UI kept it (`true` or `false`); closed until one was kept.
pub fn is_open(saved: Option<&str>) -> bool {
    saved == Some("true")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_saved_height_within_its_range() {
        assert_eq!(height(None), 220.);
        assert_eq!(height(Some("300")), 300.);
        assert_eq!(height(Some("40")), 120.);
        assert_eq!(height(Some("4000")), 640.);
        assert_eq!(height(Some("0")), 220.);
        assert_eq!(height(Some("tall")), 220.);
    }

    #[test]
    fn opens_only_when_it_was_left_open() {
        assert!(is_open(Some("true")));
        assert!(!is_open(Some("false")));
        assert!(!is_open(None));
    }
}
