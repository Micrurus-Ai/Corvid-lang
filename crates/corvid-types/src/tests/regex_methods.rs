//! B03 regular expressions: compile-time checks of pattern literals,
//! replacement templates and `split_regex`, and the `RegexMatch`
//! record's fields.

use super::check;
use crate::errors::TypeErrorKind;

fn kinds(src: &str) -> Vec<TypeErrorKind> {
    check(src).errors.into_iter().map(|e| e.kind).collect()
}

#[test]
fn regex_methods_typecheck_with_literal_patterns() {
    let checked = check(
        "agent main(text: String) -> Int:
    seen = text.contains_regex(\"[0-9]+\")
    first = text.find_regex(\"(?<year>\\\\d{4})-(\\\\d{2})\")
    every = text.find_all_regex(\"\\\\w+\")
    cleaned = text.replace_regex(\"\\\\s+\", \" \")
    dated = text.replace_regex(\"(?<year>\\\\d{4})-(\\\\d{2})\", \"$2/${year}\")
    parts = text.split_regex(\",\\\\s*\")
    total = every.length() + parts.length() + cleaned.length() + dated.length()
    return match first:
        Some(m) -> m.start + m.end + m.text.length() + m.groups.length() + total
        None -> 0
",
    );
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn an_invalid_pattern_is_a_compile_error_naming_the_problem() {
    let errors = kinds("agent main(text: String) -> Bool:\n    return text.contains_regex(\"(ab\")\n");
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::InvalidRegexPattern { method, problem }
            if method == "contains_regex" && problem == "unclosed group at position 0")),
        "got {errors:?}"
    );
    let message = TypeErrorKind::InvalidRegexPattern {
        method: "contains_regex".into(),
        problem: "unclosed group at position 0".into(),
    }
    .message();
    assert_eq!(message, "invalid regular expression in `contains_regex`: unclosed group at position 0");
}

#[test]
fn a_pattern_computed_at_run_time_is_refused() {
    let errors = kinds("agent main(text: String, pattern: String) -> Bool:\n    return text.contains_regex(pattern)\n");
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::RegexPatternNotLiteral { method } if method == "contains_regex")),
        "got {errors:?}"
    );
}

#[test]
fn a_literal_replacement_must_refer_to_existing_groups() {
    let errors = kinds("agent main(text: String) -> String:\n    return text.replace_regex(\"(x)\", \"$1a\")\n");
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::InvalidRegexReplacement { hint, .. }
            if hint.contains("write `${1}a`"))),
        "got {errors:?}"
    );
    let missing = kinds("agent main(text: String) -> String:\n    return text.replace_regex(\"x\", \"$1\")\n");
    assert!(
        missing.iter().any(|kind| matches!(kind, TypeErrorKind::InvalidRegexReplacement { problem, .. }
            if problem.contains("group 1") && problem.contains("only group 0"))),
        "got {missing:?}"
    );
}

#[test]
fn a_replacement_computed_at_run_time_is_not_checked_as_a_template() {
    let checked = check(
        "agent main(text: String, replacement: String) -> String:
    return text.replace_regex(\"x\", replacement)
",
    );
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn split_regex_refuses_a_pattern_that_can_match_empty_text() {
    let errors = kinds("agent main(text: String) -> List<String>:\n    return text.split_regex(\"\\\\s*\")\n");
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::RegexSplitMatchesEmpty { pattern } if pattern == "\\s*")),
        "got {errors:?}"
    );
}

#[test]
fn regex_match_fields_are_typed_and_unknown_fields_are_refused() {
    let errors = kinds(
        "agent main(text: String) -> Int:
    return match text.find_regex(\"x\"):
        Some(m) -> m.position
        None -> 0
",
    );
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::UnknownField { struct_name, field }
            if struct_name == "RegexMatch" && field == "position")),
        "got {errors:?}"
    );
    let annotated = check(
        "agent first_group(m: RegexMatch) -> Option<String>:
    return m.groups[1]
",
    );
    assert!(annotated.errors.is_empty(), "got {:?}", annotated.errors);
}

#[test]
fn regex_match_is_a_type_not_a_value() {
    let errors = kinds("agent main() -> Int:\n    m = RegexMatch\n    return 0\n");
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::TypeAsValue { name } if name == "RegexMatch")),
        "got {errors:?}"
    );
}

#[test]
fn regex_matches_have_no_json_form() {
    let errors = kinds(
        "agent main(text: String) -> String:
    return match text.find_regex(\"x\"):
        Some(m) -> m.to_json()
        None -> \"\"
",
    );
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::NoJsonForm { ty, .. } if ty == "RegexMatch")),
        "got {errors:?}"
    );
}
