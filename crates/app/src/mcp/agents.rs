//! Who uses DBForge's MCP server, as Settings › Agents shows it: the known
//! agents, each with what the core has seen of it ([`AgentSeen`]), how it was
//! used, which of them count as connected, the agent shown when none was
//! picked, and the agent statements run today. Pure, so they are tested
//! without a window.

use chrono::TimeZone;
use dbforge_core::mcp::AgentSeen;

use super::setup::{AGENTS, AgentId};

/// How long after its last call an agent still counts as connected.
pub const CONNECTED_WITHIN_MS: i64 = 10 * 60 * 1000;

/// One row of the agents' list: a known agent and what DBForge has seen of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listed {
    pub agent: AgentId,
    /// Whether it has used DBForge, in this run or in a saved session.
    pub used: bool,
    /// Milliseconds since the epoch of its last call in this run; `None` when it made none.
    pub last_active_ms: Option<i64>,
}

/// How an agent has used DBForge, as its row says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usage {
    /// It made a call within [`CONNECTED_WITHIN_MS`].
    Connected,
    /// It made a call this run, this long ago.
    UsedAgo(Ago),
    /// Only a saved session knows it.
    UsedBefore,
    NotUsed,
}

/// How long ago an agent was last active, rounded down to the unit it is said in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ago {
    Minutes(i64),
    Hours(i64),
    Days(i64),
}

/// The setup for the agent that calls itself `name` (its MCP `clientInfo.name`).
pub fn agent_of(name: &str) -> AgentId {
    let name = name.trim().to_lowercase();
    if name.contains("claude") {
        AgentId::Claude
    } else if name.contains("codex") {
        AgentId::Codex
    } else if name.contains("opencode") {
        AgentId::Opencode
    } else if name == "pi" || name.starts_with("pi-") {
        AgentId::Pi
    } else {
        AgentId::Other
    }
}

/// The agents' list: every known agent in the order DBForge offers them, each
/// with the most recent activity of the agents the core has seen under it
/// (the ones DBForge doesn't know count as "Other agent").
pub fn listed(seen: &[AgentSeen]) -> Vec<Listed> {
    AGENTS
        .into_iter()
        .map(|agent| {
            let mut row = Listed { agent, used: false, last_active_ms: None };
            for seen in seen.iter().filter(|seen| agent_of(&seen.name) == agent) {
                row.used = true;
                row.last_active_ms = row.last_active_ms.max(seen.last_active_ms);
            }
            row
        })
        .collect()
}

/// How `row`'s agent has used DBForge, at `now_ms`.
pub fn usage(row: &Listed, now_ms: i64) -> Usage {
    match row.last_active_ms {
        Some(at) if is_connected(Some(at), now_ms) => Usage::Connected,
        Some(at) => Usage::UsedAgo(ago(at, now_ms)),
        None if row.used => Usage::UsedBefore,
        None => Usage::NotUsed,
    }
}

/// The agent shown: `picked`, else the most recently active, else the first
/// used before, else Claude Code.
pub fn shown_agent(picked: Option<AgentId>, listed: &[Listed]) -> AgentId {
    picked
        .or_else(|| {
            listed
                .iter()
                .filter(|row| row.last_active_ms.is_some())
                .max_by_key(|row| row.last_active_ms)
                .map(|row| row.agent)
        })
        .or_else(|| listed.iter().find(|row| row.used).map(|row| row.agent))
        .unwrap_or(AgentId::Claude)
}

/// Whether an agent last active at `last_active_ms` still counts as connected at `now_ms`.
pub fn is_connected(last_active_ms: Option<i64>, now_ms: i64) -> bool {
    last_active_ms.is_some_and(|at| now_ms - at < CONNECTED_WITHIN_MS)
}

/// How many of `listed` count as connected at `now_ms`.
pub fn connected(listed: &[Listed], now_ms: i64) -> usize {
    listed.iter().filter(|row| is_connected(row.last_active_ms, now_ms)).count()
}

/// How long before `now_ms` an agent was last active at `at_ms`.
pub fn ago(at_ms: i64, now_ms: i64) -> Ago {
    let minutes = (now_ms - at_ms).max(0) / 60_000;
    match minutes {
        ..60 => Ago::Minutes(minutes),
        60..1440 => Ago::Hours(minutes / 60),
        _ => Ago::Days(minutes / 1440),
    }
}

