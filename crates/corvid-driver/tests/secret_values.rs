//! End to end: a program imports `secret` from std/secrets, reads a
//! webhook key from the environment and passes it to a tool. The tool
//! receives the key's text; the recorded trace holds `<redacted>` in its
//! place and nowhere holds the text; and the trace replays, re-reading
//! the secret from the environment and matching the redacted argument.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{ProgrammaticApprover, Runtime};
use corvid_vm::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};

const SOURCE: &str = "
import \"./std/secrets\" use secret

tool sign_webhook(key: Secret, body: String) -> String

agent main() -> Result<String, String>:
    key = secret(\"WEBHOOK_KEY\")?
    same = secret(\"WEBHOOK_KEY\")?
    if key != same:
        return Err(\"a secret must equal itself\")
    return Ok(sign_webhook(key, \"order.paid\"))
";

const KEY: &str = "whsec_live_51HdEzKpQ7rT";

/// Stage a project whose `src/std/` vendors the modules the source
/// imports, so `import \"./std/secrets\"` resolves through the normal
/// module pipeline.
fn compile(source: &str) -> corvid_ir::IrFile {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("repo root");
    let project = tempfile::tempdir().expect("tempdir");
    let std_dir = project.path().join("src").join("std");
    std::fs::create_dir_all(&std_dir).unwrap();
    for file in ["effects.cor", "secrets.cor"] {
        std::fs::copy(repo.join("std").join(file), std_dir.join(file)).unwrap();
    }
    let main_path = project.path().join("src").join("main.cor");
    std::fs::write(&main_path, source).unwrap();
    compile_to_ir_with_config_at_path(source, &main_path, None).expect("the secrets program compiles")
}

fn ok_text(value: Value) -> String {
    match value {
        Value::ResultOk(inner) => match inner.get() {
            Value::String(s) => s.to_string(),
            other => panic!("expected Ok(String), got Ok({other:?})"),
        },
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_reaches_its_tool_and_never_its_trace() {
    std::env::set_var("WEBHOOK_KEY", KEY);
    let ir = compile(SOURCE);

    let received = Arc::new(Mutex::new(Vec::new()));
    let seen = received.clone();
    let trace_dir = tempfile::tempdir().unwrap();
    let record = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tool("sign_webhook", move |args| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(args.clone());
                let key = args[0].as_str().unwrap_or_default();
                Ok(serde_json::json!(format!("t=1,v1={}", key.len())))
            }
        })
        .trace_to(trace_dir.path())
        .build();
    let recorded = ok_text(run_ir_with_runtime(&ir, None, vec![], &record).await.expect("the recorded run"));
    assert_eq!(recorded, format!("t=1,v1={}", KEY.len()));
    assert_eq!(
        received.lock().unwrap().as_slice(),
        [vec![serde_json::json!(KEY), serde_json::json!("order.paid")]],
        "the tool receives the secret's text"
    );

    let trace_path = record.tracer().path().to_path_buf();
    drop(record);
    let trace = std::fs::read_to_string(&trace_path).expect("the trace file");
    assert!(!trace.contains(KEY), "the trace must never hold the secret:\n{trace}");
    assert!(!trace.contains("whsec_"), "not even part of it:\n{trace}");
    assert!(trace.contains(r#""args":["<redacted>","order.paid"]"#), "the tool call records <redacted>:\n{trace}");
    assert!(trace.contains(r#""ok":"<redacted>""#), "the secret read records <redacted>:\n{trace}");

    // Replay re-reads the secret from the environment, here a different
    // key, and still matches: the recorded and replayed arguments are
    // both `<redacted>` at the secret's position. The tool is not called;
    // its recorded result is substituted.
    std::env::set_var("WEBHOOK_KEY", "whsec_rotated_since_recording");
    let replay = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tool("sign_webhook", |_args| async move { panic!("replay must substitute the recorded result") })
        .replay_from(&trace_path)
        .build();
    let replayed = ok_text(run_ir_with_runtime(&ir, None, vec![], &replay).await.expect("the replayed run"));
    assert_eq!(replayed, recorded);
    std::env::set_var("WEBHOOK_KEY", KEY);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_whose_only_agent_returns_a_secret_does_not_run_as_a_program() {
    let ir = compile(
        "
effect secrets_read:
    reversible: true

tool secret(name: String) -> Result<Secret, String> uses secrets_read

agent load() -> Result<Secret, String>:
    return secret(\"WEBHOOK_KEY\")
",
    );
    let runtime = Runtime::builder().build();
    let error = run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect_err("refused");
    assert!(error.to_string().contains("returns a `Secret`, which would be printed"), "{error}");
}

/// A tool that echoes the secret it was given: the program receives the
/// real text, the trace records `<redacted>` marked as scrubbed, and a
/// replay substitutes the scrubbed form with a warning naming the tool.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_echoed_secret_is_scrubbed_in_the_trace_and_replay_warns() {
    std::env::set_var("ECHOED_WEBHOOK_KEY", KEY);
    let ir = compile(
        "
import \"./std/secrets\" use secret

tool echo_key(key: Secret) -> String

agent main() -> Result<Int, String>:
    key = secret(\"ECHOED_WEBHOOK_KEY\")?
    echoed = echo_key(key)
    return Ok(echoed.length())
",
    );
    let trace_dir = tempfile::tempdir().unwrap();
    let record = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tool("echo_key", |args| async move { Ok(serde_json::json!(format!("key={}", args[0].as_str().unwrap_or_default()))) })
        .trace_to(trace_dir.path())
        .build();
    let recorded = run_ir_with_runtime(&ir, None, vec![], &record).await.expect("the recorded run");
    let real_length = ("key=".len() + KEY.len()) as i64;
    assert!(
        matches!(&recorded, Value::ResultOk(inner) if inner.get() == Value::Int(real_length)),
        "the program receives the real echoed text: {recorded:?}"
    );
    let trace_path = record.tracer().path().to_path_buf();
    drop(record);
    let trace = std::fs::read_to_string(&trace_path).unwrap();
    assert!(!trace.contains(KEY), "{trace}");
    assert!(trace.contains(r#""result":{"__corvid_secret_scrubbed__":"key=<redacted>"}"#), "{trace}");

    let replay = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .replay_from(&trace_path)
        .build();
    let replayed = run_ir_with_runtime(&ir, None, vec![], &replay).await;
    let warnings = replay.replay_secret_scrub_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("`echo_key`") && warnings[0].contains("<redacted>"), "{}", warnings[0]);
    // The substituted result is the scrubbed text, so this run's result
    // (14) differs from the recording's (27), and replay reports the
    // divergence at the end of the run rather than passing silently.
    let error = replayed.expect_err("the replayed result differs from the recording").to_string();
    assert!(error.contains("run_completed") && error.contains("14"), "{error}");
}
