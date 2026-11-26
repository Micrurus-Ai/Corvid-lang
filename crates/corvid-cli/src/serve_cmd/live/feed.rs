//! Translate RT-2 commit notifications into frames for one authorized query.
use super::protocol::{Change, Delivery, Row};
use super::shared::{QueryKey, SharedQueries, Step};
use super::{Failure, Subscription};
use crate::serve_cmd::ReplyEncoding;
use corvid_driver::{run_ir_with_runtime, Runtime, Value};
use corvid_ir::{for_each_expr, IrAgent, IrExprKind, IrFile, IrRoute, IrStmt, IrTable, IrType};
use corvid_types::Type;
use futures::stream::BoxStream;
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

pub(super) fn snapshot(rows: &[LiveRow]) -> Result<Delivery, Failure> {
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

/// Whether the handler's body reads its `actor` parameter. A query that
/// does not returns the same rows to every caller.
fn reads_actor(ir: &IrFile, agent: &IrAgent) -> bool {
    let Some(actor) = agent.params.iter().find(|p| p.name == "actor") else {
        return false;
    };
    let mut reads = false;
    for_each_expr(ir, &mut |owner, expr| {
        if owner == agent.name
            && matches!(&expr.kind, IrExprKind::Local { local_id, .. } if *local_id == actor.local_id)
        {
            reads = true;
        }
    });
    reads
}

/// A path argument or caller as text that is equal exactly when the values
/// are. Both are compiler-built records, so each field is written in its
/// public JSON form, in declaration order.
fn sharing_key(encoding: &ReplyEncoding, value: &Value, ty: &Type) -> Result<String, Failure> {
    let (Type::RouteParams(fields), Value::Struct(record)) = (ty, value) else {
        return Err(Failure::Unavailable);
    };
    let mut key = Vec::with_capacity(fields.len());
    for (name, field_ty) in fields {
        let field = record.get_field(name).ok_or(Failure::Unavailable)?;
        let public = encoding.encode(&field, field_ty).map_err(|_| Failure::Unavailable)?;
        key.push(serde_json::json!([name, public]));
    }
    Ok(serde_json::Value::Array(key).to_string())
}

pub(super) fn subscribe(
    queries: &Arc<SharedQueries>,
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
    // The declaration names the row type. Its query returns a list of those rows.
    let encoding = ReplyEncoding {
        ty: agent.return_ty.clone(),
        types,
    };
    let mut args = Vec::with_capacity(agent.params.len());
    let mut key = QueryKey {
        handler: handler.clone(),
        path: String::new(),
        actor: None,
    };
    let reads_actor = queries.reads_actor(&handler, || reads_actor(&ir, agent));
    for param in &agent.params {
        let value = match param.name.as_str() {
            "path" => &path,
            "actor" => &actor,
            _ => return Err(Failure::Unavailable),
        };
        match param.name.as_str() {
            "path" => key.path = sharing_key(&encoding, value, &param.ty)?,
            _ if reads_actor => key.actor = Some(sharing_key(&encoding, value, &param.ty)?),
            _ => {}
        }
        args.push(value.clone());
    }
    Ok(queries.join(key, move || evaluate(ir, runtime, encoding, handler, table, args)))
}

/// Run the query once, then once per relevant commit, yielding the rows and
/// the frames that carry a subscriber to them: a snapshot first, then changes.
fn evaluate(
    ir: Arc<IrFile>,
    runtime: Runtime,
    encoding: ReplyEncoding,
    handler: String,
    table: IrTable,
    args: Vec<Value>,
) -> BoxStream<'static, Step> {
    let mut commits = runtime.subscribe_table_commits();
    async_stream::try_stream! {
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
            let mut frames = Vec::new();
            if let Some(written) = &batch {
                // A newer commit may already be visible to this read. Its removal
                // revision is unknown until we consume that notification.
                let unknown = WrittenRows::default();
                let revisions = if commits.is_empty() { written } else { &unknown };
                match live_changes(&previous, &next, revisions) {
                    LiveMessage::Unchanged => {}
                    LiveMessage::Snapshot(rows) => frames.push(snapshot(&rows)?),
                    LiveMessage::Changes(changes) => for change in changes {
                        frames.push(change_frame(change)?);
                    }
                }
            } else {
                frames.push(snapshot(&next)?);
            }
            yield (next.clone(), frames);
            previous = next;
            batch = wait_for_commit(&mut commits, &table.database.name, &table.name).await?;
        }
    }
    .boxed()
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

    /// The live cards fixture over a fresh database, with agents that write
    /// cards and a second live query that reads its caller.
    fn board_program(dir: &tempfile::TempDir) -> Arc<IrFile> {
        let database = dir.path().join("boards.sqlite");
        rusqlite::Connection::open(&database).unwrap().execute_batch(
            "CREATE TABLE Card (card INTEGER PRIMARY KEY NOT NULL, board TEXT NOT NULL, revision INTEGER NOT NULL)"
        ).unwrap();
        let source = include_str!("../../../tests/fixtures/live_cards.cor")
            .replace(
                "boards.sqlite",
                &database.to_string_lossy().replace('\\', "/"),
            )
            .replace(
                "    route GET \"/archive\"",
                "    live \"/my_cards\" -> Card requires permission(\"board.view\"):\n        return Card.where(fn (c) -> c.board == actor.id)\n    route GET \"/archive\"",
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
        Arc::new(corvid_driver::compile_to_ir(&program).unwrap())
    }

    fn caller(id: &str) -> Value {
        let Value::Struct(stub) = crate::serve_cmd::stub_actor_value() else {
            unreachable!("the caller is a record")
        };
        let mut fields = stub.with_fields(|fields| fields.clone());
        fields.insert("id".into(), Value::String(id.into()));
        Value::Struct(corvid_driver::StructValue::new(stub.type_id(), "actor", fields))
    }

    async fn write(ir: &IrFile, runtime: &Runtime, agent: &str, id: i64, board: &str) {
        run_ir_with_runtime(
            ir,
            Some(agent),
            vec![Value::Int(id), Value::String(board.into())],
            runtime,
        )
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscribers_to_one_board_share_one_evaluation_per_commit() {
        let dir = tempfile::tempdir().unwrap();
        let ir = board_program(&dir);
        let runtime = Runtime::builder().build();
        let routes = Routes::new(&ir).unwrap();
        let (route, path) = routes.resolve(&ir, "/boards/home/cards").unwrap();
        let types = Arc::new(ir.types.clone());
        let queries = Arc::new(SharedQueries::default());
        write(&ir, &runtime, "add", 4, "home").await;
        let mut watchers = Vec::new();
        for n in 0..50 {
            let mut watcher = subscribe(
                &queries,
                ir.clone(),
                runtime.clone(),
                types.clone(),
                route,
                path.clone(),
                caller(&format!("person-{n}")),
            )
            .unwrap();
            assert_eq!(
                frame(&mut watcher).await,
                json!({"type":"snapshot","rows":[{"revision":"1","row":{"card":4,"board":"home","revision":1}}]})
            );
            watchers.push(watcher);
        }
        assert_eq!(queries.shared_count(), 1);
        write(&ir, &runtime, "add", 7, "home").await;
        for watcher in &mut watchers {
            assert_eq!(
                frame(watcher).await,
                json!({"type":"change","kind":"added","index":1,"revision":"1","row":{"card":7,"board":"home","revision":1}})
            );
        }
        // The first snapshot and the one commit: two evaluations, not 100.
        assert_eq!(queries.evaluations(), 2);
        drop(watchers);
        assert_eq!(queries.shared_count(), 0, "the last watcher to leave stops it");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_query_that_reads_its_caller_is_shared_only_by_that_caller() {
        let dir = tempfile::tempdir().unwrap();
        let ir = board_program(&dir);
        let runtime = Runtime::builder().build();
        let routes = Routes::new(&ir).unwrap();
        let (route, path) = routes.resolve(&ir, "/my_cards").unwrap();
        let types = Arc::new(ir.types.clone());
        let queries = Arc::new(SharedQueries::default());
        write(&ir, &runtime, "add", 1, "ada").await;
        write(&ir, &runtime, "add", 2, "grace").await;
        let open = |id: &str| {
            subscribe(
                &queries,
                ir.clone(),
                runtime.clone(),
                types.clone(),
                route,
                path.clone(),
                caller(id),
            )
            .unwrap()
        };
        let (mut ada, mut ada_again, mut grace) = (open("ada"), open("ada"), open("grace"));
        let only = |card: i64, board: &str| {
            json!({"type":"snapshot","rows":[{"revision":"1","row":{"card":card,"board":board,"revision":1}}]})
        };
        assert_eq!(frame(&mut ada).await, only(1, "ada"));
        assert_eq!(frame(&mut ada_again).await, only(1, "ada"));
        assert_eq!(frame(&mut grace).await, only(2, "grace"));
        assert_eq!(queries.shared_count(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reopened_query_reads_offline_commits_then_forwards_matching_writes() {
        let dir = tempfile::tempdir().unwrap();
        let ir = board_program(&dir);
        let runtime = Runtime::builder().build();
        let routes = Routes::new(&ir).unwrap();
        let (route, path) = routes.resolve(&ir, "/boards/home/cards").unwrap();
        let types = Arc::new(ir.types.clone());
        let queries = Arc::new(SharedQueries::default());
        let open = || {
            subscribe(
                &queries,
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
