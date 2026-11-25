//! The operations a table offers as methods (B09): `Shelf.get(key)`,
//! `Shelf.insert(row)`, `Shelf.save(row)` and `Shelf.delete(row)`.
//!
//! The checker types these calls, the effect analyzer charges their
//! effects, and IR lowering lowers them. All three decide whether a
//! call is a table operation with [`table_call_target`], so they
//! cannot disagree.

use corvid_ast::{BinaryOp, Block, DatabaseDecl, Decl, Expr, File, Literal, Span, TableDecl, UnaryOp};
use corvid_resolve::{Binding, DeclKind, SymbolTable};
use std::collections::HashMap;

/// A table operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableOp {
    /// `Shelf.get(key) -> Option<Shelf>`; a key of several fields takes
    /// their values in declaration order, `Edit.get(proposal, shelf)`.
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

/// A query over a table (B09 P2): `Shelf.where(fn (s) -> cond)` or
/// `Shelf.all()`, then optionally `.order_by(fn (s) -> s.column)`,
/// `.descending()` and `.take(n)`, in that order. The whole chain is
/// one expression of type `List<Shelf>`; its parts are not values.
#[derive(Debug, Clone, Copy)]
pub struct TableQuery<'a, 'e> {
    pub table: &'a TableDecl,
    /// The `where` function, or `None` for `all()`.
    pub filter: Option<&'e Expr>,
    pub order: Option<&'e Expr>,
    pub descending: bool,
    pub take: Option<&'e Expr>,
}

/// The query a method call names, when the call ends a query chain:
/// `receiver.method(args)` where the chain roots at `Table.where` or
/// `Table.all`. `None` for any other call.
pub fn table_query_target<'a, 'e>(
    receiver: &'e Expr,
    method: &str,
    args: &'e [Expr],
    file: &'a File,
    symbols: &SymbolTable,
    bindings: &HashMap<Span, Binding>,
) -> Option<TableQuery<'a, 'e>> {
    // Stages from the outermost call inward; each appears at most once,
    // in the order `take`, `descending`, `order_by`, then the root.
    let mut stages: Vec<(&str, &'e [Expr], &'e Expr)> = vec![(method, args, receiver)];
    let mut current = receiver;
    while let Expr::Call { callee, args, .. } = current {
        let Expr::FieldAccess { target, field, .. } = callee.as_ref() else { return None };
        if matches!(field.name.as_str(), "where" | "all") {
            stages.push((field.name.as_str(), args, target));
            break;
        }
        stages.push((field.name.as_str(), args, target));
        current = target;
    }
    let (root, root_args, table_name) = stages.pop()?;
    let Expr::Ident { name, .. } = table_name else { return None };
    let Binding::Decl(id) = bindings.get(&name.span)? else { return None };
    let entry = symbols.get(*id);
    if entry.kind != DeclKind::Type {
        return None;
    }
    let table = table_decl(file, entry.span)?;
    let filter = match (root, root_args) {
        ("where", [condition]) => Some(condition),
        ("all", []) => None,
        _ => return None,
    };
    let mut query = TableQuery { table, filter, order: None, descending: false, take: None };
    let mut rank = 0;
    for (stage, stage_args, _) in stages.iter().rev() {
        let next = match (*stage, *stage_args) {
            ("order_by", [key]) if rank < 1 => {
                query.order = Some(key);
                1
            }
            ("descending", []) if rank < 2 && query.order.is_some() => {
                query.descending = true;
                2
            }
            ("take", [count]) if rank < 3 => {
                query.take = Some(count);
                3
            }
            _ => return None,
        };
        rank = next;
    }
    Some(query)
}

/// [`table_query_target`] for a whole call: `callee(args)` where the
/// callee is the chain's last method.
pub fn table_query_call<'a, 'e>(
    callee: &'e Expr,
    args: &'e [Expr],
    file: &'a File,
    symbols: &SymbolTable,
    bindings: &HashMap<Span, Binding>,
) -> Option<TableQuery<'a, 'e>> {
    let Expr::FieldAccess { target, field, .. } = callee else { return None };
    table_query_target(target, &field.name, args, file, symbols, bindings)
}

