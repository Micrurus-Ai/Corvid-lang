//! Stream value family — `StreamValue` (the user-facing handle),
//! `StreamSender` (the driver-side push handle), and the typed
//! per-chunk record `StreamChunk` carrying value + cost +
//! confidence + token count for budget accounting and replay.
//!
//! The split between `Bounded` and `Unbounded` mpsc receivers /
//! senders backs the language's two `BackpressurePolicy`
//! variants — `block` (bounded; producer awaits when full) and
//! `drop_oldest` / `drop_newest` (unbounded; producer always
//! makes progress). The `StreamSenderKind` enum captures the
//! two-channel-flavor split so `StreamSender::send_chunk` can
//! pick the right `mpsc::Sender` method.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use corvid_ast::BackpressurePolicy;
use corvid_runtime::ProvenanceChain;

use super::{
    value_confidence, InterpError, ResumeTokenValue, StreamResumeContext, Value,
    UNBOUNDED_STREAM_WARN_THRESHOLD,
};

pub struct StreamValue(Arc<StreamInner>);

impl Drop for StreamInner {
    fn drop(&mut self) {
        if let Some(task) = self.producer.get() {
            task.abort();
        }
    }
}

struct StreamInner {
    producer: std::sync::OnceLock<tokio::task::AbortHandle>,
    receiver: AsyncMutex<StreamReceiver>,
    backpressure: BackpressurePolicy,
    provenance: Mutex<ProvenanceChain>,
    resume: Mutex<Option<ResumeHistory>>,
    pending: Arc<AtomicUsize>,
}

struct ResumeHistory {
    context: StreamResumeContext,
    delivered: Vec<StreamChunk>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StreamChunk {
    pub value: Value,
    pub cost: f64,
    pub confidence: f64,
    pub tokens: u64,
}

enum StreamReceiver {
    Bounded(mpsc::Receiver<Result<StreamChunk, InterpError>>),
    Unbounded(mpsc::UnboundedReceiver<Result<StreamChunk, InterpError>>),
}

pub(crate) struct StreamSender {
    pending: Arc<AtomicUsize>,
    warned_unbounded: AtomicBool,
    kind: StreamSenderKind,
}

enum StreamSenderKind {
    Bounded(mpsc::Sender<Result<StreamChunk, InterpError>>),
    Unbounded(mpsc::UnboundedSender<Result<StreamChunk, InterpError>>),
}

impl StreamValue {
    pub(crate) fn channel(backpressure: BackpressurePolicy) -> (StreamSender, Self) {
        let bounded = |capacity| {
            let (tx, rx) = mpsc::channel(capacity);
            (StreamSenderKind::Bounded(tx), StreamReceiver::Bounded(rx))
        };
        let (kind, receiver) = match &backpressure {
            BackpressurePolicy::Bounded(size) => bounded(*size as usize),
            BackpressurePolicy::PullsFrom(_) => bounded(1),
            BackpressurePolicy::Unbounded => {
                let (tx, rx) = mpsc::unbounded_channel();
                (
                    StreamSenderKind::Unbounded(tx),
                    StreamReceiver::Unbounded(rx),
                )
            }
        };
        let pending = Arc::new(AtomicUsize::new(0));
        let stream = Self(Arc::new(StreamInner {
            producer: Default::default(),
            receiver: AsyncMutex::new(receiver),
            backpressure,
            provenance: Mutex::new(ProvenanceChain::new()),
            resume: Mutex::new(None),
            pending: pending.clone(),
        }));
        let sender = StreamSender {
            kind,
            pending,
            warned_unbounded: AtomicBool::new(false),
        };
        (sender, stream)
    }

    pub(crate) fn spawn_producer(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        let task = tokio::spawn(future);
        self.0.producer.set(task.abort_handle()).expect("one producer per stream");
    }

    pub async fn next(&self) -> Option<Result<Value, InterpError>> {
        self.next_chunk()
            .await
            .map(|item| item.map(|chunk| chunk.value))
    }

