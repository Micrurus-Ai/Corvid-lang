//! Which types a value written as text in a request can have: a path
//! segment (`/boards/{board: Int}`) or a query field (`?status=Open`).
//!
//! The checker refuses any other type in those places, and the server
//! reads the text through the public JSON decoder with this shape, so
//! the two can never disagree: a type is accepted here exactly when the
//! server reads it exactly.

use crate::types::Type;

/// How a type is written as one piece of request text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextShape {
    /// An `Int`: an optional `-` and digits, within range.
    Int,
    /// A finite `Float`.
    Float,
    /// `true` or `false`.
    Bool,
    /// Any text.
    String,
    /// A type whose public JSON is its exact text: `Decimal`,
    /// `Duration`, `Instant`, `TimeZone`, `ZonedTime`, `Currency`,
    /// `Locale` or `Bytes`, read by that type's own parser.
    Exact,
    /// A sum type whose variants all have no fields, written by the
    /// variant's name (`Open`).
    PlainVariant,
}

/// How `ty` is written as one piece of request text, or `None` when it
/// cannot be. `plain_variants` says whether a declared type is a sum
/// type whose variants all have no fields; the checker answers from the
/// declarations and the server from the IR, about the same program.
pub fn text_shape(ty: &Type, plain_variants: &dyn Fn(&Type) -> bool) -> Option<TextShape> {
    Some(match ty {
        Type::Int => TextShape::Int,
        Type::Float => TextShape::Float,
        Type::Bool => TextShape::Bool,
        Type::String => TextShape::String,
        Type::Decimal
        | Type::Duration
        | Type::Instant
        | Type::TimeZone
        | Type::ZonedTime
        | Type::Currency
        | Type::Locale
        | Type::Bytes => TextShape::Exact,
        Type::Struct(_) | Type::ImportedStruct(_) if plain_variants(ty) => TextShape::PlainVariant,
        _ => return None,
    })
}

/// How a query field of type `ty` is read: one piece of text, an
/// `Option` of one (absent is `None`), or a `List` of one (the name
/// repeated, absent is the empty list). `None` when it cannot be.
pub fn query_field_shape(ty: &Type, plain_variants: &dyn Fn(&Type) -> bool) -> Option<QueryFieldShape> {
    Some(match ty {
        Type::Option(inner) => QueryFieldShape::Optional(text_shape(inner, plain_variants)?),
        Type::List(inner) => QueryFieldShape::Repeated(text_shape(inner, plain_variants)?),
        other => QueryFieldShape::One(text_shape(other, plain_variants)?),
    })
}

/// How a query field is read from the query string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryFieldShape {
    /// Exactly one value; a missing one is refused.
    One(TextShape),
    /// At most one value; a missing one is `None`.
    Optional(TextShape),
    /// Every value given under the name, in order; none is `[]`.
    Repeated(TextShape),
}

/// The types a request text can have, for an error message.
pub const TEXT_TYPES: &str =
    "`Int`, `Float`, `Bool`, `String`, `Decimal`, `Duration`, `Instant`, `TimeZone`, `ZonedTime`, `Currency`, `Locale`, `Bytes`, or a type of plain variants";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_have_a_text_shape_and_containers_do_not() {
        let none = |_: &Type| false;
        assert_eq!(text_shape(&Type::Int, &none), Some(TextShape::Int));
        assert_eq!(text_shape(&Type::Decimal, &none), Some(TextShape::Exact));
        assert_eq!(text_shape(&Type::List(Box::new(Type::Int)), &none), None, "a path segment is one value");
        assert_eq!(text_shape(&Type::Option(Box::new(Type::Int)), &none), None);
    }

    #[test]
    fn a_query_field_may_be_optional_or_repeated() {
        let none = |_: &Type| false;
        assert_eq!(query_field_shape(&Type::Option(Box::new(Type::Instant)), &none), Some(QueryFieldShape::Optional(TextShape::Exact)));
        assert_eq!(query_field_shape(&Type::List(Box::new(Type::Int)), &none), Some(QueryFieldShape::Repeated(TextShape::Int)));
        assert_eq!(query_field_shape(&Type::List(Box::new(Type::List(Box::new(Type::Int)))), &none), None);
    }
}
