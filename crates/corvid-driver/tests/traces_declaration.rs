//! Lineage L3a: a served program declares whether its requests are
//! recorded. `traces: none` is required with a `server` block, declared
//! once, and `traces` stays an ordinary name everywhere else. A block that
//! records requests (L3b) says where and for how long.

use corvid_driver::compile_to_ir;

const SERVED: &str = r#"type Health:
    ok: Bool

traces: none

server api:
    route GET "/health" -> json Health:
        return Health(true)
"#;

fn errors(source: &str) -> Vec<String> {
    match compile_to_ir(source) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

#[test]
fn a_served_program_declares_traces_none_and_compiles() {
    assert!(errors(SERVED).is_empty(), "{:#?}", errors(SERVED));
}

#[test]
fn leaving_traces_out_of_a_served_program_is_refused_with_the_line_to_add() {
    let without = SERVED.replace("traces: none\n\n", "");
    assert!(
        errors(&without).iter().any(|e| e.contains("server `api`") && e.contains("`traces: none`")),
        "{:#?}",
        errors(&without)
    );
}

#[test]
fn traces_is_declared_once() {
    let twice = SERVED.replace("traces: none\n", "traces: none\n\ntraces: none\n");
    assert!(errors(&twice).iter().any(|e| e.contains("declared twice")), "{:#?}", errors(&twice));
}

fn recording(items: &str) -> String {
    SERVED.replace("traces: none\n", &format!("traces:\n{items}"))
}

#[test]
fn a_recording_block_says_where_and_for_how_long() {
    let full = recording("    location: env(\"TRACE_DIR\")\n    keep: 720 hours\n");
    assert!(errors(&full).is_empty(), "{:#?}", errors(&full));
    let no_keep = recording("    location: env(\"TRACE_DIR\")\n");
    assert!(errors(&no_keep).iter().any(|e| e.contains("no default")), "{:#?}", errors(&no_keep));
    let no_location = recording("    keep: 720 hours\n");
    assert!(errors(&no_location).iter().any(|e| e.contains("says where")), "{:#?}", errors(&no_location));
    let too_short = recording("    location: \"traces\"\n    keep: 30 minutes\n");
    assert!(errors(&too_short).iter().any(|e| e.contains("1 hour")), "{:#?}", errors(&too_short));
}

#[test]
fn traces_stays_an_ordinary_name_and_a_program_without_a_server_needs_no_block() {
    let source = r#"agent traces(count: Int) -> Int:
    traces = count + 1
    return traces

agent main() -> Int:
    return traces(1)
"#;
    assert!(errors(source).is_empty(), "{:#?}", errors(source));
}
