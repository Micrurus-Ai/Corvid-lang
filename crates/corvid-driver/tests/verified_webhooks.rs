//! Verified webhooks (B16): what the checker accepts in a route's
//! `verified <scheme>(...)` clause, and what the contract publishes.
//! The server half is `crates/corvid-cli/tests/serve_webhooks.rs`.

use corvid_driver::{compile_to_application_contract_with_config, compile_to_ir};

const PROGRAM: &str = r#"type PaymentEvent:
    id: String

traces: none

server payments:
    route POST "/webhooks/stripe" body PaymentEvent verified stripe(secret: "STRIPE_WEBHOOK_SECRET", within: 5 minutes) -> json String:
        return body.id
    route POST "/webhooks/github" body Bytes verified github(secret: "GITHUB_WEBHOOK_SECRET", replays: accepted) -> json Int:
        return body.length()
    route POST "/raw" body Bytes -> json Int:
        return body.length()
"#;

fn errors(source: &str) -> Vec<String> {
    match compile_to_ir(source) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

fn refused(clause: &str, needle: &str) {
    let source = PROGRAM.replace(
        r#"verified stripe(secret: "STRIPE_WEBHOOK_SECRET", within: 5 minutes)"#,
        clause,
    );
    let found = errors(&source);
    assert!(found.iter().any(|e| e.contains(needle)), "`{clause}` should be refused with `{needle}`: {found:#?}");
}

#[test]
fn a_verified_route_compiles_and_the_contract_marks_it() {
    assert!(errors(PROGRAM).is_empty(), "{:#?}", errors(PROGRAM));
    let contract = compile_to_application_contract_with_config(PROGRAM, "app.cor", "test", None).unwrap();
    let routes = serde_json::to_value(&contract.routes).unwrap();
    let route = |path: &str| routes.as_array().unwrap().iter().find(|r| r["path"] == path).unwrap().clone();
    let stripe = route("/webhooks/stripe");
    assert_eq!(stripe["verified"], serde_json::json!({ "scheme": "stripe", "signature_headers": ["Stripe-Signature"], "within_secs": 300 }));
    assert_eq!(route("/webhooks/github")["verified"]["signature_headers"], serde_json::json!(["X-Hub-Signature-256"]));
    assert!(route("/raw").get("verified").is_none(), "an ordinary route is not marked");
    assert_eq!(route("/raw")["request_media_type"], "application/octet-stream", "`body Bytes` is the raw body");
    assert!(!routes.to_string().contains("WEBHOOK_SECRET"), "the secret's name is never published: {routes}");
}

#[test]
fn the_replay_window_is_written_and_fits_the_scheme() {
    refused(r#"verified stripe(secret: "STRIPE_WEBHOOK_SECRET")"#, "no default window");
    refused(r#"verified stripe(secret: "STRIPE_WEBHOOK_SECRET", replays: accepted)"#, "signs a timestamp");
    refused(r#"verified github(secret: "STRIPE_WEBHOOK_SECRET", within: 5 minutes)"#, "cannot be checked");
    refused(r#"verified github(secret: "STRIPE_WEBHOOK_SECRET")"#, "replays: accepted");
    refused(r#"verified stripe(secret: "STRIPE_WEBHOOK_SECRET", within: 10 seconds)"#, "30 seconds");
    refused(r#"verified stripe(secret: "STRIPE_WEBHOOK_SECRET", within: 2 hours)"#, "1 hour");
}

#[test]
fn an_unknown_scheme_an_empty_secret_and_a_missing_secret_are_refused() {
    refused(r#"verified shopify(secret: "KEY", within: 5 minutes)"#, "`standard_webhooks`");
    refused(r#"verified stripe(secret: "", within: 5 minutes)"#, "`secret:` is empty");
    refused(r#"verified stripe(within: 5 minutes)"#, "without a `secret`");
}

/// Narrowed for now: only `corvid serve` checks the signature, so native
/// code refuses a verified route's handler by name instead of compiling
/// it for a host that would run it unchecked.
#[test]
fn native_code_refuses_a_verified_route_rather_than_run_it_unchecked() {
    let ir = compile_to_ir(PROGRAM).unwrap_or_else(|e| panic!("{e:#?}"));
    let reason = corvid_driver::native_ability(&ir).expect_err("a verified route is not native yet").to_string();
    assert!(reason.contains("/webhooks/stripe") && reason.contains("verified"), "{reason}");
}

#[test]
fn a_verified_route_takes_no_sign_in_and_has_a_body_to_sign() {
    let with_requires = PROGRAM.replace("-> json String:", "-> json String requires authenticated:");
    assert!(errors(&with_requires).iter().any(|e| e.contains("takes no `requires`")), "{:#?}", errors(&with_requires));
    let on_get = PROGRAM.replace(
        r#"route POST "/webhooks/github" body Bytes"#,
        r#"route GET "/webhooks/github""#,
    );
    assert!(errors(&on_get).iter().any(|e| e.contains("carries no body")), "{:#?}", errors(&on_get));
}