/// The column `expr` reads when it is `row.column`, where `row` is the
/// query function's parameter and `column` a declared column. The
/// checker and IR lowering both read query conditions through this.
pub fn query_column(table: &TableDecl, row: &str, expr: &Expr) -> Option<String> {
    let Expr::FieldAccess { target, field, .. } = expr else { return None };
    let Expr::Ident { name, .. } = target.as_ref() else { return None };
    (name.name == row && table.record.fields.iter().any(|f| f.name.name == field.name)).then(|| field.name.clone())
}

/// A comparison of a column with a value, written column first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryCompare {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

impl QueryCompare {
    fn from_op(op: BinaryOp) -> Option<Self> {
        Some(match op {
            BinaryOp::Eq => Self::Eq,
            BinaryOp::NotEq => Self::NotEq,
            BinaryOp::Lt => Self::Lt,
            BinaryOp::LtEq => Self::LtEq,
            BinaryOp::Gt => Self::Gt,
            BinaryOp::GtEq => Self::GtEq,
            _ => return None,
        })
    }

    /// The same comparison with its sides swapped: `5 < s.n` is `s.n > 5`.
    fn flipped(self) -> Self {
        match self {
            Self::Lt => Self::Gt,
            Self::LtEq => Self::GtEq,
            Self::Gt => Self::Lt,
            Self::GtEq => Self::LtEq,
            same => same,
        }
    }
}

/// A string test on a text column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryTextTest {
    Contains,
    StartsWith,
    EndsWith,
}

impl QueryTextTest {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "contains" => Self::Contains,
            "starts_with" => Self::StartsWith,
            "ends_with" => Self::EndsWith,
            _ => return None,
        })
    }
}

/// A `where` condition in the form the database runs. Values compared
/// with columns are expressions the agent evaluates before the query.
#[derive(Debug, Clone)]
pub enum QueryCondition<'e> {
    Const(bool),
    /// A `Bool` column read on its own.
    Column(String),
    And(Box<QueryCondition<'e>>, Box<QueryCondition<'e>>),
    Or(Box<QueryCondition<'e>>, Box<QueryCondition<'e>>),
    Not(Box<QueryCondition<'e>>),
    Compare { column: String, op: QueryCompare, value: &'e Expr },
    Text { column: String, test: QueryTextTest, value: &'e Expr },
}

/// Why part of a `where` condition cannot run in the database.
#[derive(Debug, Clone)]
pub struct UntranslatableCondition {
    pub span: Span,
    pub message: &'static str,
}

/// Read a `where` condition over the row parameter `row` into the form
/// the database runs. The checker reports the error this returns, and
/// IR lowering lowers the condition it returns, so what type-checks is
/// exactly what runs.
pub fn query_condition<'e>(
    table: &TableDecl,
    row: &str,
    condition: &'e Expr,
) -> Result<QueryCondition<'e>, UntranslatableCondition> {
    let refuse = |span: Span, message: &'static str| Err(UntranslatableCondition { span, message });
    match condition {
        Expr::BinOp { op: BinaryOp::And, left, right, .. } => Ok(QueryCondition::And(
            Box::new(query_condition(table, row, left)?),
            Box::new(query_condition(table, row, right)?),
        )),
        Expr::BinOp { op: BinaryOp::Or, left, right, .. } => Ok(QueryCondition::Or(
            Box::new(query_condition(table, row, left)?),
            Box::new(query_condition(table, row, right)?),
        )),
        Expr::UnOp { op: UnaryOp::Not, operand, .. } => {
            Ok(QueryCondition::Not(Box::new(query_condition(table, row, operand)?)))
        }
        Expr::BinOp { op, left, right, span } if QueryCompare::from_op(*op).is_some() => {
            let op = QueryCompare::from_op(*op).unwrap_or(QueryCompare::Eq);
            match (query_column(table, row, left), query_column(table, row, right)) {
                (Some(column), None) => Ok(QueryCondition::Compare { column, op, value: right }),
                (None, Some(column)) => Ok(QueryCondition::Compare { column, op: op.flipped(), value: left }),
                _ => refuse(*span, "a comparison in `where` compares one column with a value from the agent"),
            }
        }
        Expr::Call { callee, args, span } => {
            const ONLY: &str = "only `contains`, `starts_with` and `ends_with` calls on a column can run in the database";
            let Expr::FieldAccess { target, field, .. } = callee.as_ref() else { return refuse(*span, ONLY) };
            match (QueryTextTest::from_name(&field.name), query_column(table, row, target), args.as_slice()) {
                (Some(test), Some(column), [value]) => Ok(QueryCondition::Text { column, test, value }),
                _ => refuse(*span, ONLY),
            }
        }
        Expr::Literal { value: Literal::Bool(b), .. } => Ok(QueryCondition::Const(*b)),
        other => match query_column(table, row, other) {
            Some(column) => Ok(QueryCondition::Column(column)),
            None => refuse(
                other.span(),
                "this part of the `where` condition cannot run in the database; use comparisons, `and`, `or`, `not`, `== None`, and `contains`, `starts_with` or `ends_with` on a column",
            ),
        },
    }
}

