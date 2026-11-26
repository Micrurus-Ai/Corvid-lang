//! B10-2 — `corvid serve` answers `POST /actions/<name>` for every
//! `action`. Each test starts a real server. The order is checked from
//! the outside: who may call comes before the input is read, and the
//! body never runs for a refused caller or a refused input.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const CSRF_SECRET: &str = "corvid-test-csrf-secret-0123456789";

struct ServedApp {
    child: Child,
    port: u16,
    _dir: tempfile::TempDir,
}

impl Drop for ServedApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Serve `source`, after `seed` has prepared the durable state directory.
fn serve(source: &str, seed: impl FnOnce(&Path)) -> ServedApp {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("main.cor");
    let data = dir.path().join("state");
    std::fs::create_dir_all(&data).unwrap();
    seed(&data);
    std::fs::write(&src, source).unwrap();
    let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let child = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .arg("serve")
        .arg(&src)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .env("CORVID_SERVE_DATA_DIR", &data)
        .env("CORVID_CSRF_SECRET", CSRF_SECRET)
        .env("CORVID_OAUTH_GOOGLE_CLIENT_ID", "test-client-id")
        .env("CORVID_OAUTH_GOOGLE_CLIENT_SECRET", "test-client-secret")
        .current_dir(dir.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let app = ServedApp { child, port, _dir: dir };
    let deadline = Instant::now() + Duration::from_secs(60);
    while request(app.port, "GET", "/healthz", "", "", &[]).is_none() {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    app
}

fn request(
    port: u16,
    method: &str,
    path: &str,
    content_type: &str,
    body: &str,
    headers: &[(String, String)],
) -> Option<(u16, Value)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    write!(stream, "{head}\r\n{body}").ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    let status = raw.lines().next()?.split_whitespace().nth(1)?.parse().ok()?;
    let reply = raw.split("\r\n\r\n").nth(1).unwrap_or("");
    Some((status, serde_json::from_str(reply).unwrap_or(Value::String(reply.to_string()))))
}

fn post_json(app: &ServedApp, path: &str, body: &Value, headers: &[(String, String)]) -> (u16, Value) {
    request(app.port, "POST", path, "application/json", &body.to_string(), headers).expect("request completes")
}

fn post_form(app: &ServedApp, path: &str, body: &str) -> (u16, Value) {
    request(app.port, "POST", path, "application/x-www-form-urlencoded", body, &[]).expect("request completes")
}

const BOARD: &str = r#"type Column:
    | Todo
    | Done

type MoveCard:
    card: Int
    to: Column
    position: Int where between(0, 500)

type Moved:
    card: Int
    position: Int

action move_card(move: MoveCard) -> Moved requires anyone:
    return Moved(move.card, move.position)

action rename(card: Int, title: String) -> String requires anyone:
    return title + " #" + card.to_string()

action ping() -> String requires anyone:
    return "pong"

action label(card: Int, labels: List<String>, note: Option<String>) -> String requires anyone:
    shown = match note:
        Some(text) -> text
        None -> "no note"
    return card.to_string() + ": " + labels.join(", ") + " (" + shown + ")"

action nudge(card: Int, move: MoveCard) -> Int requires anyone:
    return card + move.position
"#;

#[test]
fn an_action_without_a_server_block_is_served_and_reads_json_and_forms() {
    let app = serve(BOARD, |_| {});

    let (status, reply) = post_json(&app, "/actions/move_card", &json!({"card": 3, "to": {"tag": "variant", "type": "Column", "variant": "Done", "fields": []}, "position": 7}), &[]);
    assert_eq!((status, &reply), (200, &json!({"card": 3, "position": 7})), "one record is the whole input");

    let (status, reply) = post_form(&app, "/actions/move_card", "card=4&to=Todo&position=9");
    assert_eq!((status, &reply), (200, &json!({"card": 4, "position": 9})), "a form posts the record's fields");

    let (status, reply) = post_json(&app, "/actions/rename", &json!({"card": 3, "title": "Ship"}), &[]);
    assert_eq!((status, &reply), (200, &json!("Ship #3")), "several parameters are one field each");

    let (status, reply) = post_json(&app, "/actions/ping", &json!({}), &[]);
    assert_eq!((status, &reply), (200, &json!("pong")));
}

#[test]
fn an_input_the_types_refuse_is_a_400_naming_the_field() {
    let app = serve(BOARD, |_| {});

    let (status, reply) = post_form(&app, "/actions/move_card", "card=4&to=Todo&position=900");
    assert_eq!(status, 400, "{reply}");
    assert!(reply["detail"].as_str().unwrap().contains("position"), "the refinement names its field: {reply}");

    let (status, reply) = post_json(&app, "/actions/rename", &json!({"card": "three", "title": "Ship"}), &[]);
    assert_eq!((status, reply["parameter"].as_str()), (400, Some("card")), "{reply}");

    let (status, reply) = post_json(&app, "/actions/rename", &json!({"card": 3, "title": "Ship", "owner": "x"}), &[]);
    assert_eq!(status, 400, "an undeclared field is refused: {reply}");

    let (status, reply) = post_form(&app, "/actions/rename", "card=three&title=Ship");
    assert_eq!(status, 400, "a form field that does not read as its type: {reply}");
    assert!(reply["detail"].as_str().unwrap().contains("card"), "{reply}");

    let (status, reply) = post_form(&app, "/actions/nudge", "card=1&move=x");
    assert_eq!(status, 415, "a record parameter beside others cannot come from one form field: {reply}");
}

#[test]
fn a_form_posts_one_field_per_parameter() {
    let app = serve(BOARD, |_| {});
    let (status, reply) = post_form(&app, "/actions/rename", "card=3&title=Ship+it");
    assert_eq!((status, &reply), (200, &json!("Ship it #3")));

    let (status, reply) = post_form(&app, "/actions/label", "card=3&labels=urgent&labels=blocked");
    assert_eq!((status, &reply), (200, &json!("3: urgent, blocked (no note)")), "a repeated name fills a list; an optional may be left out");
    let (status, reply) = post_form(&app, "/actions/label", "card=3&note=soon");
    assert_eq!((status, &reply), (200, &json!("3:  (soon)")));
}

fn seed_actor(data: &Path, actor: &str, role: Option<&str>) -> Vec<(String, String)> {
    use corvid_runtime::{mint_csrf_token, AuthActor, SessionAuthRuntime, SessionCreate};
    let auth = SessionAuthRuntime::open(data.join("auth.sqlite")).unwrap();
    auth.upsert_actor(AuthActor {
        id: actor.into(),
        tenant_id: "public".into(),
        display_name: actor.into(),
        actor_kind: "user".into(),
        auth_method: "oauth".into(),
        assurance_level: "aal1".into(),
        role_fingerprint: String::new(),
        permission_fingerprint: String::new(),
        created_ms: 0,
        updated_ms: 0,
    })
    .unwrap();
    if let Some(role) = role {
        auth.grant_actor_role(actor, role, 1).unwrap();
    }
    let token = format!("token-{actor}");
    let binding = format!("csrf-{actor}");
    auth.create_session(SessionCreate {
        id: format!("session-{actor}"),
        actor_id: actor.into(),
        tenant_id: "public".into(),
        raw_token: token.clone(),
        issued_ms: 1,
        expires_ms: 9_000_000_000_000,
        csrf_binding_id: binding.clone(),
    })
    .unwrap();
    let csrf = mint_csrf_token(&binding, CSRF_SECRET.as_bytes()).unwrap();
    vec![
        ("Cookie".to_string(), format!("corvid_session={token}; corvid_csrf={csrf}")),
        ("X-CSRF-Token".to_string(), csrf),
    ]
}

const GUARDED: &str = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        editor: "board.edit"

action archive(card: Int) -> Int requires permission("board.edit"):
    return card

action claim(card: Int, by: Caller) -> String requires authenticated:
    return by.tenant + "/" + by.id + " claims " + card.to_string()

action retire(card: Int, by: Caller) -> Int requires permission("board.edit"):
    return card

server board:
    route POST "/cards/{card: Int}/retire" -> json Int requires authenticated:
        return retire(path.card, actor)
"#;

const APPROVED: &str = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        editor: "board.edit"
        reviewer: "approvals.decide"

@approval(role: "reviewer", risk: "board_change", data: "internal", expires_ms: 900000, max_cost_usd: $1.0, irreversible: false)
action wipe_column(column: Int) -> Int requires permission("board.edit"):
    approve WipeColumn(column)
    return column
"#;

#[test]
fn an_action_that_reaches_approve_queues_it_for_the_declared_reviewer() {
    let mut editor = Vec::new();
    let mut reviewer = Vec::new();
    let app = serve(APPROVED, |data| {
        editor = seed_actor(data, "editor-1", Some("editor"));
        reviewer = seed_actor(data, "reviewer-1", Some("reviewer"));
    });
    let (status, queued) = post_json(&app, "/actions/wipe_column", &json!({"column": 2}), &editor);
    assert_eq!(status, 202, "the approve queues instead of being denied: {queued}");
    let id = queued["approval_id"].as_str().expect("an approval id").to_string();

    let (status, decided) =
        request(app.port, "POST", &format!("/__approvals/{id}/approve"), "application/json", "", &reviewer).unwrap();
    assert_eq!(status, 200, "{decided}");
    assert_eq!(decided["result"], json!(2), "approving runs the action: {decided}");

    let (_, contract) = request(app.port, "GET", "/.well-known/corvid", "application/json", "", &[]).unwrap();
    assert_eq!(contract["actions"][0]["approval"]["role"], "reviewer");
}

#[test]
fn a_route_cannot_lend_its_caller_to_an_action_that_requires_more() {
    let mut editor = Vec::new();
    let mut viewer = Vec::new();
    let app = serve(GUARDED, |data| {
        editor = seed_actor(data, "editor-1", Some("editor"));
        viewer = seed_actor(data, "viewer-1", None);
    });
    let (status, reply) = post_json(&app, "/cards/5/retire", &json!({}), &viewer);
    assert_eq!((status, reply["error"].as_str()), (403, Some("insufficient_authority")), "{reply}");
    let (status, reply) = post_json(&app, "/cards/5/retire", &json!({}), &editor);
    assert_eq!((status, &reply), (200, &json!(5)));
}

#[test]
fn an_action_reads_its_verified_caller_and_a_sent_one_is_refused() {
    let mut editor = Vec::new();
    let app = serve(GUARDED, |data| editor = seed_actor(data, "editor-1", Some("editor")));

    let (status, reply) = post_json(&app, "/actions/claim", &json!({"card": 3}), &editor);
    assert_eq!((status, &reply), (200, &json!("public/editor-1 claims 3")));

    let forged = json!({"card": 3, "by": {"id": "someone-else", "tenant": "other"}});
    let (status, reply) = post_json(&app, "/actions/claim", &forged, &editor);
    assert_eq!(status, 400, "the caller is never read from the input: {reply}");
}

#[test]
fn who_may_call_is_checked_before_the_input_is_read() {
    let mut editor = Vec::new();
    let mut viewer = Vec::new();
    let app = serve(GUARDED, |data| {
        editor = seed_actor(data, "editor-1", Some("editor"));
        viewer = seed_actor(data, "viewer-1", None);
    });

    let (status, _) = post_json(&app, "/actions/archive", &json!({"card": "not a number"}), &[]);
    assert_eq!(status, 401, "an anonymous caller is refused before its bad input is even read");

    let (status, _) = post_json(&app, "/actions/archive", &json!({"card": 3}), &viewer);
    assert_eq!(status, 403, "a caller without the permission is refused");

    let without_csrf: Vec<(String, String)> = editor.iter().filter(|(name, _)| name == "Cookie").cloned().collect();
    let (status, _) = post_json(&app, "/actions/archive", &json!({"card": 3}), &without_csrf);
    assert_eq!(status, 403, "a cookie session must send the CSRF token");

    let (status, reply) = post_json(&app, "/actions/archive", &json!({"card": 3}), &editor);
    assert_eq!((status, &reply), (200, &json!(3)));
}

#[test]
fn openapi_publishes_each_action_with_its_input_and_caller() {
    let app = serve(GUARDED, |_| {});
    let (_, spec) = request(app.port, "GET", "/openapi.json", "application/json", "", &[]).unwrap();
    let archive = &spec["paths"]["/actions/archive"]["post"];
    let input = &archive["requestBody"]["content"]["application/json"]["schema"];
    assert_eq!(input["required"], json!(["card"]), "{archive}");
    assert_eq!(input["additionalProperties"], json!(false));
    assert_eq!(archive["security"], json!([{"corvidSession": ["permission:board.edit"]}]));
    let (_, contract) = request(app.port, "GET", "/.well-known/corvid", "application/json", "", &[]).unwrap();
    assert_eq!(contract["actions"][0]["name"], "archive");
}
