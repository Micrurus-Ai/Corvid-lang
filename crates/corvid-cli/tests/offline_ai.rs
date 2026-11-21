//! Test source-level and provider-level mocks, then replay the provider exchanges offline.
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn recorded_tool_conversation_and_source_mock_run_offline() {
    let project = tempfile::tempdir().unwrap();
    let source = r#"fn double(n: Int) -> Int:
    return n + n

prompt answer(question: String, calculate: (Int) -> Int) -> String:
    "Question: {question}"

mock answer(question: String, calculate: (Int) -> Int) -> String:
    return "source mock " + question

agent main() -> String:
    return answer("price", double)

test uses_source_mock:
    assert main() == "source mock price"

test uses_recording from_trace "recording.jsonl":
    result = main()
    assert result == "recorded answer" with confidence 1.0 over 2 runs
    assert called answer
"#;
    let main = project.path().join("main.cor");
    fs::write(&main, source).unwrap();
    let command = || {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_corvid"));
        cmd.current_dir(project.path());
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("CORVID_") || name.ends_with("API_KEY") {
                cmd.env_remove(key);
            }
        }
        cmd
    };
    let replies = json!({"answer":[{"role":"assistant","tool_calls":[{
        "id":"call_1","type":"function","function":{"name":"calculate","arguments":"{\"n\":21}"}
    }]},"recorded answer"]});
    let run = command()
        .args(["run", "main.cor"])
        .env("CORVID_MODEL", "offline")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_REPLIES", replies.to_string())
        .output()
        .unwrap();
    assert!(
        run.status.success() && String::from_utf8_lossy(&run.stdout).contains("recorded answer"),
        "{run:?}"
    );
    let trace = fs::read_dir(project.path().join("target/trace"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .unwrap();
    let data = fs::read_to_string(&trace).unwrap();
    let events: Vec<serde_json::Value> = data
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let calls: Vec<_> = events.iter().filter(|e| e["kind"] == "llm_call").collect();
    assert_eq!(calls.len(), 2, "{data}");
    let exchange = calls[1]["args"].as_array().unwrap().last().unwrap();
    assert_eq!(exchange["tools"][0]["function"]["name"], "calculate");
    assert_eq!(exchange["history"][0]["tool_calls"][0]["id"], "call_1");
    assert_eq!(exchange["history"][1], json!({
        "role": "tool", "tool_call_id": "call_1", "content": "42"
    }));
    fs::copy(trace, project.path().join("recording.jsonl")).unwrap();
    let replay = command()
        .args(["replay", "recording.jsonl", "--source", "main.cor"])
        .output()
        .unwrap();
    assert!(replay.status.success(), "{replay:?}");
    let test = || command().args(["test", "main.cor"]).output().unwrap();
    for _ in 0..2 {
        let output = test();
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("2 passed"),
            "{output:?}"
        );
    }
    fs::write(
        &main,
        source.replace("answer(\"price\", double)", "answer(\"changed\", double)"),
    )
    .unwrap();
    let different = test();
    assert!(!different.status.success(), "{different:?}");
    assert!(
        !String::from_utf8_lossy(&different.stdout).contains("PASS uses_recording"),
        "{different:?}"
    );
}
