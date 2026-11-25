//! A bounded pool of Postgres connections for one database (B09).
//!
//! The `database` block declares how many connections the database may be
//! asked for (`connections: 10`), because a managed Postgres caps them and
//! going over is an outage. Every table operation borrows a connection for
//! the one statement it runs and returns it; an `atomic:` block borrows one
//! for the whole block. When every connection is in use, the next caller
//! waits its turn instead of opening another, so load queues in the
//! program rather than failing at the database.

use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::postgres::PostgresTables;
use crate::errors::RuntimeError;

/// The connections to one Postgres database.
pub struct PostgresPool {
    url: String,
    tls: bool,
    idle: Mutex<Vec<PostgresTables>>,
    turns: Arc<Semaphore>,
}

impl PostgresPool {
    /// A pool that opens up to `size` connections, as they are needed.
    pub fn new(url: String, tls: bool, size: u32) -> Self {
        Self { url, tls, idle: Mutex::new(Vec::new()), turns: Arc::new(Semaphore::new(size.max(1) as usize)) }
    }

    /// A connection of the caller's own until the returned value drops:
    /// an idle one that is still open, or a new one. Waits while every
    /// connection is in use.
    pub async fn borrow(self: &Arc<Self>) -> Result<BorrowedPostgres, RuntimeError> {
        let turn = self
            .turns
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RuntimeError::Other("the Postgres connection pool was closed".into()))?;
        let reused = {
            let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
            // A connection the server closed while idle is dropped here.
            loop {
                match idle.pop() {
                    Some(connection) if connection.is_closed() => continue,
                    other => break other,
                }
            }
        };
        let connection = match reused {
            Some(connection) => connection,
            None => PostgresTables::connect(&self.url, self.tls).await?,
        };
        Ok(BorrowedPostgres { connection: Some(connection), pool: self.clone(), reusable: true, _turn: turn })
    }

    /// How many connections are open and idle, for tests.
    #[cfg(test)]
    fn idle_count(&self) -> usize {
        self.idle.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// A connection borrowed from a [`PostgresPool`]. Dropping it returns the
/// connection, unless it closed or a transaction on it did not end
/// cleanly, and gives the turn to the next caller.
pub struct BorrowedPostgres {
    connection: Option<PostgresTables>,
    pool: Arc<PostgresPool>,
    reusable: bool,
    _turn: OwnedSemaphorePermit,
}

impl BorrowedPostgres {
    pub fn connection(&self) -> &PostgresTables {
        self.connection.as_ref().expect("a borrowed connection is present until it is returned")
    }

    /// Never hand this connection to another caller: a transaction on it
    /// may still be open.
    pub fn discard(&mut self) {
        self.reusable = false;
    }
}

impl Drop for BorrowedPostgres {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            if self.reusable && !connection.is_closed() {
                self.pool.idle.lock().unwrap_or_else(|e| e.into_inner()).push(connection);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_url() -> Option<String> {
        std::env::var("CORVID_TEST_POSTGRES_URL").ok()
    }

    #[tokio::test]
    async fn borrowers_wait_for_a_turn_and_connections_are_reused() {
        let Some(url) = test_url() else {
            eprintln!("skipping: set CORVID_TEST_POSTGRES_URL to test the Postgres pool");
            return;
        };
        let pool = Arc::new(PostgresPool::new(url, false, 2));
        let first = pool.borrow().await.expect("first connection");
        let second = pool.borrow().await.expect("second connection");
        // Both connections are in use, so a third caller waits.
        let waiting = tokio::time::timeout(Duration::from_millis(300), pool.borrow()).await;
        assert!(waiting.is_err(), "a third borrow must wait while two are out");
        drop(first);
        let third = tokio::time::timeout(Duration::from_secs(5), pool.borrow())
            .await
            .expect("a returned connection frees a turn")
            .expect("third connection");
        assert_eq!(third.connection().execute("SELECT 1", &[]).await.expect("runs"), 1);
        drop(second);
        drop(third);
        assert_eq!(pool.idle_count(), 2, "both connections are kept for the next callers");
    }

    #[tokio::test]
    async fn a_discarded_connection_is_never_handed_out_again() {
        let Some(url) = test_url() else {
            eprintln!("skipping: set CORVID_TEST_POSTGRES_URL to test the Postgres pool");
            return;
        };
        let pool = Arc::new(PostgresPool::new(url, false, 1));
        let mut borrowed = pool.borrow().await.expect("connection");
        borrowed.connection().execute("BEGIN", &[]).await.expect("begin");
        borrowed.discard();
        drop(borrowed);
        assert_eq!(pool.idle_count(), 0, "a connection with an open transaction is dropped");
        let fresh = pool.borrow().await.expect("a new connection");
        // A fresh connection has no transaction open, so BEGIN succeeds.
        fresh.connection().execute("BEGIN", &[]).await.expect("begin on a fresh connection");
    }
}
