//! Aggregates J1: `count`, `sum`, `min` and `max` on table queries, on
//! SQLite and, when `CORVID_TEST_POSTGRES_URL` is set, on Postgres. Each
//! runs in the database and must give exactly what the same computation
//! gives in Corvid over the rows: an `Int` total that leaves 64 bits is
//! refused, `Decimal` and `Duration` totals are exact, and `min` and `max`
//! order values as `order_by` does, skipping missing ones.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[path = "support/table_engines.rs"]
mod table_engines;
use table_engines::TestDatabase;

fn program(settings: &str) -> String {
    format!(
        r#"
database ledger:
{settings}

table Item:
    key id: Int
    list: String
    points: Int
    price: Decimal
    spent: Duration
    due: Option<Instant>

fn at(text: String) -> Instant:
    return match text.parse_instant():
        Ok(instant) -> instant
        Err(_) -> Instant.from_epoch_milliseconds(0)

fn span(text: String) -> Duration:
    return match text.parse_duration():
        Ok(duration) -> duration
        Err(_) -> at("1970-01-01T00:00:00Z") - at("1970-01-01T00:00:00Z")

agent add(id: Int, list: String, points: Int, price: Decimal, spent: String, due: String) -> Int:
    due_at: Option<Instant> = None
    if due != "":
        due_at = Some(at(due))
    return match Item.insert(Item(id, list, points, price, span(spent), due_at)):
        Ok(_) -> 1
        Err(_) -> 0

agent stock() -> Int:
    return add(1, "home", 3, 10.0, "PT2H", "2026-07-11T08:30:00Z") + add(2, "home", 4, 9.50, "PT59M", "") + add(3, "home", 5, 0.001, "PT1S", "2026-01-01T00:00:00Z") + add(4, "away", 1, 100, "PT1H", "") + add(5, "home", -2, -3, "PT0.5S", "2026-07-11T08:30:00.5Z")

fn decimal_text(value: Option<Decimal>) -> String:
    return match value:
        Some(d) -> d.to_string()
        None -> "none"

fn instant_text(value: Option<Instant>) -> String:
    return match value:
        Some(i) -> i.to_string()
        None -> "none"

agent in_database(list: String) -> String:
    count = Item.where(fn (i) -> i.list == list).count()
    points = Item.where(fn (i) -> i.list == list).sum(fn (i) -> i.points)
    price = Item.where(fn (i) -> i.list == list).sum(fn (i) -> i.price)
    spent = Item.where(fn (i) -> i.list == list).sum(fn (i) -> i.spent)
    cheapest = Item.where(fn (i) -> i.list == list).min(fn (i) -> i.price)
    dearest = Item.where(fn (i) -> i.list == list).max(fn (i) -> i.price)
    first_due = Item.where(fn (i) -> i.list == list).min(fn (i) -> i.due)
    last_due = Item.where(fn (i) -> i.list == list).max(fn (i) -> i.due)
    return count.to_string() + " " + points.to_string() + " " + price.to_string() + " " + spent.to_string() + " " + decimal_text(cheapest) + " " + decimal_text(dearest) + " " + instant_text(first_due) + " " + instant_text(last_due)

agent in_memory(list: String) -> String:
    count = 0
    points = 0
    price: Decimal = 0
    spent = span("PT0S")
    cheapest: Option<Decimal> = None
    dearest: Option<Decimal> = None
    first_due: Option<Instant> = None
    last_due: Option<Instant> = None
    for item in Item.where(fn (i) -> i.list == list):
        count = count + 1
        points = points + item.points
        price = price + item.price
        spent = spent + item.spent
        cheapest = match cheapest:
            Some(c) -> Some(lower(c, item.price))
            None -> Some(item.price)
        dearest = match dearest:
            Some(d) -> Some(higher(d, item.price))
            None -> Some(item.price)
        first_due = earlier(first_due, item.due)
        last_due = later(last_due, item.due)
    return count.to_string() + " " + points.to_string() + " " + price.to_string() + " " + spent.to_string() + " " + decimal_text(cheapest) + " " + decimal_text(dearest) + " " + instant_text(first_due) + " " + instant_text(last_due)

fn lower(a: Decimal, b: Decimal) -> Decimal:
    if b < a:
        return b
    return a

fn higher(a: Decimal, b: Decimal) -> Decimal:
    if b > a:
        return b
    return a

fn earlier(held: Option<Instant>, next: Option<Instant>) -> Option<Instant>:
    return match next:
        None -> held
        Some(n) -> match held:
            None -> Some(n)
            Some(h) -> pick_earlier(h, n)

fn pick_earlier(h: Instant, n: Instant) -> Option<Instant>:
    if n < h:
        return Some(n)
    return Some(h)

fn later(held: Option<Instant>, next: Option<Instant>) -> Option<Instant>:
    return match next:
        None -> held
        Some(n) -> match held:
            None -> Some(n)
            Some(h) -> pick_later(h, n)

fn pick_later(h: Instant, n: Instant) -> Option<Instant>:
    if n > h:
        return Some(n)
    return Some(h)

agent main() -> String:
    stocked = stock()
    return stocked.to_string() + " | " + in_database("home") + " | " + in_memory("home") + " | " + in_database("none") + " | " + in_memory("none")

agent overflow() -> Int:
    a = add(10, "big", 9223372036854775807, 1, "PT1S", "")
    b = add(11, "big", 1, 1, "PT1S", "")
    return Item.where(fn (i) -> i.list == "big").sum(fn (i) -> i.points)
"#
    )
}

