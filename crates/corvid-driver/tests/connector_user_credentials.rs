//! End to end: a connector operation with its own `auth: bearer(token)`
//! (user credentials C1). The program reads a person's token as a
//! `Secret` and passes it to the operation. The provider receives it as
//! the bearer credential in place of the connector's own key; the
//! recorded trace holds `<redacted>` in its place and nowhere holds the
//! token; and the trace replays without the token or the provider.

#[path = "support/std_project.rs"]
mod std_project;

use corvid_driver::run_ir_with_runtime;
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use corvid_vm::Value;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const USER_TOKEN_ENV: &str = "CORVID_TEST_USER_GH_TOKEN";
const USER_TOKEN: &str = "gho_person_7Hq2LmZ";

const SOURCE: &str = "
import \"./std/secrets\" use secret

effect http_read:
    cost: 1.0

type Repo:
    name: String

connector github:
    timeout: 10s
    redirects: never
    published: no
    base_url: \"http://api.example.com\"
    auth: bearer(secret(\"CORVID_TEST_APP_GH_TOKEN\"))
    modes: [real, replay]
    operation my_repo(token: Secret) -> Repo uses http_read:
        GET \"/user/repo\"
        auth: bearer(token)

agent main() -> Result<String, String>:
    token = secret(\"CORVID_TEST_USER_GH_TOKEN\")?
    found = my_repo(token)
    return Ok(found.name)
";

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
async fn an_operation_sends_the_persons_token_and_never_records_it() {
    let ir = std_project::compile_with_std(SOURCE, &["effects.cor", "secrets.cor"]);
    let server = MockServer::start().await;
    // Matches only with the person's token, not the connector's own key,
    // which is not even set.
    Mock::given(method("GET"))
        .and(path("/user/repo"))
        .and(header("authorization", format!("Bearer {USER_TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"name":"mine"}"#))
        .mount(&server)
        .await;
    std::env::set_var(USER_TOKEN_ENV, USER_TOKEN);
    let trace_dir = tempfile::tempdir().unwrap();
    let record = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_builder(
            reqwest::Client::builder().resolve("api.example.com", *server.address()),
        ))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .trace_to(trace_dir.path())
        .build();
    let recorded = ok_text(run_ir_with_runtime(&ir, None, vec![], &record).await.expect("the real run"));
    assert_eq!(recorded, "mine");

    let trace_path = record.tracer().path().to_path_buf();
    drop(record);
    drop(server);
    let trace = std::fs::read_to_string(&trace_path).expect("the trace file");
    assert!(!trace.contains(USER_TOKEN), "the trace must never hold the token:\n{trace}");
    assert!(!trace.contains("gho_"), "not even part of it:\n{trace}");
    assert!(trace.contains(r#""args":["<redacted>"]"#), "the call records <redacted>:\n{trace}");

    // Replay, with another token in the environment and no provider:
    // the recorded call matches at the redacted position.
    std::env::set_var(USER_TOKEN_ENV, "gho_someone_else");
    let replay = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .replay_from(&trace_path)
        .build();
    let replayed = ok_text(run_ir_with_runtime(&ir, None, vec![], &replay).await.expect("the replayed run"));
    assert_eq!(replayed, recorded);
    std::env::remove_var(USER_TOKEN_ENV);
}
