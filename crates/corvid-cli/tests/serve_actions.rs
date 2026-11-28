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

const ASSISTED: &str = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        editor: "board.edit"
        admin: "board.admin"

action tag_card(card: Int, label: String, by: Caller) -> String requires permission("board.edit"):
    assistants: confirm "Add a label to a card."
    return by.id + " tagged " + card.to_string() + " " + label

action wipe_board(board: Int) -> Int requires permission("board.edit"):
    return board

action audit(board: Int) -> Int requires permission("board.admin"):
    assistants: confirm "Audit a board."
    return board
"#;

fn mcp(app: &ServedApp, body: Value, headers: &[(String, String)]) -> (u16, Value) {
    post_json(app, "/mcp", &body, headers)
}

#[test]
fn an_assistant_sees_and_calls_only_what_its_person_may_and_the_person_confirms() {
    let mut editor = Vec::new();
    let mut viewer = Vec::new();
    let app = serve(ASSISTED, |data| {
        editor = seed_actor(data, "editor-1", Some("editor"));
        viewer = seed_actor(data, "viewer-1", None);
    });
    let call = |name: &str, arguments: Value| json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": name, "arguments": arguments}});

    let (status, _) = mcp(&app, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}), &[]);
    assert_eq!(status, 401, "an assistant acts only inside a signed-in session");

    let (_, listed) = mcp(&app, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}), &editor);
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["tag_card"], "only offered actions this person may call: {listed}");
    let schema = &listed["result"]["tools"][0]["inputSchema"];
    assert_eq!(schema["required"], json!(["card", "label"]), "the caller is never input: {schema}");

    for hidden in ["wipe_board", "audit"] {
        let (_, reply) = mcp(&app, call(hidden, json!({"board": 1})), &editor);
        assert_eq!(reply["error"]["code"], -32602, "`{hidden}` is unknown, never refused: {reply}");
    }
    let (_, refused) = mcp(&app, call("tag_card", json!({"card": "x", "label": "urgent"})), &editor);
    assert_eq!(refused["result"]["isError"], true, "{refused}");

    let (_, proposed) = mcp(&app, call("tag_card", json!({"card": 3, "label": "urgent"})), &editor);
    let pending = &proposed["result"]["structuredContent"];
    assert_eq!(pending["status"], "confirmation_required", "the action does not run yet: {proposed}");
    let confirm = pending["confirm"].as_str().unwrap().to_string();

    let (status, _) = post_json(&app, &confirm, &json!({}), &viewer);
    assert_eq!(status, 403, "only the person the assistant acts for confirms");
    let (status, result) = post_json(&app, &confirm, &json!({}), &editor);
    assert_eq!((status, &result), (200, &json!("editor-1 tagged 3 urgent")), "confirming runs it as that person");
    let (status, _) = post_json(&app, &confirm, &json!({}), &editor);
    assert_eq!(status, 404, "a confirmation runs once");

    let (_, proposed) = mcp(&app, call("tag_card", json!({"card": 4, "label": "later"})), &editor);
    let decline = proposed["result"]["structuredContent"]["decline"].as_str().unwrap().to_string();
    let (status, declined) = post_json(&app, &decline, &json!({}), &editor);
    assert_eq!((status, declined["status"].as_str()), (200, Some("declined")));
}

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

/// The raw reply to a request: status line, headers and body.
fn raw_request(port: u16, path: &str, body: &str, headers: &[(String, String)]) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connects");
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut head = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    write!(stream, "{head}\r\n{body}").unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    raw
}

