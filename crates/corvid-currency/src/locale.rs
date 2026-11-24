//! `Locale`: a BCP 47 locale whose language has CLDR data.

use crate::error::CurrencyError;
use icu_locale::{LocaleExpander, TransformResult};
use icu_locale_core::LanguageIdentifier;
use std::fmt;

/// A locale such as `de-DE` or `fr-CH`. Its language must have CLDR
/// data, so a tag is never silently formatted with the root locale's
/// conventions; a region or script without its own data falls back
/// within that language, as CLDR intends (`de-LU` formats as German).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Locale {
    locale: icu_locale_core::Locale,
}

impl Locale {
    pub fn parse(tag: &str) -> Result<Locale, CurrencyError> {
        let unknown = |reason: &str| CurrencyError::UnknownLocale { tag: tag.to_string(), reason: reason.to_string() };
        let locale: icu_locale_core::Locale = tag
            .parse()
            .map_err(|_| unknown("it is not a well-formed BCP 47 tag such as en-US"))?;
        let mut language = LanguageIdentifier::from(locale.id.language);
        // CLDR's likely subtags fill in a script and a region for every
        // language it has data for, and leave an unknown one unchanged.
        if language.language.is_unknown()
            || LocaleExpander::new_common().maximize(&mut language) == TransformResult::Unmodified
        {
            return Err(unknown("CLDR has no data for its language"));
        }
        Ok(Locale { locale })
    }

    pub(crate) fn cldr(&self) -> &icu_locale_core::Locale {
        &self.locale
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.locale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_with_cldr_languages_parse() {
        for tag in ["en-US", "de-DE", "de-LU", "fr-CH", "sw-KE", "yo-NG", "zh-Hant-TW", "ar-EG", "ja"] {
            assert!(Locale::parse(tag).is_ok(), "{tag}");
        }
        assert_eq!(Locale::parse("en-us").unwrap().to_string(), "en-US", "tags are canonicalized");
    }

    #[test]
    fn unknown_languages_and_malformed_tags_are_refused() {
        let reason = |tag: &str| match Locale::parse(tag) {
            Err(CurrencyError::UnknownLocale { reason, .. }) => reason,
            other => panic!("{tag}: {other:?}"),
        };
        assert_eq!(reason("tlh"), "CLDR has no data for its language");
        assert_eq!(reason("und"), "CLDR has no data for its language");
        assert_eq!(reason("en_US!"), "it is not a well-formed BCP 47 tag such as en-US");
    }
}
