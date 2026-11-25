//! Table and database checks (B09).
//!
//! A table is a record type stored in a database. The checker enforces
//! the decisions the source must state (a key, an engine, a location,
//! which database holds each table) and types the table's operations:
//! `get`, `insert`, `save` and `delete`, and query chains (`where`,
//! `all`, `order_by`, `descending`, `take`).

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::table_ops::{table_method_target, QueryCondition, TableOp};
use crate::types::Type;
use corvid_ast::{Block, DatabaseDecl, DatabaseLocation, Decl, Expr, Ident, Span, Stmt, TableDecl, TypeRef, Visibility};
use corvid_resolve::{Binding, BuiltIn, DeclKind};

/// The engines the runtime runs today.
const ENGINES: &[&str] = &["sqlite", "postgres"];
/// Engines on the roadmap, refused with a pointer to their slice.
const PLANNED_ENGINES: &[&str] = &["mysql"];
/// How a networked engine's connection is encrypted.
const TLS_MODES: &[&str] = &["required", "disabled"];

impl<'a> Checker<'a> {
    pub(super) fn check_table(&mut self, table: &TableDecl) {
        let name = table.name().name.clone();
        let span = table.record.span;
        if table.record.visibility != Visibility::Private {
            self.errors.push(TypeError::new(TypeErrorKind::TableNotPublic { table: name.clone() }, span));
        }
        for field in table.declared_fields() {
            if field.name.name == TableDecl::REVISION_FIELD {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TableFieldReserved { table: name.clone(), field: field.name.name.clone() },
                    field.span,
                ));
            }
        }
        if table.keys.is_empty() {
            self.errors.push(TypeError::new(TypeErrorKind::TableKeyRequired { table: name.clone() }, span));
        }
        // Each field of the key is a plain `Int` or `String`, whether the
        // key is one field or several.
        for key in &table.keys {
            let declared = table.declared_fields().iter().find(|f| f.name.name == key.name);
            let type_name = declared.and_then(|f| match &f.ty {
                TypeRef::Named { name, .. } => Some(name.name.as_str()),
                _ => None,
            });
            if !matches!(type_name, Some("Int" | "String")) {
                let found = declared.map(|f| type_ref_text(&f.ty)).unwrap_or_default();
                self.errors.push(TypeError::new(
                    TypeErrorKind::TableKeyType { table: name.clone(), key: key.name.clone(), found },
                    key.span,
                ));
            }
        }
        self.check_field_refinements(&table.record);
        self.check_column_history(table);
        self.check_unique_rules(table);

        let databases: Vec<&DatabaseDecl> = self
            .file
            .decls
            .iter()
            .filter_map(|decl| match decl {
                Decl::Database(db) if db.tables.iter().any(|t| t.name == name) => Some(db),
                _ => None,
            })
            .collect();
        match databases.as_slice() {
            [] => self.errors.push(TypeError::new(TypeErrorKind::TableOutsideDatabase { table: name }, span)),
            [_] => {}
            [first, second, ..] => self.errors.push(TypeError::new(
                TypeErrorKind::TableInTwoDatabases {
                    table: name,
                    first: first.name.name.clone(),
                    second: second.name.name.clone(),
                },
                second.span,
            )),
        }
    }

    /// `renamed from` and `retired` name stored columns, so each stored
    /// name appears once and is never a column the table declares now.
    fn check_column_history(&mut self, table: &TableDecl) {
        let declared: Vec<&str> = table.record.fields.iter().map(|f| f.name.name.as_str()).collect();
        let mut stored: Vec<&str> = Vec::new();
        let history = table
            .renamed
            .iter()
            .map(|rename| (&rename.from, format!("`{}` is renamed from `{}`", rename.field.name, rename.from.name)))
            .chain(table.retired.iter().map(|name| (name, format!("`{}` is retired", name.name))));
        let mut conflicts = Vec::new();
        for (name, clause) in history {
            if declared.contains(&name.name.as_str()) {
                conflicts.push((name.span, format!("{clause}, but `{}` is a column the table declares", name.name)));
            } else if stored.contains(&name.name.as_str()) {
                conflicts.push((name.span, format!("{clause}, but `{}` is already named by another clause", name.name)));
            } else {
                stored.push(&name.name);
            }
        }
        for (span, message) in conflicts {
            self.errors.push(TypeError::new(
                TypeErrorKind::TableColumnHistoryConflict { table: table.name().name.clone(), message },
                span,
            ));
        }
    }

    /// Each `unique` line names columns of the table that the database can
    /// compare, once each, and says something the key does not already.
    fn check_unique_rules(&mut self, table: &TableDecl) {
        use crate::table_columns::{column_kind, ColumnKind};
        let mut seen: Vec<Vec<&str>> = Vec::new();
        for rule in &table.unique {
            let mut columns: Vec<&str> = Vec::new();
            for column in rule {
                let problem = match table.declared_fields().iter().find(|f| f.name.name == column.name) {
                    None => Some(format!("`{}` is not a column of the table", column.name)),
                    Some(field) if column_kind(&self.type_ref_to_type(&field.ty)).0 == ColumnKind::Json => {
                        Some(format!("`{}` is stored as JSON, so the database cannot compare it", column.name))
                    }
                    Some(_) if columns.contains(&column.name.as_str()) => Some(format!("`{}` is named twice", column.name)),
                    Some(_) => None,
                };
                match problem {
                    Some(message) => self.errors.push(TypeError::new(
                        TypeErrorKind::TableUniqueRule { table: table.name().name.clone(), message },
                        column.span,
                    )),
                    None => columns.push(&column.name),
                }
            }
            let mut sorted = columns.clone();
            sorted.sort_unstable();
            let mut key: Vec<&str> = table.keys.iter().map(|k| k.name.as_str()).collect();
            key.sort_unstable();
            let span = rule.first().map(|c| c.span).unwrap_or(table.record.span);
            let message = if sorted == key {
                Some("the key's columns are already unique".to_string())
            } else if seen.contains(&sorted) {
                Some("the same columns are already unique on another line".to_string())
            } else {
                None
            };
            match message {
                Some(message) => self.errors.push(TypeError::new(
                    TypeErrorKind::TableUniqueRule { table: table.name().name.clone(), message },
                    span,
                )),
                None => seen.push(sorted),
            }
        }
    }

    pub(super) fn check_database(&mut self, database: &DatabaseDecl) {
        let name = database.name.name.clone();
        let span = database.span;
        match &database.engine {
            None => self.errors.push(TypeError::new(
                TypeErrorKind::DatabaseSettingRequired {
                    database: name.clone(),
                    setting: "engine".into(),
                    allowed: quoted(ENGINES),
                },
                span,
            )),
            Some(engine) if ENGINES.contains(&engine.name.as_str()) => {}
            Some(engine) => {
                let known = PLANNED_ENGINES.contains(&engine.name.as_str());
                let engine_name = if known { engine.name.clone() } else { format!("{} (not a known engine)", engine.name) };
                self.errors.push(TypeError::new(
                    TypeErrorKind::DatabaseEngineUnavailable {
                        database: name.clone(),
                        engine: engine_name,
                        available: quoted(ENGINES),
                    },
                    engine.span,
                ));
            }
        }
        if database.location.is_none() {
            self.errors.push(TypeError::new(
                TypeErrorKind::DatabaseSettingRequired {
                    database: name.clone(),
                    setting: "location".into(),
                    allowed: "a path such as `\"library.sqlite\"`, or `env(\"NAME\")`".into(),
                },
                span,
            ));
        }
        self.check_database_connection(database);
        if database.tables.is_empty() {
            self.errors.push(TypeError::new(
                TypeErrorKind::DatabaseSettingRequired {
                    database: name.clone(),
                    setting: "tables".into(),
                    allowed: "the `table` declarations it holds".into(),
                },
                span,
            ));
        }
        for table in &database.tables {
            if self.table_named(table).is_none() {
                self.errors.push(TypeError::new(
                    TypeErrorKind::DatabaseTableNotATable { database: name.clone(), name: table.name.clone() },
                    table.span,
                ));
            }
        }
    }

    /// `atomic:` (B09 P7a). The body is checked as any block, then
    /// refused anything the database cannot undo, any way out of the
    /// block that would not say whether to keep its writes, and tables
    /// in more than one database.
    pub(super) fn check_atomic(&mut self, body: &Block, span: Span) {
        self.check_block(body);
        let mut refusals = Vec::new();
        atomic_exits(body, 0, &mut refusals);
        let mut visited = Vec::new();
        self.atomic_contents(body, None, &mut visited, &mut refusals);

        let tables = crate::table_ops::atomic_tables(body, self.file, self.symbols, self.bindings);
        let mut databases: Vec<&str> = Vec::new();
        for table in &tables {
            if let Some(db) = crate::table_ops::table_database(self.file, &table.name().name) {
                if !databases.contains(&db.name.name.as_str()) {
                    databases.push(&db.name.name);
                }
            }
        }
        if databases.len() > 1 {
            refusals.push((
                span,
                format!(
                    "an `atomic:` block writes to one database, but this one reaches {}; a transaction cannot span databases",
                    quoted(&databases)
                ),
            ));
        }
        for (span, message) in refusals {
            self.errors.push(TypeError::new(TypeErrorKind::AtomicBlockRefused { message }, span));
        }
    }

    /// Refuse what the database cannot undo in `body`, following the
    /// agents it calls. `through` names the agent being followed.
    fn atomic_contents(
        &self,
        body: &Block,
        through: Option<&str>,
        visited: &mut Vec<String>,
        refusals: &mut Vec<(Span, String)>,
    ) {
        let mut stmts = Vec::new();
        let mut calls = Vec::new();
        corvid_ast::visit::walk_block(body, &mut |stmt| stmts.push(stmt), &mut |expr| {
            if let Expr::Call { callee, args, span } = expr {
                calls.push((callee.as_ref(), args.as_slice(), *span));
            }
        });
        let at = |span: Span, what: &str| {
            let via = through.map(|agent| format!(" (in agent `{agent}`, called inside the block)")).unwrap_or_default();
            (span, format!("an `atomic:` block cannot hold {what}{via}, because the database cannot undo it"))
        };
        for stmt in stmts {
            match stmt {
                Stmt::Atomic { span, .. } if through.is_some() => refusals.push(at(*span, "another `atomic:` block")),
                Stmt::Approve { span, .. } => refusals.push(at(*span, "an `approve`")),
                Stmt::Yield { span, .. } => refusals.push(at(*span, "a `yield`")),
                Stmt::Parallel { span, .. } => refusals.push(at(*span, "a `parallel:` block")),
                _ => {}
            }
        }
        for (callee, args, span) in calls {
            let table_operation = crate::table_ops::table_call_target(callee, self.file, self.symbols, self.bindings).is_some()
                || crate::table_ops::table_query_call(callee, args, self.file, self.symbols, self.bindings).is_some();
            if table_operation {
                continue;
            }
            if let Some(agent) = crate::table_ops::called_agent(callee, self.file, self.symbols, self.bindings) {
                if !visited.contains(&agent.name.name) {
                    visited.push(agent.name.name.clone());
                    self.atomic_contents(&agent.body, Some(&agent.name.name), visited, refusals);
                }
                continue;
            }
            if let Some(what) = self.call_the_database_cannot_undo(callee) {
                refusals.push(at(span, &what));
            }
        }
    }

    /// A call that acts outside the database, described for an error.
    fn call_the_database_cannot_undo(&self, callee: &Expr) -> Option<String> {
        if let Some((import, function)) = crate::effects::python_call_target(callee, self.file, self.symbols, self.bindings) {
            return Some(format!("a call to Python function `{}.{}`", import.module, function.name.name));
        }
        match callee {
            Expr::Ident { name, .. } => match self.bindings.get(&name.span) {
                Some(Binding::Decl(id)) => match self.symbols.get(*id).kind {
                    DeclKind::Tool => Some(format!("a call to tool `{}`", name.name)),
                    DeclKind::Prompt => Some(format!("a call to prompt `{}`", name.name)),
                    DeclKind::Fixture => Some(format!("a call to fixture `{}`", name.name)),
                    DeclKind::ImportedUse
                        if !self
                            .module_resolution
                            .and_then(|modules| modules.lookup_imported_use(&name.name))
                            .is_some_and(|target| matches!(target.export.kind, DeclKind::Fn | DeclKind::Type)) =>
                    {
                        Some(format!("a call to imported `{}`", name.name))
                    }
                    _ => None,
                },
                Some(Binding::BuiltIn(BuiltIn::Ask | BuiltIn::Choose)) => Some(format!("`{}(...)`", name.name)),
                _ => None,
            },
            Expr::FieldAccess { target, field, .. } => {
                let Expr::Ident { name, .. } = target.as_ref() else { return None };
                let module_alias = matches!(self.bindings.get(&name.span),
                    Some(Binding::Decl(id)) if self.symbols.get(*id).kind == DeclKind::Import);
                let member = self.module_resolution.map(|modules| modules.lookup_member(&name.name, &field.name));
                let acts = module_alias
                    && matches!(member,
                        Some(corvid_resolve::ModuleLookup::Found { export, .. })
                            if matches!(export.kind, DeclKind::Agent | DeclKind::Tool | DeclKind::Prompt | DeclKind::Fixture));
                acts.then(|| format!("a call to imported `{}.{}`", name.name, field.name))
            }
            _ => None,
        }
    }

    /// A networked engine's connection string comes from the
    /// environment and its encryption is stated; a file has neither.
    fn check_database_connection(&mut self, database: &DatabaseDecl) {
        /// The most connections a pool may declare: more than any one
        /// Postgres server is configured to accept by default (100).
        const MAX_CONNECTIONS: i64 = 1000;
        let name = database.name.name.clone();
        let invalid = |setting: &str, message: &str, span| {
            TypeError::new(
                TypeErrorKind::DatabaseSettingInvalid {
                    database: name.clone(),
                    setting: setting.into(),
                    message: message.into(),
                },
                span,
            )
        };
        match database.engine.as_ref().map(|e| e.name.as_str()) {
            Some("postgres") => {
                if matches!(database.location, Some(DatabaseLocation::Path(_))) {
                    self.errors.push(invalid(
                        "location",
                        "is written in source, but a Postgres connection string usually holds a password; read it from the environment with `env(\"NAME\")`",
                        database.span,
                    ));
                }
                match &database.tls {
                    None => self.errors.push(TypeError::new(
                        TypeErrorKind::DatabaseSettingRequired {
                            database: name.clone(),
                            setting: "tls".into(),
                            allowed: "`required` or `disabled`".into(),
                        },
                        database.span,
                    )),
                    Some(tls) if TLS_MODES.contains(&tls.name.as_str()) => {}
                    Some(tls) => self.errors.push(invalid(
                        "tls",
                        &format!("is `{}`; it is `required` or `disabled`", tls.name),
                        tls.span,
                    )),
                }
                // How many connections the database may be asked for is a
                // cost and availability decision (a managed database caps
                // them), so the source states it.
                match database.connections {
                    None => self.errors.push(TypeError::new(
                        TypeErrorKind::DatabaseSettingRequired {
                            database: name.clone(),
                            setting: "connections".into(),
                            allowed: format!("how many connections the pool holds at most, from 1 to {MAX_CONNECTIONS}"),
                        },
                        database.span,
                    )),
                    Some((count, span)) if !(1..=MAX_CONNECTIONS).contains(&count) => self.errors.push(invalid(
                        "connections",
                        &format!("is {count}; it is from 1 to {MAX_CONNECTIONS}"),
                        span,
                    )),
                    Some(_) => {}
                }
            }
            Some("sqlite") => {
                if let Some(tls) = &database.tls {
                    self.errors.push(invalid("tls", "is set, but a SQLite database is a file and has no connection to encrypt", tls.span));
                }
                if let Some((_, span)) = database.connections {
                    self.errors.push(invalid(
                        "connections",
                        "is set, but a SQLite database is a file: Corvid opens it as needed and queues its writers",
                        span,
                    ));
                }
            }
            _ => {}
        }
    }

    fn table_named(&self, name: &Ident) -> Option<&'a TableDecl> {
        self.file.decls.iter().find_map(|decl| match decl {
            Decl::Table(table) if table.name().name == name.name => Some(table),
            _ => None,
        })
    }

    /// `Shelf.get(key)`, `Shelf.insert(row)`, `Shelf.save(row)` or
    /// `Shelf.delete(row)`. Returns `None` when the callee is not a
    /// table operation, so ordinary method dispatch continues.
    pub(super) fn check_table_method_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Option<Type> {
        let (table, op) = table_method_target(receiver, &method.name, self.file, self.symbols, self.bindings)?;
        let row_id = self.symbols.lookup_def(&table.name().name)?;
        let row = Type::Struct(row_id);
        // `get` takes the key's values in declaration order; the other
        // operations take one row.
        let (params, ret) = match op {
            TableOp::Get => {
                let key_types = table
                    .keys
                    .iter()
                    .map(|key| {
                        table
                            .declared_fields()
                            .iter()
                            .find(|f| f.name.name == key.name)
                            .map(|field| self.type_ref_to_type(&field.ty))
                            .unwrap_or(Type::Unknown)
                    })
                    .collect::<Vec<_>>();
                (key_types, Type::Option(Box::new(row.clone())))
            }
            TableOp::Insert => (vec![row.clone()], Type::Result(Box::new(row.clone()), Box::new(row.clone()))),
            TableOp::Save | TableOp::Delete => (
                vec![row.clone()],
                Type::Result(Box::new(row.clone()), Box::new(Type::Option(Box::new(row.clone())))),
            ),
        };
        let label = format!("{}.{}", table.name().name, op.name());
        if args.len() != params.len() {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch { callee: label, expected: params.len(), got: args.len() },
                span,
            ));
            for arg in args {
                let _ = self.check_expr(arg);
            }
            return Some(ret);
        }
        for (arg, param) in args.iter().zip(&params) {
            let got = self.check_expr_as(arg, Some(param));
            if !got.is_assignable_to(param) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: param.display_name(),
                        got: got.display_name(),
                        context: format!("argument to `{label}`"),
                    },
                    arg.span(),
                ));
            }
        }
        Some(ret)
    }
}

