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
use corvid_ast::{DatabaseDecl, DatabaseLocation, Decl, Expr, Ident, Span, TableDecl, TypeRef, Visibility};

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
        match &table.key {
            None => self.errors.push(TypeError::new(TypeErrorKind::TableKeyRequired { table: name.clone() }, span)),
            Some(key) => {
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
        }
        self.check_field_refinements(&table.record);
        self.check_column_history(table);

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

    /// A networked engine's connection string comes from the
    /// environment and its encryption is stated; a file has neither.
    fn check_database_connection(&mut self, database: &DatabaseDecl) {
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
            }
            Some("sqlite") => {
                if let Some(tls) = &database.tls {
                    self.errors.push(invalid("tls", "is set, but a SQLite database is a file and has no connection to encrypt", tls.span));
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
        let (param, ret) = match op {
            TableOp::Get => {
                let key_ty = table
                    .key
                    .as_ref()
                    .and_then(|key| table.declared_fields().iter().find(|f| f.name.name == key.name))
                    .map(|field| self.type_ref_to_type(&field.ty))
                    .unwrap_or(Type::Unknown);
                (key_ty, Type::Option(Box::new(row.clone())))
            }
            TableOp::Insert => (row.clone(), Type::Result(Box::new(row.clone()), Box::new(row.clone()))),
            TableOp::Save | TableOp::Delete => (
                row.clone(),
                Type::Result(Box::new(row.clone()), Box::new(Type::Option(Box::new(row.clone())))),
            ),
        };
        let label = format!("{}.{}", table.name().name, op.name());
        if args.len() != 1 {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch { callee: label, expected: 1, got: args.len() },
                span,
            ));
            for arg in args {
                let _ = self.check_expr(arg);
            }
            return Some(ret);
        }
        let got = self.check_expr_as(&args[0], Some(&param));
        if !got.is_assignable_to(&param) {
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeMismatch {
                    expected: param.display_name(),
                    got: got.display_name(),
                    context: format!("argument to `{label}`"),
                },
                args[0].span(),
            ));
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
            if column.is_none() {
                self.query_error(order.span(), "`order_by` takes a function that reads one column, such as `fn (s) -> s.title`");
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

    /// A column the database can compare: not one stored as JSON.
    fn check_column_is_queryable(&mut self, table: &TableDecl, column: &str, span: Span) {
        let Some(field) = table.record.fields.iter().find(|f| f.name.name == column) else { return };
        let ty = self.type_ref_to_type(&field.ty);
        if !queryable_column_type(&ty) {
            self.query_error(span, &format!("column `{column}` is stored as JSON, so the database cannot compare it"));
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

/// The column `expr` reads, through the one rule IR lowering also uses.
fn column_read(table: &TableDecl, row: &corvid_ast::LambdaParam, expr: &Expr) -> Option<String> {
    crate::table_ops::query_column(table, &row.name.name, expr)
}

/// Scalar columns and options of scalars; other types are stored as JSON.
fn queryable_column_type(ty: &Type) -> bool {
    match ty {
        Type::Int | Type::Float | Type::String | Type::Bool => true,
        Type::Tainted(inner) => queryable_column_type(inner),
        Type::Option(inner) => matches!(**inner, Type::Int | Type::Float | Type::String | Type::Bool),
        _ => false,
    }
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
    fn a_postgres_connection_is_read_from_the_environment_and_its_tls_stated() {
        let block = |settings: &str| messages(&format!("database d:\n    engine: postgres\n{settings}\n    tables: T\n\ntable T:\n    key id: Int\n"));
        assert!(block("    location: env(\"DATABASE_URL\")\n    tls: required").is_empty());
        assert!(block("    location: env(\"DATABASE_URL\")\n    tls: disabled").is_empty());
        let literal = block("    location: \"postgres://user:secret@db/app\"\n    tls: required");
        assert!(literal.iter().any(|m| m.contains("read it from the environment")), "{literal:#?}");
        let silent = block("    location: env(\"DATABASE_URL\")");
        assert!(silent.iter().any(|m| m.contains("does not say `tls`")), "{silent:#?}");
        let unknown = block("    location: env(\"DATABASE_URL\")\n    tls: maybe");
        assert!(unknown.iter().any(|m| m.contains("`required` or `disabled`")), "{unknown:#?}");
        let file = messages("database d:\n    engine: sqlite\n    location: \"x\"\n    tls: required\n    tables: T\n\ntable T:\n    key id: Int\n");
        assert!(file.iter().any(|m| m.contains("no connection to encrypt")), "{file:#?}");
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