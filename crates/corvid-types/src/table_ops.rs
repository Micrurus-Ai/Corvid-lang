//! The operations a table offers as methods (B09): `Shelf.get(key)`,
//! `Shelf.insert(row)`, `Shelf.save(row)` and `Shelf.delete(row)`.
//!
//! The checker types these calls, the effect analyzer charges their
//! effects, and IR lowering lowers them. All three decide whether a
//! call is a table operation with [`table_call_target`], so they
//! cannot disagree.

use corvid_ast::{Decl, Expr, File, Span, TableDecl};
use corvid_resolve::{Binding, DeclKind, SymbolTable};
use std::collections::HashMap;

/// A table operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableOp {
    /// `Shelf.get(key) -> Option<Shelf>`
    Get,
    /// `Shelf.insert(row) -> Result<Shelf, Shelf>`: `Err` holds the row
    /// already stored under that key.
    Insert,
    /// `Shelf.save(row) -> Result<Shelf, Option<Shelf>>`: succeeds only
    /// if the stored row still has `row.revision`; `Err` holds the row
    /// stored now, or `None` if it was deleted.
    Save,
    /// `Shelf.delete(row) -> Result<Shelf, Option<Shelf>>`, checked like
    /// `save`; `Ok` holds the deleted row.
    Delete,
}

impl TableOp {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "get" => Self::Get,
            "insert" => Self::Insert,
            "save" => Self::Save,
            "delete" => Self::Delete,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Insert => "insert",
            Self::Save => "save",
            Self::Delete => "delete",
        }
    }

    /// The built-in effect the operation uses, shared with `std.db`.
    pub fn effect(self) -> &'static str {
        match self {
            Self::Get => "db_egress_read",
            Self::Insert | Self::Save | Self::Delete => "db_egress_write",
        }
    }

    pub fn writes(self) -> bool {
        !matches!(self, Self::Get)
    }
}

/// The table and operation a call's callee names: `Shelf.save` where
/// `Shelf` resolves to a `table` declaration and `save` is an
/// operation. `None` for any other callee.
pub fn table_call_target<'a>(
    callee: &Expr,
    file: &'a File,
    symbols: &SymbolTable,
    bindings: &HashMap<Span, Binding>,
) -> Option<(&'a TableDecl, TableOp)> {
    let Expr::FieldAccess { target, field, .. } = callee else { return None };
    table_method_target(target, &field.name, file, symbols, bindings)
}

/// [`table_call_target`] for a method call already split into its
/// receiver and method name.
pub fn table_method_target<'a>(
    receiver: &Expr,
    method: &str,
    file: &'a File,
    symbols: &SymbolTable,
    bindings: &HashMap<Span, Binding>,
) -> Option<(&'a TableDecl, TableOp)> {
    let Expr::Ident { name, .. } = receiver else { return None };
    let Binding::Decl(id) = bindings.get(&name.span)? else { return None };
    let entry = symbols.get(*id);
    if entry.kind != DeclKind::Type {
        return None;
    }
    let op = TableOp::from_name(method)?;
    let table = table_decl(file, entry.span)?;
    Some((table, op))
}

/// The table declared at `span`, if the type there is a table.
pub fn table_decl(file: &File, span: Span) -> Option<&TableDecl> {
    file.decls.iter().find_map(|decl| match decl {
        Decl::Table(table) if table.record.span == span => Some(table),
        _ => None,
    })
}
