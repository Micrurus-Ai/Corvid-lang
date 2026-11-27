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

    /// The pool under concurrent agents (B19): how many edits a pool size
    /// buys, and how long an agent waits at the pool for a turn. Each agent
    /// edits like the runtime's `get` then a revision-checked `update`: two
    /// statements, each borrowing its own connection. Agents edit disjoint
    /// rows, so the numbers are the pool's and the database's, not conflicts.
    /// A measurement, not a check, so it is ignored by default; run it with
    /// `CORVID_TEST_POSTGRES_URL` set:
    /// `cargo test --release -p corvid-runtime --lib pool_under_concurrent_agents -- --ignored --nocapture`.
    /// `CORVID_POOL_LOAD_SECONDS` sets each cell's length (default 3).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "a load measurement; run on demand with --ignored"]
    async fn pool_under_concurrent_agents() {
        use crate::db::DbValue;
        use std::time::Instant;
        const ROWS: i64 = 1024;

        let Some(url) = test_url() else {
            eprintln!("skipping: set CORVID_TEST_POSTGRES_URL to measure the Postgres pool");
            return;
        };
        let seconds: u64 = std::env::var("CORVID_POOL_LOAD_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
        let setup = PostgresTables::connect(&url, false).await.expect("connect");
        for sql in [
            "DROP TABLE IF EXISTS corvid_pool_load",
            "CREATE TABLE corvid_pool_load (id BIGINT PRIMARY KEY, revision BIGINT NOT NULL)",
            "INSERT INTO corvid_pool_load SELECT n, 1 FROM generate_series(0, 1023) AS n",
        ] {
            setup.execute(sql, &[]).await.expect(sql);
        }

        fn percentile(sorted: &[Duration], p: f64) -> f64 {
            if sorted.is_empty() {
                return 0.0;
            }
            let at = ((sorted.len() as f64 - 1.0) * p).round() as usize;
            sorted[at].as_secs_f64() * 1000.0
        }

        println!("| Agents | Connections | Edits/s | Wait at the pool p50 / p95 / p99, ms | Edit p50 / p95, ms |");
        println!("| ---: | ---: | ---: | ---: | ---: |");
        for agents in [8usize, 64] {
            for size in [1u32, 2, 4, 8, 16, 32] {
                if size as usize > agents {
                    continue;
                }
                let pool = Arc::new(PostgresPool::new(url.clone(), false, size));
                // Open every connection first, so the cells measure turns,
                // not connecting.
                let mut warm = Vec::new();
                for _ in 0..size {
                    // Each held borrow forces the next to open a connection.
                    warm.push(pool.borrow().await.expect("open a connection"));
                }
                drop(warm);
                let deadline = Instant::now() + Duration::from_secs(seconds);
                let started = Instant::now();
                let tasks = (0..agents)
                    .map(|agent| {
                        let pool = pool.clone();
                        tokio::spawn(async move {
                            let columns = vec!["revision".to_string()];
                            let (mut waits, mut edits) = (Vec::new(), Vec::new());
                            let mut n = 0i64;
                            while Instant::now() < deadline {
                                let id = agent as i64 + agents as i64 * (n % (ROWS / agents as i64));
                                n += 1;
                                let edit = Instant::now();
                                let borrowed = pool.borrow().await.expect("borrow to read");
                                waits.push(edit.elapsed());
                                let rows = borrowed
                                    .connection()
                                    .select("SELECT revision FROM corvid_pool_load WHERE id = $1", &[DbValue::Integer(id)], &columns)
                                    .await
                                    .expect("read");
                                drop(borrowed);
                                let DbValue::Integer(revision) = rows[0][0] else { panic!("revision is an integer") };
                                let asked = Instant::now();
                                let borrowed = pool.borrow().await.expect("borrow to write");
                                waits.push(asked.elapsed());
                                let changed = borrowed
                                    .connection()
                                    .execute(
                                        "UPDATE corvid_pool_load SET revision = revision + 1 WHERE id = $1 AND revision = $2",
                                        &[DbValue::Integer(id), DbValue::Integer(revision)],
                                    )
                                    .await
                                    .expect("write");
                                drop(borrowed);
                                assert_eq!(changed, 1, "agents edit disjoint rows, so no edit conflicts");
                                edits.push(edit.elapsed());
                            }
                            (waits, edits)
                        })
                    })
                    .collect::<Vec<_>>();
                let (mut waits, mut edits) = (Vec::new(), Vec::new());
                for task in tasks {
                    let (w, e) = task.await.expect("agent");
                    waits.extend(w);
                    edits.extend(e);
                }
                let elapsed = started.elapsed().as_secs_f64();
                waits.sort();
                edits.sort();
                println!(
                    "| {agents} | {size} | {:.0} | {:.2} / {:.2} / {:.2} | {:.2} / {:.2} |",
                    edits.len() as f64 / elapsed,
                    percentile(&waits, 0.50),
                    percentile(&waits, 0.95),
                    percentile(&waits, 0.99),
                    percentile(&edits, 0.50),
                    percentile(&edits, 0.95),
                );
            }
        }
        setup.execute("DROP TABLE corvid_pool_load", &[]).await.expect("drop");
    }
}
