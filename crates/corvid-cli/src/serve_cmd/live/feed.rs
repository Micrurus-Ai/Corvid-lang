//! Translate RT-2 commit notifications into frames for one authorized query.
use super::protocol::{Change, Delivery, Row};
use super::shared::{QueryKey, SharedQueries, Step};
use super::{Failure, Subscription};
use crate::serve_cmd::ReplyEncoding;
use corvid_driver::{run_agent_reading_written_rows, run_ir_with_runtime, Runtime, Value, WrittenKeys};
use corvid_ir::{for_each_expr, IrAgent, IrExprKind, IrFile, IrRoute, IrStmt, IrTable, IrTableFilter, IrType};
use corvid_runtime::tables::{
    column_kind, live_changes, merge_rechecked, ColumnKind, LiveChange, LiveChangeKind,
    LiveMessage, LiveRow, OrderColumn, RowOrder, TableCommit, TableEngine, WrittenRows,
    MOST_ROWS_RECHECKED,
};
use corvid_types::Type;
use futures::stream::BoxStream;
use std::sync::atomic::{AtomicU64, Ordering};
use futures::StreamExt;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::broadcast::{error::RecvError, Receiver};

/// A live declaration's query: its table, and its `order_by` column and
/// direction.
struct LiveQuery<'a> {
    table: &'a IrTable,
    order: Option<&'a str>,
    descending: bool,
}

fn live_query(route: &IrRoute) -> Option<LiveQuery<'_>> {
    let [IrStmt::Return {
        value: Some(expr), ..
    }] = route.body.stmts.as_slice()
    else {
        return None;
    };
    match &expr.kind {
        IrExprKind::TableQuery {
            table,
            order,
            descending,
            ..
        } => Some(LiveQuery {
            table,
            order: order.as_deref(),
            descending: *descending,
        }),
        _ => None,
    }
}

fn query_table(route: &IrRoute) -> Option<&IrTable> {
    live_query(route).map(|query| query.table)
}

