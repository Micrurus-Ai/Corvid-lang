//! One evaluation per distinct live query (live-at-scale L1).
//!
//! Subscriptions whose query returns the same rows share one evaluation:
//! the same declaration, the same path argument and, only when the query
//! reads it, the same caller. Each commit re-runs a shared query once and
//! its frames go to every subscriber. Authority is still checked for each
//! subscriber before it joins; sharing an evaluation never shares a
//! permission.
use super::protocol::Delivery;
use super::{Failure, Subscription};
use corvid_runtime::tables::LiveRow;
use futures::stream::{BoxStream, StreamExt};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::broadcast::{self, error::RecvError, Receiver, Sender};

/// What makes two subscriptions see the same rows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct QueryKey {
    pub handler: String,
    /// The path argument in its public JSON form.
    pub path: String,
    /// The caller in its public JSON form, present only when the query
    /// reads `actor`.
    pub actor: Option<String>,
}

/// One evaluation's result: the rows the query now returns and the frames
/// that carry a subscriber from the previous rows to them.
pub(super) type Step = Result<(Vec<LiveRow>, Vec<Delivery>), Failure>;

type Frames = Arc<Result<Vec<Delivery>, Failure>>;

/// Frames queued per shared query before a slow subscriber is resynced
/// from the current snapshot instead.
const BACKLOG: usize = 64;

struct Shared {
    sender: Sender<Frames>,
    /// The rows of the last evaluation; `None` until the first finishes.
    latest: Option<Arc<Vec<LiveRow>>>,
    evaluations: u64,
    task: tokio::task::AbortHandle,
}

/// The running shared evaluations of one server.
#[derive(Default)]
pub(in crate::serve_cmd) struct SharedQueries {
    running: Mutex<HashMap<QueryKey, Shared>>,
    reads_actor: Mutex<HashMap<String, bool>>,
    /// Rows the evaluations have read from the database, all told.
    pub(super) rows_read: Arc<std::sync::atomic::AtomicU64>,
}

impl SharedQueries {
    fn running(&self) -> MutexGuard<'_, HashMap<QueryKey, Shared>> {
        self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether the handler reads its caller, computed once per handler.
    pub(super) fn reads_actor(&self, handler: &str, compute: impl FnOnce() -> bool) -> bool {
        let mut known = self
            .reads_actor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *known.entry(handler.to_string()).or_insert_with(compute)
    }

    /// Join the shared evaluation for `key`, starting it from `evaluate`
    /// when none is running. `evaluate` yields the first snapshot and then
    /// one step per commit.
    pub(super) fn join(
        self: &Arc<Self>,
        key: QueryKey,
        evaluate: impl FnOnce() -> BoxStream<'static, Step>,
    ) -> Subscription {
        let (receiver, latest) = {
            let mut running = self.running();
            let shared = running.entry(key.clone()).or_insert_with(|| {
                let (sender, _) = broadcast::channel(BACKLOG);
                let task = tokio::spawn(publish(self.clone(), key.clone(), evaluate()));
                Shared {
                    sender,
                    latest: None,
                    evaluations: 0,
                    task: task.abort_handle(),
                }
            });
            (shared.sender.subscribe(), shared.latest.clone())
        };
        let mut member = Member {
            receiver: Some(receiver),
            key,
            queries: self.clone(),
        };
        async_stream::try_stream! {
            if let Some(rows) = latest {
                yield super::feed::snapshot(&rows)?;
            }
            loop {
                let frames = match member.receiver().recv().await {
                    Ok(frames) => frames,
                    Err(RecvError::Lagged(_)) => {
                        // Too far behind for changes: start again from the
                        // current rows, as a lagged commit receiver does.
                        let rows = member.resync().ok_or(Failure::Unavailable)?;
                        if let Some(rows) = rows {
                            yield super::feed::snapshot(&rows)?;
                        }
                        continue;
                    }
                    Err(RecvError::Closed) => Err(Failure::Unavailable)?,
                };
                match frames.as_ref() {
                    Ok(frames) => for frame in frames {
                        yield frame.clone();
                    },
                    Err(failure) => Err(*failure)?,
                }
            }
        }
        .boxed()
    }

    /// Record one evaluation and send its frames. Returns whether the
    /// evaluation should continue.
    fn record(&self, key: &QueryKey, step: Step) -> bool {
        let mut running = self.running();
        let Some(shared) = running.get_mut(key) else {
            return false;
        };
        shared.evaluations += 1;
        match step {
            Ok((rows, frames)) => {
                shared.latest = Some(Arc::new(rows));
                if !frames.is_empty() {
                    // No receiver is not an error: the last one may be
                    // leaving, and its guard ends this evaluation.
                    let _ = shared.sender.send(Arc::new(Ok(frames)));
                }
                true
            }
            Err(failure) => {
                let _ = shared.sender.send(Arc::new(Err(failure)));
                running.remove(key);
                false
            }
        }
    }

    #[cfg(test)]
    pub(super) fn shared_count(&self) -> usize {
        self.running().len()
    }

    #[cfg(test)]
    pub(super) fn rows_read(&self) -> u64 {
        self.rows_read.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(super) fn evaluations(&self) -> u64 {
        self.running().values().map(|s| s.evaluations).sum()
    }
}

async fn publish(
    queries: Arc<SharedQueries>,
    key: QueryKey,
    mut steps: BoxStream<'static, Step>,
) {
    while let Some(step) = steps.next().await {
        if !queries.record(&key, step) {
            return;
        }
    }
    queries.record(&key, Err(Failure::Unavailable));
}

/// One subscriber's place in a shared evaluation. The last one to leave
/// stops the evaluation.
struct Member {
    receiver: Option<Receiver<Frames>>,
    key: QueryKey,
    queries: Arc<SharedQueries>,
}

impl Member {
    fn receiver(&mut self) -> &mut Receiver<Frames> {
        self.receiver.as_mut().expect("a member holds its receiver until dropped")
    }

    /// A fresh receiver and the current rows, taken together so no frame
    /// falls between them. `None` when the evaluation has ended.
    fn resync(&mut self) -> Option<Option<Arc<Vec<LiveRow>>>> {
        let running = self.queries.running();
        let shared = running.get(&self.key)?;
        self.receiver = Some(shared.sender.subscribe());
        Some(shared.latest.clone())
    }
}

impl Drop for Member {
    fn drop(&mut self) {
        let mut running = self.queries.running();
        self.receiver = None;
        if running
            .get(&self.key)
            .is_some_and(|shared| shared.sender.receiver_count() == 0)
        {
            if let Some(shared) = running.remove(&self.key) {
                shared.task.abort();
            }
        }
    }
}
