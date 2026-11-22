//! Who uses DBForge's MCP server, as Settings › Agents shows it: the agents
//! the core has seen ([`AgentSeen`]) named as DBForge knows them, which of
//! them count as connected, the known agents left to add, the agent picked
//! when none was, and the agent statements run today. Pure, so they are
//! tested without a window.

use chrono::TimeZone;
use dbforge_core::mcp::AgentSeen;

use super::setup::{AGENTS, AgentId};

/// How long after its last call an agent still counts as connected.
pub const CONNECTED_WITHIN_MS: i64 = 10 * 60 * 1000;

/// An agent in the list: a known one by its setup, or one DBForge has no
/// setup for by its own name (its setup is then [`AgentId::Other`]'s).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Pick {
    pub agent: AgentId,
    /// The agent's own name, for one DBForge has no setup for; `None` for a known one and for "Other agent".
    pub name: Option<String>,
}

impl Pick {
    pub fn known(agent: AgentId) -> Self {
        Self { agent, name: None }
    }
}

/// One of "Your agents": an agent that has used DBForge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub pick: Pick,
    /// Milliseconds since the epoch of its last call in this run; `None` when only a saved session knows it.
    pub last_active_ms: Option<i64>,
}

/// How long ago an agent was last seen, rounded down to the unit it is said in.
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

/// "Your agents", from the core's list (most recently active first): one
/// entry per known agent, keeping its most recent activity, and one per name
/// for the agents DBForge has no setup for.
pub fn your_agents(seen: &[AgentSeen]) -> Vec<Seen> {
    let mut agents: Vec<Seen> = Vec::new();
    for agent in seen {
        let known = agent_of(&agent.name);
        let pick = match known {
            AgentId::Other => Pick { agent: known, name: Some(agent.name.clone()) },
            _ => Pick::known(known),
        };
        match agents.iter_mut().find(|listed| listed.pick == pick) {
            Some(listed) => listed.last_active_ms = listed.last_active_ms.max(agent.last_active_ms),
            None => agents.push(Seen { pick, last_active_ms: agent.last_active_ms }),
        }
    }
    agents
}

/// "Add one": the known agents not among `yours`, in the order DBForge offers
/// them. "Other agent" always stays: the agents it stands for are listed by
/// their own names, so it sets up any agent DBForge doesn't know.
pub fn to_add(yours: &[Seen]) -> Vec<AgentId> {
    AGENTS.into_iter().filter(|&agent| !yours.iter().any(|seen| seen.pick == Pick::known(agent))).collect()
}

/// The agent shown: `picked` while it is still listed, else the first of
/// `yours`, else Claude Code.
pub fn shown_pick(picked: Option<&Pick>, yours: &[Seen], add: &[AgentId]) -> Pick {
    let listed =
        |pick: &Pick| yours.iter().any(|seen| &seen.pick == pick) || (pick.name.is_none() && add.contains(&pick.agent));
    match picked {
        Some(pick) if listed(pick) => pick.clone(),
        _ => yours.first().map_or_else(|| Pick::known(AgentId::Claude), |seen| seen.pick.clone()),
    }
}

/// Whether an agent last active at `last_active_ms` still counts as connected at `now_ms`.
pub fn is_connected(last_active_ms: Option<i64>, now_ms: i64) -> bool {
    last_active_ms.is_some_and(|at| now_ms - at < CONNECTED_WITHIN_MS)
}

/// How many of `yours` count as connected at `now_ms`.
pub fn connected(yours: &[Seen], now_ms: i64) -> usize {
    yours.iter().filter(|seen| is_connected(seen.last_active_ms, now_ms)).count()
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

    fn named(name: &str, last_active_ms: Option<i64>) -> Seen {
        Seen { pick: Pick { agent: AgentId::Other, name: Some(name.into()) }, last_active_ms }
    }

    fn known(agent: AgentId, last_active_ms: Option<i64>) -> Seen {
        Seen { pick: Pick::known(agent), last_active_ms }
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
    fn lists_each_known_agent_once_and_unknown_ones_by_name() {
        let listed = your_agents(&[
            seen("claude-code", Some(9_000)),
            seen("aider", Some(5_000)),
            seen("claude-desktop", Some(1_000)),
            seen("codex", None),
            seen("agent", None),
        ]);
        assert_eq!(
            listed,
            [
                known(AgentId::Claude, Some(9_000)),
                named("aider", Some(5_000)),
                known(AgentId::Codex, None),
                named("agent", None),
            ]
        );
        // The most recent activity wins, whichever comes first.
        assert_eq!(
            your_agents(&[seen("claude-desktop", None), seen("claude-code", Some(3))]),
            [known(AgentId::Claude, Some(3))]
        );
    }

    #[test]
    fn offers_the_known_agents_not_yet_seen_and_always_other() {
        assert_eq!(to_add(&[]), AGENTS);
        assert_eq!(
            to_add(&[known(AgentId::Codex, None), named("aider", None), known(AgentId::Claude, Some(1))]),
            [AgentId::Opencode, AgentId::Pi, AgentId::Other]
        );
    }

    #[test]
    fn shows_the_picked_agent_while_listed_else_the_first_seen_else_claude_code() {
        let yours = [named("aider", Some(5)), known(AgentId::Codex, None)];
        let add = to_add(&yours);
        assert_eq!(shown_pick(None, &[], &to_add(&[])), Pick::known(AgentId::Claude));
        assert_eq!(shown_pick(None, &yours, &add), yours[0].pick);
        let pi = Pick::known(AgentId::Pi);
        assert_eq!(shown_pick(Some(&pi), &yours, &add), pi);
        assert_eq!(shown_pick(Some(&Pick::known(AgentId::Codex)), &yours, &add), Pick::known(AgentId::Codex));
        assert_eq!(shown_pick(Some(&Pick::known(AgentId::Other)), &yours, &add), Pick::known(AgentId::Other));
        let gone = Pick { agent: AgentId::Other, name: Some("gone".into()) };
        assert_eq!(shown_pick(Some(&gone), &yours, &add), yours[0].pick, "no longer listed");
    }

    #[test]
    fn counts_an_agent_connected_for_ten_minutes_after_its_last_call() {
        let now = 100 * 60_000;
        assert!(is_connected(Some(now), now));
        assert!(is_connected(Some(now - CONNECTED_WITHIN_MS + 1), now));
        assert!(!is_connected(Some(now - CONNECTED_WITHIN_MS), now));
        assert!(!is_connected(None, now));
        let yours = [known(AgentId::Claude, Some(now - 1_000)), named("aider", Some(0)), known(AgentId::Pi, None)];
        assert_eq!(connected(&yours, now), 1);
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