/// Every table an `atomic:` block reads or writes, through an operation
/// or a query in its body or in an agent it calls (an agent called
/// inside the block runs inside its transaction), in source order with
/// repeats removed. The checker's one-database rule and IR lowering
/// both read the block's tables here.
pub fn atomic_tables<'a>(
    body: &'a Block,
    file: &'a File,
    symbols: &SymbolTable,
    bindings: &HashMap<Span, Binding>,
) -> Vec<&'a TableDecl> {
    let mut tables: Vec<&'a TableDecl> = Vec::new();
    let mut visited = Vec::new();
    collect_atomic_tables(body, file, symbols, bindings, &mut tables, &mut visited);
    tables
}

fn collect_atomic_tables<'a>(
    body: &'a Block,
    file: &'a File,
    symbols: &SymbolTable,
    bindings: &HashMap<Span, Binding>,
    tables: &mut Vec<&'a TableDecl>,
    visited: &mut Vec<String>,
) {
    let mut called = Vec::new();
    corvid_ast::visit::walk_block(body, &mut |_| {}, &mut |expr| {
        let Expr::Call { callee, args, .. } = expr else { return };
        let table = table_call_target(callee, file, symbols, bindings)
            .map(|(table, _)| table)
            .or_else(|| table_query_call(callee, args, file, symbols, bindings).map(|query| query.table));
        match table {
            Some(table) => {
                if !tables.iter().any(|t| t.name().name == table.name().name) {
                    tables.push(table);
                }
            }
            None => called.extend(called_agent(callee, file, symbols, bindings)),
        }
    });
    for agent in called {
        if visited.contains(&agent.name.name) {
            continue;
        }
        visited.push(agent.name.name.clone());
        collect_atomic_tables(&agent.body, file, symbols, bindings, tables, visited);
    }
}

/// The agent a call's callee names in this file, if it names one.
pub fn called_agent<'a>(
    callee: &Expr,
    file: &'a File,
    symbols: &SymbolTable,
    bindings: &HashMap<Span, Binding>,
) -> Option<&'a corvid_ast::AgentDecl> {
    let Expr::Ident { name, .. } = callee else { return None };
    let Binding::Decl(id) = bindings.get(&name.span)? else { return None };
    let entry = symbols.get(*id);
    if entry.kind != DeclKind::Agent {
        return None;
    }
    file.decls.iter().find_map(|decl| match decl {
        Decl::Agent(agent) if agent.name.name == entry.name => Some(agent),
        _ => None,
    })
}

/// The `database` block that lists `table`.
pub fn table_database<'a>(file: &'a File, table: &str) -> Option<&'a DatabaseDecl> {
    table_database_with(file, table, &|_| None)
}

/// [`table_database`], following an `in` that names an imported database.
pub fn table_database_with<'a>(
    file: &'a File,
    table: &str,
    imported: &dyn Fn(&str) -> Option<&'a DatabaseDecl>,
) -> Option<&'a DatabaseDecl> {
    match table_placement_with(file, table, imported) {
        TablePlacement::In(database) => Some(database),
        _ => None,
    }
}