/// B10-2b-5 — a plain browser form posted to an action. It carries the
/// CSRF token in the reserved `corvid_csrf` field, checked by the same
/// double-submit rule as the header, and gets a page back: `303` to the
/// page it came from on success, or a page naming the refusal.
#[test]
fn a_browser_form_carries_its_token_in_a_field_and_returns_to_its_page() {
    let mut editor = Vec::new();
    let app = serve(GUARDED, |data| editor = seed_actor(data, "ana", Some("editor")));
    let cookie: Vec<(String, String)> = editor.iter().filter(|(name, _)| name == "Cookie").cloned().collect();
    let csrf = editor.iter().find(|(name, _)| name == "X-CSRF-Token").unwrap().1.clone();
    let browser = |referer: &str| {
        let mut headers = cookie.clone();
        headers.push(("Accept".into(), "text/html,application/xhtml+xml".into()));
        headers.push(("Referer".into(), referer.into()));
        headers
    };

    let done = raw_request(app.port, "/actions/archive", &format!("card=3&corvid_csrf={csrf}"), &browser("http://localhost/board/7?tab=open"));
    assert!(done.starts_with("HTTP/1.1 303"), "success returns to the page: {done}");
    assert!(done.to_ascii_lowercase().contains("location: /board/7?tab=open"), "{done}");

    let elsewhere = raw_request(app.port, "/actions/archive", &format!("card=3&corvid_csrf={csrf}"), &browser("https://evil.example/phish"));
    assert!(elsewhere.to_ascii_lowercase().contains("location: /\r\n"), "another site is never a destination: {elsewhere}");

    let no_token = raw_request(app.port, "/actions/archive", "card=3&corvid_csrf=", &browser("http://localhost/board"));
    assert!(no_token.starts_with("HTTP/1.1 403"), "an empty field fails closed: {no_token}");
    assert!(no_token.contains("archive did not run") && no_token.contains("href=\"/board\""), "a page with a way back: {no_token}");

    let refused = raw_request(app.port, "/actions/archive", &format!("card=three&corvid_csrf={csrf}"), &browser("http://localhost/board"));
    assert!(refused.starts_with("HTTP/1.1 400") && refused.contains("card"), "the page names the refused field: {refused}");

    // A form that does not ask for a page gets the action's own answer.
    let (status, body) = request(app.port, "POST", "/actions/archive", "application/x-www-form-urlencoded", &format!("card=3&corvid_csrf={csrf}"), &cookie)
        .expect("request completes");
    assert_eq!((status, body), (200, json!(3)));
}

const ALLOWED: &str = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        editor: "board.edit"
    delegation: up_to 8 hours

action pin_card(card: Int, by: Caller) -> String requires permission("board.edit"):
    assistants: allow "Pin a card to the top of its column."
    return by.id + " pinned " + card.to_string()

action tag_card(card: Int, label: String, by: Caller) -> String requires permission("board.edit"):
    assistants: confirm "Add a label to a card."
    return by.id + " tagged " + card.to_string() + " " + label
"#;

/// B11-1 — `assistants: allow`: inside the person's own session an
/// assistant runs the action without asking each time, as that person,
/// while a `confirm` action still waits for them.
#[test]
fn an_allowed_action_runs_in_the_persons_session_without_confirmation() {
    let mut editor = Vec::new();
    let mut viewer = Vec::new();
    let app = serve(ALLOWED, |data| {
        editor = seed_actor(data, "editor-1", Some("editor"));
        viewer = seed_actor(data, "viewer-1", None);
    });
    let call = |name: &str, arguments: Value| json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": name, "arguments": arguments}});

    let (_, ran) = mcp(&app, call("pin_card", json!({"card": 3})), &editor);
    let done = &ran["result"]["structuredContent"];
    assert_eq!(done["status"], "done", "no confirmation is asked: {ran}");
    assert_eq!(done["result"], "editor-1 pinned 3", "it runs as the person: {ran}");

    let (_, waits) = mcp(&app, call("tag_card", json!({"card": 3, "label": "urgent"})), &editor);
    assert_eq!(waits["result"]["structuredContent"]["status"], "confirmation_required", "`confirm` keeps its meaning: {waits}");

    let (_, unknown) = mcp(&app, call("pin_card", json!({"card": 3})), &viewer);
    assert_eq!(unknown["error"]["code"], -32602, "allow never widens what the person may do: {unknown}");
}