fn parts(out: Value) -> Vec<String> {
    let Value::String(text) = out else { panic!("expected text, got {out:?}") };
    text.split(" | ").map(str::to_string).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aggregates_in_the_database_equal_the_same_computation_over_the_rows() {
    for database in TestDatabase::every_engine().await {
        let source = program(&database.settings());
        database.create_tables(&source).await;
        let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
        let runtime = Runtime::builder().build();
        let out = parts(run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect("program runs"));
        let label = database.label();
        assert_eq!(out[0], "5", "on {label}");
        assert_eq!(out[1], out[2], "the database's aggregates equal the rows' on {label}");
        assert_eq!(out[3], out[4], "and for no rows on {label}");
        let home: Vec<&str> = out[1].split(' ').collect();
        assert_eq!(&home[..2], ["4", "10"], "count and Int sum on {label}");
        assert_eq!(home[4], "-3", "the least price by value on {label}");
        assert_eq!(home[5], "10.0", "the greatest price by value, not text (9.50 < 10.0) on {label}");
        assert_eq!(&out[3].split(' ').collect::<Vec<_>>()[4..], ["none"; 4], "no rows, no extremes, on {label}");
        database.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_int_total_past_64_bits_is_refused_not_wrapped() {
    for database in TestDatabase::every_engine().await {
        let source = program(&database.settings());
        database.create_tables(&source).await;
        let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
        let runtime = Runtime::builder().build();
        let out = run_ir_with_runtime(&ir, Some("overflow"), vec![], &runtime).await;
        assert!(out.is_err(), "an overflowing sum must fail on {}, got {out:?}", database.label());
        database.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_run_replays_its_aggregates_without_the_database() {
    let database = TestDatabase::sqlite();
    let source = program(&database.settings());
    database.create_tables(&source).await;
    let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
    let trace_dir = tempfile::tempdir().expect("trace dir");
    let record = Runtime::builder().trace_to(trace_dir.path()).build();
    let recorded = run_ir_with_runtime(&ir, Some("main"), vec![], &record).await.expect("recorded run");
    let trace_path = record.tracer().path().to_path_buf();
    drop(record);
    database.finish().await;
    let replay = Runtime::builder().replay_from(&trace_path).build();
    let replayed = run_ir_with_runtime(&ir, Some("main"), vec![], &replay).await.expect("replayed run");
    assert_eq!(replayed, recorded);
}

fn refusal(body: &str) -> String {
    let source = format!(
        "{}\nagent probe() -> Int:\n    {body}\n    return 0\n",
        program("    engine: sqlite\n    location: \"ledger.sqlite\"")
    );
    match compile_to_ir(&source) {
        Ok(_) => panic!("must be refused: {body}"),
        Err(errors) => format!("{errors:#?}"),
    }
}

#[test]
fn aggregates_refuse_what_cannot_be_exact() {
    assert!(
        refusal("n = Item.all().order_by(fn (i) -> i.points).count()").contains("`order_by`, `descending` and `take` cannot change it"),
    );
    assert!(refusal("n = Item.all().sum(fn (i) -> i.list)").contains("`sum` adds an `Int`, `Decimal` or `Duration` column"));
    assert!(refusal("n = Item.all().sum(fn (i) -> i.points + 1)").contains("reads one column"));
}
