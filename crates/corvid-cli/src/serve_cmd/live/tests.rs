use super::protocol::{Change, Envelope, Request};
use super::*;
use axum::extract::ws::Message;
use futures::{channel::mpsc, StreamExt};
use serde_json::{json, Value};
use tokio::time::{advance, Duration};

#[test]
fn rejects_malformed_messages_and_action_calls() {
    for text in [
        r#"[]"#,
        r#"{"type":"action","id":"x"}"#,
        r#"{"type":"subscribe","id":"","path":"/cards"}"#,
        r#"{"type":"subscribe","id":"x","path":"//other/cards"}"#,
        r#"{"type":"subscribe","id":"x","path":"https://other/cards"}"#,
        r#"{"type":"unsubscribe","id":"x","id":"y"}"#,
        r#"{"type":"subscribe","id":"x","path":"/cards","tenant":"b"}"#,
    ] {
        assert!(Request::parse(text).is_none(), "{text}");
    }
}

#[test]
fn requires_the_exact_browser_origin() {
    let mut headers = HeaderMap::new();
    headers.insert("host", "localhost:9000".parse().unwrap());
    for (origin, base, accepted) in [
        ("http://localhost:9000", None, true),
        ("https://localhost:9000", None, false),
        ("http://localhost:9001", None, false),
        (
            "https://public.example",
            Some("https://public.example:443"),
            true,
        ),
        (
            "https://public.example.other",
            Some("https://public.example"),
            false,
        ),
        ("null", None, false),
    ] {
        headers.insert("origin", origin.parse().unwrap());
        assert_eq!(same_origin(&headers, base), accepted);
    }
    headers.append("origin", "http://localhost:9000".parse().unwrap());
    assert!(!same_origin(&headers, None));
}

fn row() -> Row {
    Row {
        revision: u64::MAX,
        row: json!({"card":4}),
    }
}

#[test]
fn ordered_changes_preserve_revision_precision_and_removals_omit_index() {
    for (change, kind, index) in [
        (
            Change::Added {
                index: 1,
                row: row(),
            },
            "added",
            Some(1),
        ),
        (
            Change::Updated {
                index: 0,
                row: row(),
            },
            "updated",
            Some(0),
        ),
        (Change::Removed { row: row() }, "removed", None),
    ] {
        let frame = serde_json::to_value(Envelope {
            id: "cards",
            delivery: Delivery::Change { change },
        })
        .unwrap();
        let mut expected = json!({"type":"change","id":"cards","kind":kind,
            "row":{"card":4},"revision":"18446744073709551615"});
        if let Some(index) = index {
            expected["index"] = json!(index);
        }
        assert_eq!(frame, expected);
    }
}

type Updates = mpsc::UnboundedSender<Result<Delivery, Failure>>;
struct Source(mpsc::UnboundedSender<(String, Updates)>);

impl AuthorizedSource for Source {
    fn open(&mut self, path: &str) -> Result<Subscription, Failure> {
        if path == "/denied" {
            return Err(Failure::Forbidden);
        }
        if path == "/expired" {
            return Err(Failure::InvalidSession);
        }
        let (updates, changes) = mpsc::unbounded();
        self.0.unbounded_send((path.into(), updates)).unwrap();
        Ok(Subscription {
            rows: vec![row()],
            changes: changes.boxed(),
        })
    }
}

use futures::{Sink, Stream};
use std::{
    pin::Pin,
    task::{Context, Poll},
};

struct FrameSocket {
    writer: mpsc::UnboundedSender<Message>,
    reader: mpsc::UnboundedReceiver<Result<Message, ()>>,
}

impl Stream for FrameSocket {
    type Item = Result<Message, ()>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.reader).poll_next(cx)
    }
}

impl Sink<Message> for FrameSocket {
    type Error = mpsc::SendError;
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.writer).poll_ready(cx)
    }
    fn start_send(mut self: Pin<&mut Self>, msg: Message) -> Result<(), Self::Error> {
        Pin::new(&mut self.writer).start_send(msg)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.writer).poll_close(cx)
    }
}