impl<'a> Checker<'a> {
    /// A query chain ending in `receiver.method(args)`: `Shelf.where(...)`
    /// or `Shelf.all()`, then optional `order_by`, `descending`, `take`.
    /// Returns `None` when the call does not end a query.
    pub(super) fn check_table_query_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Option<Type> {
        let query = crate::table_ops::table_query_target(receiver, &method.name, args, self.file, self.symbols, self.bindings)?;
        let row_id = self.symbols.lookup_def(&query.table.name().name)?;
        let row = Type::Struct(row_id);
        let function = |ret: Type| Type::Function {
            params: vec![row.clone()],
            ret: Box::new(ret),
            effect: corvid_ast::Effect::Safe,
        };
        if let Some(filter) = query.filter {
            let ty = self.check_expr_as(filter, Some(&function(Type::Bool)));
            self.require_function_of_row(filter, &ty, &row, "where");
            if let Expr::Lambda { params, body, .. } = filter {
                if let Some(param) = params.first() {
                    self.check_translatable_condition(query.table, param, body);
                }
            }
        }
        if let Some(order) = query.order {
            let ty = self.check_expr_as(order, Some(&function(Type::Unknown)));
            self.require_function_of_row(order, &ty, &row, "order_by");
            let column = match order {
                Expr::Lambda { params, body, .. } => params.first().and_then(|p| column_read(query.table, p, body)),
                _ => None,
            };
            match column {
                None => self.query_error(order.span(), "`order_by` takes a function that reads one column, such as `fn (s) -> s.title`"),
                Some(column) => self.check_column_is_orderable(query.table, &column, order.span()),
            }
        }
        if let Some(count) = query.take {
            let got = self.check_expr_as(count, Some(&Type::Int));
            if !got.is_assignable_to(&Type::Int) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: "Int".into(),
                        got: got.display_name(),
                        context: "argument to `take`".into(),
                    },
                    count.span(),
                ));
            }
        }
        let _ = span;
        Some(Type::List(Box::new(row)))
    }

    fn require_function_of_row(&mut self, expr: &Expr, ty: &Type, row: &Type, stage: &str) {
        if !matches!(expr, Expr::Lambda { .. }) {
            self.query_error(expr.span(), &format!("`{stage}` takes a function written in place, such as `fn (s) -> ...`, so the database can run it"));
            return;
        }
        if let Type::Function { params, .. } = ty {
            if params.first().is_some_and(|p| p != row && !matches!(p, Type::Unknown)) {
                self.query_error(expr.span(), &format!("the `{stage}` function's parameter must be the table's row"));
            }
        }
    }

    /// Refuse the parts of a `where` condition the database cannot run,
    /// through the same reading IR lowering uses.
    fn check_translatable_condition(&mut self, table: &TableDecl, row: &corvid_ast::LambdaParam, condition: &Expr) {
        match crate::table_ops::query_condition(table, &row.name.name, condition) {
            Ok(read) => self.check_query_condition(table, row, &read, condition.span()),
            Err(refusal) => self.query_error(refusal.span, refusal.message),
        }
    }

    /// Every column the condition compares must be stored as a scalar,
    /// and every value compared with one must not read the row.
    fn check_query_condition(&mut self, table: &TableDecl, row: &corvid_ast::LambdaParam, condition: &QueryCondition<'_>, span: Span) {
        match condition {
            QueryCondition::Const(_) => {}
            QueryCondition::Column(column) => self.check_column_is_queryable(table, column, span),
            QueryCondition::And(left, right) | QueryCondition::Or(left, right) => {
                self.check_query_condition(table, row, left, span);
                self.check_query_condition(table, row, right, span);
            }
            QueryCondition::Not(inner) => self.check_query_condition(table, row, inner, span),
            QueryCondition::Compare { column, value, .. } | QueryCondition::Text { column, value, .. } => {
                self.check_column_is_queryable(table, column, value.span());
                self.check_agent_value(row, value);
            }
        }
    }

    /// A column the database can compare: not one stored as JSON. The
    /// rule is the one the runtime stores columns by.
    fn check_column_is_queryable(&mut self, table: &TableDecl, column: &str, span: Span) {
        let Some(field) = table.record.fields.iter().find(|f| f.name.name == column) else { return };
        let ty = self.type_ref_to_type(&field.ty);
        if crate::table_columns::column_kind(&ty).0 == crate::table_columns::ColumnKind::Json {
            self.query_error(span, &format!("column `{column}` is stored as JSON, so the database cannot compare it"));
        }
    }

    /// A column the database can order as Corvid does: not one stored as
    /// JSON, whose text would sort in the wrong order, and not one whose
    /// type has no order.
    fn check_column_is_orderable(&mut self, table: &TableDecl, column: &str, span: Span) {
        use crate::table_columns::{column_kind, ColumnKind};
        let Some(field) = table.record.fields.iter().find(|f| f.name.name == column) else { return };
        let ty = self.type_ref_to_type(&field.ty);
        let kind = column_kind(&ty).0;
        if kind == ColumnKind::Json {
            self.query_error(span, &format!("column `{column}` is stored as JSON, so the database cannot order it"));
        } else if !kind.is_ordered() {
            self.errors.push(TypeError::new(
                TypeErrorKind::TableOrderUnordered { column: column.to_string(), type_name: kind.type_name().to_string() },
                span,
            ));
        }
    }

    /// A value compared with a column is computed by the agent before
    /// the query runs, so it must not read the row.
    fn check_agent_value(&mut self, row: &corvid_ast::LambdaParam, value: &Expr) {
        if mentions_ident(value, &row.name.name) {
            self.query_error(value.span(), "a value compared with a column cannot read the row; compare one column with a value from the agent");
        }
    }

    fn query_error(&mut self, span: Span, message: &str) {
        self.errors.push(TypeError::new(
            TypeErrorKind::TableQueryNotTranslatable { message: message.to_string() },
            span,
        ));
    }
}

