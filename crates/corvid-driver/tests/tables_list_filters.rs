//! J3: `values.contains(row.column)` inside a `where` runs in the database
//! as `column IN (...)`, on SQLite and, when `CORVID_TEST_POSTGRES_URL` is
//! set, on Postgres. Each filter must keep exactly the rows the same test
//! keeps in Corvid over every row: a `None` in the list matches a missing
//! value as `==` does, an empty list matches nothing, and a `Decimal`
//! matches by value, so `10` finds `10.0`.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[path = "support/table_engines.rs"]
mod table_engines;
use table_engines::TestDatabase;

fn program(settings: &str) -> String {
    format!(
        r#"
database shop:
{settings}

table Board:
    key board: Int
    owner: String

table Card:
    key card: Int
    board: Int
    price: Decimal
    tag: Option<String>

agent put_board(board: Int, owner: String) -> Int:
    return match Board.insert(Board(board, owner)):
        Ok(_) -> 1
        Err(_) -> 0

agent put_card(card: Int, board: Int, price: Decimal, tag: Option<String>) -> Int:
    return match Card.insert(Card(card, board, price, tag)):
        Ok(_) -> 1
        Err(_) -> 0

agent stock() -> Int:
    boards = put_board(1, "ada") + put_board(2, "grace") + put_board(3, "ada")
    cards = put_card(10, 1, 10.0, Some("red")) + put_card(11, 2, 9.50, None) + put_card(12, 3, 1, Some("blue")) + put_card(13, 2, 10, Some("red")) + put_card(14, 9, 0.5, None)
    return boards + cards

fn numbers(rows: List<Card>) -> String:
    out = ""
    for row in rows:
        out = out + row.card.to_string() + ","
    return out

fn kept(rows: List<Card>, keep: (Card) -> Bool) -> String:
    out = ""
    for row in rows:
        if keep(row):
            out = out + row.card.to_string() + ","
    return out

agent main() -> String:
    stocked = stock()
    owners_boards = [1, 3]
    no_boards: List<Int> = []
    tags: List<Option<String>> = [Some("red"), None]
    prices: List<Decimal> = [10.00, 1.0]
    every = Card.all()
    return stocked.to_string() + " | " + numbers(Card.where(fn (c) -> owners_boards.contains(c.board))) + " | " + kept(every, fn (c) -> owners_boards.contains(c.board)) + " | " + numbers(Card.where(fn (c) -> no_boards.contains(c.board))) + " | " + kept(every, fn (c) -> no_boards.contains(c.board)) + " | " + numbers(Card.where(fn (c) -> tags.contains(c.tag))) + " | " + kept(every, fn (c) -> tags.contains(c.tag)) + " | " + numbers(Card.where(fn (c) -> prices.contains(c.price) and not owners_boards.contains(c.board))) + " | " + kept(every, fn (c) -> prices.contains(c.price) and not owners_boards.contains(c.board))
"#
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_list_filter_keeps_exactly_the_rows_the_same_test_keeps_in_corvid() {
    for database in TestDatabase::every_engine().await {
        let source = program(&database.settings());
        database.create_tables(&source).await;
        let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
        let runtime = Runtime::builder().build();
        let out = run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect("program runs");
        let Value::String(text) = out else { panic!("expected text, got {out:?}") };
        let parts: Vec<&str> = text.split(" | ").collect();
        let label = database.label();
        assert_eq!(parts[0], "8", "on {label}");
        for pair in parts[1..].chunks(2) {
            assert_eq!(pair[0], pair[1], "the database keeps what Corvid keeps on {label}: {text}");
        }
        assert_eq!(parts[1], "10,12,", "boards 1 and 3 on {label}");
        assert_eq!(parts[3], "", "an empty list keeps nothing on {label}");
        assert_eq!(parts[5], "10,11,13,14,", "`None` in the list matches a missing tag on {label}");
        assert_eq!(parts[7], "13,", "`10.00` matches the stored `10` by value, and `1.0` matches only card 12, on board 3, on {label}");
        database.finish().await;
    }
}

fn refusal(body: &str) -> String {
    let source = program("    engine: sqlite\n    location: \"shop.sqlite\"")
        + &format!("\nagent probe(word: String) -> List<Board>:\n    {body}\n");
    match compile_to_ir(&source) {
        Ok(_) => panic!("must be refused: {body}"),
        Err(errors) => format!("{errors:#?}"),
    }
}

#[test]
fn testing_whether_text_contains_a_column_is_refused() {
    let refused = refusal("return Board.where(fn (b) -> word.contains(b.owner))");
    assert!(refused.contains("only `values.contains(row.column)` on a `List` can run in the database"), "{refused}");
}
