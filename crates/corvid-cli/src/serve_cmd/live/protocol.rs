//! JSON payloads of the corvid.live.v1 WebSocket subprotocol.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub(super) enum Request {
    Subscribe { id: String, path: String },
    Unsubscribe { id: String },
}

impl Request {
    pub(super) fn parse(text: &str) -> Option<Self> {
        let request = serde_json::from_str(text).ok()?;
        let id = match &request {
            Self::Subscribe { id, path } => {
                if !path.starts_with('/')
                    || path.starts_with("//")
                    || path.contains(['?', '#', '\\'])
                    || path.parse::<axum::http::uri::PathAndQuery>().is_err()
                {
                    return None;
                }
                id
            }
            Self::Unsubscribe { id } => id,
        };
        (!id.is_empty()).then_some(request)
    }
}

#[derive(Serialize)]
pub(in crate::serve_cmd) struct Row {
    #[serde(serialize_with = "decimal_revision")]
    pub revision: u64,
    /// Row already encoded with the contract's public JSON mapping.
    pub row: Value,
}

fn decimal_revision<S: serde::Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(in crate::serve_cmd) enum Change {
    Added {
        index: usize,
        #[serde(flatten)]
        row: Row,
    },
    Updated {
        index: usize,
        #[serde(flatten)]
        row: Row,
    },
    Removed {
        #[serde(flatten)]
        row: Row,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(in crate::serve_cmd) enum Delivery {
    Snapshot {
        rows: Vec<Row>,
    },
    Change {
        #[serde(flatten)]
        change: Change,
    },
}

#[derive(Serialize)]
pub(super) struct Envelope<'a> {
    pub id: &'a str,
    #[serde(flatten)]
    pub delivery: Delivery,
}

pub(super) fn forbidden(id: &str) -> String {
    serde_json::json!({"type": "error", "id": id, "code": "forbidden"}).to_string()
}
