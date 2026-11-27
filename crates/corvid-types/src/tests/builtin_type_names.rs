//! A program's own type under a built-in's name. Adding `Actor` as a
//! built-in once broke every reference app, which declared or imported
//! its own `Actor`, because the built-in won every lookup. Now the
//! program's type wins in its file, with a warning naming the built-in it
//! hides, so a built-in added later never changes a program's meaning. The
//! core scalars stay reserved.

use super::check;
use crate::errors::{TypeErrorKind, TypeWarningKind};
use crate::types::Type;

fn hidden(checked: &crate::Checked) -> Vec<String> {
    checked
        .warnings
        .iter()
        .filter_map(|w| match &w.kind {
            TypeWarningKind::BuiltinTypeHidden { name } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_programs_own_type_wins_over_a_later_built_in_of_its_name() {
    // `Caller` and `JsonError` are built-ins; this program declares both.
    let checked = check(
        "type Caller:
    nickname: String

type JsonError:
    reason: String

agent greet(who: Caller, problem: JsonError) -> String:
    return who.nickname + \": \" + problem.reason
",
    );
    assert!(checked.errors.is_empty(), "the program's fields are found: {:?}", checked.errors);
    assert_eq!(hidden(&checked), ["Caller", "JsonError"]);
    let warning = checked.warnings.iter().find(|w| matches!(w.kind, TypeWarningKind::BuiltinTypeHidden { .. })).unwrap();
    assert_eq!(warning.kind.code(), "W0301");
    assert!(warning.kind.hint().unwrap().contains("still the built-in"), "{:?}", warning.kind.hint());
}

#[test]
fn a_built_in_the_program_does_not_shadow_is_unchanged_and_silent() {
    let checked = check(
        "type Order:
    id: String

agent explain(problem: JsonError) -> String:
    return problem.path + \": \" + problem.message
",
    );
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    assert!(hidden(&checked).is_empty(), "{:?}", checked.warnings);
}

#[test]
fn every_core_scalar_name_is_refused_as_a_declaration() {
    for name in ["Int", "Float", "String", "Bool", "Nothing"] {
        let checked = check(&format!("type {name}:\n    value: Int\n"));
        let refusal = checked
            .errors
            .iter()
            .find(|e| matches!(&e.kind, TypeErrorKind::ReservedTypeName { name: n } if n == name))
            .unwrap_or_else(|| panic!("`type {name}` must be refused: {:?}", checked.errors));
        assert_eq!(refusal.kind.code(), "E0308");
    }
}

/// The rule itself: every built-in except the core scalars yields to a
/// program's own declaration, so the list of yielding names grows with
/// every new built-in without anyone having to add it here.
#[test]
fn only_the_core_scalars_are_reserved() {
    for name in [
        "Decimal", "Duration", "Instant", "TimeZone", "ZonedTime", "Currency", "Locale", "Bytes", "Secret",
        "DbHandle", "TraceId", "JsonValue", "JsonBuilder", "JsonError", "RegexMatch", "Caller",
    ] {
        assert!(Type::compiler_known(name).is_some(), "{name} is a built-in");
        assert_eq!(Type::builtin_named(name, true), None, "{name} yields to the program's own type");
        assert!(Type::builtin_named(name, false).is_some(), "{name} is the built-in otherwise");
    }
    for name in ["Int", "Float", "String", "Bool", "Nothing"] {
        assert!(Type::builtin_named(name, true).is_some(), "{name} always means the built-in");
    }
}
