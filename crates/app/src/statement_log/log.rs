//! The statement console's rules, as the former web UI's `statementLog.ts`: how
//! statements read at start and those arriving meanwhile merge, what each
//! filter shows, a statement as it is pasted into a console, and how times
//! and durations are written. Pure, so they are tested without a window.

use chrono::{TimeZone, Timelike};
use dbforge_core::connections::model::Engine;
use dbforge_core::statement_log::{Entry, Origin};

use crate::connections::engine::is_relational;

/// As many as the core keeps.
pub const CAPACITY: usize = 2000;

/// The console's filter: what the statements were for, grouped as people think of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    All,
    Console,
    Agents,
    Changes,
    Reads,
    Metadata,
}

impl Kind {
    /// The filter's choices, in the order the console lists them.
    pub const ALL: [Kind; 6] = [Kind::All, Kind::Console, Kind::Agents, Kind::Changes, Kind::Reads, Kind::Metadata];
}

/// What a statement sent for `origin` counts as.
pub fn kind_of(origin: Origin) -> Kind {
    match origin {
        Origin::Console => Kind::Console,
        Origin::Edit | Origin::Admin | Origin::Dump | Origin::Import => Kind::Changes,
        Origin::Browse | Origin::Export => Kind::Reads,
        Origin::Metadata | Origin::Connect => Kind::Metadata,
        Origin::Agent => Kind::Agents,
    }
}

/// Adds `incoming` to `entries` by id: the list read at start and the
/// events that arrive meanwhile may overlap. Oldest first, at most
/// `capacity`; whether anything was added.
pub fn merge(entries: &mut Vec<Entry>, incoming: Vec<Entry>, capacity: usize) -> bool {
    let last = entries.last().map(|entry| entry.id);
    // The usual case: newer statements, in order, which go at the end as they are.
    let in_order = incoming.iter().enumerate().all(|(i, entry)| {
        match i.checked_sub(1).map(|previous| incoming[previous].id).or(last) {
            Some(before) => entry.id > before,
            None => true,
        }
    });
    if in_order {
        if incoming.is_empty() {
            return false;
        }
        entries.extend(incoming);
    } else {
        let mut known: std::collections::HashSet<u64> = entries.iter().map(|entry| entry.id).collect();
        let added: Vec<Entry> = incoming.into_iter().filter(|entry| known.insert(entry.id)).collect();
        if added.is_empty() {
            return false;
        }
        entries.extend(added);
        entries.sort_by_key(|entry| entry.id);
    }
    if entries.len() > capacity {
        entries.drain(..entries.len() - capacity);
    }
    true
}

/// The entries `kind` shows.
pub fn visible(entries: &[Entry], kind: Kind) -> Vec<&Entry> {
    entries.iter().filter(|entry| kind == Kind::All || kind_of(entry.origin) == kind).collect()
}

/// The statement as it is pasted into a console: SQL ends with `;`, other languages as they are.
pub fn runnable(entry: &Entry, engine: Option<Engine>) -> String {
    let text = entry.text.trim();
    if engine.is_some_and(is_relational) && !text.ends_with(';') { format!("{text};") } else { text.to_owned() }
}

/// The statements as one script, in the order they ran.
pub fn script<'a>(entries: impl IntoIterator<Item = &'a Entry>, engine_of: impl Fn(&str) -> Option<Engine>) -> String {
    entries.into_iter().map(|entry| runnable(entry, engine_of(&entry.connection_id))).collect::<Vec<_>>().join("\n\n")
}

/// `14:03:27.512`: when `at` (milliseconds since the epoch) was, in `zone`.
pub fn clock_time<Tz: TimeZone>(at: u64, zone: &Tz) -> String {
    match zone.timestamp_millis_opt(at as i64).single() {
        Some(time) => format!(
            "{:02}:{:02}:{:02}.{:03}",
            time.hour(),
            time.minute(),
            time.second(),
            time.timestamp_subsec_millis()
        ),
        None => String::new(),
    }
}

