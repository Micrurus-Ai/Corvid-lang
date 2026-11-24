//! Table and database checks (B09).
//!
//! A table is a record type stored in a database. The checker enforces
//! the decisions the source must state (a key, an engine, a location,
//! which database holds each table) and types the table's operations:
//! `get`, `insert`, `save` and `delete`.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::table_ops::{table_method_target, TableOp};
use crate::types::Type;
use corvid_ast::{DatabaseDecl, Decl, Expr, Ident, Span, TableDecl, TypeRef, Visibility};

/// The engines the runtime runs today.
const ENGINES: &[&str] = &["sqlite"];
/// Engines on the roadmap, refused with a pointer to their slice.
const PLANNED_ENGINES: &[&str] = &["postgres", "mysql"];

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
        let planned = messages("database library:\n    engine: postgres\n    location: \"x\"\n    tables: Shelf\n\ntable Shelf:\n    key shelf: Int\n");
        assert!(planned.iter().any(|m| m.contains("names engine `postgres`")), "{planned:#?}");
    }

    #[test]
    fn revision_is_reserved_and_pure_code_cannot_touch_tables() {
        let reserved = check("database d:\n    engine: sqlite\n    location: \"x\"\n    tables: T\n\ntable T:\n    key id: Int\n    revision: Int\n");
        assert!(reserved.errors.iter().any(|e| matches!(e.kind, TypeErrorKind::TableFieldReserved { .. })));
        let pure = messages(&format!("{LIBRARY}\nfn peek(id: Int) -> Option<Shelf>:\n    return Shelf.get(id)\n"));
        assert!(pure.iter().any(|m| m.contains("reads the database")), "{pure:#?}");
    }
}