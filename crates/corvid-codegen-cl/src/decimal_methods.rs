//! The `Decimal` operations native code calls (B06): which runtime
//! symbol runs each operator and method, and its call shape.
//!
//! Native code holds a `Decimal` as a String containing its exact
//! canonical text. Every operation is one call into
//! `corvid-runtime/src/ffi_bridge/decimal.rs`, which parses the
//! operands, computes with `corvid-decimal` (the code the interpreter
//! calls) and returns canonical text. The import declarations, the
//! lowering, the tier picker and native match arms all read this table,
//! so an operation is native everywhere or nowhere. A misspelled symbol
//! fails to link.

use corvid_ast::BinaryOp;
use corvid_decimal::RoundingMode;
use corvid_types::BuiltinMethodKind;

/// One runtime entry point for `Decimal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecimalOperation {
    Add,
    Sub,
    Mul,
    Rem,
    Negate,
    /// Numeric order as -1, 0 or 1; every comparison operator.
    Compare,
    /// The exact widening of an `Int`: `n.to_decimal()` and every
    /// `Int` operand of a decimal operator.
    FromInt,
    FromFloat,
    ToFloat,
    ToIntTruncated,
    Scale,
    Abs,
    Round(RoundingMode),
    /// `(a / b).round_<mode>(places)`, fused by IR lowering.
    DivideRound(RoundingMode),
    /// `text.parse_decimal()`, filling a `Result<Decimal, String>`.
    ParseText,
}

/// A Cranelift parameter or return width at a decimal entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecimalSlot {
    /// A `Decimal` or `String` pointer, an `Int`, or a Result payload slot.
    Word,
    /// A `Float`.
    Float,
}

impl DecimalOperation {
    /// Every operation, for declaring the imports.
    pub fn all() -> Vec<DecimalOperation> {
        use DecimalOperation::*;
        let mut all = vec![Add, Sub, Mul, Rem, Negate, Compare, FromInt, FromFloat, ToFloat, ToIntTruncated, Scale, Abs, ParseText];
        all.extend(RoundingMode::ALL.into_iter().map(Round));
        all.extend(RoundingMode::ALL.into_iter().map(DivideRound));
        all
    }

    /// The runtime symbol. The rounding symbols carry the method-name
    /// suffix of their mode (`corvid_decimal_round_half_even`).
    pub fn symbol(self) -> String {
        use DecimalOperation::*;
        match self {
            Add => "corvid_decimal_add".into(),
            Sub => "corvid_decimal_sub".into(),
            Mul => "corvid_decimal_mul".into(),
            Rem => "corvid_decimal_rem".into(),
            Negate => "corvid_decimal_negate".into(),
            Compare => "corvid_decimal_compare".into(),
            FromInt => "corvid_decimal_from_int".into(),
            FromFloat => "corvid_decimal_from_float".into(),
            ToFloat => "corvid_decimal_to_float".into(),
            ToIntTruncated => "corvid_decimal_to_int_truncated".into(),
            Scale => "corvid_decimal_scale".into(),
            Abs => "corvid_decimal_abs".into(),
            Round(mode) => format!("corvid_decimal_round_{}", mode.suffix()),
            DivideRound(mode) => format!("corvid_decimal_divide_round_{}", mode.suffix()),
            ParseText => "corvid_string_parse_decimal".into(),
        }
    }

    /// Parameter widths and the return width.
    pub fn signature(self) -> (&'static [DecimalSlot], DecimalSlot) {
        use DecimalOperation::*;
        use DecimalSlot::*;
        match self {
            Add | Sub | Mul | Rem | Compare | Round(_) | ParseText => (&[Word, Word], Word),
            Negate | Abs | FromInt | ToIntTruncated | Scale => (&[Word], Word),
            FromFloat => (&[Float], Word),
            ToFloat => (&[Word], Float),
            DivideRound(_) => (&[Word, Word, Word], Word),
        }
    }

    /// The operation behind a binary operator with a `Decimal` operand;
    /// every comparison is one `Compare`. `/` has none: the checker
    /// accepts it only fused with its rounding.
    pub fn for_binary(op: BinaryOp) -> Option<DecimalOperation> {
        use DecimalOperation::*;
        Some(match op {
            BinaryOp::Add => Add,
            BinaryOp::Sub => Sub,
            BinaryOp::Mul => Mul,
            BinaryOp::Mod => Rem,
            BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => Compare,
            BinaryOp::Div | BinaryOp::And | BinaryOp::Or => return None,
        })
    }
}

