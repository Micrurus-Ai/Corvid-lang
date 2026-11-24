//! Compile-time checks for the arguments of the HMAC methods and of
//! `Secret.decode_base64_after`.
//!
//! The method table gives each parameter one type. This file checks
//! what that cannot express:
//!
//! - **An HMAC key is `Bytes` or a `Secret`.** Both key the same HMAC
//!   (a secret with its bytes, which never become a value), so the two
//!   spellings a provider's documentation implies are one method.
//!   `String` is refused: a key typed into text is one step from a log.
//! - **The prefix of `decode_base64_after` is a string literal.** A
//!   prefix computed at run time could test the secret's text one guess
//!   at a time; a literal states only the provider's public format.

use super::Checker;
use crate::builtin_methods::{is_hmac_key_type, string_literal_argument};
use crate::errors::{TypeError, TypeErrorKind};
use corvid_ast::Expr;

impl<'a> Checker<'a> {
    /// Checks the key of `bytes.hmac_sha256(key)` or
    /// `bytes.hmac_sha512(key)`, whose arity already matched.
    pub(super) fn check_hmac_key(&mut self, method: &str, key: &Expr) {
        let got = self.check_expr(key);
        if !is_hmac_key_type(&got) {
            self.errors.push(TypeError::new(
                TypeErrorKind::HmacKeyType { method: method.to_string(), got: got.display_name() },
                key.span(),
            ));
        }
    }

    /// Checks that the prefix of `secret.decode_base64_after(prefix)` is
    /// a string literal.
    pub(super) fn check_secret_key_prefix(&mut self, args: &[Expr]) {
        let Some(prefix) = args.first() else { return };
        if string_literal_argument(prefix).is_none() {
            self.errors.push(TypeError::new(TypeErrorKind::SecretKeyPrefixNotLiteral, prefix.span()));
        }
    }
}
