//! A verified route's clause (verified webhooks, B16):
//! `verified <scheme>(secret: "<NAME>", within: <duration>)`.
//!
//! One module answers what the clause allows, so the checker's errors,
//! the contract's window and the server's verifier agree. The schemes
//! are the ones `corvid-connector-runtime`'s verifier implements; a test
//! in `corvid-cli`, which depends on both crates, holds the two lists
//! equal.
//!
//! The replay window is security policy, so it is always written: from
//! 30 seconds, the least a provider's clock and network delay need, to 1
//! hour, past which a recorded request stays usable for too long. A
//! scheme that signs no time cannot be given a window and must say
//! `replays: accepted` instead.

use corvid_ast::{WebhookReplay, WebhookVerification};

/// The shortest window: 30 seconds.
pub const SHORTEST_WINDOW_SECONDS: u64 = 30;
/// The longest window: 1 hour.
pub const LONGEST_WINDOW_SECONDS: u64 = 60 * 60;

/// A scheme a `verified` clause can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebhookScheme {
    pub name: &'static str,
    /// Whether the scheme signs a timestamp, so a window can be checked.
    pub signs_time: bool,
    /// The request headers the provider signs with, as providers spell
    /// them; the contract and OpenAPI document them.
    pub signature_headers: &'static [&'static str],
}

/// Every scheme, in the order error messages list them.
pub const SCHEMES: &[WebhookScheme] = &[
    WebhookScheme { name: "stripe", signs_time: true, signature_headers: &["Stripe-Signature"] },
    WebhookScheme {
        name: "standard_webhooks",
        signs_time: true,
        signature_headers: &["webhook-id", "webhook-timestamp", "webhook-signature"],
    },
    WebhookScheme { name: "slack", signs_time: true, signature_headers: &["X-Slack-Signature", "X-Slack-Request-Timestamp"] },
    WebhookScheme { name: "github", signs_time: false, signature_headers: &["X-Hub-Signature-256"] },
    WebhookScheme { name: "linear", signs_time: false, signature_headers: &["Linear-Signature"] },
];

/// The scheme named `name`, if there is one.
pub fn scheme(name: &str) -> Option<&'static WebhookScheme> {
    SCHEMES.iter().find(|scheme| scheme.name == name)
}

/// Whether `scheme` signs a timestamp, or `None` for an unknown scheme.
pub fn signs_time(name: &str) -> Option<bool> {
    scheme(name).map(|scheme| scheme.signs_time)
}

/// The scheme names, in the order error messages list them.
pub fn scheme_list() -> String {
    SCHEMES.iter().map(|scheme| format!("`{}`", scheme.name)).collect::<Vec<_>>().join(", ")
}

/// The clause's replay window in whole seconds (`None` for `replays:
/// accepted`), or why the clause is not usable.
pub fn window_seconds(clause: &WebhookVerification) -> Result<Option<u64>, String> {
    let scheme = clause.scheme.name.as_str();
    let Some(signs) = signs_time(scheme) else {
        return Err(format!("`verified {scheme}` names no known scheme; the schemes are {}", scheme_list()));
    };
    if clause.secret.trim().is_empty() {
        return Err("`secret:` is empty; name the secret the provider gave you, as in `secret: \"STRIPE_WEBHOOK_SECRET\"`".into());
    }
    match (&clause.replay, signs) {
        (None, true) => Err(format!(
            "`verified {scheme}` says how old a signed request may be with `within:`, as in `within: 5 minutes`; there is no default window"
        )),
        (None, false) => Err(format!(
            "`{scheme}` signs no timestamp, so a recorded request can be sent again; write `replays: accepted` to take that on"
        )),
        (Some(WebhookReplay::Accepted { .. }), true) => Err(format!(
            "`{scheme}` signs a timestamp, so replays are refused with `within:` instead of accepted, as in `within: 5 minutes`"
        )),
        (Some(WebhookReplay::Within { .. }), false) => Err(format!(
            "`{scheme}` signs no timestamp, so `within:` cannot be checked; write `replays: accepted` instead"
        )),
        (Some(WebhookReplay::Accepted { .. }), false) => Ok(None),
        (Some(WebhookReplay::Within { count, unit, .. }), true) => {
            let written = format!("{count} {unit}");
            let duration = crate::duration_literal_value(count, unit)?;
            let nanoseconds = duration.nanoseconds();
            if nanoseconds % 1_000_000_000 != 0 {
                return Err(format!("`within: {written}` is not a whole number of seconds"));
            }
            let seconds = u64::try_from(nanoseconds / 1_000_000_000).unwrap_or(u64::MAX);
            if !(SHORTEST_WINDOW_SECONDS..=LONGEST_WINDOW_SECONDS).contains(&seconds) {
                return Err(format!(
                    "`within: {written}` is outside the allowed range: a signed request may be from `30 seconds` to `1 hour` old"
                ));
            }
            Ok(Some(seconds))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::{Ident, Span};

    fn clause(scheme: &str, replay: Option<WebhookReplay>) -> WebhookVerification {
        let span = Span::new(0, 0);
        WebhookVerification { scheme: Ident::new(scheme, span), secret: "KEY".into(), replay, span }
    }

    fn within(count: &str, unit: &str) -> Option<WebhookReplay> {
        Some(WebhookReplay::Within { count: count.into(), unit: unit.into(), span: Span::new(0, 0) })
    }

    fn accepted() -> Option<WebhookReplay> {
        Some(WebhookReplay::Accepted { span: Span::new(0, 0) })
    }

    #[test]
    fn a_window_is_between_thirty_seconds_and_an_hour() {
        assert_eq!(window_seconds(&clause("stripe", within("5", "minutes"))), Ok(Some(300)));
        assert_eq!(window_seconds(&clause("slack", within("30", "seconds"))), Ok(Some(30)));
        assert_eq!(window_seconds(&clause("standard_webhooks", within("1", "hour"))), Ok(Some(3600)));
        assert!(window_seconds(&clause("stripe", within("29", "seconds"))).unwrap_err().contains("30 seconds"));
        assert!(window_seconds(&clause("stripe", within("61", "minutes"))).unwrap_err().contains("1 hour"));
    }

    #[test]
    fn a_scheme_takes_exactly_the_window_it_can_check() {
        assert_eq!(window_seconds(&clause("github", accepted())), Ok(None));
        assert!(window_seconds(&clause("stripe", None)).unwrap_err().contains("no default window"));
        assert!(window_seconds(&clause("stripe", accepted())).unwrap_err().contains("signs a timestamp"));
        assert!(window_seconds(&clause("github", None)).unwrap_err().contains("replays: accepted"));
        assert!(window_seconds(&clause("github", within("5", "minutes"))).unwrap_err().contains("cannot be checked"));
        assert!(window_seconds(&clause("discord", within("5", "minutes"))).unwrap_err().contains("`stripe`"));
    }
}
