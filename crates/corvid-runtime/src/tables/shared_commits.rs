//! Commits shared between server processes (B14 RT-6).
//!
//! Several processes serving one application on one Postgres database
//! each run their own commit feed. A write committed through one must
//! reach live subscribers connected to another, so every committed write
//! on Postgres also sends `pg_notify('corvid_commits', payload)`, and
//! [`Runtime::share_table_commits`] listens on that channel and forwards
//! other processes' commits into this process's feed.
//!
//! A notification sent inside a transaction is delivered only when it
//! commits, so an `atomic:` block sends its notification inside its
//! transaction and Postgres itself guarantees that a rolled-back block
//! announces nothing. A process tags its notifications with its own
//! instance id and skips them on receipt, since it announced them
//! locally already. When the listening connection drops, the listener
//! reconnects and announces a commit with no tables, which means "any
//! table in this database may have changed": subscribers take a fresh
//! snapshot instead of missing what was committed while it was away.

use super::{TableCommit, TableRef};
use crate::db::DbValue;
use crate::errors::RuntimeError;
use crate::Runtime;
use corvid_ast::DatabaseLocation;
use futures::StreamExt;
use serde_json::{json, Value as J};
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

/// The Postgres channel commits are shared on.
pub(crate) const COMMIT_CHANNEL: &str = "corvid_commits";

/// The `application_name` a listening connection gives itself.
pub const LISTENER_NAME: &str = "corvid commit listener";

/// A fresh tag for one runtime: 128 random bits as hex.
pub(crate) fn new_instance_id() -> std::sync::Arc<str> {
    use rand_core::RngCore;
    let mut bytes = [0u8; 16];
    rand_core::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>().into()
}

/// The statement that shares a commit, and its parameters. The payload
/// is `{"origin", "schema", "database", "tables"}`: which process
/// committed, the Postgres schema its tables resolve to, the Corvid
/// database's name and the tables written. A channel is shared by every
/// schema of a Postgres database, so the server itself stamps
/// `current_schema()` and a listener keeps only its own schema's
/// commits: two applications in two schemas never see each other's.
pub(crate) fn notify_statement(instance: &str, database: &str, tables: &BTreeSet<String>) -> (&'static str, [DbValue; 4]) {
    (
        "SELECT pg_notify($1::text, json_build_object('origin', $2::text, 'schema', current_schema(), \
         'database', $3::text, 'tables', $4::text::json)::text)",
        [
            DbValue::Text(COMMIT_CHANNEL.into()),
            DbValue::Text(instance.to_string()),
            DbValue::Text(database.to_string()),
            DbValue::Text(json!(tables).to_string()),
        ],
    )
}

/// The tables of a commit another process made in `schema` on
/// `database`, or `None` for this process's own notification, another
/// schema's or database's, or a payload that is not a commit.
fn received_commit(instance: &str, schema: &str, database: &str, payload: &str) -> Option<BTreeSet<String>> {
    let parsed: J = serde_json::from_str(payload).ok()?;
    let field = |name: &str| parsed.get(name).and_then(J::as_str);
    if field("origin")? == instance || field("schema")? != schema || field("database")? != database {
        return None;
    }
    parsed.get("tables")?.as_array()?.iter().map(|t| t.as_str().map(str::to_string)).collect()
}

impl Runtime {
    /// Tell the other processes on a Postgres database about a write
    /// outside an `atomic:` block that succeeded; a block tells them
    /// inside its transaction instead. Runs only live, never in a replay.
    /// If the notification cannot be sent the write has still committed,
    /// and the failure is loud rather than a subscriber silently missing
    /// it.
    pub(super) async fn notify_other_processes(&self, table: &TableRef<'_>) -> Result<(), RuntimeError> {
        if !table.engine.is_postgres() || table.transaction.is_some() {
            return Ok(());
        }
        let tables = std::iter::once(table.name.to_string()).collect();
        let (sql, params) = notify_statement(&self.instance_id, table.database, &tables);
        self.table_execute(table, sql, &params).await.map(|_| ()).map_err(|e| {
            RuntimeError::Other(format!(
                "the write to `{}` committed, but other server processes could not be told about it: {e}",
                table.name
            ))
        })
    }

