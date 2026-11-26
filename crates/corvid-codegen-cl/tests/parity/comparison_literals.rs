//! A comparison's literal side takes the other side's type (the checker's
//! literal re-read), and both tiers compare the values alike.

use super::assert_parity_bool;

#[test]
fn an_optional_decimal_compares_with_a_literal_option() {
    assert_parity_bool(
        "agent f() -> Bool:\n    price: Option<Decimal> = Some(10.50)\n    return price == Some(10.5) and price != Some(10) and Some(2.50) == Some(2.5)\n",
        true,
    );
}

#[test]
fn a_time_zone_and_a_currency_compare_with_text_literals() {
    assert_parity_bool(
        "agent f() -> Bool:\n    home: TimeZone = \"Europe/Berlin\"\n    money: Currency = \"EUR\"\n    return home == \"Europe/Berlin\" and money != \"USD\" and home != \"UTC\"\n",
        true,
    );
}