/// A `return`, or a `break` or `continue` that is not inside a loop of
/// its own within the block, would leave an `atomic:` block without
/// saying whether to keep its writes.
fn atomic_exits(block: &Block, loops: usize, refusals: &mut Vec<(Span, String)>) {
    const FINISH: &str = "finish the block, then leave";
    for stmt in &block.stmts {
        match stmt {
            Stmt::Return { span, .. } => refusals.push((
                *span,
                format!("a `return` inside an `atomic:` block would leave without saying whether to keep its writes; {FINISH}"),
            )),
            Stmt::Break { span } | Stmt::Continue { span } if loops == 0 => refusals.push((
                *span,
                format!("a `break` or `continue` here would leave the `atomic:` block without saying whether to keep its writes; {FINISH}"),
            )),
            Stmt::For { body, .. } | Stmt::While { body, .. } => atomic_exits(body, loops + 1, refusals),
            Stmt::If { then_block, else_block, .. } => {
                atomic_exits(then_block, loops, refusals);
                if let Some(else_block) = else_block {
                    atomic_exits(else_block, loops, refusals);
                }
            }
            Stmt::Atomic { span, .. } => refusals.push((*span, "an `atomic:` block cannot hold another one; its writes already commit together".into())),
            _ => {}
        }
    }
}

