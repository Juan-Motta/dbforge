//! Which agents use the MCP server: each tool call records the agent's own
//! name and version (its MCP `clientInfo`) with the time, while the app runs.
//! The app is told ([`Event::McpAgentsChanged`]) when an agent first acts in
//! this run, and again at most every [`ANNOUNCE_EVERY_MS`] while it keeps acting.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::events::{Event, Events};

/// How long a busy agent's activity goes unannounced: a burst of calls tells the app once.
pub const ANNOUNCE_EVERY_MS: i64 = 30_000;

/// An agent that has used DBForge, as the agents list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSeen {
    /// The agent's own name for itself (its MCP `clientInfo.name`, e.g. `claude-code`).
    pub name: String,
    pub version: Option<String>,
    /// Milliseconds since the epoch of its last tool call in this run of the
    /// app; `None` when it is only known from a saved session.
    pub last_active_ms: Option<i64>,
}

/// What is kept about one agent while the app runs.
struct Activity {
    version: Option<String>,
    last_active_ms: i64,
    /// When the app was last told about this agent.
    announced_ms: i64,
}

/// The agents that called a tool in this run, by name. Shared by every session.
#[derive(Default)]
pub struct AgentRegistry {
    agents: Mutex<HashMap<String, Activity>>,
}

impl AgentRegistry {
    /// Records a tool call by agent `name` now, telling `events` when the app should hear of it.
    pub fn record(&self, events: &dyn Events, name: &str, version: Option<&str>) {
        self.record_at(events, name, version, now_ms());
    }

    /// Records a tool call by agent `name` at `at_ms`. An agent without a name is not recorded.
    pub(crate) fn record_at(&self, events: &dyn Events, name: &str, version: Option<&str>, at_ms: i64) {
        if name.is_empty() {
            return;
        }
        let announce = {
            let mut agents = self.lock();
            let version = version.filter(|version| !version.is_empty()).map(str::to_owned);
            match agents.get_mut(name) {
                Some(activity) => {
                    activity.version = version;
                    activity.last_active_ms = at_ms;
                    let due = at_ms - activity.announced_ms >= ANNOUNCE_EVERY_MS;
                    if due {
                        activity.announced_ms = at_ms;
                    }
                    due
                }
                None => {
                    agents.insert(name.to_owned(), Activity { version, last_active_ms: at_ms, announced_ms: at_ms });
                    true
                }
            }
        };
        // Emitted outside the lock: the sink may take its time.
        if announce {
            events.emit(Event::McpAgentsChanged);
        }
    }

    /// The agents that acted in this run, in no particular order.
    pub fn seen(&self) -> Vec<AgentSeen> {
        self.lock()
            .iter()
            .map(|(name, activity)| AgentSeen {
                name: name.clone(),
                version: activity.version.clone(),
                last_active_ms: Some(activity.last_active_ms),
            })
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Activity>> {
        self.agents.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The agents that acted in this run (`active`) and the ones known from saved
/// sessions (`saved`, as name and version, oldest first): one per name, the
/// newest version winning, the most recently active first, then the rest by name.
pub(crate) fn merge(active: Vec<AgentSeen>, saved: Vec<(String, Option<String>)>) -> Vec<AgentSeen> {
    let mut agents: HashMap<String, AgentSeen> = HashMap::new();
    for (name, version) in saved.into_iter().filter(|(name, _)| !name.is_empty()) {
        agents.insert(name.clone(), AgentSeen { name, version, last_active_ms: None });
    }
    for agent in active {
        agents.insert(agent.name.clone(), agent);
    }
    let mut agents: Vec<AgentSeen> = agents.into_values().collect();
    agents.sort_by(|a, b| match (a.last_active_ms, b.last_active_ms) {
        (Some(a), Some(b)) => b.cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.cmp(&b.name),
    });
    agents
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use super::*;

    /// The events emitted, for a test to look at.
    #[derive(Default)]
    pub(crate) struct Heard(pub Mutex<Vec<Event>>);

    impl Events for Heard {
        fn emit(&self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }

    impl Heard {
        fn agents_changed(&self) -> usize {
            self.0.lock().unwrap().iter().filter(|event| matches!(event, Event::McpAgentsChanged)).count()
        }
    }

    fn agent(name: &str, version: Option<&str>, last_active_ms: Option<i64>) -> AgentSeen {
        AgentSeen { name: name.into(), version: version.map(Into::into), last_active_ms }
    }

    #[test]
    fn records_each_agent_once_with_its_latest_version_and_time() {
        let registry = AgentRegistry::default();
        registry.record_at(&Heard::default(), "claude-code", Some("1.0"), 1_000);
        registry.record_at(&Heard::default(), "claude-code", Some("1.1"), 2_000);
        registry.record_at(&Heard::default(), "codex", None, 1_500);

        let mut seen = registry.seen();
        seen.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(seen, [agent("claude-code", Some("1.1"), Some(2_000)), agent("codex", None, Some(1_500))]);
    }

    #[test]
    fn an_agent_without_a_name_is_not_recorded() {
        let registry = AgentRegistry::default();
        let heard = Heard::default();
        registry.record_at(&heard, "", Some("1"), 1_000);
        assert!(registry.seen().is_empty());
        assert_eq!(heard.agents_changed(), 0);
    }

    #[test]
    fn record_uses_the_current_time() {
        let registry = AgentRegistry::default();
        let before = now_ms();
        registry.record(&Heard::default(), "pi", Some("2"));
        let last = registry.seen()[0].last_active_ms.unwrap();
        assert!(last >= before && last <= now_ms());
    }

    #[test]
    fn the_app_hears_of_an_agent_s_first_call_then_at_most_every_30_seconds() {
        let registry = AgentRegistry::default();
        let heard = Heard::default();
        registry.record_at(&heard, "claude-code", None, 0);
        assert_eq!(heard.agents_changed(), 1, "first call");
        registry.record_at(&heard, "claude-code", None, 1_000);
        registry.record_at(&heard, "claude-code", None, ANNOUNCE_EVERY_MS - 1);
        assert_eq!(heard.agents_changed(), 1, "a burst is announced once");
        registry.record_at(&heard, "claude-code", None, ANNOUNCE_EVERY_MS);
        assert_eq!(heard.agents_changed(), 2, "30 s after the last announcement");
        registry.record_at(&heard, "claude-code", None, ANNOUNCE_EVERY_MS + 1_000);
        assert_eq!(heard.agents_changed(), 2);
        registry.record_at(&heard, "codex", None, ANNOUNCE_EVERY_MS + 1_000);
        assert_eq!(heard.agents_changed(), 3, "another agent's first call");
    }

    #[test]
    fn merges_saved_sessions_with_this_run_s_activity() {
        let active = vec![agent("codex", Some("0.9"), Some(1_000)), agent("claude-code", Some("2.0"), Some(5_000))];
        let saved = vec![
            ("opencode".into(), Some("1".into())),
            ("claude-code".into(), Some("1.0".into())),
            ("aider".into(), None),
            ("opencode".into(), Some("2".into())),
            ("".into(), Some("1".into())),
        ];
        assert_eq!(
            merge(active, saved),
            [
                agent("claude-code", Some("2.0"), Some(5_000)),
                agent("codex", Some("0.9"), Some(1_000)),
                agent("aider", None, None),
                agent("opencode", Some("2"), None),
            ]
        );
    }
}