    /// Forward commits other processes make on this Postgres `database`
    /// into this process's commit feed, from now on. Returns once the
    /// listening connection is open; it reconnects by itself after a
    /// drop and then announces that any table may have changed.
    pub async fn share_table_commits(
        &self,
        database: &str,
        location: &DatabaseLocation,
        tls: bool,
    ) -> Result<(), RuntimeError> {
        let url = super::resolve_location(location)?;
        let feed = self.table_commits.clone();
        let instance = self.instance_id.clone();
        let database = database.to_string();
        let first = listen(&url, tls).await?;
        tokio::spawn(async move {
            let mut listening = Some(first);
            let mut delay = Duration::from_millis(500);
            loop {
                let session = match listening.take() {
                    Some(session) => session,
                    None => match listen(&url, tls).await {
                        Ok(session) => {
                            delay = Duration::from_millis(500);
                            // Commits made while away were missed.
                            let _ = feed.send(TableCommit { database: database.clone(), tables: BTreeSet::new() });
                            session
                        }
                        Err(_) => {
                            tokio::time::sleep(delay).await;
                            delay = (delay * 2).min(Duration::from_secs(30));
                            continue;
                        }
                    },
                };
                let Listening { client, schema, mut notifications } = session;
                while let Some(payload) = notifications.recv().await {
                    if let Some(tables) = received_commit(&instance, &schema, &database, &payload) {
                        // A commit with no tables would mean "anything may
                        // have changed", which only the reconnect says.
                        if !tables.is_empty() {
                            let _ = feed.send(TableCommit { database: database.clone(), tables });
                        }
                    }
                }
                drop(client);
            }
        });
        Ok(())
    }
}

/// An open `LISTEN` connection: its client, kept alive, the schema its
/// tables resolve to, and the payloads it receives.
struct Listening {
    client: tokio_postgres::Client,
    schema: String,
    notifications: tokio::sync::mpsc::UnboundedReceiver<String>,
}

async fn listen(url: &str, tls: bool) -> Result<Listening, RuntimeError> {
    let failed = |e: tokio_postgres::Error| RuntimeError::Other(format!("cannot listen for shared table commits: {e}"));
    let (sender, notifications) = tokio::sync::mpsc::unbounded_channel();
    let client = if tls {
        let (client, connection) = tokio_postgres::connect(url, super::VerifiedTls::new()?).await.map_err(failed)?;
        tokio::spawn(forward(connection, sender));
        client
    } else {
        let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls).await.map_err(failed)?;
        tokio::spawn(forward(connection, sender));
        client
    };
    // Named, so an operator can tell it apart in `pg_stat_activity`.
    client
        .batch_execute(&format!("SET application_name = '{LISTENER_NAME}'; LISTEN {COMMIT_CHANNEL}"))
        .await
        .map_err(failed)?;
    // The same URL the tables are reached through, so the same schema.
    let schema: Option<String> = client.query_one("SELECT current_schema()", &[]).await.map_err(failed)?.get(0);
    let schema = schema.ok_or_else(|| {
        RuntimeError::Other("cannot listen for shared table commits: the connection's `search_path` names no existing schema".into())
    })?;
    Ok(Listening { client, schema, notifications })
}

/// Drive a connection, passing each commit notification on; ends when
/// the connection does, which closes the channel and tells the listener
/// to reconnect.
async fn forward<S, T>(mut connection: tokio_postgres::Connection<S, T>, sender: tokio::sync::mpsc::UnboundedSender<String>)
where
    S: AsyncRead + AsyncWrite + Unpin,
    T: AsyncRead + AsyncWrite + Unpin,
{
    let mut messages = futures::stream::poll_fn(move |cx| connection.poll_message(cx));
    while let Some(message) = messages.next().await {
        match message {
            Ok(tokio_postgres::AsyncMessage::Notification(note)) if note.channel() == COMMIT_CHANNEL => {
                if sender.send(note.payload().to_string()).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_keeps_only_other_processes_commits_in_its_own_schema_and_database() {
        let payload = r#"{"origin":"one","schema":"app","database":"board","tables":["Card","Note"]}"#;
        let tables: BTreeSet<String> = ["Card", "Note"].iter().map(|t| t.to_string()).collect();
        assert_eq!(received_commit("two", "app", "board", payload), Some(tables));
        assert_eq!(received_commit("one", "app", "board", payload), None, "a process announced its own commit already");
        assert_eq!(received_commit("two", "other_app", "board", payload), None, "another schema is another application");
        assert_eq!(received_commit("two", "app", "archive", payload), None, "another database");
        assert_eq!(received_commit("two", "app", "board", "not json"), None);
    }

    #[test]
    fn instance_ids_differ() {
        assert_ne!(new_instance_id(), new_instance_id());
        assert_eq!(new_instance_id().len(), 32);
    }
}
