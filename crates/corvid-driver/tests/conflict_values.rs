//! B09 - `Conflict<Row>`, the value that says why a table write did not
//! happen: `Changed(current)`, `Deleted` or `Taken(holder)`. These pin
//! the type in a real program: building each case, matching on it, the
//! checker's coverage rule, and its public JSON form. How each table
//! operation returns only the cases it can produce is tested with the
//! tables.

use corvid_driver::{compile_to_ir, run_ir_with_runtime, Diagnostic};
use corvid_runtime::Runtime;
use corvid_vm::Value;

const TYPES: &str = "
type Member:
    id: Int
    email: String

agent describe(conflict: Conflict<Member>) -> String:
    return match conflict:
        Changed(current) -> \"changed, now \" + current.email
        Deleted -> \"deleted\"
        Taken(holder) -> \"taken by member \" + holder.id.to_string()
";

async fn run(body: &str) -> Value {
    let source = format!("{TYPES}\n{body}");
    let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("source must compile: {errors:#?}"));
    run_ir_with_runtime(&ir, None, vec![], &Runtime::builder().build()).await.expect("program must run")
}

fn compile_errors(body: &str) -> Vec<Diagnostic> {
    compile_to_ir(&format!("{TYPES}\n{body}")).expect_err("source must be refused")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_case_is_built_and_matched_by_name() {
    let out = run(r#"
agent main() -> String:
    ann = Member(1, "ann@example.com")
    return describe(Changed(ann)) + " | " + describe(Deleted) + " | " + describe(Taken(ann))
"#)
    .await;
    assert_eq!(out, Value::String("changed, now ann@example.com | deleted | taken by member 1".into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conflict_travels_inside_a_result_and_nests_in_a_pattern() {
    let out = run(r#"
agent store(email: String) -> Result<Member, Conflict<Member>>:
    if email == "":
        return Err(Deleted)
    return Err(Taken(Member(9, email)))

agent main() -> String:
    return match store("bo@example.com"):
        Ok(saved) -> "saved"
        Err(Taken(holder)) -> holder.email + " is taken"
        Err(Changed(current)) -> "changed"
        Err(Deleted) -> "gone"
"#)
    .await;
    assert_eq!(out, Value::String("bo@example.com is taken".into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conflict_crosses_json_as_a_sum_type() {
    let out = run(r#"
agent main() -> String:
    conflict: Conflict<Member> = Taken(Member(2, "cy@example.com"))
    gone: Conflict<Member> = Deleted
    return conflict.to_json() + " " + gone.to_json()
"#)
    .await;
    assert_eq!(
        out,
        Value::String(
            r#"{"fields":[{"email":"cy@example.com","id":2}],"tag":"variant","type":"Conflict","variant":"Taken"} {"fields":[],"tag":"variant","type":"Conflict","variant":"Deleted"}"#.into()
        )
    );
}

#[test]
fn a_match_on_a_written_conflict_covers_every_case() {
    let errors = compile_errors(
        r#"
agent half(conflict: Conflict<Member>) -> String:
    return match conflict:
        Changed(current) -> "changed"
        Deleted -> "deleted"
"#,
    );
    assert!(
        errors.iter().any(|e| e.message.contains("non-exhaustive match")),
        "a missing `Taken` arm is refused: {errors:#?}"
    );
}

#[test]
fn a_case_pattern_needs_a_conflict() {
    let errors = compile_errors(
        r#"
agent wrong(member: Option<Member>) -> String:
    return match member:
        Taken(holder) -> "taken"
        _ -> "other"
"#,
    );
    assert!(errors.iter().any(|e| e.message.contains("got `a `Taken` pattern`")), "{errors:#?}");
}

#[test]
fn deleted_carries_no_row() {
    let errors = compile_errors(
        r#"
agent wrong(member: Member) -> Conflict<Member>:
    return Deleted(member)
"#,
    );
    assert!(errors.iter().any(|e| e.message.contains("`Deleted`, which carries no row")), "{errors:#?}");
}

/// Each table write returns only the conflicts it can produce.
mod table_writes {
    use corvid_driver::compile_to_ir;

    const TABLES: &str = "
database library:
    engine: sqlite
    location: \"library.sqlite\"

table Shelf:
    key shelf: Int
    title: String

table Member:
    key id: Int
    email: String
    unique email
";

    fn errors(body: &str) -> Vec<String> {
        match compile_to_ir(&format!("{TABLES}\n{body}")) {
            Ok(_) => Vec::new(),
            Err(errors) => errors.into_iter().map(|e| e.message).collect(),
        }
    }

    #[test]
    fn an_insert_match_is_complete_with_taken_alone() {
        let found = errors(
            "agent add(row: Shelf) -> String:\n    return match Shelf.insert(row):\n        Ok(stored) -> \"stored\"\n        Err(Taken(holder)) -> holder.title\n",
        );
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn an_arm_an_insert_can_never_reach_is_refused() {
        let found = errors(
            "agent add(row: Shelf) -> String:\n    return match Shelf.insert(row):\n        Ok(stored) -> \"stored\"\n        Err(Changed(current)) -> \"changed\"\n        Err(Taken(holder)) -> holder.title\n",
        );
        assert!(
            found.iter().any(|m| m.contains("this conflict is never `Changed`: it can only be `Taken`")),
            "{found:#?}"
        );
    }

    /// A save keeps its key, so only a table with `unique` values can
    /// have a save another row's value blocks.
    #[test]
    fn a_save_is_taken_only_on_a_table_with_unique_values() {
        let without_unique = errors(
            "agent store(row: Shelf) -> String:\n    return match Shelf.save(row):\n        Ok(saved) -> \"saved\"\n        Err(Changed(current)) -> \"changed\"\n        Err(Deleted) -> \"gone\"\n",
        );
        assert!(without_unique.is_empty(), "{without_unique:#?}");
        let taken_arm = errors(
            "agent store(row: Shelf) -> String:\n    return match Shelf.save(row):\n        Ok(saved) -> \"saved\"\n        Err(Changed(current)) -> \"changed\"\n        Err(Deleted) -> \"gone\"\n        Err(Taken(holder)) -> \"taken\"\n",
        );
        assert!(taken_arm.iter().any(|m| m.contains("this conflict is never `Taken`")), "{taken_arm:#?}");
        let missing_taken = errors(
            "agent rename(row: Member) -> String:\n    return match Member.save(row):\n        Ok(saved) -> \"saved\"\n        Err(Changed(current)) -> \"changed\"\n        Err(Deleted) -> \"gone\"\n",
        );
        assert!(missing_taken.iter().any(|m| m.contains("non-exhaustive match")), "{missing_taken:#?}");
    }

    /// A conflict passes to a function taking any conflict.
    #[test]
    fn a_narrower_conflict_passes_where_any_is_expected() {
        let found = errors(
            "agent explain(conflict: Conflict<Shelf>) -> String:\n    return match conflict:\n        Changed(current) -> \"changed\"\n        Deleted -> \"gone\"\n        Taken(holder) -> \"taken\"\n\nagent add(row: Shelf) -> String:\n    return match Shelf.insert(row):\n        Ok(stored) -> \"stored\"\n        Err(conflict) -> explain(conflict)\n",
        );
        assert!(found.is_empty(), "{found:#?}");
    }
}
