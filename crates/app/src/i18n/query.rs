//! The texts of the query console, its results and the query history.

/// The query console's and its results' texts in one language.
pub struct QueryMessages {
    /// A query tab's title.
    pub query_tab: fn(usize) -> String,
    pub close_tab: fn(&str) -> String,
    pub default_database: &'static str,
    pub database_to_query: &'static str,
    pub could_not_cancel: fn(&str) -> String,
    /// The History button's tooltip, with its shortcut.
    pub history_title: fn(&str) -> String,
    pub history: &'static str,
    pub cancel: &'static str,
    pub cancelling: &'static str,
    pub run: &'static str,
    pub running: &'static str,
    /// The Analyze checkbox's tooltip.
    pub analyze_title: &'static str,
    pub analyze: &'static str,
    pub explain: &'static str,
    pub explaining: &'static str,
    /// With nothing run yet, given the shortcut that runs the query.
    pub placeholder: fn(&str) -> String,
    pub rows_affected: fn(u64) -> String,
    pub rows: fn(u64) -> String,
    pub rows_showing_first: fn(u64, usize) -> String,
    pub no_statements: &'static str,
    pub result: fn(usize) -> String,
    pub stopped_tab: &'static str,
    pub stopped_after: fn(usize) -> String,
    pub statement_executed: &'static str,
    pub history_panel: HistoryMessages,
    pub plan: PlanMessages,
}

/// An explained statement's plan: its totals, nodes and warnings.
pub struct PlanMessages {
    pub execution: &'static str,
    pub planning: &'static str,
    pub rows_returned: &'static str,
    pub estimate_only: &'static str,
    pub turn_on_analyze: &'static str,
    pub warnings: &'static str,
    pub none: &'static str,
    pub plan_nodes: &'static str,
    pub node: &'static str,
    pub rows_column: &'static str,
    pub estimated_rows: &'static str,
    pub self_time: &'static str,
    pub self_cost: &'static str,
    pub share: &'static str,
    pub has_warnings: &'static str,
    pub node_details: &'static str,
    pub using_index: fn(&str) -> String,
    pub stale_statistics: fn(&str, &str) -> String,
    pub discarded_rows: fn(&str) -> String,
}

/// The history panel's texts in one language.
pub struct HistoryMessages {
    pub title: &'static str,
    pub close: &'static str,
    pub search_placeholder: &'static str,
    pub errors: &'static str,
    pub this_connection: &'static str,
    pub all: &'static str,
    pub no_runs_match: &'static str,
    pub empty_history: &'static str,
    /// Given how a click that opens a new tab is written (`⌘-click`).
    pub hint: fn(&str) -> String,
    pub today: &'static str,
    pub yesterday: &'static str,
    /// An older day: its weekday (0 is Monday), month (1 is January) and day.
    pub day: fn(u32, u32, u32) -> String,
    pub outcome_error: &'static str,
    pub statements: fn(usize) -> String,
    pub rows: fn(u64) -> String,
}

/// `n` with a comma between thousands, as the web UI wrote counts in both languages.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_thousands_with_commas() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(18_204), "18,204");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }
}
