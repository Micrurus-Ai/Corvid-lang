//! B06 currency formatting in the checker: `Currency` and `Locale`
//! literals checked against CLDR, and the formatting signature.

use super::check;
use crate::errors::TypeErrorKind;

fn kinds(src: &str) -> Vec<TypeErrorKind> {
    check(src).errors.into_iter().map(|e| e.kind).collect()
}

#[test]
fn currency_and_locale_literals_typecheck_where_they_are_meant() {
    let checked = check(
        "agent main(total: Decimal) -> String:
    euro: Currency = \"EUR\"
    home: Locale = \"de-DE\"
    places = euro.minor_digits()
    shown = total.round_half_even(places).format_currency(euro, home)
    return shown + total.format_currency(\"USD\", \"en-US\") + euro.to_string() + home.to_string()
",
    );
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn unknown_codes_and_languages_are_compile_errors() {
    let errors = kinds("agent main(x: Decimal) -> String:\n    return x.format_currency(\"EURO\", \"tlh\")\n");
    let problems: Vec<_> = errors
        .iter()
        .filter_map(|kind| match kind {
            TypeErrorKind::UnknownCurrencyOrLocale { problem } => Some(problem.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        problems,
        [
            "`EURO` is not an ISO 4217 currency code, such as USD or EUR",
            "`tlh` is not a locale Corvid can format for: CLDR has no data for its language"
        ],
        "got {errors:?}"
    );
}

#[test]
fn run_time_codes_go_through_a_result() {
    let errors = kinds("agent main(x: Decimal, code: String) -> String:\n    return x.format_currency(code, \"en-US\")\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::TypeMismatch { .. })), "a String is not a Currency: {errors:?}");
    let checked = check(
        "agent main(x: Decimal, code: String, tag: String) -> Bool:\n    return code.parse_currency().is_ok() and tag.parse_locale().is_ok()\n",
    );
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn only_decimals_format_as_money() {
    let errors = kinds("agent main(x: Float) -> String:\n    return x.format_currency(\"USD\", \"en-US\")\n");
    assert!(!errors.is_empty(), "a Float has no format_currency");
}
