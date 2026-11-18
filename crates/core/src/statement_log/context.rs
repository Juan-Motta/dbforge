use std::fmt::Display;
use std::future::Future;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::{Entry, Origin, StatementLog};

/// What a command is doing, for every statement run while it does it.
#[derive(Clone)]
struct Scope {
    log: StatementLog,
    connection_id: String,
    origin: Origin,
}

tokio::task_local! {
    static SCOPE: Scope;
    /// The text to record in place of the statement's own, e.g. with its
    /// parameters written as literals.
    static SHOWN: String;
}

/// Runs `work` with its statements recorded in `log`, as sent on
/// `connection_id` for `origin`.
pub async fn scoped<F: Future>(log: &StatementLog, connection_id: &str, origin: Origin, work: F) -> F::Output {
    let scope = Scope { log: log.clone(), connection_id: connection_id.to_owned(), origin };
    SCOPE.scope(scope, work).await
}

/// Runs `work` with its statements recorded as `text`: for a statement whose
/// parameters are bound, the same statement with them written as literals.
pub async fn shown_as<F: Future>(text: impl Into<String>, work: F) -> F::Output {
    SHOWN.scope(text.into(), work).await
}

/// A statement being sent: started when created, recorded when dropped, so
/// one abandoned halfway (a stream read only to its first row, a cancelled
/// future) is recorded too. Does nothing outside [`scoped`].
pub struct Recording {
    scope: Option<Scope>,
    database: Option<String>,
    text: String,
    at: u64,
    started: Instant,
    returned: u64,
    affected: Option<u64>,
    error: Option<String>,
}

impl Recording {
    /// Starts recording a statement sent to `database`; `text` is only built
    /// when something records it.
    pub fn start(database: Option<&str>, text: impl FnOnce() -> String) -> Self {
        let scope = SCOPE.try_with(Scope::clone).ok();
        let text = match &scope {
            Some(_) => SHOWN.try_with(String::clone).unwrap_or_else(|_| text()),
            None => String::new(),
        };
        Self {
            scope,
            database: database.filter(|name| !name.is_empty()).map(str::to_owned),
            text,
            at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
            started: Instant::now(),
            returned: 0,
            affected: None,
            error: None,
        }
    }

    /// One more row came back.
    pub fn row(&mut self) {
        self.returned += 1;
    }

    /// Rows came back, `count` of them.
    pub fn rows(&mut self, count: u64) {
        self.returned += count;
    }

    /// The server says the statement changed `count` rows.
    pub fn affected(&mut self, count: u64) {
        *self.affected.get_or_insert(0) += count;
    }

    pub fn fail(&mut self, error: impl Display) {
        self.error = Some(error.to_string());
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        let Some(scope) = self.scope.take() else { return };
        let rows = if self.returned > 0 { Some(self.returned) } else { self.affected };
        scope.log.push(Entry {
            id: 0,
            at: self.at,
            connection_id: scope.connection_id,
            database: self.database.take(),
            origin: scope.origin,
            text: std::mem::take(&mut self.text),
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            rows: if self.error.is_some() { None } else { rows },
            error: self.error.take(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statement_log::store::tests::Collected;

    fn log() -> (StatementLog, Collected) {
        let sink = Collected::default();
        (StatementLog::new(10, sink.clone()), sink)
    }

    #[tokio::test]
    async fn records_inside_a_scope_with_its_connection_and_origin() {
        let (log, _) = log();
        scoped(&log, "pg", Origin::Browse, async {
            let mut ok = Recording::start(Some("shop"), || "SELECT 1".into());
            ok.rows(2);
            drop(ok);
            let mut failed = Recording::start(None, || "SELEC".into());
            failed.row();
            failed.fail("syntax error");
        })
        .await;

        let entries = log.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            (entries[0].connection_id.as_str(), entries[0].database.as_deref(), entries[0].origin),
            ("pg", Some("shop"), Origin::Browse)
        );
        assert_eq!(
            (entries[0].text.as_str(), entries[0].rows, entries[0].error.as_deref()),
            ("SELECT 1", Some(2), None)
        );
        assert_eq!((entries[1].rows, entries[1].error.as_deref()), (None, Some("syntax error")));
    }

    #[tokio::test]
    async fn outside_a_scope_nothing_is_recorded_and_the_text_is_never_built() {
        let (log, sink) = log();
        let recording = Recording::start(None, || panic!("text built outside a scope"));
        drop(recording);
        assert!(log.entries().is_empty() && sink.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn shown_as_replaces_the_text_and_rows_fall_back_to_affected() {
        let (log, _) = log();
        scoped(&log, "pg", Origin::Edit, async {
            shown_as("UPDATE t SET a = 'x'", async {
                let mut recording = Recording::start(None, || "UPDATE t SET a = $1".into());
                recording.affected(3);
            })
            .await;
            // A statement abandoned before any result still counts, without rows.
            drop(Recording::start(None, || "SELECT pg_sleep(9)".into()));
        })
        .await;

        let entries = log.entries();
        assert_eq!((entries[0].text.as_str(), entries[0].rows), ("UPDATE t SET a = 'x'", Some(3)));
        assert_eq!((entries[1].text.as_str(), entries[1].rows), ("SELECT pg_sleep(9)", None));
    }
}
