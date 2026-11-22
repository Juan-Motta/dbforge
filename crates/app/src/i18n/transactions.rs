//! The texts of a console's transaction mode, its open-transaction capsule,
//! and the dialog asking before leaving one open.

/// The transaction texts in one language.
pub struct TransactionMessages {
    /// The mode's two segments and their tooltips.
    pub auto_commit: &'static str,
    pub manual: &'static str,
    pub auto_tooltip: &'static str,
    pub manual_tooltip: &'static str,
    /// Auto-commit while a transaction is open.
    pub auto_locked: &'static str,
    /// Given the tag that makes a console start in Manual.
    pub manual_by_default: fn(&str) -> String,
    /// The capsule's title while a transaction is open.
    pub open: &'static str,
    /// Its title after a statement failed (PostgreSQL), and what still works.
    pub failed: &'static str,
    pub only_rollback: &'static str,
    /// Its title once the transaction has been idle `minutes`.
    pub idle: fn(u64) -> String,
    pub changes: fn(u32) -> String,
    pub rows: fn(u64) -> String,
    /// The tooltip of the time it has been open.
    pub open_for: &'static str,
    pub commit: &'static str,
    pub rollback: &'static str,
    pub committing: &'static str,
    pub rolling_back: &'static str,
    /// The Commit button's tooltip, with its shortcut.
    pub commit_tooltip: fn(&str) -> String,
    pub rollback_tooltip: &'static str,
    /// The notices after Commit and Rollback: changes and rows.
    pub committed: fn(u32, u64) -> String,
    pub rolled_back: fn(u32, u64) -> String,
    /// The notice of a transaction idle for `minutes`, on a connection.
    pub idle_notice: fn(&str, u64) -> String,
    /// A script that starts or ends a transaction itself, in Manual: the statement's keyword.
    pub control_refused: fn(&str) -> String,
    /// Another database or container picked while a transaction is open.
    pub change_refused: &'static str,
    /// Editing or deleting a connection one of whose consoles has a transaction open.
    pub connection_busy: fn(&str) -> String,
    /// The tooltip of a tab's dot while its console has a transaction open.
    pub tab_dot: &'static str,
    pub dialog: LeaveMessages,
}

/// The dialog asking before closing a tab, or quitting, with a transaction open.
pub struct LeaveMessages {
    /// Closing a tab: its title.
    pub close_title: fn(&str) -> String,
    /// Quitting: how many transactions are open.
    pub quit_title: fn(usize) -> String,
    pub not_committed: &'static str,
    /// How long one has been open.
    pub open_for: fn(&str) -> String,
    pub explanation: &'static str,
    /// Why Commit is off: a statement failed in one of them.
    pub only_rollback: &'static str,
    pub cancel: &'static str,
    pub rollback_and_close: &'static str,
    pub commit_and_close: &'static str,
    /// Quitting, with how many are open.
    pub rollback_and_quit: fn(usize) -> String,
    pub commit_and_quit: fn(usize) -> String,
    pub rolling_back: &'static str,
    pub committing: &'static str,
    /// The keys under the list: the default button's label.
    pub keys: fn(&str) -> String,
}
