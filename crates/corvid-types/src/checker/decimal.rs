//! Compile-time rules for `Decimal` (B06).
//!
//! Three rules a signature cannot express live here:
//!
//! 1. **Number literals become exact decimals where a `Decimal` is
//!    meant.** A literal in a `Decimal` slot (`price: Decimal = 19.99`)
//!    or next to a `Decimal` operand (`price * 1.08`) is read from its
//!    source digits by `corvid_decimal`, the code the runtime uses, and
//!    typed `Decimal`. A literal with no exact value is a compile error.
//! 2. **Operators.** `Decimal` mixes with `Int` (the conversion is
//!    exact) but never with `Float`.
//! 3. **Division names its rounding.** `a / b` on decimals is accepted
//!    only as the receiver of a `round_<mode>(places)` call, which IR
//!    lowering fuses into one rounded division.

use super::Checker;
use crate::builtin_methods::{decimal_rounding_mode, number_literal_text, BuiltinMethodKind};
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{BinaryOp, Decl, ExtendMethodKind, Expr, File, Ident, Literal, Param, Span, TypeRef, UnaryOp};

/// Whether `e` is built only from number literals, signs and
/// arithmetic, so it has no type of its own until its context gives
/// one: `19.99`, `-0.5`, `1 + 0.08`.
pub(crate) fn is_number_literal_expr(e: &Expr) -> bool {
    match e {
        Expr::Literal { value: Literal::Int(_) | Literal::Float { .. }, .. } => true,
        Expr::UnOp { op: UnaryOp::Neg | UnaryOp::Pos, operand, .. } => is_number_literal_expr(operand),
        Expr::BinOp { op, left, right, .. } => {
            matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod)
                && is_number_literal_expr(left)
                && is_number_literal_expr(right)
        }
        _ => false,
    }
}

/// The literal integer an expression spells, sign included: the
/// `places` argument of `round_half_even(2)` or `round_floor(-1)`.
fn literal_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal { value: Literal::Int(n), .. } => Some(*n),
        Expr::UnOp { op: UnaryOp::Neg, operand, .. } => literal_int(operand).map(|n| -n),
        Expr::UnOp { op: UnaryOp::Pos, operand, .. } => literal_int(operand),
        _ => None,
    }
}

fn is_literal_zero(e: &Expr) -> bool {
    match e {
        Expr::Literal { value: Literal::Int(0), .. } => true,
        Expr::Literal { value: Literal::Float { value, .. }, .. } => *value == 0.0,
        Expr::UnOp { op: UnaryOp::Neg | UnaryOp::Pos, operand, .. } => is_literal_zero(operand),
        _ => false,
    }
}

fn is_comparison(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq
    )
}

fn operator_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "`+`",
        BinaryOp::Sub => "`-`",
        BinaryOp::Mul => "`*`",
        BinaryOp::Div => "`/`",
        BinaryOp::Mod => "`%`",
        BinaryOp::Eq => "`==`",
        BinaryOp::NotEq => "`!=`",
        BinaryOp::Lt => "`<`",
        BinaryOp::LtEq => "`<=`",
        BinaryOp::Gt => "`>`",
        BinaryOp::GtEq => "`>=`",
        BinaryOp::And | BinaryOp::Or => "a logical operator",
    }
}

impl<'a> Checker<'a> {
    /// Types a number literal that sits in a `Decimal` slot, checking
    /// that its digits have an exact `Decimal` value.
    pub(super) fn check_decimal_literal(&mut self, value: &Literal, span: Span) -> Type {
        let Some(text) = number_literal_text(value) else {
            return Type::Unknown;
        };
        if let Err(problem) = corvid_decimal::Decimal::parse(&text) {
            self.errors.push(TypeError::new(
                TypeErrorKind::InvalidDecimalLiteral { literal: text, problem: problem.to_string() },
                span,
            ));
        }
        Type::Decimal
    }

    /// Checks one operand of an operator, reading it as a `Decimal`
    /// when it is made of number literals and a `Decimal` is expected.
    pub(super) fn check_operand(&mut self, operand: &Expr, expected: Option<&Type>) -> Type {
        match expected {
            Some(Type::Decimal) if is_number_literal_expr(operand) => {
                self.check_expr_as(operand, Some(&Type::Decimal))
            }
            _ => self.check_expr(operand),
        }
    }