/// Where a table's rows live (B09 P11): the database its `in` clause
/// names, or else its module's one `database` block. The checker reports
/// every case but the first, and every other layer reads the placement
/// through [`table_database`], so they never disagree.
pub enum TablePlacement<'a> {
    In(&'a DatabaseDecl),
    /// No `in` clause, and the module declares no database.
    NoDatabase,
    /// No `in` clause, and the module declares several.
    Ambiguous(Vec<&'a DatabaseDecl>),
    /// The `in` clause names something that is not a database here.
    UnknownDatabase(&'a corvid_ast::Ident),
    /// There is no table by that name.
    NotATable,
}

pub fn table_placement<'a>(file: &'a File, table: &str) -> TablePlacement<'a> {
    table_placement_with(file, table, &|_| None)
}

/// [`table_placement`], following an `in` that names a database imported
/// from another module through `imported` (see [`imported_database`]).
pub fn table_placement_with<'a>(
    file: &'a File,
    table: &str,
    imported: &dyn Fn(&str) -> Option<&'a DatabaseDecl>,
) -> TablePlacement<'a> {
    let Some(decl) = file.decls.iter().find_map(|decl| match decl {
        Decl::Table(t) if t.name().name == table => Some(t),
        _ => None,
    }) else {
        return TablePlacement::NotATable;
    };
    let databases: Vec<&DatabaseDecl> = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Database(db) => Some(db),
            _ => None,
        })
        .collect();
    match &decl.database {
        Some(named) => match databases.iter().find(|db| db.name.name == named.name) {
            Some(db) => TablePlacement::In(db),
            None => match imported(&named.name) {
                Some(db) => TablePlacement::In(db),
                None => TablePlacement::UnknownDatabase(named),
            },
        },
        None => match databases.as_slice() {
            [] => TablePlacement::NoDatabase,
            [only] => TablePlacement::In(only),
            _ => TablePlacement::Ambiguous(databases),
        },
    }
}

/// The `public database` that `name` imports into a module (B09 P11).
/// `current` is the module being checked or lowered; `None` is the root,
/// whose `use` names are in `modules.imported_uses`. An imported module's
/// own `use` names are read from its `import` lines, because a program's
/// module resolution keeps only the root's.
pub fn imported_database<'m>(
    modules: &'m corvid_resolve::ModuleResolution,
    current: Option<&corvid_resolve::ResolvedModule>,
    name: &str,
) -> Option<&'m DatabaseDecl> {
    let (declaring, original) = importing_module(modules, current, name)?;
    declaring.file.decls.iter().find_map(|decl| match decl {
        Decl::Database(db) if db.name.name == original && !matches!(db.visibility, corvid_ast::Visibility::Private) => Some(db),
        _ => None,
    })
}

/// The module a `use` name in `current` (the root when `None`) imports
/// from, and the name the declaration has there.
pub fn importing_module<'m>(
    modules: &'m corvid_resolve::ModuleResolution,
    current: Option<&corvid_resolve::ResolvedModule>,
    name: &str,
) -> Option<(&'m corvid_resolve::ResolvedModule, String)> {
    Some(match current {
        None => {
            let target = modules.lookup_imported_use(name)?;
            (modules.all_modules.get(&target.module_path)?, target.export.name.clone())
        }
        Some(module) => {
            let (import, item) = module.file.decls.iter().find_map(|decl| match decl {
                Decl::Import(import) => import
                    .use_items
                    .iter()
                    .find(|item| item.alias.as_ref().map_or(&item.name.name, |a| &a.name) == name)
                    .map(|item| (import, item)),
                _ => None,
            })?;
            let wanted = corvid_resolve::resolve_import_path(&module.path, &import.module);
            let declaring = modules.all_modules.values().find(|m| same_file(&m.path, &wanted))?;
            (declaring, item.name.name.clone())
        }
    })
}

