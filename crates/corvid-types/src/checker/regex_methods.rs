//! Compile-time checks for the regular-expression methods (B03).
//!
//! The method table gives `contains_regex`, `find_regex`,
//! `find_all_regex`, `replace_regex` and `split_regex` their
//! signatures. This file checks what a signature cannot express: the
//! pattern is a string literal the regex parser accepts, a literal
//! replacement refers only to groups the pattern has, and `split_regex`
//! cannot split on empty matches. The checks call `corvid_text::regex`,
//! the code the runtime compiles patterns with, so a program that
//! passes cannot hit an invalid pattern at run time.

use super::Checker;
use crate::builtin_methods::{string_literal_argument, BuiltinMethodKind};
use crate::errors::{TypeError, TypeErrorKind};
use corvid_ast::Expr;

impl<'a> Checker<'a> {
    /// Check the pattern (and, for `replace_regex`, the replacement) of
    /// a regex method call whose arity already matched.
    pub(super) fn check_regex_arguments(&mut self, kind: BuiltinMethodKind, method: &str, args: &[Expr]) {
        let Some(pattern_arg) = args.first() else { return };
        let Some(pattern) = string_literal_argument(pattern_arg) else {
            self.errors.push(TypeError::new(
                TypeErrorKind::RegexPatternNotLiteral { method: method.to_string() },
                pattern_arg.span(),
            ));
            return;
        };
        let info = match corvid_text::regex::check_pattern(pattern) {
            Ok(info) => info,
            Err(problem) => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::InvalidRegexPattern { method: method.to_string(), problem },
                    pattern_arg.span(),
                ));
                return;
            }
        };
        match kind {
            BuiltinMethodKind::StringSplitRegex if info.can_match_empty => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::RegexSplitMatchesEmpty { pattern: pattern.to_string() },
                    pattern_arg.span(),
                ));
            }
            BuiltinMethodKind::StringReplaceRegex => {
                // A replacement computed at run time is inserted
                // verbatim, so only a literal is a template to check.
                let Some(replacement_arg) = args.get(1) else { return };
                let Some(template) = string_literal_argument(replacement_arg) else { return };
                if let Err(error) = corvid_text::regex::check_replacement(template, &info) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::InvalidRegexReplacement { problem: error.message, hint: error.hint },
                        replacement_arg.span(),
                    ));
                }
            }
            _ => {}
        }
    }
}
