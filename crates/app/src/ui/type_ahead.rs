//! Typing letters on a list of choices jumps to the one they start, as the
//! former web UI's dropdowns and cell choice editor did: letters typed in a
//! row (less than [`PAUSE`] apart) build one prefix; after a pause the next
//! letter starts again. A single letter moves past the current choice, so
//! pressing it again walks through the choices it starts.

use std::time::{Duration, Instant};

use gpui::Keystroke;

/// How long after a letter the next one still adds to the same prefix.
pub const PAUSE: Duration = Duration::from_millis(700);

/// The letters typed in a row.
#[derive(Debug, Default, Clone)]
pub struct TypeAhead {
    typed: String,
    at: Option<Instant>,
}

impl TypeAhead {
    /// Adds `text` typed `now`, and returns the choice it jumps to among
    /// `labels` from `current` (the highlighted or chosen one), if any.
    pub fn type_text(&mut self, text: &str, now: Instant, labels: &[&str], current: Option<usize>) -> Option<usize> {
        if !self.running(now) {
            self.typed.clear();
        }
        self.typed.push_str(text);
        self.at = Some(now);
        if labels.is_empty() {
            return None;
        }
        let current = current.unwrap_or(0).min(labels.len() - 1);
        // A run of letters keeps refining the choice it is on; a new one moves on.
        let from = if self.typed.chars().count() > 1 { current } else { (current + 1) % labels.len() };
        match_typed(labels, &self.typed, from)
    }
}

impl TypeAhead {
    /// Whether a run of letters is still going at `now`.
    pub fn running(&self, now: Instant) -> bool {
        self.at.is_some_and(|at| now.saturating_duration_since(at) < PAUSE)
    }
}

/// The first label at or after `from` (wrapping around) that starts with `typed`, ignoring case.
pub fn match_typed(labels: &[&str], typed: &str, from: usize) -> Option<usize> {
    let prefix = typed.to_lowercase();
    (0..labels.len())
        .map(|offset| (from + offset) % labels.len())
        .find(|&index| labels[index].to_lowercase().starts_with(&prefix))
}

/// What a key press types for a type-ahead: a printable character with no
/// ⌘ / Ctrl. A space only continues a run (Space opens or chooses on its own).
pub fn typed_text(keystroke: &Keystroke, running: bool) -> Option<&str> {
    let modifiers = &keystroke.modifiers;
    if modifiers.platform || modifiers.control || modifiers.function {
        return None;
    }
    let text = match (keystroke.key_char.as_deref(), keystroke.key.as_str()) {
        (Some(text), _) => text,
        (None, "space") => " ",
        (None, key) if key.chars().count() == 1 => key,
        _ => return None,
    };
    let printable = !text.is_empty() && text.chars().all(|c| !c.is_control());
    (printable && (running || !text.trim().is_empty())).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENGINES: [&str; 4] = ["PostgreSQL", "MySQL", "MariaDB", "MongoDB"];

    #[test]
    fn jumps_to_the_next_option_starting_with_what_was_typed() {
        assert_eq!(match_typed(&ENGINES, "m", 0), Some(1));
        assert_eq!(match_typed(&ENGINES, "m", 2), Some(2));
        assert_eq!(match_typed(&ENGINES, "mo", 0), Some(3));
        assert_eq!(match_typed(&ENGINES, "MO", 0), Some(3), "case is ignored");
        assert_eq!(match_typed(&ENGINES, "p", 3), Some(0), "it wraps around");
        assert_eq!(match_typed(&ENGINES, "x", 0), None);
        assert_eq!(match_typed(&[], "m", 0), None);
    }

    #[test]
    fn letters_typed_in_a_row_build_one_prefix_and_a_pause_starts_again() {
        let start = Instant::now();
        let mut typing = TypeAhead::default();
        // On PostgreSQL: "m" moves on to MySQL, "a" refines to MariaDB.
        assert_eq!(typing.type_text("m", start, &ENGINES, Some(0)), Some(1));
        assert_eq!(typing.type_text("a", start + Duration::from_millis(300), &ENGINES, Some(1)), Some(2));
        // After a pause "m" starts a new run, moving past MariaDB.
        let later = start + Duration::from_millis(300) + PAUSE;
        assert_eq!(typing.type_text("m", later, &ENGINES, Some(2)), Some(3));
        // The same letter again walks through what it starts.
        let again = later + PAUSE;
        assert_eq!(typing.type_text("m", again, &ENGINES, Some(3)), Some(1));
        // Nothing starts with it: no jump.
        assert_eq!(typing.type_text("z", again + PAUSE, &ENGINES, Some(1)), None);
    }

    #[test]
    fn types_printable_characters_without_command_or_control() {
        let key = |source: &str| Keystroke::parse(source).unwrap();
        let typed = |source: &str, running: bool| typed_text(&key(source), running).map(str::to_owned);
        assert_eq!(typed("m", false).as_deref(), Some("m"));
        assert_eq!(typed("cmd-m", false), None);
        assert_eq!(typed("ctrl-m", false), None);
        assert_eq!(typed("enter", false), None);
        assert_eq!(typed("down", false), None);
        assert_eq!(typed("space", false), None, "a lone space opens or chooses");
        assert_eq!(typed("space", true).as_deref(), Some(" "), "inside a run it is part of the label");
    }
}
