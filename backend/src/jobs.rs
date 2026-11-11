//! Long work the UI follows and can stop: dumps, restores and exports. Each
//! runs under an id the UI chose, which `cancel_job` stops it by.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::State;
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, Result};

/// The jobs running now, by id.
#[derive(Default)]
pub struct Jobs(Mutex<HashMap<String, CancellationToken>>);

impl Jobs {
    /// Registers job `id` until the returned `Job` is dropped.
    pub fn start(&self, id: &str) -> Job<'_> {
        let token = CancellationToken::new();
        self.0.lock().unwrap().insert(id.to_string(), token.clone());
        Job { jobs: self, id: id.to_string(), token }
    }

    /// Stops job `id`; nothing happens when it already ended.
    pub fn cancel(&self, id: &str) {
        if let Some(token) = self.0.lock().unwrap().get(id) {
            token.cancel();
        }
    }
}

/// A running job: it leaves the registry when dropped, however it ended.
pub struct Job<'a> {
    jobs: &'a Jobs,
    id: String,
    token: CancellationToken,
}

impl Job<'_> {
    /// Cancelled once the UI asks to stop the job.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }
}

impl Drop for Job<'_> {
    fn drop(&mut self) {
        self.jobs.0.lock().unwrap().remove(&self.id);
    }
}

/// The error a cancelled job ends with.
pub fn cancelled() -> AppError {
    AppError::Invalid("Cancelled.".into())
}

/// `work`, unless `cancel` comes first: then `work` is dropped, which stops a
/// query it runs even while it waits for the server.
pub async fn unless_cancelled<T>(cancel: &CancellationToken, work: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::select! {
        result = work => result,
        () = cancel.cancelled() => Err(cancelled()),
    }
}

#[tauri::command]
pub fn cancel_job(jobs: State<'_, Jobs>, job_id: String) {
    jobs.cancel(&job_id);
}

/// Says yes at most once per `period`: the first time, then once it has
/// passed again. Progress is sent this way, not once per line or row.
pub struct Every {
    period: Duration,
    last: Option<Instant>,
}

impl Every {
    pub fn new(period: Duration) -> Self {
        Self { period, last: None }
    }

    pub fn due(&mut self) -> bool {
        let now = Instant::now();
        let due = self.last.is_none_or(|last| now.duration_since(last) >= self.period);
        if due {
            self.last = Some(now);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_can_be_cancelled_while_it_runs_and_is_forgotten_after() {
        let jobs = Jobs::default();
        let job = jobs.start("a");
        jobs.cancel("a");
        assert!(job.token().is_cancelled());
        drop(job);
        assert!(jobs.0.lock().unwrap().is_empty());
        // A cancel that comes after the end is ignored.
        jobs.cancel("a");
    }

    #[tokio::test]
    async fn cancelling_stops_work_that_is_still_waiting() {
        let cancel = CancellationToken::new();
        assert_eq!(unless_cancelled(&cancel, async { Ok(1) }).await.unwrap(), 1);
        cancel.cancel();
        let waiting = std::future::pending::<Result<()>>();
        assert_eq!(unless_cancelled(&cancel, waiting).await.unwrap_err().to_string(), "Cancelled.");
    }
}