/// A module as a walk over calls sees it: its declarations and name
/// bindings, and which loaded module it is (`None` for the root).
#[derive(Clone, Copy)]
pub struct ModuleView<'m> {
    pub file: &'m File,
    pub symbols: &'m SymbolTable,
    pub bindings: &'m HashMap<Span, Binding>,
    pub module: Option<&'m corvid_resolve::ResolvedModule>,
}

/// The public agent an imported `use` name calls, with the view of the
/// module that declares it.
pub fn imported_agent<'m>(
    modules: &'m corvid_resolve::ModuleResolution,
    from: ModuleView<'m>,
    callee: &Expr,
) -> Option<(ModuleView<'m>, &'m corvid_ast::AgentDecl)> {
    let Expr::Ident { name, .. } = callee else { return None };
    let Binding::Decl(id) = from.bindings.get(&name.span)? else { return None };
    if from.symbols.get(*id).kind != DeclKind::ImportedUse {
        return None;
    }
    let (declaring, original) = importing_module(modules, from.module, &name.name)?;
    let agent = declaring.file.decls.iter().find_map(|decl| match decl {
        Decl::Agent(agent) if agent.name.name == original && !matches!(agent.visibility, corvid_ast::Visibility::Private) => Some(agent),
        _ => None,
    })?;
    let view = ModuleView {
        file: declaring.file.as_ref(),
        symbols: &declaring.resolved.symbols,
        bindings: &declaring.resolved.bindings,
        module: Some(declaring),
    };
    Some((view, agent))
}

/// The databases an `atomic:` block writes to, following the agents it
/// calls into other modules, each found once. An `atomic:` block's
/// transaction is on one database, so the checker refuses more than one,
/// and IR lowering opens the transaction on the one it finds.
pub fn atomic_databases<'m>(
    body: &'m Block,
    view: ModuleView<'m>,
    modules: Option<&'m corvid_resolve::ModuleResolution>,
) -> Vec<&'m DatabaseDecl> {
    let mut databases: Vec<&'m DatabaseDecl> = Vec::new();
    let mut visited: Vec<(usize, String)> = Vec::new();
    collect_atomic_databases(body, view, modules, &mut databases, &mut visited);
    databases
}

fn collect_atomic_databases<'m>(
    body: &'m Block,
    view: ModuleView<'m>,
    modules: Option<&'m corvid_resolve::ModuleResolution>,
    databases: &mut Vec<&'m DatabaseDecl>,
    visited: &mut Vec<(usize, String)>,
) {
    let mut followed: Vec<(ModuleView<'m>, &'m corvid_ast::AgentDecl)> = Vec::new();
    corvid_ast::visit::walk_block(body, &mut |_| {}, &mut |expr| {
        let Expr::Call { callee, args, .. } = expr else { return };
        let table = table_call_target(callee, view.file, view.symbols, view.bindings)
            .map(|(table, _)| table)
            .or_else(|| table_query_call(callee, args, view.file, view.symbols, view.bindings).map(|query| query.table));
        if let Some(table) = table {
            let imported = |database: &str| modules.and_then(|m| imported_database(m, view.module, database));
            if let Some(database) = table_database_with(view.file, &table.name().name, &imported) {
                if !databases.iter().any(|known| std::ptr::eq(*known, database)) {
                    databases.push(database);
                }
            }
        } else if let Some(agent) = called_agent(callee, view.file, view.symbols, view.bindings) {
            followed.push((view, agent));
        } else if let Some(found) = modules.and_then(|m| imported_agent(m, view, callee)) {
            followed.push(found);
        }
    });
    for (next, agent) in followed {
        let key = (next.file as *const File as usize, agent.name.name.clone());
        if visited.contains(&key) {
            continue;
        }
        visited.push(key);
        collect_atomic_databases(&agent.body, next, modules, databases, visited);
    }
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// The table declared at `span`, if the type there is a table.
pub fn table_decl(file: &File, span: Span) -> Option<&TableDecl> {
    file.decls.iter().find_map(|decl| match decl {
        Decl::Table(table) if table.record.span == span => Some(table),
        _ => None,
    })
}
