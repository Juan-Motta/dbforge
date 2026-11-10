use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::Entry;

/// Where each new entry goes as soon as it is recorded (the app's window).
pub trait Sink: Send + Sync {
    fn logged(&self, entry: &Entry);
}

/// The last statements run, newest last, up to a capacity; older ones are dropped.
/// Cheap to clone: clones share the same entries.
#[derive(Clone)]
pub struct StatementLog(Arc<Inner>);

struct Inner {
    entries: Mutex<VecDeque<Entry>>,
    next_id: AtomicU64,
    capacity: usize,
    sink: Box<dyn Sink>,
}

impl StatementLog {
    pub fn new(capacity: usize, sink: impl Sink + 'static) -> Self {
        Self(Arc::new(Inner {
            entries: Mutex::new(VecDeque::with_capacity(capacity)),
            next_id: AtomicU64::new(1),
            capacity,
            sink: Box::new(sink),
        }))
    }

    /// Keeps `entry` under the next id, and hands it to the sink.
    pub(super) fn push(&self, mut entry: Entry) {
        entry.id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut entries = self.0.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if entries.len() == self.0.capacity {
                entries.pop_front();
            }
            entries.push_back(entry.clone());
        }
        self.0.sink.logged(&entry);
    }

    /// The entries kept, oldest first.
    pub fn entries(&self) -> Vec<Entry> {
        self.0.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().cloned().collect()
    }

    pub fn clear(&self) {
        self.0.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::statement_log::Origin;

    /// A sink that keeps what it was handed, for tests.
    #[derive(Clone, Default)]
    pub struct Collected(pub Arc<Mutex<Vec<Entry>>>);

    impl Sink for Collected {
        fn logged(&self, entry: &Entry) {
            self.0.lock().unwrap().push(entry.clone());
        }
    }

    pub fn entry(text: &str) -> Entry {
        Entry {
            id: 0,
            at: 0,
            connection_id: "c".into(),
            database: None,
            origin: Origin::Console,
            text: text.into(),
            elapsed_ms: 0,
            rows: None,
            error: None,
        }
    }

    #[test]
    fn keeps_the_newest_up_to_its_capacity_and_hands_each_to_the_sink() {
        let sink = Collected::default();
        let log = StatementLog::new(2, sink.clone());
        for text in ["one", "two", "three"] {
            log.push(entry(text));
        }

        let kept: Vec<_> = log.entries().into_iter().map(|e| (e.id, e.text)).collect();
        assert_eq!(kept, [(2, "two".to_owned()), (3, "three".to_owned())]);
        assert_eq!(sink.0.lock().unwrap().len(), 3);

        log.clear();
        assert!(log.entries().is_empty());
        log.push(entry("four"));
        assert_eq!(log.entries()[0].id, 4, "ids keep increasing after a clear");
    }
}