/// `8 ms`, `1.24 s`.
pub fn duration(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms} ms");
    }
    // JavaScript's `toFixed(2)` rounds an exact half up, where Rust's
    // formatting rounds it to even: 1125 ms is 1.13 s, as the former web UI wrote it.
    // Only multiples of 125 ms are exact in binary.
    if ms.is_multiple_of(125) && ms % 10 == 5 {
        let hundredths = ms / 10 + 1;
        return format!("{}.{:02} s", hundredths / 100, hundredths % 100);
    }
    format!("{:.2} s", ms as f64 / 1000.)
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, TimeZone};

    use super::*;

    fn entry(id: u64, text: &str, origin: Origin) -> Entry {
        Entry {
            id,
            at: 0,
            connection_id: "pg".into(),
            database: Some("shop".into()),
            origin,
            text: text.into(),
            elapsed_ms: 1,
            rows: Some(1),
            error: None,
        }
    }

    fn select(id: u64) -> Entry {
        entry(id, &format!("SELECT {id}"), Origin::Console)
    }

    fn ids(entries: &[Entry]) -> Vec<u64> {
        entries.iter().map(|entry| entry.id).collect()
    }

    #[test]
    fn adds_new_entries_by_id_in_order_without_the_ones_already_there() {
        let mut entries = vec![select(1), select(3)];
        assert!(merge(&mut entries, vec![select(3), select(2), select(4)], CAPACITY));
        assert_eq!(ids(&entries), [1, 2, 3, 4]);
    }

    #[test]
    fn keeps_the_newest_up_to_the_capacity_and_the_same_list_when_nothing_is_new() {
        let mut entries = vec![select(1), select(2)];
        assert!(!merge(&mut entries, vec![select(2)], CAPACITY));
        assert!(!merge(&mut entries, Vec::new(), CAPACITY));
        assert_eq!(ids(&entries), [1, 2]);
        assert!(merge(&mut entries, vec![select(3)], 2));
        assert_eq!(ids(&entries), [2, 3]);
    }

    #[test]
    fn filters_by_what_the_statements_were_for() {
        let entries = [
            entry(1, "a", Origin::Console),
            entry(2, "b", Origin::Edit),
            entry(3, "c", Origin::Metadata),
            entry(4, "d", Origin::Browse),
            entry(5, "e", Origin::Agent),
        ];
        let shown = |kind| visible(&entries, kind).into_iter().map(|entry| entry.id).collect::<Vec<_>>();
        assert_eq!(shown(Kind::All).len(), 5);
        assert_eq!(shown(Kind::Changes), [2]);
        assert_eq!(shown(Kind::Reads), [4]);
        assert_eq!(shown(Kind::Metadata), [3]);
        assert_eq!(shown(Kind::Agents), [5]);
        assert_eq!(shown(Kind::Console), [1]);
        assert_eq!(kind_of(Origin::Connect), Kind::Metadata);
        assert_eq!(kind_of(Origin::Export), Kind::Reads);
        for origin in [Origin::Admin, Origin::Dump, Origin::Import] {
            assert_eq!(kind_of(origin), Kind::Changes, "{origin:?}");
        }
    }

    #[test]
    fn ends_sql_with_a_semicolon_and_leaves_other_languages_as_they_are() {
        let console = |text| entry(1, text, Origin::Console);
        assert_eq!(runnable(&console("SELECT 1 "), Some(Engine::Postgres)), "SELECT 1;");
        assert_eq!(runnable(&console("COMMIT;"), Some(Engine::Sqlserver)), "COMMIT;");
        assert_eq!(runnable(&console("GET \"k\""), Some(Engine::Redis)), "GET \"k\"");
        assert_eq!(
            runnable(&console("db.getCollection(\"a\").find({})"), Some(Engine::Mongodb)),
            "db.getCollection(\"a\").find({})"
        );
        assert_eq!(runnable(&console("SELECT 1"), None), "SELECT 1", "an unknown connection's language is unknown");
    }

    #[test]
    fn joins_statements_in_order_each_runnable_on_its_engine() {
        let redis = Entry { connection_id: "r".into(), ..entry(2, "GET \"k\"", Origin::Browse) };
        let entries = [entry(1, "BEGIN", Origin::Edit), redis];
        let engine_of = |id: &str| Some(if id == "r" { Engine::Redis } else { Engine::Postgres });
        assert_eq!(script(&entries, engine_of), "BEGIN;\n\nGET \"k\"");
    }

    #[test]
    fn writes_the_time_of_day_with_milliseconds_and_short_durations() {
        let zone = FixedOffset::west_opt(5 * 3600).unwrap();
        let at = zone.with_ymd_and_hms(2026, 11, 10, 9, 5, 7).unwrap().timestamp_millis() as u64 + 42;
        assert_eq!(clock_time(at, &zone), "09:05:07.042");
        assert_eq!(duration(8), "8 ms");
        assert_eq!(duration(999), "999 ms");
        assert_eq!(duration(1240), "1.24 s");
        assert_eq!(duration(1000), "1.00 s");
        assert_eq!(duration(1125), "1.13 s", "an exact half rounds up, as toFixed does");
        assert_eq!(duration(61_375), "61.38 s");
    }
}
