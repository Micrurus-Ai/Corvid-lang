//! A route that returns `Stream<T>` answers with Server-Sent Events in
//! the one stream protocol both tiers read
//! (`docs/proposals/stream-failures.md`, part 2):
//!
//! | event       | data                                   | when                |
//! | ----------- | -------------------------------------- | ------------------- |
//! | `chunk`     | the item's public JSON                 | each item           |
//! | `completed` | `null`                                 | the stream ended    |
//! | `failed`    | `{"error": code, "detail": sentence}`  | a terminal failure  |
//!
//! A `Result` item travels inside `chunk` (`{"tag":"err","err":...}`), so
//! a typed `Err` the program handles is a value in the same loop as `Ok`,
//! never a transport failure. `failed` is reserved for a stop the program
//! did not turn into a value, and its data is the same public answer an
//! ordinary route's error body gives ([`super::public_failure`]). A body
//! that ends with neither terminal event is a lost connection, which the
//! client reports as an interrupted stream.

use axum::response::sse::Event;
use axum::response::{IntoResponse, Response, Sse};
use corvid_types::Type;
use corvid_vm::StreamValue;

use super::public_failure::{log_failure, public_failure, unencodable_item, PublicFailure};
use super::ReplyEncoding;
use corvid_driver::RunError;

/// The SSE event name for one item.
pub(super) const CHUNK: &str = "chunk";
/// The SSE event name for the normal end of a route stream.
pub(super) const COMPLETED: &str = "completed";
/// The SSE event name for a terminal failure.
pub(super) const FAILED: &str = "failed";

/// Answer a `Stream<T>` route: flush each item the moment it arrives.
pub(super) fn route_stream_response(
    stream: StreamValue,
    stream_ty: &Type,
    reply: ReplyEncoding,
    route: String,
) -> Response {
    let item_ty = match stream_ty {
        Type::Stream(item) => (**item).clone(),
        other => other.clone(),
    };
    let events = async_stream::stream! {
        loop {
            match stream.next().await {
                Some(Ok(item)) => match reply.encode(&item, &item_ty) {
                    Ok(payload) => {
                        yield Ok::<_, std::convert::Infallible>(
                            Event::default().event(CHUNK).data(payload.to_string()),
                        );
                    }
                    Err(reason) => {
                        let failure = unencodable_item();
                        log_failure(&route, &failure, &reason);
                        yield Ok(failed(&failure));
                        break;
                    }
                },
                Some(Err(err)) => {
                    let full = err.to_string();
                    let failure = public_failure(&RunError::Interp(err));
                    log_failure(&route, &failure, &full);
                    yield Ok(failed(&failure));
                    break;
                }
                None => {
                    yield Ok(Event::default().event(COMPLETED).data("null"));
                    break;
                }
            }
        }
    };
    Sse::new(events).into_response()
}

fn failed(failure: &PublicFailure) -> Event {
    Event::default().event(FAILED).data(failure.body().to_string())
}