/// B11-2b and B11-3 — a person grants an assistant authority, and the
/// assistant acts with the grant's bearer token: only where the grant, the
/// source and the person agree, and never after the grant is revoked.
#[test]
fn a_grant_lets_an_assistant_act_for_its_person_until_it_is_revoked() {
    let mut editor = Vec::new();
    let app = serve(ALLOWED, |data| editor = seed_actor(data, "editor-1", Some("editor")));
    let grant = |body: Value| post_json(&app, "/__delegations", &body, &editor);

    let (status, beyond) = grant(json!({"label": "Desk assistant", "actions": [{"action": "pin_card", "mode": "allow"}], "expires_in_secs": 9 * 3600}));
    assert_eq!((status, beyond["error"].as_str()), (400, Some("expiry_beyond_cap")), "the cap is `up_to 8 hours`: {beyond}");
    let (status, widened) = grant(json!({"label": "Desk assistant", "actions": [{"action": "tag_card", "mode": "allow"}], "expires_in_secs": 3600}));
    assert_eq!((status, widened["error"].as_str()), (400, Some("mode_beyond_source")), "a grant cannot widen the source: {widened}");

    let (status, made) = grant(json!({
        "label": "Desk assistant",
        "actions": [{"action": "pin_card", "mode": "allow"}, {"action": "tag_card", "mode": "confirm"}],
        "expires_in_secs": 3600,
    }));
    assert_eq!(status, 201, "{made}");
    let token = made["token"].as_str().expect("the token is shown once").to_string();
    let id = made["id"].as_str().unwrap().to_string();
    let bearer = vec![("Authorization".to_string(), format!("Bearer {token}"))];

    // On `/mcp`, with the grant alone: no session cookie, no CSRF.
    let (_, listed) = mcp(&app, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}), &bearer);
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["pin_card", "tag_card"], "{listed}");
    let call = |name: &str, arguments: Value| json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
    let (_, ran) = mcp(&app, call("pin_card", json!({"card": 5})), &bearer);
    assert_eq!(ran["result"]["structuredContent"]["result"], "editor-1 pinned 5", "it runs as the person: {ran}");
    let (_, waits) = mcp(&app, call("tag_card", json!({"card": 5, "label": "x"})), &bearer);
    assert_eq!(waits["result"]["structuredContent"]["status"], "confirmation_required", "{waits}");

    // On `/actions/<name>`: `allow` runs, a narrower mode points to `/mcp`.
    let (status, body) = post_json(&app, "/actions/pin_card", &json!({"card": 6}), &bearer);
    assert_eq!((status, body), (200, json!("editor-1 pinned 6")));
    let (status, body) = post_json(&app, "/actions/tag_card", &json!({"card": 6, "label": "x"}), &bearer);
    assert_eq!((status, body["error"].as_str()), (403, Some("confirmation_required")), "{body}");

    // The person lists and revokes it; the token stops at once.
    let (status, mine) = request(app.port, "GET", "/__delegations", "application/json", "", &editor).expect("lists");
    assert_eq!(status, 200);
    assert_eq!(mine["delegations"][0]["status"], "live", "{mine}");
    assert!(mine.to_string().find(&token).is_none(), "a token is never shown again: {mine}");
    let (status, _) = request(app.port, "DELETE", &format!("/__delegations/{id}"), "application/json", "", &editor).expect("revokes");
    assert_eq!(status, 204);
    let (status, _) = post_json(&app, "/actions/pin_card", &json!({"card": 7}), &bearer);
    assert_eq!(status, 401, "a revoked grant authorizes nothing");
}

#[test]
fn without_a_delegation_line_no_grant_can_be_made() {
    let mut editor = Vec::new();
    let app = serve(GUARDED, |data| editor = seed_actor(data, "ana", Some("editor")));
    let (status, _) = post_json(&app, "/__delegations", &json!({"label": "x", "actions": [], "expires_in_secs": 60}), &editor);
    assert_eq!(status, 404, "the grant routes are not served");
}
