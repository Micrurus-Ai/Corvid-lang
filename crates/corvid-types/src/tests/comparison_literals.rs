//! A side of a comparison written only as literals takes the other
//! side's type, as an annotated binding gives one its type: `Some(10)`
//! against an `Option<Decimal>`, a text literal against a `TimeZone` or
//! `Currency`, a list of them against a list.

use super::*;

fn errors(src: &str) -> Vec<String> {
    check(src).errors.iter().map(|e| e.message()).collect()
}

#[test]
fn a_literal_compares_with_the_type_on_the_other_side() {
    for src in [
        "agent f(price: Option<Decimal>) -> Bool:\n    return price == Some(10)\n",
        "agent f(price: Option<Decimal>) -> Bool:\n    return Some(10.50) != price\n",
        "agent f(price: Option<Decimal>) -> Bool:\n    return price == None\n",
        "agent f(home: TimeZone) -> Bool:\n    return home == \"Europe/Berlin\"\n",
        "agent f(code: Currency) -> Bool:\n    return \"USD\" != code\n",
        "agent f(homes: List<TimeZone>) -> Bool:\n    return homes == [\"Europe/Berlin\", \"UTC\"]\n",
        "agent f(found: Result<Decimal, String>) -> Bool:\n    return found == Ok(2.50)\n",
    ] {
        assert_eq!(errors(src), Vec::<String>::new(), "{src}");
    }
}

/// Re-reading the literal checks it as the other side's type would, so
/// a literal that is not one is an error that names it.
#[test]
fn a_literal_that_is_not_the_other_side_s_type_is_still_an_error() {
    let found = errors("agent f(home: TimeZone) -> Bool:\n    return home == \"Europe/Nowhere\"\n");
    assert!(found.iter().any(|m| m.contains("`Europe/Nowhere` is not a time zone")), "{found:?}");
    let found = errors("agent f(count: Option<Int>) -> Bool:\n    return count == Some(\"ten\")\n");
    assert!(!found.is_empty(), "a text literal is not an Int: {found:?}");
}

/// Only literals are re-read: a computed value keeps its own type.
#[test]
fn a_computed_side_is_not_re_read() {
    let found = errors("agent f(home: TimeZone, name: String) -> Bool:\n    return home == name\n");
    assert!(!found.is_empty(), "a String variable is not a TimeZone: {found:?}");
}

/// A secret and text never mix, literal or not.
#[test]
fn a_text_literal_never_compares_with_a_secret() {
    let found = errors("agent f(key: Secret) -> Bool:\n    return key == \"whsec_x\"\n");
    assert!(found.iter().any(|m| m.contains("`Secret` and `String`")), "{found:?}");
}
