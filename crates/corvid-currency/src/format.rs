//! Formatting a `Decimal` amount in a currency and locale.

use crate::currency::Currency;
use crate::error::CurrencyError;
use crate::locale::Locale;
use corvid_decimal::Decimal;
use icu_experimental::dimension::currency::formatter::CurrencyFormatter;

/// `amount` in `currency`, written the way `locale` writes money:
/// `$1,234.50` in en-US, `1.234,50 €` in de-DE. An amount with fewer
/// fractional digits than the currency shows is padded; one with more
/// is an error, because formatting never rounds.
pub fn format(amount: &Decimal, currency: Currency, locale: &Locale) -> Result<String, CurrencyError> {
    let digits = currency.minor_digits();
    if amount.scale() > digits {
        return Err(CurrencyError::TooManyDigits { amount: amount.to_string(), code: currency.to_string(), digits });
    }
    let value: fixed_decimal::Decimal = amount.to_string().parse().expect("Decimal text is a valid fixed decimal");
    let formatter = CurrencyFormatter::try_new_symbol(locale.cldr().into(), currency.cldr(), Default::default())
        .expect("CLDR currency data is compiled in for every locale Locale::parse accepts");
    let text = formatter.format_fixed_decimal(&value).to_string();
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formatted(amount: &str, code: &str, tag: &str) -> Result<String, CurrencyError> {
        format(&Decimal::parse(amount).unwrap(), Currency::parse(code).unwrap(), &Locale::parse(tag).unwrap())
    }

    #[test]
    fn each_locale_writes_money_its_own_way() {
        assert_eq!(formatted("1234.5", "USD", "en-US").unwrap(), "$1,234.50");
        assert_eq!(formatted("1234.5", "EUR", "de-DE").unwrap(), "1.234,50\u{a0}€");
        assert_eq!(formatted("1234.50", "CHF", "fr-CH").unwrap(), "1'234,50\u{a0}CHF");
        assert_eq!(formatted("1234", "JPY", "ja-JP").unwrap(), "￥1,234");
        assert_eq!(formatted("-12.5", "USD", "en-US").unwrap(), "-$12.50");
        assert_eq!(formatted("1.250", "KWD", "en-US").unwrap(), "KWD\u{a0}1.250");
    }

    #[test]
    fn formatting_never_rounds() {
        assert_eq!(
            formatted("1.005", "USD", "en-US"),
            Err(CurrencyError::TooManyDigits { amount: "1.005".into(), code: "USD".into(), digits: 2 })
        );
        let error = formatted("1234.5", "JPY", "ja-JP").unwrap_err().to_string();
        assert!(error.contains("round_half_even(0)"), "{error}");
    }
}