    /// The type of `l op r` when either side is a `Decimal`, or `None`
    /// to leave the operator to the ordinary rules. `lt` and `rt` have
    /// `Grounded` and `Tainted` already stripped.
    pub(super) fn check_decimal_binop(
        &mut self,
        op: BinaryOp,
        l: &Expr,
        r: &Expr,
        lt: &Type,
        rt: &Type,
        span: Span,
    ) -> Option<Type> {
        if !matches!(lt, Type::Decimal) && !matches!(rt, Type::Decimal) {
            return None;
        }
        // A literal next to a Decimal is a Decimal: `price * 1.08`.
        let lt = if !matches!(lt, Type::Decimal) && is_number_literal_expr(l) {
            self.check_expr_as(l, Some(&Type::Decimal))
        } else {
            lt.clone()
        };
        let rt = if !matches!(rt, Type::Decimal) && is_number_literal_expr(r) {
            self.check_expr_as(r, Some(&Type::Decimal))
        } else {
            rt.clone()
        };
        let operand = |t: &Type| matches!(t, Type::Decimal | Type::Int | Type::Unknown);
        if matches!(lt, Type::Float) || matches!(rt, Type::Float) {
            self.errors.push(TypeError::new(
                TypeErrorKind::DecimalFloatMix { context: operator_name(op).to_string() },
                l.span().merge(r.span()),
            ));
            return Some(if is_comparison(op) { Type::Bool } else { Type::Unknown });
        }
        if !operand(&lt) || !operand(&rt) {
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeMismatch {
                    expected: "Decimal or Int".into(),
                    got: format!("{} and {}", lt.display_name(), rt.display_name()),
                    context: format!("{} operator", operator_name(op)),
                },
                l.span().merge(r.span()),
            ));
            return Some(if is_comparison(op) { Type::Bool } else { Type::Unknown });
        }
        if matches!(op, BinaryOp::Div | BinaryOp::Mod) && is_literal_zero(r) {
            self.errors.push(TypeError::new(TypeErrorKind::DecimalDivisionByZero, r.span()));
        }
        if op == BinaryOp::Div && self.rounded_division != Some(span) {
            self.errors.push(TypeError::new(TypeErrorKind::DecimalDivisionNeedsRounding, span));
        }
        Some(if is_comparison(op) { Type::Bool } else { Type::Decimal })
    }

    /// Checks a method receiver, permitting a `Decimal` `/` when the
    /// method rounds it: `(total / 3).round_half_even(2)`.
    pub(super) fn check_method_receiver(&mut self, target: &Expr, method: &Ident) -> Type {
        let rounds = matches!(target, Expr::BinOp { op: BinaryOp::Div, .. })
            && decimal_rounding_mode(&method.name).is_some();
        if !rounds {
            return self.check_expr(target);
        }
        let outer = self.rounded_division.replace(target.span());
        let ty = self.check_expr(target);
        self.rounded_division = outer;
        ty
    }

    /// Refuses `Decimal` wherever a value leaves the program as JSON —
    /// tool, prompt and connector signatures, routes, stores and table
    /// rows — until B06's JSON slice gives it an exact JSON form.
    /// Without this a decimal would cross as whatever the boundary's
    /// fallback produced, which is not a decision anyone made. Runs
    /// after `record_signature_types`, so every type is resolved.
    pub(super) fn check_decimal_boundaries(&mut self, file: &File) {
        for decl in &file.decls {
            match decl {
                Decl::Tool(t) => self.check_signature_boundary(&t.params, &t.return_ty, "a tool signature"),
                Decl::Prompt(p) => self.check_signature_boundary(&p.params, &p.return_ty, "a prompt signature"),
                Decl::Connector(c) => {
                    for op in &c.operations {
                        self.check_signature_boundary(&op.params, &op.return_ty, "a connector operation");
                    }
                }
                Decl::Server(s) => {
                    for route in &s.routes {
                        for param in &route.path_params {
                            self.check_boundary(&param.ty, "a route");
                        }
                        for ty in route.query_ty.iter().chain(route.body_ty.iter()) {
                            self.check_boundary(ty, "a route");
                        }
                        self.check_boundary(&route.response.ty, "a route");
                    }
                }
                Decl::Extend(ext) => {
                    for method in &ext.methods {
                        match &method.kind {
                            ExtendMethodKind::Tool(t) => {
                                self.check_signature_boundary(&t.params, &t.return_ty, "a tool signature")
                            }
                            ExtendMethodKind::Prompt(p) => {
                                self.check_signature_boundary(&p.params, &p.return_ty, "a prompt signature")
                            }
                            ExtendMethodKind::Agent(_) => {}
                        }
                    }
                }
                Decl::Store(store) => {
                    for field in &store.fields {
                        self.check_boundary(&field.ty, "a store field");
                    }
                }
                Decl::Table(table) => {
                    for field in &table.record.fields {
                        self.check_boundary(&field.ty, "a table column");
                    }
                }
                _ => {}
            }
        }
    }

    fn check_signature_boundary(&mut self, params: &[Param], return_ty: &TypeRef, boundary: &str) {
        for param in params {
            self.check_boundary(&param.ty, boundary);
        }
        self.check_boundary(return_ty, boundary);
    }

    fn check_boundary(&mut self, ty_ref: &TypeRef, boundary: &str) {
        let Some(ty) = self.type_refs.get(&ty_ref.span()).cloned() else { return };
        if self.reaches_decimal_without_json_form(&ty) {
            self.errors.push(TypeError::new(
                TypeErrorKind::DecimalAtBoundary { boundary: boundary.to_string() },
                ty_ref.span(),
            ));
        }
    }

    /// Checks what the `Decimal` method signatures cannot: a literal
    /// `places` argument is between 0 and 28.
    pub(super) fn check_decimal_method_arguments(&mut self, kind: BuiltinMethodKind, args: &[Expr]) {
        if !matches!(kind, BuiltinMethodKind::DecimalRound(_)) {
            return;
        }
        let Some(places) = args.first() else { return };
        if let Some(n) = literal_int(places) {
            if corvid_decimal::check_places(n).is_err() {
                self.errors.push(TypeError::new(
                    TypeErrorKind::DecimalPlacesOutOfRange { places: n },
                    places.span(),
                ));
            }
        }
    }
}
