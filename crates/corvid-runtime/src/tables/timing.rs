//! Where a table operation spends its time in the database (lineage from
//! real runs, L2): waiting for a connection or a turn to write, and
//! running its statements.
//!
//! An operation may run several statements, each borrowing a connection
//! of its own, so its timing adds up across them. The recorded-operation
//! wrapper measures an operation inside [`measured`]. Each statement adds
//! to that operation's total through [`record`] without a parameter
//! threaded through every call. Outside a measured operation (a `std.db`
//! query, an `atomic:` block's own `BEGIN`), [`record`] does nothing.

use std::cell::Cell;
use std::future::Future;
use std::time::Duration;

/// One operation's time in the database.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timing {
    pub wait: Duration,
    pub statement: Duration,
    pub statements: u32,
}

tokio::task_local! {
    static CURRENT: Cell<Timing>;
}

/// Run `operation`, and return what its statements recorded.
pub(crate) async fn measured<F: Future>(operation: F) -> (F::Output, Timing) {
    CURRENT
        .scope(Cell::new(Timing::default()), async move {
            let output = operation.await;
            (output, CURRENT.with(Cell::get))
        })
        .await
}

/// Add one statement's wait and running time to the operation being
/// measured, if any.
pub(crate) fn record(wait: Duration, statement: Duration) {
    let _ = CURRENT.try_with(|current| {
        let mut timing = current.get();
        timing.wait += wait;
        timing.statement += statement;
        timing.statements += 1;
        current.set(timing);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn statements_add_up_within_one_operation_and_nowhere_else() {
        record(Duration::from_millis(9), Duration::from_millis(9));
        let ((), timing) = measured(async {
            record(Duration::from_millis(2), Duration::from_millis(1));
            tokio::task::yield_now().await;
            record(Duration::from_millis(3), Duration::from_millis(4));
        })
        .await;
        assert_eq!(timing, Timing { wait: Duration::from_millis(5), statement: Duration::from_millis(5), statements: 2 });
        let ((), fresh) = measured(async {}).await;
        assert_eq!(fresh, Timing::default(), "each operation starts from zero");
    }
}
