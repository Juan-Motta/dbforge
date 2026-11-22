//! A SQL console's transactions in Manual mode: the rules that decide what
//! it shows and refuses, and the dialog asking before a tab closes, or the
//! app quits, with a transaction open.

pub mod leave;
pub mod rules;
#[cfg(test)]
mod tests;

use std::time::{SystemTime, UNIX_EPOCH};

pub use leave::{LeaveDialog, LeaveEvent, Leaving, Pending, key_bindings};

/// Now, in milliseconds since the Unix epoch: the clock the core stamps transactions with.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}
