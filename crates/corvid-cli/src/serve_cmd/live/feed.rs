//! Translate RT-2 commit notifications into frames for one authorized query.
use super::protocol::{Change, Delivery, Row};
use super::{Failure, Subscription};
use crate::serve_cmd::ReplyEncoding;
use corvid_driver::{run_ir_with_runtime, Runtime, Value};
use corvid_ir::{IrExprKind, IrFile, IrRoute, IrStmt, IrTable, IrType};
use corvid_runtime::tables::{
    live_changes, LiveChange, LiveChangeKind, LiveMessage, LiveRow, TableCommit, TableEngine,
    WrittenRows,
};
use futures::StreamExt;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::broadcast::{error::RecvError, Receiver};

fn query_table(route: &IrRoute) -> Option<&IrTable> {
    let [IrStmt::Return {
        value: Some(expr), ..
    }] = route.body.stmts.as_slice()
    else {
        return None;
    };
    match &expr.kind {
        IrExprKind::TableQuery { table, .. } => Some(table),
        _ => None,
    }
}

pub(in crate::serve_cmd) async fn share_commits(
    ir: &IrFile,
    runtime: &Runtime,
) -> anyhow::Result<()> {
    let mut listening = HashSet::new();
    for route in ir
        .servers
        .iter()
        .flat_map(|s| &s.routes)
        .filter(|r| r.response_kind == corvid_ast::RouteResponseKind::Live)
    {
        let db = &query_table(route)
            .ok_or_else(|| anyhow::anyhow!("live query {} has no table", route.path))?
            .database;
        if listening.insert(&db.name) {
            let engine =
                TableEngine::from_declaration(&db.engine, db.tls.as_deref(), db.connections)
                    .ok_or_else(|| {
                        anyhow::anyhow!("unsupported database engine for {}", db.name)
                    })?;
            if let TableEngine::Postgres { tls, .. } = engine {
                runtime
                    .share_table_commits(&db.name, &db.location, tls)
                    .await?;
            }
        }
    }
    Ok(())
}

fn wire_row(revision: i64, row: serde_json::Value) -> Result<Row, Failure> {
    Ok(Row {
        revision: revision.try_into().map_err(|_| Failure::Unavailable)?,
        row,
    })
}

fn snapshot(rows: &[LiveRow]) -> Result<Delivery, Failure> {
    Ok(Delivery::Snapshot {
        rows: rows
            .iter()
            .map(|r| wire_row(r.revision, r.row.clone()))
            .collect::<Result<_, _>>()?,
    })
}

fn change_frame(change: LiveChange) -> Result<Delivery, Failure> {
    let row = wire_row(change.revision, change.row)?;
    let index = change.index;
    let change = match change.kind {
        LiveChangeKind::Added => Change::Added {
            row,
            index: index.ok_or(Failure::Unavailable)?,
        },
        LiveChangeKind::Updated => Change::Updated {
            row,
            index: index.ok_or(Failure::Unavailable)?,
        },
        LiveChangeKind::Removed => Change::Removed { row },
    };
    Ok(Delivery::Change { change })
}

/// Wait for a relevant commit. Lag requires a full replacement snapshot.
async fn wait_for_commit(
    commits: &mut Receiver<TableCommit>,
    database: &str,
    table: &str,
) -> Result<Option<WrittenRows>, Failure> {
    loop {
        let commit = match commits.recv().await {
            Ok(commit) => commit,
            Err(RecvError::Lagged(_)) => {
                *commits = commits.resubscribe();
                return Ok(None);
            }
            Err(RecvError::Closed) => return Err(Failure::Unavailable),
        };
        let relevant = |c: &TableCommit| {
            c.database == database && (c.tables.is_empty() || c.tables.contains(table))
        };
        if !relevant(&commit) {
            continue;
        }
        let mut written = WrittenRows::default();
        let mut complete = true;
        let mut merge = |commit: TableCommit| {
            if relevant(&commit) {
                if let Some(rows) = commit.rows.get(table) {
                    written.merge(rows);
                } else {
                    complete = false;
                }
            }
        };
        merge(commit);
        // Bound this batch even if another process is continuously writing.
        for _ in 0..commits.len() {
            match commits.try_recv() {
                Ok(commit) => merge(commit),
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                    *commits = commits.resubscribe();
                    return Ok(None);
                }
                Err(_) => break,
            }
        }
        return Ok(Some(if complete {
            written
        } else {
            WrittenRows::default()
        }));
    }
}

