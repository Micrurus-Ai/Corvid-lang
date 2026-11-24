//! `Currency`: an ISO 4217 currency from the bundled CLDR data.

use crate::error::CurrencyError;
use icu_experimental::dimension::currency::CurrencyType;
use icu_experimental::dimension::provider::currency::displayname::CurrencyDisplaynameV1;
use icu_experimental::dimension::provider::currency::fractions::CurrencyFractionsV1;
use icu_provider::prelude::*;
use std::fmt;

/// An ISO 4217 currency, such as `EUR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Currency {
    code: CurrencyType,
    /// The three ASCII letters, kept for display.
    letters: [u8; 3],
}

impl Currency {
    /// Accepts an uppercase ISO 4217 code that CLDR names. A currency
    /// CLDR has no name for is refused rather than formatted with a
    /// guessed symbol.
    pub fn parse(code: &str) -> Result<Currency, CurrencyError> {
        let unknown = || CurrencyError::UnknownCurrency { code: code.to_string() };
        if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(unknown());
        }
        let currency = CurrencyType::try_from_str(code).map_err(|_| unknown())?;
        let attributes = DataMarkerAttributes::try_from_str(code).map_err(|_| unknown())?;
        let english = DataLocale::from(&icu_locale_core::locale!("en"));
        let named: Result<DataResponse<CurrencyDisplaynameV1>, _> = icu_experimental::provider::Baked.load(DataRequest {
            id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &english),
            ..Default::default()
        });
        let letters = [code.as_bytes()[0], code.as_bytes()[1], code.as_bytes()[2]];
        named.map(|_| Currency { code: currency, letters }).map_err(|_| unknown())
    }

    /// Corvid's `text.parse_currency()`: trims whitespace and on
    /// failure says what an ISO 4217 code looks like. Both tiers call
    /// this, so they return the same `Err`.
    pub fn parse_user_text(text: &str) -> Result<Currency, String> {
        Currency::parse(text.trim()).map_err(|error| error.to_string())
    }

    pub fn code(&self) -> &str {
        std::str::from_utf8(&self.letters).expect("validated ASCII letters")
    }

    pub(crate) fn cldr(&self) -> CurrencyType {
        self.code
    }

    /// The digits after the point the currency shows: 2 for USD, 0 for
    /// JPY, 3 for KWD.
    pub fn minor_digits(&self) -> u8 {
        let fractions: DataPayload<CurrencyFractionsV1> = icu_experimental::provider::Baked
            .load(Default::default())
            .expect("CLDR currency fractions are compiled in")
            .payload;
        fractions.get().resolve(self.code).digits
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_text_is_trimmed_and_its_error_names_the_code() {
        assert_eq!(Currency::parse_user_text(" EUR ").unwrap().code(), "EUR");
        assert_eq!(Currency::parse_user_text("EURO").unwrap_err(), "`EURO` is not an ISO 4217 currency code, such as USD or EUR");
    }

    #[test]
    fn known_codes_parse_with_their_minor_digits() {
        assert_eq!(Currency::parse("USD").unwrap().minor_digits(), 2);
        assert_eq!(Currency::parse("JPY").unwrap().minor_digits(), 0);
        assert_eq!(Currency::parse("KWD").unwrap().minor_digits(), 3);
        assert_eq!(Currency::parse("EUR").unwrap().to_string(), "EUR");
    }

    #[test]
    fn unknown_codes_are_refused() {
        for code in ["XYZ", "usd", "US", "EURO", ""] {
            assert_eq!(Currency::parse(code), Err(CurrencyError::UnknownCurrency { code: code.into() }), "{code}");
        }
    }
}