/// How native code runs a `Decimal` builtin method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeDecimalMethod {
    /// One call into the runtime.
    Call(DecimalOperation),
    /// `to_string()`: the value already is its canonical text.
    CanonicalText,
}

/// The native form of a `Decimal` method, or `None` when it has none.
pub fn native_decimal_method(kind: BuiltinMethodKind) -> Option<NativeDecimalMethod> {
    use BuiltinMethodKind::*;
    use DecimalOperation as Op;
    use NativeDecimalMethod::*;
    Some(match kind {
        DecimalRound(mode) => Call(Op::Round(mode)),
        DecimalDivideRound(mode) => Call(Op::DivideRound(mode)),
        DecimalAbs => Call(Op::Abs),
        DecimalScale => Call(Op::Scale),
        DecimalToString => CanonicalText,
        DecimalToFloat => Call(Op::ToFloat),
        DecimalToIntTruncated => Call(Op::ToIntTruncated),
        IntToDecimal => Call(Op::FromInt),
        FloatToDecimal => Call(Op::FromFloat),
        StringParseDecimal => Call(Op::ParseText),
        // `format_currency` is a currency method: `currency_methods`
        // names its native form.
        _ => return None,
    })
}

/// Whether native code compiles this `Decimal` method.
pub fn is_native_decimal_method(kind: BuiltinMethodKind) -> bool {
    native_decimal_method(kind).is_some()
}

/// Whether a binary operator works on `Decimal` values: either operand
/// is a `Decimal` (the other may be an `Int`, widened exactly).
pub fn is_decimal_binary(op: BinaryOp, left: &corvid_types::Type, right: &corvid_types::Type) -> bool {
    use corvid_types::Type;
    DecimalOperation::for_binary(op).is_some()
        && matches!(
            (left, right),
            (Type::Decimal, Type::Decimal | Type::Int) | (Type::Int, Type::Decimal)
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_decimal_method_has_a_native_form_here_and_format_currency_in_the_currency_table() {
        use BuiltinMethodKind::*;
        let mut kinds = vec![DecimalAbs, DecimalScale, DecimalToString, DecimalToFloat, DecimalToIntTruncated, IntToDecimal, FloatToDecimal, StringParseDecimal];
        kinds.extend(RoundingMode::ALL.into_iter().map(DecimalRound));
        kinds.extend(RoundingMode::ALL.into_iter().map(DecimalDivideRound));
        for kind in kinds {
            assert!(kind.is_decimal(), "{kind:?}");
            assert!(is_native_decimal_method(kind), "{kind:?} needs a native form");
        }
        assert!(!is_native_decimal_method(DecimalFormatCurrency));
        assert!(crate::is_native_currency_method(DecimalFormatCurrency));
    }

    #[test]
    fn symbols_are_distinct_and_rounding_symbols_name_their_mode() {
        let all = DecimalOperation::all();
        let symbols: std::collections::HashSet<String> = all.iter().map(|op| op.symbol()).collect();
        assert_eq!(symbols.len(), all.len());
        assert_eq!(all.len(), 25, "a new operation needs a runtime entry point");
        assert_eq!(DecimalOperation::Round(RoundingMode::HalfEven).symbol(), "corvid_decimal_round_half_even");
        assert_eq!(
            DecimalOperation::DivideRound(RoundingMode::AwayFromZero).symbol(),
            "corvid_decimal_divide_round_away_from_zero"
        );
    }

    #[test]
    fn only_decimal_operands_route_here() {
        use corvid_types::Type;
        assert!(is_decimal_binary(BinaryOp::Mul, &Type::Decimal, &Type::Int));
        assert!(is_decimal_binary(BinaryOp::Lt, &Type::Int, &Type::Decimal));
        assert!(!is_decimal_binary(BinaryOp::Add, &Type::Int, &Type::Int));
        assert!(!is_decimal_binary(BinaryOp::Div, &Type::Decimal, &Type::Decimal));
        assert!(!is_decimal_binary(BinaryOp::And, &Type::Decimal, &Type::Decimal));
    }
}
