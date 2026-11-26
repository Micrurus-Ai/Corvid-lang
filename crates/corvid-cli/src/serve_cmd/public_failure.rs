//! `corvid serve`'s side of the public failure answers. The mapping from
//! a handler error to its code and fixed sentence lives in
//! [`corvid_driver::public_failure`], where the server `corvid build`
//! generates reads it too; this module logs the full error for the
//! operator.

pub(super) use corvid_driver::public_failure::{public_failure, unencodable_item, PublicFailure};

/// Write the full error to the server's standard error, tagged with the
/// route, so the operator sees what the public answer leaves out.
pub(super) fn log_failure(route: &str, failure: &PublicFailure, full: &str) {
    eprintln!("corvid serve: {route} failed ({}): {full}", failure.error);
}
