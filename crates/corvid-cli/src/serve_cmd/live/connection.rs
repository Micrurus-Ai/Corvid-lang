//! One page connection owns all its subscriptions. No forwarding tasks survive it.
use super::protocol::{forbidden, Delivery, Envelope, Request};
use super::{AuthorizedSource, Failure, Session, SessionStatus};
use axum::extract::ws::{CloseFrame, Message};
use futures::{stream, stream::BoxStream, Sink, SinkExt, Stream, StreamExt};
use std::{collections::BTreeMap, time::Duration};
use tokio::time::{sleep_until, timeout, Instant};
use tokio_stream::StreamMap;

const IDLE: Duration = Duration::from_secs(30);

enum Outgoing {
    Rows(String),
    Control(Message),
}

#[derive(Default)]
struct Page {
    paths: BTreeMap<String, String>,
    streams: StreamMap<String, BoxStream<'static, Result<Delivery, Failure>>>,
}

impl Page {
    fn remove(&mut self, id: &str) {
        self.paths.remove(id);
        self.streams.remove(id);
    }

    fn request(
        &mut self,
        source: &mut dyn AuthorizedSource,
        text: &str,
    ) -> Result<Option<Outgoing>, u16> {
        match Request::parse(text).ok_or(1002u16)? {
            Request::Subscribe { id, path } if !self.paths.contains_key(&id) => {
                self.paths.insert(id.clone(), path);
                self.open(source, &id).map(Some)
            }
            Request::Unsubscribe { id } if self.paths.contains_key(&id) => {
                self.remove(&id);
                Ok(None)
            }
            _ => Err(1002),
        }
    }

    fn open(&mut self, source: &mut dyn AuthorizedSource, id: &str) -> Result<Outgoing, u16> {
        match source.open(&self.paths[id]) {
            Ok(subscription) => {
                let end = stream::once(async { Err(Failure::Unavailable) });
                self.streams
                    .insert(id.to_owned(), subscription.changes.chain(end).boxed());
                self.reply(
                    id,
                    Ok(Delivery::Snapshot {
                        rows: subscription.rows,
                    }),
                )
            }
            Err(failure) => self.reply(id, Err(failure)),
        }
    }

    fn reply(&mut self, id: &str, result: Result<Delivery, Failure>) -> Result<Outgoing, u16> {
        match result {
            Ok(delivery) => serde_json::to_string(&Envelope { id, delivery })
                .map(Outgoing::Rows)
                .map_err(|_| 1011),
            Err(Failure::Forbidden) => {
                self.remove(id);
                Ok(Outgoing::Control(Message::Text(forbidden(id))))
            }
            Err(Failure::InvalidSession) => Err(1008),
            Err(Failure::Unavailable) => Err(1011),
        }
    }
}

struct Heartbeat {
    deadline: Instant,
    waiting_for_pong: bool,
}

impl Heartbeat {
    fn activity(&mut self, now: Instant) {
        if !self.waiting_for_pong {
            self.deadline = now + IDLE;
        }
    }

    fn expired(&mut self, now: Instant) -> Result<(), u16> {
        if self.waiting_for_pong {
            return Err(1002);
        }
        self.waiting_for_pong = true;
        self.deadline = now + IDLE;
        Ok(())
    }
}

pub(super) async fn run<S, E>(
    mut socket: S,
    session: Session,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) where
    S: Sink<Message> + Stream<Item = Result<Message, E>> + Unpin,
{
    let outcome = tokio::select! {
        _ = shutdown.wait_for(|stop| *stop) => Err(1000),
        outcome = process(&mut socket, session) => outcome,
    };
    let code = match outcome {
        Ok(()) => return,
        Err(code) => code,
    };
    let frame = CloseFrame {
        code,
        reason: "".into(),
    };
    let _ = timeout(IDLE, socket.send(Message::Close(Some(frame)))).await;
}

async fn process<S, E>(socket: &mut S, mut session: Session) -> Result<(), u16>
where
    S: Sink<Message> + Stream<Item = Result<Message, E>> + Unpin,
{
    let mut page = Page::default();
    let mut refresh = Vec::new();
    let mut heartbeat = Heartbeat {
        deadline: Instant::now() + IDLE,
        waiting_for_pong: false,
    };
    loop {
        let (status, changed) = {
            let status = session.status.borrow_and_update();
            (*status, status.has_changed())
        };
        if changed {
            // Release every old filter and its pending data before rechecking any path.
            page.streams.clear();
            refresh = page.paths.keys().rev().cloned().collect();
        }
        match status {
            SessionStatus::Invalid => return Err(1008),
            SessionStatus::Active => {}
        }
        let outgoing = if Instant::now() >= heartbeat.deadline {
            heartbeat.expired(Instant::now())?;
            Some(Outgoing::Control(Message::Ping(Vec::new())))
        } else if let Some(id) = refresh.pop() {
            Some(page.open(session.source.as_mut(), &id)?)
        } else {
            tokio::select! {
                result = session.status.changed() => {
                    result.map_err(|_| 1008u16)?;
                    session.status.mark_changed();
                    continue;
                }
                _ = sleep_until(heartbeat.deadline) => continue,
                message = socket.next() => {
                    heartbeat.activity(Instant::now());
                    match message {
                        Some(Ok(Message::Text(text))) =>
                            page.request(session.source.as_mut(), &text)?,
                        Some(Ok(Message::Pong(payload))) => {
                            if payload.is_empty() {
                                heartbeat.waiting_for_pong = false;
                                heartbeat.activity(Instant::now());
                            }
                            None
                        }
                        // Tungstenite queues the protocol Pong and peer close replies.
                        Some(Ok(Message::Ping(_))) => None,
                        Some(Ok(Message::Close(_))) => {
                            let _ = timeout(IDLE, socket.flush()).await;
                            return Ok(());
                        }
                        None => return Ok(()),
                        Some(Err(_)) | Some(Ok(Message::Binary(_))) => return Err(1002),
                    }
                }
                Some((id, result)) = page.streams.next(), if !page.streams.is_empty() => {
                    if session.status.has_changed().map_err(|_| 1008u16)? { continue; }
                    Some(page.reply(&id, result)?)
                }
            }
        };
        if let Some(outgoing) = outgoing {
            let writable = futures::future::poll_fn(|cx| {
                Sink::poll_ready(std::pin::Pin::new(&mut *socket), cx)
            });
            tokio::time::timeout_at(heartbeat.deadline, writable)
                .await
                .map_err(|_| 1002u16)?
                .map_err(|_| 1011u16)?;
            {
                // Synchronize the final enqueue with the authority publisher.
                // Never hold a watch read lock across a socket wait.
                session.status.has_changed().map_err(|_| 1008u16)?;
                let status = session.status.borrow();
                if matches!(*status, SessionStatus::Invalid) {
                    return Err(1008);
                }
                let message = match outgoing {
                    Outgoing::Rows(_) if status.has_changed() => continue,
                    Outgoing::Rows(text) => Message::Text(text),
                    Outgoing::Control(message) => message,
                };
                Sink::start_send(std::pin::Pin::new(&mut *socket), message).map_err(|_| 1011u16)?;
            }
            tokio::time::timeout_at(heartbeat.deadline, socket.flush())
                .await
                .map_err(|_| 1002u16)?
                .map_err(|_| 1011u16)?;
            heartbeat.activity(Instant::now());
        }
    }
}
