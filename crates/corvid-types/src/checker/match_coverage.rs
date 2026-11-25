//! Whether a set of patterns matches every value of a type, at any depth.
//!
//! `match` exhaustiveness asks, per constructor of the scrutinee, whether
//! the arms that use it cover every value of its payload. That question
//! recurses: `Err(Some(x))` and `Err(None)` together cover `Err(_)`
//! because `Some(x)` and `None` cover the `Option`. The walk is the
//! standard one for pattern matrices: a row per arm, a column per value
//! still to match; specialise the rows by each constructor of the first
//! column's type and require every constructor to be covered. A type with
//! unboundedly many values (a number, text) is covered only by a pattern
//! that matches anything, so literals never make a match exhaustive.

use super::Checker;
use crate::types::Type;
use corvid_ast::{Literal, Pattern};

/// One cell of the matrix: a pattern from the source, or the "anything"
/// that a constructor's unmatched payload becomes.
#[derive(Clone, Copy)]
enum Cell<'p> {
    Any,
    Pattern(&'p Pattern),
}

/// A constructor of a type with finitely many values.
struct Constructor {
    kind: ConstructorKind,
    /// The payload's types, in order.
    fields: Vec<Type>,
}

enum ConstructorKind {
    /// `Some`, `Ok`, `None`, a sum variant: matched by name.
    Named(String),
    /// `true` or `false`.
    Bool(bool),
    /// A record's one constructor, with its field names in order.
    Record(Vec<String>),
}

impl<'a> Checker<'a> {
    /// Whether `rows` (each a sequence of patterns, one per entry of
    /// `types`) together match every combination of values of `types`.
    pub(super) fn rows_cover(&mut self, rows: &[Vec<&Pattern>], types: &[Type]) -> bool {
        let rows: Vec<Vec<Cell<'_>>> = rows.iter().map(|row| row.iter().map(|p| Cell::Pattern(p)).collect()).collect();
        self.matrix_covers(rows, types)
    }

    fn matrix_covers(&mut self, rows: Vec<Vec<Cell<'_>>>, types: &[Type]) -> bool {
        let Some((first, rest)) = types.split_first() else {
            // Nothing left to match: covered if any row reached here.
            return !rows.is_empty();
        };
        match self.constructors(first) {
            // Unboundedly many values: only rows that take anything here
            // can cover, and they must cover the remaining columns.
            None => {
                let remaining: Vec<Vec<Cell<'_>>> =
                    rows.into_iter().filter(|row| self.takes_anything(row[0])).map(|row| row[1..].to_vec()).collect();
                self.matrix_covers(remaining, rest)
            }
            Some(constructors) => constructors.iter().all(|constructor| {
                let specialised: Vec<Vec<Cell<'_>>> = rows
                    .iter()
                    .filter_map(|row| {
                        let mut cells = if self.takes_anything(row[0]) {
                            vec![Cell::Any; constructor.fields.len()]
                        } else {
                            payload_for(row[0], constructor)?
                        };
                        cells.extend_from_slice(&row[1..]);
                        Some(cells)
                    })
                    .collect();
                let mut columns = constructor.fields.clone();
                columns.extend_from_slice(rest);
                self.matrix_covers(specialised, &columns)
            }),
        }
    }

    /// Whether a cell matches every value: `_`, a binding, or `x @ p`
    /// around such a pattern. A record pattern is specialised instead, so
    /// its fields are checked one by one.
    fn takes_anything(&self, cell: Cell<'_>) -> bool {
        match cell {
            Cell::Any => true,
            Cell::Pattern(Pattern::At { inner, .. }) => self.takes_anything(Cell::Pattern(inner)),
            Cell::Pattern(Pattern::Record { .. }) => false,
            Cell::Pattern(pattern) => self.pattern_is_irrefutable(pattern),
        }
    }

    /// The constructors of `ty` when it has finitely many: `Some`/`None`,
    /// `Ok`/`Err`, `true`/`false`, a sum type's variants, or a record's
    /// one constructor. `None` for every other type.
    fn constructors(&mut self, ty: &Type) -> Option<Vec<Constructor>> {
        let named = |name: &str, fields: Vec<Type>| Constructor { kind: ConstructorKind::Named(name.into()), fields };
        Some(match ty {
            Type::Tainted(inner) => return self.constructors(inner),
            Type::Option(inner) => vec![named("Some", vec![(**inner).clone()]), named("None", Vec::new())],
            Type::Result(ok, err) => vec![named("Ok", vec![(**ok).clone()]), named("Err", vec![(**err).clone()])],
            // Only the cases this conflict can be need covering.
            Type::Conflict(row, cases) => cases
                .cases()
                .map(|case| named(case.name(), case.field().map(|_| (**row).clone()).into_iter().collect()))
                .collect(),
            Type::Bool => vec![
                Constructor { kind: ConstructorKind::Bool(true), fields: Vec::new() },
                Constructor { kind: ConstructorKind::Bool(false), fields: Vec::new() },
            ],
            Type::Struct(owner) => {
                let decl = self.types_by_id.get(owner).copied()?;
                if decl.variants.is_empty() {
                    vec![Constructor {
                        kind: ConstructorKind::Record(decl.fields.iter().map(|f| f.name.name.clone()).collect()),
                        fields: decl.fields.iter().map(|f| self.type_ref_to_type(&f.ty)).collect(),
                    }]
                } else {
                    decl.variants
                        .iter()
                        .map(|v| named(&v.name.name, v.fields.iter().map(|f| self.type_ref_to_type(&f.ty)).collect()))
                        .collect()
                }
            }
            _ => return None,
        })
    }
}

/// The payload a pattern gives `constructor`, or `None` if it matches a
/// different one.
fn payload_for<'p>(cell: Cell<'p>, constructor: &Constructor) -> Option<Vec<Cell<'p>>> {
    let Cell::Pattern(pattern) = cell else { return Some(vec![Cell::Any; constructor.fields.len()]) };
    match (pattern, &constructor.kind) {
        (Pattern::At { inner, .. }, _) => payload_for(Cell::Pattern(inner), constructor),
        (Pattern::Name { name, .. }, ConstructorKind::Named(expected)) => {
            (name.name == *expected && constructor.fields.is_empty()).then(Vec::new)
        }
        (Pattern::Variant { name, args, .. }, ConstructorKind::Named(expected)) if name.name == *expected => {
            Some(args.iter().map(Cell::Pattern).chain(std::iter::repeat(Cell::Any)).take(constructor.fields.len()).collect())
        }
        (Pattern::Literal { value: Literal::Bool(b), .. }, ConstructorKind::Bool(expected)) => (b == expected).then(Vec::new),
        // A field the pattern leaves out, or binds by name, matches anything.
        (Pattern::Record { fields, .. }, ConstructorKind::Record(names)) => Some(
            names
                .iter()
                .map(|field| match fields.iter().find(|f| f.name.name == *field).and_then(|f| f.pattern.as_ref()) {
                    Some(sub) => Cell::Pattern(sub),
                    None => Cell::Any,
                })
                .collect(),
        ),
        _ => None,
    }
}