    pub(crate) async fn next_chunk(&self) -> Option<Result<StreamChunk, InterpError>> {
        let mut receiver = self.0.receiver.lock().await;
        let item = match &mut *receiver {
            StreamReceiver::Bounded(rx) => rx.recv().await,
            StreamReceiver::Unbounded(rx) => rx.recv().await,
        };
        if item.is_some() {
            self.0.pending.fetch_sub(1, Ordering::AcqRel);
        }
        if let Some(Ok(chunk)) = &item {
            if let Some(resume) = self
                .0
                .resume
                .lock()
                .expect("stream resume state poisoned")
                .as_mut()
            {
                resume.delivered.push(chunk.clone());
            }
            if let Some(chain) = chunk.provenance() {
                self.0
                    .provenance
                    .lock()
                    .expect("stream provenance poisoned")
                    .merge(chain);
            }
        }
        item
    }

    pub fn provenance(&self) -> ProvenanceChain {
        self.0
            .provenance
            .lock()
            .expect("stream provenance poisoned")
            .clone()
    }

    pub fn backpressure(&self) -> &BackpressurePolicy {
        &self.0.backpressure
    }

    pub(crate) fn set_resume_context(&self, context: StreamResumeContext) {
        *self.0.resume.lock().expect("stream resume state poisoned") = Some(ResumeHistory {
            context,
            delivered: Vec::new(),
        });
    }

    pub(crate) fn resume_token(&self) -> Option<ResumeTokenValue> {
        let history = self.0.resume.lock().expect("stream resume state poisoned");
        let resume = history.as_ref()?;
        Some(ResumeTokenValue {
            prompt_name: resume.context.prompt_name.clone(),
            args: resume.context.args.clone(),
            delivered: resume.delivered.clone(),
            provider_session: resume.context.provider_session.clone(),
        })
    }

    pub(super) fn backpressure_label(&self) -> String {
        self.backpressure().label()
    }
}

impl Clone for StreamValue {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl PartialEq for StreamValue {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl StreamSender {
    pub async fn send(&self, item: Result<Value, InterpError>) -> bool {
        self.send_chunk(item.map(StreamChunk::new)).await
    }

    pub(crate) async fn send_chunk(&self, item: Result<StreamChunk, InterpError>) -> bool {
        match &self.kind {
            StreamSenderKind::Bounded(sender) => {
                let Ok(permit) = sender.reserve().await else {
                    return false;
                };
                self.pending.fetch_add(1, Ordering::AcqRel);
                permit.send(item);
            }
            StreamSenderKind::Unbounded(sender) => {
                let pending_after_send = self.pending.fetch_add(1, Ordering::AcqRel) + 1;
                if sender.send(item).is_err() {
                    self.pending.fetch_sub(1, Ordering::AcqRel);
                    return false;
                }
                if pending_after_send > UNBOUNDED_STREAM_WARN_THRESHOLD
                    && !self.warned_unbounded.swap(true, Ordering::AcqRel)
                {
                    eprintln!(
                        "warning: unbounded stream buffer exceeded {} queued items",
                        UNBOUNDED_STREAM_WARN_THRESHOLD
                    );
                }
            }
        }
        true
    }
}

impl StreamChunk {
    pub fn new(value: Value) -> Self {
        Self {
            confidence: value_confidence(&value),
            value,
            cost: 0.0,
            tokens: 0,
        }
    }

    pub fn with_metrics(value: Value, cost: f64, confidence: f64, tokens: u64) -> Self {
        Self {
            value,
            cost,
            confidence,
            tokens,
        }
    }

    pub fn provenance(&self) -> Option<&ProvenanceChain> {
        match &self.value {
            Value::Grounded(grounded) => Some(&grounded.provenance),
            _ => None,
        }
    }
}

#[cfg(test)]
mod consumer_lifetime_tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn dropping_last_consumer_wakes_a_blocked_producer() {
        let (sender, stream) = StreamValue::channel(BackpressurePolicy::Bounded(1));
        let other_consumer = stream.clone();
        drop(stream);
        assert!(sender.send(Ok(Value::Int(1))).await);
        let second = sender.send(Ok(Value::Int(2)));
        tokio::pin!(second);
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut second)
            .await
            .is_err());
        drop(other_consumer);
        assert!(!tokio::time::timeout(Duration::from_secs(1), second)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn unbounded_producer_observes_consumer_drop() {
        let (sender, stream) = StreamValue::channel(BackpressurePolicy::Unbounded);
        drop(stream);
        assert!(!sender.send(Ok(Value::Int(1))).await);
    }
}