pub(super) fn subscribe(
    ir: Arc<IrFile>,
    runtime: Runtime,
    types: Arc<Vec<IrType>>,
    route: &IrRoute,
    path: Value,
    actor: Value,
) -> Result<Subscription, Failure> {
    let table = query_table(route).ok_or(Failure::Unavailable)?.clone();
    let handler = route.handler_agent.clone();
    let agent = ir
        .agents
        .iter()
        .find(|a| a.name == handler)
        .ok_or(Failure::Unavailable)?;
    let args = agent
        .params
        .iter()
        .map(|p| match p.name.as_str() {
            "path" => Ok(path.clone()),
            "actor" => Ok(actor.clone()),
            _ => Err(Failure::Unavailable),
        })
        .collect::<Result<Vec<_>, _>>()?;
    // The declaration names the row type. Its query returns a list of those rows.
    let encoding = ReplyEncoding {
        ty: agent.return_ty.clone(),
        types,
    };
    let mut commits = runtime.subscribe_table_commits();
    Ok(async_stream::try_stream! {
        let mut previous = Vec::new();
        let mut batch = None;
        loop {
            let value = run_ir_with_runtime(&ir, Some(&handler), args.clone(), &runtime)
                .await.map_err(|_| Failure::Unavailable)?;
            let json = encoding.encode(&value, &encoding.ty).map_err(|_| Failure::Unavailable)?;
            let rows = match json {
                serde_json::Value::Array(rows) => Ok(rows),
                _ => Err(Failure::Unavailable),
            }?;
            let next = rows.into_iter()
                .map(|row| LiveRow::from_json(row, &table.keys).ok_or(Failure::Unavailable))
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(written) = &batch {
                // A newer commit may already be visible to this read. Its removal
                // revision is unknown until we consume that notification.
                let unknown = WrittenRows::default();
                let revisions = if commits.is_empty() { written } else { &unknown };
                match live_changes(&previous, &next, revisions) {
                    LiveMessage::Unchanged => {}
                    LiveMessage::Snapshot(rows) => yield snapshot(&rows)?,
                    LiveMessage::Changes(changes) => for change in changes {
                        yield change_frame(change)?;
                    }
                }
            } else {
                yield snapshot(&next)?;
            }
            previous = next;
            batch = wait_for_commit(&mut commits, &table.database.name, &table.name).await?;
        }
    }
    .boxed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve_cmd::live::Routes;
    use serde_json::json;
    use std::time::Duration;

    async fn frame(stream: &mut Subscription) -> serde_json::Value {
        let delivery = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("query delivery timed out")
            .expect("subscription ended")
            .expect("query failed");
        serde_json::to_value(delivery).unwrap()
    }

    #[tokio::test]
    async fn commit_filter_handles_database_resync_and_receiver_lag() {
        let (sender, mut receiver) = tokio::sync::broadcast::channel(4);
        let announce = |database: &str, tables: &[&str]| {
            sender
                .send(TableCommit::of_tables(
                    database,
                    tables.iter().map(|name| name.to_string()),
                ))
                .unwrap();
        };
        announce("archive", &[]);
        announce("boards", &["Note"]);
        let read = wait_for_commit(&mut receiver, "boards", "Card");
        tokio::pin!(read);
        assert!(futures::poll!(&mut read).is_pending());
        announce("boards", &[]);
        assert!(read.await.unwrap().unwrap().is_empty());

        let mut slow = sender.subscribe();
        for _ in 0..5 {
            announce("boards", &["Card"]);
        }
        assert!(wait_for_commit(&mut slow, "boards", "Card")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reopened_query_reads_offline_commits_then_forwards_matching_writes() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("boards.sqlite");
        rusqlite::Connection::open(&database).unwrap().execute_batch(
            "CREATE TABLE Card (card INTEGER PRIMARY KEY NOT NULL, board TEXT NOT NULL, revision INTEGER NOT NULL)"
        ).unwrap();
        let source = include_str!("../../../tests/fixtures/live_cards.cor").replace(
            "boards.sqlite",
            &database.to_string_lossy().replace('\\', "/"),
        );
        let program = source
            + r#"
agent add(card: Int, board: String) -> String:
    return match Card.insert(Card(card, board)):
        Ok(row) -> "added"
        Err(Taken(holder)) -> "taken"

agent move_card(card: Int, board: String) -> String:
    return match Card.get(card):
        Some(row) -> saved(Card.save(Card { board: board, ..row }))
        None -> "missing"

fn saved(result: Result<Card, Conflict<Card>>) -> String:
    return match result:
        Ok(row) -> "saved"
        Err(_) -> "conflict"
"#;
        let ir = Arc::new(corvid_driver::compile_to_ir(&program).unwrap());
        let runtime = Runtime::builder().build();
        let routes = Routes::new(&ir).unwrap();
        let (route, path) = routes.resolve(&ir, "/boards/home/cards").unwrap();
        let types = Arc::new(ir.types.clone());
        let open = || {
            subscribe(
                ir.clone(),
                runtime.clone(),
                types.clone(),
                route,
                path.clone(),
                Value::Nothing,
            )
            .unwrap()
        };
        let write = |agent: &'static str, id, board: &str| {
            run_ir_with_runtime(
                &ir,
                Some(agent),
                vec![Value::Int(id), Value::String(board.into())],
                &runtime,
            )
        };
        write("add", 4, "away").await.unwrap();
        let mut first = open();
        assert_eq!(
            frame(&mut first).await,
            json!({"type":"snapshot","rows":[]})
        );
        drop(first);
        write("move_card", 4, "home").await.unwrap();
        write("add", 9, "away").await.unwrap();
        let mut reopened = open();
        let initial = frame(&mut reopened).await;
        assert_eq!(
            initial,
            json!({"type":"snapshot","rows":[{"revision":"2","row":{"card":4,"board":"home","revision":2}}]})
        );
        write("add", 3, "away").await.unwrap();
        write("add", 7, "home").await.unwrap();
        assert_eq!(
            frame(&mut reopened).await,
            json!({"type":"change","kind":"added","index":1,"revision":"1","row":{"card":7,"board":"home","revision":1}})
        );
        write("move_card", 4, "away").await.unwrap();
        assert_eq!(
            frame(&mut reopened).await,
            json!({"type":"change","kind":"removed","revision":"3","row":{"card":4,"board":"home","revision":2}})
        );
    }
}
