//! How the history writes its runs: grouped by day (Today, Yesterday, then
//! dated), with the time, how long each took and what it produced.

use chrono::{DateTime, Datelike, NaiveDate, TimeZone};
use dbforge_core::history::HistoryEntry;

use crate::i18n::HistoryMessages;

/// When `executed_at` (milliseconds since the epoch) was, in `zone`.
fn local<Tz: TimeZone>(executed_at: u64, zone: &Tz) -> Option<DateTime<Tz>> {
    zone.timestamp_millis_opt(executed_at as i64).single()
}

/// Splits newest-first entries into Today, Yesterday and dated groups, in
/// `now`'s time zone, keeping their order.
pub fn group_by_day<'a, Tz: TimeZone>(
    entries: &'a [HistoryEntry],
    now: &DateTime<Tz>,
    m: &HistoryMessages,
) -> Vec<(String, Vec<&'a HistoryEntry>)> {
    let today = now.date_naive();
    let mut groups: Vec<(String, Vec<&HistoryEntry>)> = Vec::new();
    for entry in entries {
        let day = local(entry.executed_at, &now.timezone()).map_or(today, |at| at.date_naive());
        let label = day_label(day, today, m);
        match groups.last_mut() {
            Some((last, group)) if *last == label => group.push(entry),
            _ => groups.push((label, vec![entry])),
        }
    }
    groups
}

fn day_label(day: NaiveDate, today: NaiveDate, m: &HistoryMessages) -> String {
    match (today - day).num_days() {
        0 => m.today.into(),
        1 => m.yesterday.into(),
        _ => (m.day)(day.weekday().num_days_from_monday(), day.month(), day.day()),
    }
}

/// `14:05`, in `zone`.
pub fn format_time<Tz: TimeZone>(executed_at: u64, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    local(executed_at, zone).map(|at| at.format("%H:%M").to_string()).unwrap_or_default()
}

/// `38 ms`, `8.2 s`, `2.5 min`.
pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.)
    } else {
        format!("{:.1} min", ms as f64 / 60_000.)
    }
}

/// What a run produced: "14 rows", "3 statements", or "error".
pub fn outcome_label(entry: &HistoryEntry, m: &HistoryMessages) -> String {
    if entry.error.is_some() {
        m.outcome_error.into()
    } else if entry.statements > 1 {
        (m.statements)(entry.statements)
    } else {
        (m.rows)(entry.rows)
    }
}

/// The first non-empty line, so a multi-line query reads well in one row.
pub fn first_line(sql: &str) -> &str {
    sql.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;
    use crate::i18n::Language;

    fn entry(id: &str, executed_at: u64) -> HistoryEntry {
        HistoryEntry {
            id: id.into(),
            executed_at,
            connection_id: "c1".into(),
            connection_name: "local".into(),
            database: None,
            container: None,
            sql: "SELECT 1".into(),
            elapsed_ms: 1,
            statements: 1,
            rows: 0,
            error: None,
        }
    }

    fn en() -> &'static HistoryMessages {
        &Language::En.messages().query.history_panel
    }

    #[test]
    fn labels_today_yesterday_and_older_days_keeping_the_order() {
        // Bogotá, five hours behind UTC: the days turn at its midnight.
        let zone = FixedOffset::west_opt(5 * 3600).unwrap();
        let at =
            |day: u32, hour: u32| zone.with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap().timestamp_millis() as u64;
        let now = zone.with_ymd_and_hms(2026, 11, 6, 15, 0, 0).unwrap();
        let entries = [entry("a", at(6, 9)), entry("b", at(6, 0)), entry("c", at(5, 23)), entry("d", at(2, 12))];
        let groups: Vec<(String, Vec<&str>)> = group_by_day(&entries, &now, en())
            .into_iter()
            .map(|(label, group)| (label, group.into_iter().map(|e| e.id.as_str()).collect()))
            .collect();
        assert_eq!(
            groups,
            [("Today".into(), vec!["a", "b"]), ("Yesterday".into(), vec!["c"]), ("Mon, Nov 2".into(), vec!["d"]),]
        );
        let es = &Language::Es.messages().query.history_panel;
        let labels: Vec<String> = group_by_day(&entries, &now, es).into_iter().map(|(label, _)| label).collect();
        assert_eq!(labels, ["Hoy", "Ayer", "lun, 2 nov"]);
        assert_eq!(format_time(at(6, 9) + 5 * 60_000, &zone), "09:05");
    }

    #[test]
    fn writes_durations() {
        assert_eq!(format_duration(38), "38 ms");
        assert_eq!(format_duration(8200), "8.2 s");
        assert_eq!(format_duration(150_000), "2.5 min");
    }

    #[test]
    fn names_errors_scripts_and_row_counts() {
        let with = |change: fn(&mut HistoryEntry)| {
            let mut entry = entry("1", 0);
            change(&mut entry);
            entry
        };
        assert_eq!(outcome_label(&with(|e| e.error = Some("boom".into())), en()), "error");
        assert_eq!(outcome_label(&with(|e| e.statements = 3), en()), "3 statements");
        assert_eq!(outcome_label(&with(|e| e.rows = 1), en()), "1 row");
        assert_eq!(outcome_label(&with(|e| e.rows = 18204), en()), "18,204 rows");
        let es = &Language::Es.messages().query.history_panel;
        assert_eq!(outcome_label(&with(|e| e.rows = 1), es), "1 fila");
    }

    #[test]
    fn skips_blank_lines_and_indentation() {
        assert_eq!(first_line("\n   SELECT id\n  FROM t"), "SELECT id");
        assert_eq!(first_line(""), "");
    }
}
