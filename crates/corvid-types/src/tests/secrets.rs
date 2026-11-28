//! `Secret` in the checker: what a program may do with a secret
//! (compare two, pass one to a tool, hold one in a record) and every
//! refusal that keeps its text inside the program, with its message.

use super::check;
use crate::errors::{TypeError, TypeErrorKind};

const SECRET_TOOL: &str = "effect secrets_read:
    reversible: true

tool secret(name: String) -> Result<Secret, String> uses secrets_read
";

fn assert_clean(src: &str) {
    let checked = check(src);
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

/// The one error `src` produces that `pick` selects.
fn the_error(src: &str, pick: impl Fn(&TypeErrorKind) -> bool) -> TypeError {
    let errors = check(src).errors;
    let mut matching = errors.iter().filter(|e| pick(&e.kind));
    let error = matching.next().unwrap_or_else(|| panic!("no matching error; got {errors:?}")).clone();
    assert!(matching.next().is_none(), "more than one matching error: {errors:?}");
    error
}

fn message_and_hint(error: &TypeError) -> (String, String) {
    (error.kind.message(), error.kind.hint().unwrap_or_default())
}

#[test]
fn a_program_reads_compares_and_passes_secrets() {
    assert_clean(&format!(
        "{SECRET_TOOL}
type Credentials:
    user: String
    password: Secret

tool sign(key: Secret, body: String) -> String
tool log_in(credentials: Credentials) -> Bool

agent signature(body: String) -> Result<String, String>:
    key = secret(\"WEBHOOK_KEY\")?
    return Ok(sign(key, body))

agent same(a: Secret, b: Secret) -> Bool:
    return a == b and not (a != b)

agent enter(user: String) -> Result<Bool, String>:
    password = secret(\"PASSWORD\")?
    return Ok(log_in(Credentials(user, password)))

agent keys(a: Secret, b: Secret) -> List<Secret>:
    return [a, b]
"
    ));
}

#[test]
fn secrets_have_no_order() {
    for op in ["<", "<=", ">", ">="] {
        let error = the_error(
            &format!("agent compare(a: Secret, b: Secret) -> Bool:\n    return a {op} b\n"),
            |k| matches!(k, TypeErrorKind::SecretOperatorMismatch { .. }),
        );
        let (message, hint) = message_and_hint(&error);
        assert_eq!(message, "this operator does not combine `Secret` and `Secret`");
        assert!(hint.contains("no order") && hint.contains("constant-time"), "{op}: {hint}");
    }
}

#[test]
fn a_secret_never_mixes_with_text() {
    for body in ["a + \"x\"", "\"key=\" + a", "a == \"hunter2\"", "\"hunter2\" != a"] {
        let ret = if body.contains('+') { "String" } else { "Bool" };
        let error = the_error(
            &format!("agent mix(a: Secret) -> {ret}:\n    return {body}\n"),
            |k| matches!(k, TypeErrorKind::SecretOperatorMismatch { .. }),
        );
        let (_, hint) = message_and_hint(&error);
        assert!(hint.contains("do not mix"), "{body}: {hint}");
    }
}

#[test]
fn secrets_do_not_add_or_compute() {
    for op in ["+", "-", "*"] {
        let error = the_error(
            &format!("agent join(a: Secret, b: Secret) -> Secret:\n    return a {op} b\n"),
            |k| matches!(k, TypeErrorKind::SecretOperatorMismatch { .. }),
        );
        let (_, hint) = message_and_hint(&error);
        assert!(hint.contains("only `==` and `!=`"), "{op}: {hint}");
    }
}

#[test]
fn no_method_returns_a_secrets_text() {
    for method in ["to_string()", "to_json()", "length()", "to_upper()", "utf8()"] {
        let error = the_error(
            &format!("agent reveal(a: Secret) -> String:\n    x = a.{method}\n    return \"\"\n"),
            |k| matches!(k, TypeErrorKind::SecretMethod { .. }),
        );
        let (message, hint) = message_and_hint(&error);
        let name = method.trim_end_matches("()");
        assert_eq!(
            message,
            format!("a `Secret` has no method `{name}`: its text can never be read back or turned into other text")
        );
        assert!(hint.contains("`==`") && hint.contains("`<redacted>`"), "{hint}");
    }
}

#[test]
fn a_secret_is_not_a_string() {
    let errors = check("agent reveal(a: Secret) -> String:\n    return a\n").errors;
    assert!(errors.iter().any(|e| matches!(e.kind, TypeErrorKind::ReturnTypeMismatch { .. } | TypeErrorKind::TypeMismatch { .. })), "{errors:?}");
    let errors = check("tool sign(key: Secret) -> String\n\nagent call() -> String:\n    return sign(\"hunter2\")\n").errors;
    assert!(!errors.is_empty(), "a literal is not a Secret");
}

#[test]
fn to_json_and_from_json_refuse_a_record_holding_a_secret() {
    let source = "type Credentials:
    user: String
    password: Secret

agent encode(c: Credentials) -> String:
    return c.to_json()

agent decode(text: String) -> Bool:
    return match Credentials.from_json(text):
        Ok(c) -> true
        Err(e) -> false
";
    let errors = check(source).errors;
    let reasons: Vec<String> = errors
        .iter()
        .filter_map(|e| match &e.kind {
            TypeErrorKind::NoJsonForm { method, reason, .. } => Some(format!("{method}: {reason}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        vec![
            "to_json: field `Credentials.password`: a `Secret` has no JSON form, because its text must never leave the program".to_string(),
            "from_json: field `Credentials.password`: a `Secret` has no JSON form, because its text must never leave the program".to_string(),
        ],
        "{errors:?}"
    );
}

#[test]
fn routes_refuse_a_secret_in_their_inputs_and_response() {
    let source = "type Login:
    user: String
    password: Secret

server api:
    route POST \"/login\" body Login -> json Bool:
        return true
    route GET \"/key\" -> json Secret:
        return secret_value()

tool secret_value() -> Secret
";
    let errors = check(source).errors;
    let messages: Vec<String> = errors
        .iter()
        .filter(|e| matches!(e.kind, TypeErrorKind::SecretAtBoundary { .. }))
        .map(|e| e.kind.message())
        .collect();
    assert_eq!(
        messages,
        vec![
            "a route's path, query or body cannot hold a `Secret` in field `Login.password`: a secret's text never leaves the program".to_string(),
            "a route's response cannot hold a `Secret`: a secret's text never leaves the program".to_string(),
        ],
        "{errors:?}"
    );
    let hint = errors.iter().find(|e| matches!(e.kind, TypeErrorKind::SecretAtBoundary { .. })).unwrap().kind.hint().unwrap();
    assert!(hint.contains("`secret(name)`") && hint.contains("`Bool`"), "{hint}");
}

#[test]
fn a_store_refuses_a_secret_field() {
    let error = the_error(
        "session Sessions:\n    token: Secret\n",
        |k| matches!(k, TypeErrorKind::SecretAtBoundary { .. }),
    );
    assert_eq!(error.kind.message(), "a store field cannot hold a `Secret`: a secret's text never leaves the program");
}

#[test]
fn a_table_column_names_the_two_ways_to_keep_a_secret() {
    let error = the_error(
        "table Account:\n    key id: String\n    api_key: Secret\n",
        |k| matches!(k, TypeErrorKind::SecretTableColumn { .. }),
    );
    let (message, hint) = message_and_hint(&error);
    assert_eq!(
        message,
        "table column `api_key` cannot hold a `Secret`: a secret is stored only sealed, in a `Secret` or `Option<Secret>` column of a database that declares `encryption: key(secret(\"NAME\"))`"
    );
    assert!(hint.contains("encryption: key(secret("), "the sealed way: {hint}");
    assert!(hint.contains("`hash_password(value, owasp_2023())` from std/crypto"), "the hashed way: {hint}");
    assert!(hint.contains("`value.verify_password(hash)`"), "{hint}");
}

#[test]
fn no_secret_reaches_a_model() {
    let source = "type Credentials:
    user: String
    password: Secret

prompt summarize(c: Credentials) -> String:
    \"Summarize {c}\"

prompt invent() -> Secret:
    \"Make up a password\"

prompt with_tool(lookup: (Secret) -> String) -> String:
    \"Use the tool\"
";
    let errors = check(source).errors;
    let messages: Vec<String> = errors
        .iter()
        .filter(|e| matches!(e.kind, TypeErrorKind::SecretAtBoundary { .. }))
        .map(|e| e.kind.message())
        .collect();
    assert_eq!(
        messages,
        vec![
            "a prompt parameter cannot hold a `Secret` in field `Credentials.password`: a secret's text never leaves the program".to_string(),
            "a prompt's result cannot hold a `Secret`: a secret's text never leaves the program".to_string(),
            "a prompt parameter cannot hold a `Secret`: a secret's text never leaves the program".to_string(),
        ],
        "{errors:?}"
    );
}

#[test]
fn main_neither_takes_nor_returns_a_secret() {
    let error = the_error(
        &format!("{SECRET_TOOL}\nagent main() -> Result<Secret, String>:\n    return secret(\"KEY\")\n"),
        |k| matches!(k, TypeErrorKind::SecretAtBoundary { .. }),
    );
    let (message, hint) = message_and_hint(&error);
    assert_eq!(message, "the `main` agent's signature cannot hold a `Secret`: a secret's text never leaves the program");
    assert!(hint.contains("prints its result"), "{hint}");
    let error = the_error("agent main(key: Secret) -> Bool:\n    return true\n", |k| {
        matches!(k, TypeErrorKind::SecretAtBoundary { .. })
    });
    assert!(error.kind.message().contains("`main`"));
}

#[test]
fn ask_and_choose_refuse_secrets() {
    let ask = the_error("agent prompt_person() -> Secret:\n    return ask(\"password?\", Secret)\n", |k| {
        matches!(k, TypeErrorKind::SecretShownToPerson { .. })
    });
    assert_eq!(ask.kind.message(), "`ask` cannot answer with a `Secret`: a person's answer is recorded in the trace");
    let choose = the_error("agent pick(keys: List<Secret>) -> Secret:\n    return choose(keys)\n", |k| {
        matches!(k, TypeErrorKind::SecretShownToPerson { .. })
    });
    assert_eq!(choose.kind.message(), "`choose` cannot offer a `Secret`: its options are shown to a person");
}

#[test]
fn the_checker_records_every_type_reference_that_holds_a_secret() {
    let checked = check(
        "type Credentials:
    user: String
    password: Secret

type Account:
    name: String
    credentials: Credentials

agent visible(name: String) -> Int:
    return 1
",
    );
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let holding: Vec<String> = checked
        .secret_type_refs
        .iter()
        .map(|span| checked.type_refs[span].display_name())
        .collect();
    assert_eq!(holding.len(), 2, "the `password` and `credentials` fields: {holding:?}");
}

#[test]
fn a_connector_operation_takes_no_secret() {
    let error = the_error(
        "type Repo:
    name: String

connector github:
    timeout: 10s
    redirects: never
    published: no
    base_url: \"https://api.github.com\"
    modes: [real]
    operation create_repo(token: Secret) -> Repo:
        POST \"/repos\" body token
",
        |k| matches!(k, TypeErrorKind::SecretAtBoundary { .. }),
    );
    let (message, hint) = message_and_hint(&error);
    assert_eq!(
        message,
        "a connector operation's signature cannot hold a `Secret`: a secret's text never leaves the program"
    );
    assert!(hint.contains("`auth`"), "{hint}");
}

fn errors_of(src: &str) -> Vec<TypeError> {
    check(src).errors
}

/// User credentials C1: an operation's own `auth: bearer(token)`.
fn credential_connector(operation: &str) -> String {
    format!(
        "type Repo:
    name: String

connector github:
    timeout: 10s
    redirects: never
    published: no
    base_url: \"https://api.github.com\"
    modes: [real]
{operation}"
    )
}

#[test]
fn an_operation_takes_a_persons_token_for_its_own_auth() {
    let errors = errors_of(&credential_connector(
        "    operation my_repos(token: Secret, owner: String) -> Repo:
        GET \"/users/{owner}/repos\"
        auth: bearer(token)
",
    ));
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn an_operation_credential_must_be_one_secret_parameter_sent_only_by_auth() {
    let refusal = |operation: &str| {
        errors_of(&credential_connector(operation))
            .into_iter()
            .map(|e| e.kind.message())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let missing = refusal("    operation my_repos() -> Repo:\n        GET \"/user/repos\"\n        auth: bearer(token)\n");
    assert!(missing.contains("declares no parameter named `token`"), "{missing}");
    let text = refusal("    operation my_repos(token: String) -> Repo:\n        GET \"/user/repos\"\n        auth: bearer(token)\n");
    assert!(text.contains("`token` must be a `Secret`") && text.contains("it is a `String`"), "{text}");
    let twice = refusal("    operation my_repos(token: Secret) -> Repo:\n        GET \"/users/{token}/repos\"\n        auth: bearer(token)\n");
    assert!(twice.contains("only `auth:` may send it; it is also sent as a path segment"), "{twice}");
    // A second `Secret` that `auth:` does not name is still refused.
    let other = refusal("    operation my_repos(token: Secret, extra: Secret) -> Repo:\n        GET \"/user/repos\"\n        auth: bearer(token)\n");
    assert!(other.contains("a connector operation's signature cannot hold a `Secret`"), "{other}");
    assert!(!other.contains("`token`"), "the credential itself is allowed: {other}");
}

#[test]
fn an_operation_credential_is_refused_on_an_async_operation() {
    let errors = errors_of(&credential_connector(
        "    operation start(token: Secret) -> String dangerous:
        POST \"/generations\"
        auth: bearer(token)
        async:
            statuses: [queued, completed, failed]
            initial: queued
            terminal: [completed, failed]
            deadline: 600s
            deadline_target: failed
            idempotency: intent via header \"Idempotency-Key\"
            poll GET \"/generations\"
            every: adaptive
            on_protocol_change: refuse
            state queued:
                on queued -> queued
                on completed -> completed
                on failed -> failed
",
    ));
    let text = errors.iter().map(|e| e.kind.message()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("a person's token is never stored"), "{text}");
}