/// The column `expr` reads, through the one rule IR lowering also uses.
fn column_read(table: &TableDecl, row: &corvid_ast::LambdaParam, expr: &Expr) -> Option<String> {
    crate::table_ops::query_column(table, &row.name.name, expr)
}

fn mentions_ident(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Ident { name: ident, .. } => ident.name == name,
        Expr::Literal { .. } => false,
        Expr::FieldAccess { target, .. } => mentions_ident(target, name),
        Expr::Call { callee, args, .. } => mentions_ident(callee, name) || args.iter().any(|a| mentions_ident(a, name)),
        Expr::BinOp { left, right, .. } => mentions_ident(left, name) || mentions_ident(right, name),
        Expr::UnOp { operand, .. } => mentions_ident(operand, name),
        Expr::Index { target, index, .. } => mentions_ident(target, name) || mentions_ident(index, name),
        Expr::List { items, .. } => items.iter().any(|i| mentions_ident(i, name)),
        // Anything richer is not a plain agent value; treat it as reading the row.
        _ => true,
    }
}

fn quoted(names: &[&str]) -> String {
    names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")
}

fn type_ref_text(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Named { name, .. } => name.name.clone(),
        TypeRef::Generic { name, args, .. } => {
            format!("{}<{}>", name.name, args.iter().map(type_ref_text).collect::<Vec<_>>().join(", "))
        }
        _ => "this type".into(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{typecheck, Checked, TypeErrorKind};

    fn check(src: &str) -> Checked {
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, errors) = corvid_syntax::parse_file(&tokens);
        assert!(errors.is_empty(), "parse: {errors:?}");
        let resolved = corvid_resolve::resolve(&file);
        assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
        typecheck(&file, &resolved)
    }

    fn messages(src: &str) -> Vec<String> {
        check(src).errors.iter().map(|e| e.kind.message()).collect()
    }

    const LIBRARY: &str = "
database library:
    engine: sqlite
    location: env(\"LIBRARY_DATABASE\")
    tables: Shelf

table Shelf:
    key shelf: Int
    owner: String
    title: String where len_between(1, 80)
    note: Option<String>
";

    #[test]
    fn table_operations_are_typed_from_the_row() {
        let src = format!(
            "{LIBRARY}
agent find(id: Int) -> Option<Shelf>:
    return Shelf.get(id)

agent add(owner: String) -> Result<Shelf, Shelf>:
    return Shelf.insert(Shelf(1, owner, \"Reading\", None))

agent rename(shelf: Shelf, title: String) -> Result<Shelf, Option<Shelf>>:
    return Shelf.save(Shelf {{ title: title, ..shelf }})

agent seen(shelf: Shelf) -> Int:
    return shelf.revision
"
        );
        let errors = messages(&src);
        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn a_wrong_key_or_row_type_is_refused() {
        let errors = messages(&format!("{LIBRARY}\nagent find(id: String) -> Option<Shelf>:\n    return Shelf.get(id)\n"));
        assert!(errors.iter().any(|m| m.contains("argument to `Shelf.get`")), "{errors:#?}");
    }

    #[test]
    fn every_consequential_setting_must_be_stated() {
        let errors = messages("database library:\n    tables: Shelf\n\ntable Shelf:\n    title: String\n");
        assert!(errors.iter().any(|m| m.contains("does not say `engine`")), "{errors:#?}");
        assert!(errors.iter().any(|m| m.contains("does not say `location`")), "{errors:#?}");
        assert!(errors.iter().any(|m| m.contains("declares no key field")), "{errors:#?}");
    }

    #[test]
    fn a_table_must_live_in_exactly_one_running_database() {
        let outside = messages("table Shelf:\n    key shelf: Int\n");
        assert!(outside.iter().any(|m| m.contains("not listed in any `database` block")), "{outside:#?}");
        let planned = messages("database library:\n    engine: mysql\n    location: \"x\"\n    tables: Shelf\n\ntable Shelf:\n    key shelf: Int\n");
        assert!(planned.iter().any(|m| m.contains("names engine `mysql`")), "{planned:#?}");
    }

    #[test]
    fn revision_is_reserved_and_pure_code_cannot_touch_tables() {
        let reserved = check("database d:\n    engine: sqlite\n    location: \"x\"\n    tables: T\n\ntable T:\n    key id: Int\n    revision: Int\n");
        assert!(reserved.errors.iter().any(|e| matches!(e.kind, TypeErrorKind::TableFieldReserved { .. })));
        let pure = messages(&format!("{LIBRARY}\nfn peek(id: Int) -> Option<Shelf>:\n    return Shelf.get(id)\n"));
        assert!(pure.iter().any(|m| m.contains("reads the database")), "{pure:#?}");
    }

    #[test]
    fn a_key_of_several_fields_is_read_with_one_value_each() {
        let table = "database d:\n    engine: sqlite\n    location: \"x\"\n    tables: Edit\n\ntable Edit:\n    key proposal: String\n    key shelf: Int\n    title: String\n";
        let ok = messages(&format!("{table}\nagent q(p: String) -> Option<Edit>:\n    return Edit.get(p, 1)\n"));
        assert!(ok.is_empty(), "{ok:#?}");
        let one = messages(&format!("{table}\nagent q(p: String) -> Option<Edit>:\n    return Edit.get(p)\n"));
        assert!(one.iter().any(|m| m.contains("Edit.get")), "{one:#?}");
        let swapped = messages(&format!("{table}\nagent q(p: String) -> Option<Edit>:\n    return Edit.get(1, p)\n"));
        assert!(swapped.iter().any(|m| m.contains("argument to `Edit.get`")), "{swapped:#?}");
        let bad = messages("database d:\n    engine: sqlite\n    location: \"x\"\n    tables: T\n\ntable T:\n    key id: Int\n    key score: Float\n");
        assert!(bad.iter().any(|m| m.contains("the key `score`")), "{bad:#?}");
    }

    #[test]
    fn an_atomic_block_holds_only_what_the_database_can_undo() {
        let program = |body: &str| {
            messages(&format!(
                "{LIBRARY}\ntool notify(id: Int) -> String\n\nprompt summarize(text: String) -> String:\n    \"Summarize {{text}}\"\n\nagent reads(id: Int) -> Option<Shelf>:\n    return Shelf.get(id)\n\nagent asks(text: String) -> String:\n    return summarize(text)\n\nagent q(shelf: Shelf) -> String:\n{body}\n    return \"done\"\n"
            ))
        };
        let accepted = program("    atomic:\n        saved = Shelf.save(shelf)\n        found = reads(1)\n        for n in [1, 2]:\n            if n == 2:\n                break\n");
        assert!(accepted.is_empty(), "{accepted:#?}");
        let tool = program("    atomic:\n        saved = Shelf.save(shelf)\n        sent = notify(1)");
        assert!(tool.iter().any(|m| m.contains("cannot hold a call to tool `notify`")), "{tool:#?}");
        let through_agent = program("    atomic:\n        text = asks(\"x\")");
        assert!(through_agent.iter().any(|m| m.contains("a call to prompt `summarize` (in agent `asks`")), "{through_agent:#?}");
        let returned = program("    atomic:\n        saved = Shelf.save(shelf)\n        return \"early\"");
        assert!(returned.iter().any(|m| m.contains("a `return` inside an `atomic:` block")), "{returned:#?}");
        let nested = program("    atomic:\n        atomic:\n            saved = Shelf.save(shelf)");
        assert!(nested.iter().any(|m| m.contains("cannot hold another one")), "{nested:#?}");
    }

    #[test]
    fn an_atomic_block_writes_to_one_database() {
        let errors = messages(
            "database a:\n    engine: sqlite\n    location: \"a\"\n    tables: T\n\ndatabase b:\n    engine: sqlite\n    location: \"b\"\n    tables: U\n\ntable T:\n    key id: Int\n\ntable U:\n    key id: Int\n\nagent q() -> String:\n    atomic:\n        t = T.get(1)\n        u = U.get(1)\n    return \"x\"\n",
        );
        assert!(errors.iter().any(|m| m.contains("a transaction cannot span databases")), "{errors:#?}");
    }

    #[test]
    fn a_postgres_connection_is_read_from_the_environment_and_its_tls_stated() {
        let block = |settings: &str| messages(&format!("database d:\n    engine: postgres\n{settings}\n    tables: T\n\ntable T:\n    key id: Int\n"));
        assert!(block("    location: env(\"DATABASE_URL\")\n    tls: required\n    connections: 10").is_empty());
        assert!(block("    location: env(\"DATABASE_URL\")\n    tls: disabled\n    connections: 1").is_empty());
        let literal = block("    location: \"postgres://user:secret@db/app\"\n    tls: required\n    connections: 10");
        assert!(literal.iter().any(|m| m.contains("read it from the environment")), "{literal:#?}");
        let silent = block("    location: env(\"DATABASE_URL\")");
        assert!(silent.iter().any(|m| m.contains("does not say `tls`")), "{silent:#?}");
        let unknown = block("    location: env(\"DATABASE_URL\")\n    tls: maybe");
        assert!(unknown.iter().any(|m| m.contains("`required` or `disabled`")), "{unknown:#?}");
        let file = messages("database d:\n    engine: sqlite\n    location: \"x\"\n    tls: required\n    tables: T\n\ntable T:\n    key id: Int\n");
        assert!(file.iter().any(|m| m.contains("no connection to encrypt")), "{file:#?}");
    }

    /// How many connections Postgres is asked for is a cost and
    /// availability decision, so it is declared, never defaulted.
    #[test]
    fn a_postgres_pool_size_is_declared_and_bounded() {
        let block = |settings: &str| messages(&format!("database d:\n    engine: postgres\n    location: env(\"DATABASE_URL\")\n    tls: required\n{settings}    tables: T\n\ntable T:\n    key id: Int\n"));
        let silent = block("");
        assert!(silent.iter().any(|m| m.contains("does not say `connections`") && m.contains("from 1 to 1000")), "{silent:#?}");
        let none = block("    connections: 0\n");
        assert!(none.iter().any(|m| m.contains("is 0; it is from 1 to 1000")), "{none:#?}");
        let file = messages("database d:\n    engine: sqlite\n    location: \"x\"\n    connections: 4\n    tables: T\n\ntable T:\n    key id: Int\n");
        assert!(file.iter().any(|m| m.contains("queues its writers")), "{file:#?}");
    }

    #[test]
    fn column_history_names_each_stored_column_once() {
        let table = |body: &str| {
            messages(&format!("database d:\n    engine: sqlite\n    location: \"x\"\n    tables: T\n\ntable T:\n    key id: Int\n{body}\n"))
        };
        assert!(table("    title: String renamed from name\n    retired: Bool\n    retired legacy, old_note").is_empty());
        let declared = table("    title: String renamed from id");
        assert!(declared.iter().any(|m| m.contains("`id` is a column the table declares")), "{declared:#?}");
        let twice = table("    title: String renamed from name\n    retired name");
        assert!(twice.iter().any(|m| m.contains("already named by another clause")), "{twice:#?}");
        let revision = table("    retired revision");
        assert!(revision.iter().any(|m| m.contains("`revision` is a column the table declares")), "{revision:#?}");
    }

    #[test]
    fn a_query_chain_is_a_list_of_rows() {
        let src = format!(
            "{LIBRARY}
agent mine(owner: String, limit: Int) -> List<Shelf>:
    return Shelf.where(fn (s) -> s.owner == owner and not (s.note == None) or 3 < s.shelf).order_by(fn (s) -> s.title).descending().take(limit)

agent titled(prefix: String) -> List<Shelf>:
    return Shelf.where(fn (s) -> s.title.starts_with(prefix) or s.title.contains(\"x\") or s.title.ends_with(\"y\"))

agent every() -> List<Shelf>:
    return Shelf.all()
"
        );
        let errors = messages(&src);
        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn a_condition_the_database_cannot_run_is_refused() {
        let refused = |condition: &str, expected: &str| {
            let errors = messages(&format!(
                "{LIBRARY}\nfn shout(text: String) -> String:\n    return text\n\nfn long(text: String) -> Bool:\n    return text == \"long\"\n\nagent q() -> List<Shelf>:\n    return Shelf.where(fn (s) -> {condition})\n"
            ));
            assert!(errors.iter().any(|m| m.contains(expected)), "{condition}: {errors:#?}");
        };
        refused("s.owner == s.title", "compares one column with a value");
        refused("shout(s.title) == \"A\"", "compares one column with a value");
        refused("long(s.title)", "only `contains`, `starts_with` and `ends_with`");
        refused("s.title.contains(shout(s.owner))", "cannot read the row");
        refused("s.title.to_upper() == \"A\"", "compares one column with a value");
    }

    #[test]
    fn a_json_column_cannot_be_compared() {
        let errors = messages(
            "database d:\n    engine: sqlite\n    location: \"x\"\n    tables: T\n\ntable T:\n    key id: Int\n    tags: List<String>\n\nagent q(tags: List<String>) -> List<T>:\n    return T.where(fn (t) -> t.tags == tags)\n",
        );
        assert!(errors.iter().any(|m| m.contains("stored as JSON")), "{errors:#?}");
    }

    #[test]
    fn order_by_reads_one_column_and_take_counts() {
        let order = messages(&format!("{LIBRARY}\nagent q() -> List<Shelf>:\n    return Shelf.all().order_by(fn (s) -> s.shelf + 1)\n"));
        assert!(order.iter().any(|m| m.contains("reads one column")), "{order:#?}");
        let take = messages(&format!("{LIBRARY}\nagent q() -> List<Shelf>:\n    return Shelf.all().take(\"3\")\n"));
        assert!(take.iter().any(|m| m.contains("argument to `take`")), "{take:#?}");
    }

    #[test]
    fn pure_code_cannot_query_a_table() {
        let errors = messages(&format!("{LIBRARY}\nfn peek() -> List<Shelf>:\n    return Shelf.all()\n"));
        assert!(errors.iter().any(|m| m.contains("a query of table `Shelf`")), "{errors:#?}");
        let deterministic = messages(&format!("{LIBRARY}\n@deterministic\nagent peek() -> List<Shelf>:\n    return Shelf.all()\n"));
        assert!(deterministic.iter().any(|m| m.contains("database operation")), "{deterministic:#?}");
    }
}