/// The query's order when re-checked rows can be placed in it exactly,
/// else `None`, and every commit re-runs the full query.
fn row_order(query: &LiveQuery<'_>, types: &[IrType]) -> Option<RowOrder> {
    let table = query.table;
    let record = types.iter().find(|t| t.id == table.def_id)?;
    let column = |name: &str| {
        let field = record.fields.iter().find(|f| f.name == name)?;
        let (kind, nullable) = column_kind(&field.ty);
        Some((field.name.clone(), kind, nullable))
    };
    let order = match query.order {
        Some(name) => Some(column(name)?),
        None => None,
    };
    let keys = table.keys.iter().map(|k| column(k)).collect::<Option<Vec<_>>>()?;
    let engine = TableEngine::from_declaration(
        &table.database.engine,
        table.database.tls.as_deref(),
        table.database.connections,
    )?;
    fn as_column((name, kind, nullable): &(String, ColumnKind, bool)) -> OrderColumn<'_> {
        OrderColumn {
            name,
            kind: *kind,
            nullable: *nullable,
        }
    }
    RowOrder::of_query(
        engine,
        order.as_ref().map(|c| (as_column(c), query.descending)),
        &keys.iter().map(as_column).collect::<Vec<_>>(),
    )
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
        // Every database whose tables the query reads: the delivered
        // table's, and any its condition reads through another table (J5).
        let table = query_table(route).ok_or_else(|| anyhow::anyhow!("live query {} has no table", route.path))?;
        let mut tables = vec![table.clone()];
        tables.extend(read_tables(ir, &route.handler_agent));
        for db in tables.iter().map(|table| &table.database) {
            if !listening.insert(db.name.clone()) {
                continue;
            }
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

/// The tables a live query reads (J5): the one it delivers, whose written
/// rows can be re-checked alone, and any its condition reads through a
/// list or an inner query, a write to which can change which rows match
/// without writing them.
#[derive(Debug, Clone)]
struct Watched {
    database: String,
    table: String,
    others: Vec<(String, String)>,
}

impl Watched {
    fn reads(database: &str, table: &str, c: &TableCommit) -> bool {
        c.database == database && (c.tables.is_empty() || c.tables.contains(table))
    }

    fn relevant(&self, c: &TableCommit) -> bool {
        Self::reads(&self.database, &self.table, c) || self.others.iter().any(|(d, t)| Self::reads(d, t, c))
    }

    fn touches_others(&self, c: &TableCommit) -> bool {
        self.others.iter().any(|(d, t)| Self::reads(d, t, c))
    }
}

/// Every table the handler reads through a query: the delivered one, any
/// a list in its condition queries, and each inner `keys()` query's (J5).
fn read_tables(ir: &IrFile, handler: &str) -> Vec<IrTable> {
    fn inner_tables(filter: &IrTableFilter, out: &mut Vec<IrTable>) {
        match filter {
            IrTableFilter::And(left, right) | IrTableFilter::Or(left, right) => {
                inner_tables(left, out);
                inner_tables(right, out);
            }
            IrTableFilter::Not(inner) => inner_tables(inner, out),
            IrTableFilter::InQuery { inner, .. } => {
                out.push(inner.table.clone());
                if let Some(filter) = &inner.filter {
                    inner_tables(filter, out);
                }
            }
            _ => {}
        }
    }
    let mut tables = Vec::new();
    for_each_expr(ir, &mut |owner, expr| {
        if owner != handler {
            return;
        }
        if let IrExprKind::TableQuery { table, filter, .. } = &expr.kind {
            tables.push(table.clone());
            if let Some(filter) = filter {
                inner_tables(filter, &mut tables);
            }
        }
    });
    tables
}

/// What a live query watches: its delivered table, and every other table
/// [`read_tables`] finds.
fn watched_tables(ir: &IrFile, handler: &str, delivered: &IrTable) -> Watched {
    let own = (delivered.database.name.clone(), delivered.name.clone());
    let mut others: Vec<(String, String)> = read_tables(ir, handler)
        .into_iter()
        .map(|table| (table.database.name, table.name))
        .filter(|other| *other != own)
        .collect();
    others.sort();
    others.dedup();
    Watched { database: own.0, table: own.1, others }
}

/// Wait for a relevant commit. Lag requires a full replacement snapshot,
/// and a commit to another table the query reads requires a full re-read.
async fn wait_for_commit(
    commits: &mut Receiver<TableCommit>,
    watched: &Watched,
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
        if !watched.relevant(&commit) {
            continue;
        }
        let table = watched.table.as_str();
        let mut written = WrittenRows::default();
        let mut complete = true;
        let mut merge = |commit: TableCommit| {
            if watched.touches_others(&commit) {
                complete = false;
            }
            if Watched::reads(&watched.database, table, &commit) {
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
    let (key, evaluation) = evaluation(queries, ir, runtime, types, route, path, actor)?;
    Ok(queries.join(key, move || evaluation.run()))
}

/// The evaluation a subscription joins, and the key it shares under.
fn evaluation(
    queries: &Arc<SharedQueries>,
    ir: Arc<IrFile>,
    runtime: Runtime,
    types: Arc<Vec<IrType>>,
    route: &IrRoute,
    path: Value,
    actor: Value,
) -> Result<(QueryKey, Evaluation), Failure> {
    let query = live_query(route).ok_or(Failure::Unavailable)?;
    let order = row_order(&query, &types);
    let table = query.table.clone();
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
    let watched = watched_tables(&ir, &handler, &table);
    let evaluation = Evaluation {
        ir,
        runtime,
        encoding,
        handler,
        table,
        args,
        order,
        rows_read: queries.rows_read.clone(),
        watched,
    };
    Ok((key, evaluation))
}

/// One live query and what it needs to run.
struct Evaluation {
    ir: Arc<IrFile>,
    runtime: Runtime,
    encoding: ReplyEncoding,
    handler: String,
    table: IrTable,
    args: Vec<Value>,
    /// Set when written rows can be re-checked alone (live L2).
    order: Option<RowOrder>,
    rows_read: Arc<AtomicU64>,
    /// Every table whose commits can change the query's rows (J5).
    watched: Watched,
}

impl Evaluation {
    /// The query's rows: all of them, or only the `written` ones that
    /// still match its condition.
    async fn read(&self, written: Option<&WrittenRows>) -> Result<Vec<LiveRow>, Failure> {
        let value = match written {
            None => run_ir_with_runtime(&self.ir, Some(&self.handler), self.args.clone(), &self.runtime)
                .await
                .map_err(|_| Failure::Unavailable)?,
            Some(written) => {
                let only = WrittenKeys {
                    table: self.table.name.clone(),
                    keys: written.iter().map(|(key, _)| key.to_string()).collect(),
                };
                run_agent_reading_written_rows(&self.ir, &self.handler, self.args.clone(), &self.runtime, only)
                    .await
                    .map_err(|_| Failure::Unavailable)?
            }
        };
        let json = self
            .encoding
            .encode(&value, &self.encoding.ty)
            .map_err(|_| Failure::Unavailable)?;
        let serde_json::Value::Array(rows) = json else {
            return Err(Failure::Unavailable);
        };
        self.rows_read.fetch_add(rows.len() as u64, Ordering::Relaxed);
        rows.into_iter()
            .map(|row| LiveRow::from_json(row, &self.table.keys).ok_or(Failure::Unavailable))
            .collect()
    }

    /// The result after a commit that wrote `written`: the written rows
    /// re-checked and merged in when the commit names few enough rows and
    /// the order allows it, else the full query.
    async fn after_commit(&self, previous: &[LiveRow], written: &WrittenRows) -> Result<Vec<LiveRow>, Failure> {
        let few = !written.is_empty() && written.iter().count() <= MOST_ROWS_RECHECKED;
        if let (true, Some(order)) = (few, &self.order) {
            let rechecked = self.read(Some(written)).await?;
            if let Some(next) = merge_rechecked(previous, rechecked, written, order) {
                return Ok(next);
            }
        }
        self.read(None).await
    }

    /// Run the query once, then once per relevant commit, yielding the
    /// rows and the frames that carry a subscriber to them: a snapshot
    /// first, then changes.
    fn run(self) -> BoxStream<'static, Step> {
        let mut commits = self.runtime.subscribe_table_commits();
        async_stream::try_stream! {
            let mut previous = Vec::new();
            let mut batch: Option<WrittenRows> = None;
            loop {
                let next = match &batch {
                    Some(written) => self.after_commit(&previous, written).await?,
                    None => self.read(None).await?,
                };
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
                batch = wait_for_commit(&mut commits, &self.watched).await?;
            }
        }
        .boxed()
    }
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
        let card = Watched { database: "boards".into(), table: "Card".into(), others: Vec::new() };
        let read = wait_for_commit(&mut receiver, &card);
        tokio::pin!(read);
        assert!(futures::poll!(&mut read).is_pending());
        announce("boards", &[]);
        assert!(read.await.unwrap().unwrap().is_empty());

        let mut slow = sender.subscribe();
        for _ in 0..5 {
            announce("boards", &["Card"]);
        }
        assert!(wait_for_commit(&mut slow, &card)
            .await
            .unwrap()
            .is_none());
    }

    /// The live cards fixture over a fresh database, with agents that write
    /// cards and tasks, a live query that reads its caller, and a live task
    /// list ordered by rank, highest first.
    fn board_program(dir: &tempfile::TempDir) -> Arc<IrFile> {
        let database = dir.path().join("boards.sqlite");
        let connection = rusqlite::Connection::open(&database).unwrap();
        corvid_runtime::tables::register_sqlite_collations(&connection).unwrap();
        connection.execute_batch(
            "CREATE TABLE Card (card INTEGER PRIMARY KEY NOT NULL, board TEXT NOT NULL, revision INTEGER NOT NULL);
             CREATE TABLE Task (id INTEGER PRIMARY KEY NOT NULL, list TEXT NOT NULL, rank INTEGER NOT NULL, revision INTEGER NOT NULL);
             CREATE TABLE Bill (id INTEGER PRIMARY KEY NOT NULL, list TEXT NOT NULL, total DECIMAL TEXT COLLATE corvid_decimal NOT NULL, revision INTEGER NOT NULL);
             CREATE TABLE Board (board TEXT PRIMARY KEY NOT NULL, owner TEXT NOT NULL, revision INTEGER NOT NULL)"
        ).unwrap();
        let source = include_str!("../../../tests/fixtures/live_cards.cor")
            .replace(
                "boards.sqlite",
                &database.to_string_lossy().replace('\\', "/"),
            )
            .replace(
                "    route GET \"/archive\"",
                "    live \"/my_cards\" -> Card requires permission(\"board.view\"):\n        return Card.where(fn (c) -> c.board == actor.id)\n    live \"/lists/{list}/tasks\" -> Task requires permission(\"board.view\"):\n        return Task.where(fn (t) -> t.list == path.list).order_by(fn (t) -> t.rank).descending()\n    live \"/lists/{list}/bills\" -> Bill requires permission(\"board.view\"):\n        return Bill.where(fn (b) -> b.list == path.list).order_by(fn (b) -> b.total)\n    live \"/owners/{owner}/owned_cards\" -> Card requires permission(\"board.view\"):\n        return Card.where(fn (c) -> Board.where(fn (b) -> b.owner == path.owner).keys().contains(c.board))\n    route GET \"/archive\"",
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

table Task:
    key id: Int
    list: String
    rank: Int

agent add_task(id: Int, list: String, rank: Int) -> String:
    return match Task.insert(Task(id, list, rank)):
        Ok(row) -> "added"
        Err(_) -> "taken"

agent change_task(id: Int, list: String, rank: Int) -> String:
    return match Task.get(id):
        Some(row) -> task_written(Task.save(Task { list: list, rank: rank, ..row }))
        None -> "missing"

agent remove_task(id: Int) -> String:
    return match Task.get(id):
        Some(row) -> task_written(Task.delete(row))
        None -> "missing"

fn task_written(result: Result<Task, Conflict<Task>>) -> String:
    return match result:
        Ok(row) -> "written"
        Err(_) -> "conflict"

table Bill:
    key id: Int
    list: String
    total: Decimal

agent add_bill(id: Int, list: String, total: Decimal) -> String:
    return match Bill.insert(Bill(id, list, total)):
        Ok(row) -> "added"
        Err(_) -> "taken"

agent change_bill(id: Int, list: String, total: Decimal) -> String:
    return match Bill.get(id):
        Some(row) -> bill_written(Bill.save(Bill { list: list, total: total, ..row }))
        None -> "missing"

fn bill_written(result: Result<Bill, Conflict<Bill>>) -> String:
    return match result:
        Ok(row) -> "written"
        Err(_) -> "conflict"

table Board:
    key board: String
    owner: String

agent add_board(board: String, owner: String) -> String:
    return match Board.insert(Board(board, owner)):
        Ok(row) -> "added"
        Err(_) -> "taken"

agent give_board(board: String, owner: String) -> String:
    return match Board.get(board):
        Some(row) -> board_written(Board.save(Board { owner: owner, ..row }))
        None -> "missing"

fn board_written(result: Result<Board, Conflict<Board>>) -> String:
    return match result:
        Ok(row) -> "written"
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
    async fn a_commit_writing_one_row_of_ten_thousand_reads_only_that_row() {
        let dir = tempfile::tempdir().unwrap();
        let ir = board_program(&dir);
        let mut connection = rusqlite::Connection::open(dir.path().join("boards.sqlite")).unwrap();
        let fill = connection.transaction().unwrap();
        for card in 0..10_000 {
            fill.execute(
                "INSERT INTO Card (card, board, revision) VALUES (?1, 'home', 1)",
                [card],
            )
            .unwrap();
        }
        fill.commit().unwrap();
        let runtime = Runtime::builder().build();
        let routes = Routes::new(&ir).unwrap();
        let (route, path) = routes.resolve(&ir, "/boards/home/cards").unwrap();
        let queries = Arc::new(SharedQueries::default());
        let mut watcher = subscribe(
            &queries,
            ir.clone(),
            runtime.clone(),
            Arc::new(ir.types.clone()),
            route,
            path,
            Value::Nothing,
        )
        .unwrap();
        let first = frame(&mut watcher).await;
        assert_eq!(first["rows"].as_array().unwrap().len(), 10_000);
        assert_eq!(queries.rows_read(), 10_000);
        write(&ir, &runtime, "add", 5_000_000, "home").await;
        assert_eq!(
            frame(&mut watcher).await,
            json!({"type":"change","kind":"added","index":10_000,"revision":"1","row":{"card":5_000_000,"board":"home","revision":1}})
        );
        write(&ir, &runtime, "move_card", 17, "away").await;
        assert_eq!(
            frame(&mut watcher).await,
            json!({"type":"change","kind":"removed","revision":"2","row":{"card":17,"board":"home","revision":1}})
        );
        assert_eq!(queries.rows_read(), 10_001, "only the added row was read back");
    }

    /// The next step of an evaluation: its rows and its frames as JSON.
    async fn step(steps: &mut BoxStream<'static, Step>) -> (Vec<LiveRow>, serde_json::Value) {
        let (rows, frames) = tokio::time::timeout(Duration::from_secs(5), steps.next())
            .await
            .expect("evaluation timed out")
            .expect("evaluation ended")
            .expect("evaluation failed");
        (rows, serde_json::to_value(frames).unwrap())
    }

    /// Run `writes` against the live query at `path` twice side by side,
    /// re-checking written rows and re-running the full query, and require
    /// equal rows and frames after each write, with fewer rows read.
    async fn assert_rechecked_like_full_query(path: &str, first: (&'static str, Vec<Value>), writes: Vec<(&'static str, Vec<Value>)>) {
        let dir = tempfile::tempdir().unwrap();
        let ir = board_program(&dir);
        let runtime = Runtime::builder().build();
        let routes = Routes::new(&ir).unwrap();
        let (route, path) = routes.resolve(&ir, path).unwrap();
        let types = Arc::new(ir.types.clone());
        let task = |agent: &'static str, args: Vec<Value>| {
            let (ir, runtime) = (ir.clone(), runtime.clone());
            async move { run_ir_with_runtime(&ir, Some(agent), args, &runtime).await.unwrap() }
        };
        task(first.0, first.1).await;
        let open = || {
            let queries = Arc::new(SharedQueries::default());
            let (_, evaluation) = evaluation(
                &queries,
                ir.clone(),
                runtime.clone(),
                types.clone(),
                route,
                path.clone(),
                Value::Nothing,
            )
            .unwrap();
            (queries, evaluation)
        };
        let (rechecked_reads, rechecked) = open();
        let (full_reads, mut full) = open();
        assert!(rechecked.order.is_some(), "the order is re-checked");
        full.order = None;
        let (mut rechecked, mut full) = (rechecked.run(), full.run());
        assert_eq!(step(&mut rechecked).await, step(&mut full).await);
        for (agent, args) in writes {
            task(agent, args.clone()).await;
            let (rows, frames) = step(&mut rechecked).await;
            assert_eq!((rows, frames), step(&mut full).await, "after {agent}{args:?}");
        }
        assert!(
            rechecked_reads.rows_read() < full_reads.rows_read(),
            "{} rows re-checked, {} read by the full query",
            rechecked_reads.rows_read(),
            full_reads.rows_read()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rechecking_written_rows_gives_the_frames_of_the_full_query() {
        let (int, text) = (Value::Int, |s: &str| Value::String(s.into()));
        let first = ("add_task", vec![int(9), text("home"), int(7)]);
        let writes: Vec<(&'static str, Vec<Value>)> = vec![
            ("add_task", vec![int(1), text("home"), int(5)]),
            ("add_task", vec![int(2), text("home"), int(3)]),
            // Another list: nothing changes here.
            ("add_task", vec![int(3), text("away"), int(9)]),
            // The same rank as task 1: the key orders them.
            ("add_task", vec![int(4), text("home"), int(5)]),
            // Moves to the top.
            ("change_task", vec![int(2), text("home"), int(10)]),
            // Moves out of the list.
            ("change_task", vec![int(1), text("away"), int(5)]),
            // Moves into the list, at the bottom.
            ("change_task", vec![int(3), text("home"), int(1)]),
            ("remove_task", vec![int(4)]),
            // Moves down past two rows.
            ("change_task", vec![int(2), text("home"), int(2)]),
        ];
        assert_rechecked_like_full_query("/lists/home/tasks", first, writes).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rechecked_decimal_order_follows_value_not_text() {
        let (int, text) = (Value::Int, |s: &str| Value::String(s.into()));
        let total = |s: &str| {
            corvid_vm::decode_public_json(
                &json!(s),
                &Type::Decimal,
                &Default::default(),
                corvid_vm::UnknownFields::Reject,
            )
            .unwrap()
        };
        let first = ("add_bill", vec![int(1), text("home"), total("10.0")]);
        let writes: Vec<(&'static str, Vec<Value>)> = vec![
            // Before `10.0` by value, after it as text.
            ("add_bill", vec![int(2), text("home"), total("9.50")]),
            ("add_bill", vec![int(3), text("home"), total("-2")]),
            // Equal to `10.0`: the key orders them.
            ("add_bill", vec![int(4), text("home"), total("10.000")]),
            ("change_bill", vec![int(3), text("home"), total("100")]),
            ("change_bill", vec![int(1), text("away"), total("10.0")]),
            ("change_bill", vec![int(2), text("home"), total("0.001")]),
        ];
        assert_rechecked_like_full_query("/lists/home/bills", first, writes).await;
    }

    /// J5: a live query that filters by another table's keys follows that
    /// table's writes too. Giving a board to ada brings its cards into her
    /// list though no card was written; taking one away removes them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_live_query_follows_writes_to_the_table_its_filter_reads() {
        let dir = tempfile::tempdir().unwrap();
        let ir = board_program(&dir);
        let runtime = Runtime::builder().build();
        let routes = Routes::new(&ir).unwrap();
        let (route, path) = routes.resolve(&ir, "/owners/ada/owned_cards").unwrap();
        let queries = Arc::new(SharedQueries::default());
        let run = |agent: &'static str, args: Vec<Value>| {
            let (ir, runtime) = (ir.clone(), runtime.clone());
            async move { run_ir_with_runtime(&ir, Some(agent), args, &runtime).await.unwrap() }
        };
        let text = |s: &str| Value::String(s.into());
        run("add_board", vec![text("home"), text("ada")]).await;
        run("add_board", vec![text("away"), text("grace")]).await;
        write(&ir, &runtime, "add", 1, "home").await;
        write(&ir, &runtime, "add", 2, "away").await;
        let mut watcher = subscribe(
            &queries,
            ir.clone(),
            runtime.clone(),
            Arc::new(ir.types.clone()),
            route,
            path,
            Value::Nothing,
        )
        .unwrap();
        let card = |card: i64, board: &str| json!({"card": card, "board": board, "revision": 1});
        assert_eq!(
            frame(&mut watcher).await,
            json!({"type":"snapshot","rows":[{"revision":"1","row":card(1, "home")}]})
        );
        // Only `Board` is written: card 2 joins ada's list.
        run("give_board", vec![text("away"), text("ada")]).await;
        assert_eq!(
            frame(&mut watcher).await,
            json!({"type":"change","kind":"added","index":1,"revision":"1","row":card(2, "away")})
        );
        // `home` leaves her: card 1 leaves, though it was not written, so its
        // removal has no revision of its own and a snapshot replaces the list.
        run("give_board", vec![text("home"), text("grace")]).await;
        assert_eq!(
            frame(&mut watcher).await,
            json!({"type":"snapshot","rows":[{"revision":"1","row":card(2, "away")}]})
        );
        // A card written on her board still arrives as a change.
        write(&ir, &runtime, "add", 3, "away").await;
        assert_eq!(
            frame(&mut watcher).await,
            json!({"type":"change","kind":"added","index":1,"revision":"1","row":card(3, "away")})
        );
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