/// How many of the statements that started at `times` (milliseconds since the
/// epoch) started on the same day as `now_ms`, in `zone`.
pub fn statements_today<Tz: TimeZone>(times: impl IntoIterator<Item = u64>, now_ms: i64, zone: &Tz) -> usize {
    let day = |ms: i64| zone.timestamp_millis_opt(ms).single().map(|time| time.date_naive());
    let Some(today) = day(now_ms) else { return 0 };
    times.into_iter().filter(|&at| day(at as i64) == Some(today)).count()
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    fn seen(name: &str, last_active_ms: Option<i64>) -> AgentSeen {
        AgentSeen { name: name.into(), version: None, last_active_ms }
    }

    fn row(agent: AgentId, used: bool, last_active_ms: Option<i64>) -> Listed {
        Listed { agent, used, last_active_ms }
    }

    fn unused() -> Vec<Listed> {
        AGENTS.into_iter().map(|agent| row(agent, false, None)).collect()
    }

    #[test]
    fn names_each_agent_by_the_setup_it_takes() {
        for (name, agent) in [
            ("claude-code", AgentId::Claude),
            ("Claude Code", AgentId::Claude),
            ("codex-mcp-client", AgentId::Codex),
            ("opencode", AgentId::Opencode),
            ("pi", AgentId::Pi),
            ("PI", AgentId::Pi),
            ("pi-coding-agent", AgentId::Pi),
            ("pipeline", AgentId::Other),
            ("api-client", AgentId::Other),
            ("aider", AgentId::Other),
            ("agent", AgentId::Other),
        ] {
            assert_eq!(agent_of(name), agent, "{name}");
        }
    }

    #[test]
    fn always_lists_the_five_known_agents_in_order() {
        assert_eq!(listed(&[]), unused());
        let agents: Vec<AgentId> =
            listed(&[seen("pi", Some(1)), seen("aider", None)]).iter().map(|row| row.agent).collect();
        assert_eq!(agents, AGENTS);
    }

    #[test]
    fn folds_unknown_names_into_other_agent_keeping_the_most_recent_activity() {
        let rows = listed(&[
            seen("claude-code", Some(9_000)),
            seen("aider", Some(5_000)),
            seen("claude-desktop", Some(1_000)),
            seen("codex", None),
            seen("agent", Some(7_000)),
        ]);
        assert_eq!(
            rows,
            [
                row(AgentId::Claude, true, Some(9_000)),
                row(AgentId::Codex, true, None),
                row(AgentId::Opencode, false, None),
                row(AgentId::Pi, false, None),
                row(AgentId::Other, true, Some(7_000)),
            ]
        );
        // The most recent activity wins, whichever comes first.
        assert_eq!(listed(&[seen("agent", None), seen("aider", Some(3))])[4], row(AgentId::Other, true, Some(3)));
    }

    #[test]
    fn says_how_each_agent_was_used() {
        let now = 100 * 60_000;
        assert_eq!(usage(&row(AgentId::Claude, true, Some(now - 1_000)), now), Usage::Connected);
        assert_eq!(
            usage(&row(AgentId::Claude, true, Some(now - CONNECTED_WITHIN_MS)), now),
            Usage::UsedAgo(Ago::Minutes(10))
        );
        assert_eq!(usage(&row(AgentId::Codex, true, None), now), Usage::UsedBefore);
        assert_eq!(usage(&row(AgentId::Pi, false, None), now), Usage::NotUsed);
    }

    #[test]
    fn shows_the_picked_agent_else_the_most_recent_else_the_first_used_else_claude_code() {
        assert_eq!(shown_agent(None, &unused()), AgentId::Claude);
        let rows = [
            row(AgentId::Claude, false, None),
            row(AgentId::Codex, true, None),
            row(AgentId::Opencode, true, Some(5)),
            row(AgentId::Pi, false, None),
            row(AgentId::Other, true, Some(9)),
        ];
        assert_eq!(shown_agent(Some(AgentId::Pi), &rows), AgentId::Pi);
        assert_eq!(shown_agent(None, &rows), AgentId::Other, "the most recently active");
        let before = [row(AgentId::Claude, false, None), row(AgentId::Pi, true, None), row(AgentId::Other, true, None)];
        assert_eq!(shown_agent(None, &before), AgentId::Pi, "the first used before");
    }

    #[test]
    fn counts_an_agent_connected_for_ten_minutes_after_its_last_call() {
        let now = 100 * 60_000;
        assert!(is_connected(Some(now), now));
        assert!(is_connected(Some(now - CONNECTED_WITHIN_MS + 1), now));
        assert!(!is_connected(Some(now - CONNECTED_WITHIN_MS), now));
        assert!(!is_connected(None, now));
        let rows = [
            row(AgentId::Claude, true, Some(now - 1_000)),
            row(AgentId::Other, true, Some(0)),
            row(AgentId::Pi, true, None),
        ];
        assert_eq!(connected(&rows, now), 1);
    }

    #[test]
    fn says_how_long_ago_in_minutes_hours_or_days() {
        let now = 10 * 24 * 3_600_000;
        assert_eq!(ago(now - 12 * 60_000, now), Ago::Minutes(12));
        assert_eq!(ago(now - 59 * 60_000 - 59_000, now), Ago::Minutes(59));
        assert_eq!(ago(now - 60 * 60_000, now), Ago::Hours(1));
        assert_eq!(ago(now - 23 * 3_600_000 - 59 * 60_000, now), Ago::Hours(23));
        assert_eq!(ago(now - 3 * 24 * 3_600_000, now), Ago::Days(3));
        assert_eq!(ago(now + 5_000, now), Ago::Minutes(0), "a clock a little ahead");
    }

    #[test]
    fn counts_the_statements_of_the_local_day() {
        // 2026-11-21 00:30 at UTC-3 is 03:30 UTC.
        let zone = FixedOffset::west_opt(3 * 3600).unwrap();
        let midnight = zone.with_ymd_and_hms(2026, 11, 21, 0, 0, 0).unwrap().timestamp_millis();
        let now = midnight + 30 * 60_000;
        let times = [midnight - 1, midnight, midnight + 1_000, now, (midnight + 24 * 3_600_000 - 1)].map(|t| t as u64);
        assert_eq!(statements_today(times, now, &zone), 4);
        // In UTC the same instants fall on one day: 02:59:59.999 to 03:30 is all the 21st.
        assert_eq!(statements_today(times[..4].iter().copied(), now, &chrono::Utc), 4);
        assert_eq!(statements_today([midnight as u64 - 4 * 3_600_000], now, &chrono::Utc), 0);
    }
}
