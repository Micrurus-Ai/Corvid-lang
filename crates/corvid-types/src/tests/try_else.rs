//! `try ... else` in the checker: any body type, a fallback of the
//! body's type, refused inside `atomic:` and around a connector
//! operation whose `on failure` already returns its outages.

use super::check;
use crate::errors::TypeErrorKind;

fn kinds(src: &str) -> Vec<TypeErrorKind> {
    check(src).errors.into_iter().map(|e| e.kind).collect()
}

#[test]
fn a_fallback_of_the_bodys_type_checks_for_any_type() {
    let checked = check(
        "tool lookup(term: String) -> List<String>

agent search(term: String) -> List<String>:
    return try lookup(term) else []

agent count(term: String) -> Int:
    return try lookup(term).length() timeout 2000 else 0
",
    );
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn a_fallback_of_another_type_is_refused() {
    let errors = kinds(
        "tool lookup(term: String) -> Int

agent search(term: String) -> Int:
    return try lookup(term) else \"none\"
",
    );
    assert!(
        errors.iter().any(|k| matches!(k, TypeErrorKind::TryElseFallbackType { expected, got }
            if expected == "Int" && got == "String")),
        "got {errors:?}"
    );
}

#[test]
fn try_else_inside_an_atomic_block_is_refused() {
    let errors = kinds(
        "database library:
    engine: sqlite
    location: \"library.sqlite\"

table Shelf:
    key shelf: Int
    title: String

tool note(text: String) -> Int

agent save(shelf: Int) -> Int:
    atomic:
        n = try note(\"saving\") else 0
    return shelf
",
    );
    assert!(
        errors.iter().any(|k| matches!(k, TypeErrorKind::AtomicBlockRefused { message } if message.contains("try ... else"))),
        "got {errors:?}"
    );
}

#[test]
fn try_else_around_an_operation_with_on_failure_is_refused() {
    let errors = kinds(
        "effect http_read:
    cost: 1.0

type Repo:
    name: String

type FetchError:
    | Unreachable(reason: String)

connector github:
    timeout: 10s
    redirects: never
    published: no
    base_url: \"https://api.github.com\"
    modes: [real]
    operation get_repo(name: String) -> Result<Repo, FetchError> uses http_read:
        GET \"/repos/{name}\"
        on failure -> Unreachable

agent fetch(name: String) -> Result<Repo, FetchError> uses http_read:
    return try get_repo(name) else Err(Unreachable(\"down\"))
",
    );
    assert!(
        errors.iter().any(|k| matches!(k, TypeErrorKind::TryElseAroundOnFailure { operation, variant }
            if operation == "get_repo" && variant == "Unreachable")),
        "got {errors:?}"
    );
}