/// Exercise the actual connection loop at its frame boundary.
struct Client {
    status: watch::Sender<SessionStatus>,
    requests: mpsc::UnboundedSender<Result<Message, ()>>,
    frames: mpsc::UnboundedReceiver<Message>,
    opened: mpsc::UnboundedReceiver<(String, Updates)>,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Client {
    fn new() -> Self {
        let (status, receiver) = watch::channel(SessionStatus::Active);
        let (requests, reader) = mpsc::unbounded();
        let (writer, frames) = mpsc::unbounded();
        let (source, opened) = mpsc::unbounded();
        let session = Session {
            status: receiver,
            source: Box::new(Source(source)),
        };
        let (shutdown, stop) = watch::channel(false);
        let task = tokio::spawn(connection::run(
            FrameSocket { writer, reader },
            session,
            stop,
        ));
        Self {
            status,
            requests,
            frames,
            opened,
            shutdown,
            task,
        }
    }
    fn request(&self, frame: Value) {
        self.requests
            .unbounded_send(Ok(Message::Text(frame.to_string())))
            .unwrap();
    }
    async fn expect_close(mut self, code: u16) {
        assert!(
            matches!(self.frames.next().await, Some(Message::Close(Some(frame))) if frame.code == code)
        );
        self.task.await.unwrap();
    }
    async fn reply(&mut self) -> Value {
        let Some(Message::Text(text)) = self.frames.next().await else {
            panic!("expected text")
        };
        serde_json::from_str(&text).unwrap()
    }
    async fn subscribe(&mut self, id: &str) -> Updates {
        self.request(json!({"type":"subscribe","id":id,"path":"/cards"}));
        assert_eq!(
            self.reply().await,
            json!({"type":"snapshot","id":id,
            "rows":[{"revision":u64::MAX.to_string(),"row":{"card":4}}]})
        );
        self.opened.next().await.unwrap().1
    }
}

#[tokio::test]
async fn multiplexes_and_discards_handles_before_refresh() {
    let mut client = Client::new();
    let first = client.subscribe("a").await;
    let second = client.subscribe("b").await;
    client.request(json!({"type":"unsubscribe","id":"a"}));
    client.request(json!({"type":"subscribe","id":"a","path":"/denied"}));
    assert_eq!(
        client.reply().await,
        json!({"type":"error","id":"a","code":"forbidden"})
    );
    assert!(first.is_closed());
    second
        .unbounded_send(Ok(Delivery::Change {
            change: Change::Added {
                index: 0,
                row: row(),
            },
        }))
        .unwrap();
    assert_eq!(client.reply().await["id"], "b");
    second
        .unbounded_send(Ok(Delivery::Snapshot { rows: vec![] }))
        .unwrap();
    client.status.send(SessionStatus::Active).unwrap();
    assert_eq!(client.reply().await["rows"][0]["row"]["card"], 4);
    assert!(second.is_closed());
    let (_, refreshed) = client.opened.next().await.unwrap();
    refreshed.unbounded_send(Err(Failure::Forbidden)).unwrap();
    assert_eq!(
        client.reply().await,
        json!({"type":"error","id":"b","code":"forbidden"})
    );
    assert!(refreshed.is_closed());
    let last = client.subscribe("b").await;
    client.status.send(SessionStatus::Invalid).unwrap();
    client.expect_close(1008).await;
    assert!(last.is_closed());
}

#[tokio::test]
async fn rejects_binary_duplicate_ids_and_invalid_sessions() {
    let mut client = Client::new();
    let handle = client.subscribe("cards").await;
    client.request(json!({"type":"subscribe","id":"cards","path":"/cards"}));
    client.expect_close(1002).await;
    assert!(handle.is_closed());
    let client = Client::new();
    client
        .requests
        .unbounded_send(Ok(Message::Binary(vec![])))
        .unwrap();
    client.expect_close(1002).await;
    let client = Client::new();
    client.request(json!({"type":"subscribe","id":"x","path":"/expired"}));
    client.expect_close(1008).await;
}

#[tokio::test(start_paused = true)]
async fn idle_ping_requires_a_matching_pong_within_thirty_seconds() {
    let mut client = Client::new();
    let _handle = client.subscribe("cards").await;
    let idle = Duration::from_secs(30);
    advance(idle).await;
    assert!(matches!(client.frames.next().await, Some(Message::Ping(_))));
    client
        .requests
        .unbounded_send(Ok(Message::Pong(vec![])))
        .unwrap();
    tokio::task::yield_now().await;
    advance(idle).await;
    assert!(matches!(client.frames.next().await, Some(Message::Ping(_))));
    advance(Duration::from_secs(29)).await;
    client
        .requests
        .unbounded_send(Ok(Message::Pong(vec![1])))
        .unwrap();
    tokio::task::yield_now().await;
    assert!(!client.task.is_finished());
    advance(Duration::from_secs(1)).await;
    client.expect_close(1002).await;
}

#[tokio::test]
async fn shutdown_closes_all_handles_and_allows_a_fresh_connection() {
    let mut first = Client::new();
    let handle = first.subscribe("cards").await;
    first.shutdown.send(true).unwrap();
    first.expect_close(1000).await;
    assert!(handle.is_closed());
    let mut next = Client::new();
    let changes = next.subscribe("cards").await;
    changes
        .unbounded_send(Ok(Delivery::Change {
            change: Change::Updated {
                index: 0,
                row: row(),
            },
        }))
        .unwrap();
    assert_eq!(next.reply().await["kind"], "updated");
}
