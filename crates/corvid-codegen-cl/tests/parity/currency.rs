//! `Currency`, `Locale` and `Decimal.format_currency` on the native
//! tier. Both tiers format with `corvid-currency` over the same CLDR
//! data, so they must write the same text, and an amount that would
//! need rounding must stop both with the same message.

use super::scalar_harness::{assert_parity_text, assert_parity_trap};

#[test]
fn money_is_written_the_way_each_locale_writes_it() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    amount: Decimal = 1234.5
    whole: Decimal = 1234
    debt: Decimal = -12.5
    dinar: Decimal = 1.250
    return amount.format_currency("USD", "en-US") + "|" + amount.format_currency("EUR", "de-DE") + "|" + amount.format_currency("CHF", "fr-CH") + "|" + whole.format_currency("JPY", "ja-JP") + "|" + debt.format_currency("USD", "en-US") + "|" + dinar.format_currency("KWD", "en-US")
"#,
    );
    assert_eq!(out, "$1,234.50|1.234,50\u{a0}€|1'234,50\u{a0}CHF|￥1,234|-$12.50|KWD\u{a0}1.250");
}

#[test]
fn run_time_currencies_and_locales_parse_and_compare() {
    let out = assert_parity_text(
        r#"
agent digits(c: Currency) -> String:
    return [c.minor_digits()].to_json()

agent currency(r: Result<Currency, String>) -> String:
    return match r:
        Ok(c) -> "ok:" + c.to_string() + "/" + digits(c)
        Err(e) -> "err:" + e

agent locale(r: Result<Locale, String>) -> String:
    return match r:
        Ok(l) -> "ok:" + l.to_string()
        Err(e) -> "err:" + e

agent formatted(amount: Decimal, code: String, tag: String) -> Result<String, String>:
    c = code.parse_currency()?
    l = tag.parse_locale()?
    return Ok(amount.format_currency(c, l))

agent shown(r: Result<String, String>) -> String:
    return match r:
        Ok(text) -> text
        Err(e) -> e

agent main() -> String:
    price: Decimal = 19.9
    euro: Currency = "EUR"
    german: Locale = "de-DE"
    same = match " EUR ".parse_currency():
        Ok(c) -> c == euro
        Err(e) -> false
    other = match "de-AT".parse_locale():
        Ok(l) -> l != german
        Err(e) -> false
    shown_text = match "en-us".parse_locale():
        Ok(l) -> price.format_currency(euro, l)
        Err(e) -> e
    return currency(" EUR ".parse_currency()) + "|" + currency("JPY".parse_currency()) + "|" + currency("EURO".parse_currency()) + "|" + locale("en-us".parse_locale()) + "|" + locale("xx-YY".parse_locale()) + "|" + [same, other].to_json() + "|" + shown_text + "|" + euro.to_string() + " " + german.to_string() + "|" + shown(formatted(price, "GBP", "en-GB")) + "|" + shown(formatted(price, "GBP", "xx"))
"#,
    );
    let parts: Vec<&str> = out.split('|').collect();
    assert_eq!(parts[0], "ok:EUR/[2]");
    assert_eq!(parts[1], "ok:JPY/[0]");
    assert_eq!(parts[2], "err:`EURO` is not an ISO 4217 currency code, such as USD or EUR");
    assert_eq!(parts[3], "ok:en-US");
    assert!(parts[4].starts_with("err:`xx-YY` is not a locale Corvid can format for: "), "{out}");
    assert_eq!(parts[5], "[true,true]");
    assert_eq!(parts[6], "€19.90");
    assert_eq!(parts[7], "EUR de-DE");
    assert_eq!(parts[8], "£19.90");
    assert!(parts[9].starts_with("`xx` is not a locale Corvid can format for: "), "{out}");
}

#[test]
fn minor_digits_round_an_amount_before_formatting() {
    let out = assert_parity_text(
        r#"
type Price:
    amount: Decimal
    currency: Currency
    locale: Locale

agent main() -> String:
    total: Decimal = 1.005
    yen: Currency = "JPY"
    dollars = Price(total.round_half_even(2), "USD", "en-US")
    same = Price(1.00, "USD", "en-US")
    text = dollars.to_json()
    back = match Price.from_json(text):
        Ok(p) -> p.amount.format_currency(p.currency, p.locale)
        Err(e) -> e.path
    return dollars.amount.format_currency(dollars.currency, dollars.locale) + " " + total.round_half_even(yen.minor_digits()).format_currency(yen, "ja-JP") + " " + [dollars == same].to_json() + " " + text + " " + back
"#,
    );
    assert_eq!(out, r#"$1.00 ￥1 [true] {"amount":"1.00","currency":"USD","locale":"en-US"} $1.00"#);
}

#[test]
fn formatting_never_rounds_and_traps_with_the_interpreter_message() {
    assert_parity_trap(
        r#"
agent main() -> String:
    total: Decimal = 1.005
    return total.format_currency("USD", "en-US")
"#,
        "1.005 has more digits after the point than USD shows (2); formatting never rounds, so round it first, such as with round_half_even(2)",
    );
}
