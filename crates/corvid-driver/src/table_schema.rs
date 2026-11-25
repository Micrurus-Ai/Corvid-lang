//! The declared shape of a program's tables (B09), for planning
//! migrations: each table's name, key, checked column types and its
//! `renamed from` and `retired` clauses, and the engine its database runs on.

use std::path::Path;

use crate::{compile_to_ir_with_config_at_path, load_corvid_config_for, Diagnostic};
use corvid_ast::Decl;
use corvid_ir::{IrExprKind, IrFile, IrTableFilter};
use corvid_runtime::tables::{derive_indexes, QueryShape, TableEngine, TableSchema};
use corvid_types::table_ops::QueryCompare;

/// A `database` block as `corvid migrate` connects to it.
#[derive(Debug, Clone)]
pub struct DeclaredDatabase {
    pub name: String,
    pub engine: TableEngine,
    pub location: corvid_ast::DatabaseLocation,
    pub tables: Vec<String>,
}

/// Compile the program at `source_path` exactly as `corvid check` does:
/// imports resolve from the file's directory and its `corvid.toml`
/// applies. Returns the IR and the root module's parsed declarations.
fn compile_program(source: &str, source_path: &Path) -> Result<(corvid_ir::IrFile, corvid_ast::File), Vec<Diagnostic>> {
    let config = load_corvid_config_for(source_path);
    let ir = compile_to_ir_with_config_at_path(source, source_path, config.as_ref())?;
    let tokens = corvid_syntax::lex(source).map_err(|_| Vec::new())?;
    let (file, _) = corvid_syntax::parse_file(&tokens);
    Ok((ir, file))
}

/// The databases of the program at `source_path`: those its own module
/// declares and those its modules' tables live in, imported or not, each
/// with every table that lives in it. The program must compile, so each
/// names a running engine and a location.
pub fn declared_databases(source: &str, source_path: &Path) -> Result<Vec<DeclaredDatabase>, Vec<Diagnostic>> {
    let (_, file) = compile_program(source, source_path)?;
    let (modules, _, _) = crate::modules::resolve_module_graph(&file, source_path, &|_| None);
    let placed = placed_tables(&file, &modules);
    let mut databases: Vec<&corvid_ast::DatabaseDecl> = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Database(db) => Some(db),
            _ => None,
        })
        .collect();
    for (_, database) in &placed {
        if !databases.iter().any(|known| std::ptr::eq(*known, *database)) {
            databases.push(database);
        }
    }
    Ok(databases
        .into_iter()
        .filter_map(|db| {
            Some(DeclaredDatabase {
                name: db.name.name.clone(),
                engine: TableEngine::from_declaration(&db.engine.as_ref()?.name, db.tls.as_ref().map(|t| t.name.as_str()), declared_connections(db))?,
                location: db.location.clone()?,
                tables: placed
                    .iter()
                    .filter(|(_, home)| std::ptr::eq(*home, db))
                    .map(|(table, _)| table.name().name.clone())
                    .collect(),
            })
        })
        .collect())
}

/// Every table of the program, the root module's and every imported
/// module's, with the database it lives in, placed through each module's
/// own view of its imports.
fn placed_tables<'m>(
    file: &'m corvid_ast::File,
    modules: &'m corvid_resolve::ModuleResolution,
) -> Vec<(&'m corvid_ast::TableDecl, &'m corvid_ast::DatabaseDecl)> {
    let mut loaded: Vec<&corvid_resolve::ResolvedModule> = modules.all_modules.values().collect();
    loaded.sort_by(|a, b| a.path.cmp(&b.path));
    let views = std::iter::once((file, None)).chain(loaded.into_iter().map(|module| (module.file.as_ref(), Some(module))));
    let mut placed = Vec::new();
    for (module_file, module) in views {
        for decl in &module_file.decls {
            let Decl::Table(table) = decl else { continue };
            let imported = |database: &str| corvid_types::table_ops::imported_database(modules, module, database);
            if let Some(database) = corvid_types::table_ops::table_database_with(module_file, &table.name().name, &imported) {
                placed.push((table, database));
            }
        }
    }
    placed
}

/// The tables of the program at `source_path`, its own and those of every
/// module it imports, with their columns' checked types and `revision`
/// last, and the indexes its queries need. Tables in other modules count
/// because a `public database` is shared by import (B09 P11), so the
/// migrations for a database cover every table that lives in it. The
/// program must compile.
pub fn table_schemas(source: &str, source_path: &Path) -> Result<Vec<TableSchema>, Vec<Diagnostic>> {
    let (ir, file) = compile_program(source, source_path)?;
    let (modules, _, _) = crate::modules::resolve_module_graph(&file, source_path, &|_| None);
    let shapes = query_shapes(&ir);
    let placed = placed_tables(&file, &modules);
    let mut problems = Vec::new();
    for (i, (table, _)) in placed.iter().enumerate() {
        if placed[..i].iter().any(|(earlier, _)| earlier.name().name == table.name().name) {
            problems.push(Diagnostic {
                origin: None,
                span: corvid_ast::Span::new(0, 0),
                message: format!("two modules of this program each declare a table named `{}`; a database stores one table under each name", table.name().name),
                hint: Some("rename one of the tables".into()),
            });
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let mut schemas: Vec<TableSchema> = placed
        .into_iter()
        .filter_map(|(table, database)| {
            let record = ir.types.iter().find(|ty| ty.name == table.name().name)?;
            let engine = TableEngine::from_declaration(
                &database.engine.as_ref()?.name,
                database.tls.as_ref().map(|tls| tls.name.as_str()),
                declared_connections(database),
            )?;
            Some(TableSchema {
                name: record.name.clone(),
                keys: table.keys.iter().map(|key| key.name.clone()).collect(),
                columns: record.fields.iter().map(|field| (field.name.clone(), field.ty.clone())).collect(),
                renamed: table.renamed.iter().map(|r| (r.field.name.clone(), r.from.name.clone())).collect(),
                retired: table.retired.iter().map(|name| name.name.clone()).collect(),
                engine,
                unique: table.unique.iter().map(|rule| rule.iter().map(|c| c.name.clone()).collect()).collect(),
                indexes: Vec::new(),
            })
        })
        .collect();
    // A table is private to its module, so these are every query it gets.
    for schema in &mut schemas {
        let own: Vec<QueryShape> = shapes.iter().filter(|(table, _)| *table == schema.name).map(|(_, shape)| shape.clone()).collect();
        let columns: Vec<String> = schema.columns.iter().map(|(name, _)| name.clone()).collect();
        match derive_indexes(&schema.name, &schema.keys, &schema.unique, &columns, &own) {
            Ok(indexes) => schema.indexes = indexes,
            Err(message) => problems.push(Diagnostic { origin: None, span: corvid_ast::Span::new(0, 0), message, hint: None }),
        }
    }
    if problems.is_empty() { Ok(schemas) } else { Err(problems) }
}

/// What each query in the program asks of its table: the columns its
/// condition requires to be equal and the one it compares or orders by,
/// read from the condition's top-level `and`s. Anything under `or` or
/// `not` is left to the database to filter.
fn query_shapes(ir: &IrFile) -> Vec<(String, QueryShape)> {
    let mut shapes = Vec::new();
    corvid_ir::for_each_expr(ir, &mut |agent, expr| {
        if let IrExprKind::TableQuery { table, filter, order, .. } = &expr.kind {
            let mut shape = QueryShape { agent: agent.to_string(), equal: Vec::new(), range: None, order: order.clone() };
            if let Some(filter) = filter {
                read_conjuncts(filter, &mut shape);
            }
            shapes.push((table.name.clone(), shape));
        }
    });
    shapes
}

fn read_conjuncts(filter: &IrTableFilter, shape: &mut QueryShape) {
    match filter {
        IrTableFilter::And(left, right) => {
            read_conjuncts(left, shape);
            read_conjuncts(right, shape);
        }
        IrTableFilter::Compare { column, op: QueryCompare::Eq, .. } | IrTableFilter::Column(column) => shape.equal.push(column.clone()),
        IrTableFilter::Compare { column, op: QueryCompare::Lt | QueryCompare::LtEq | QueryCompare::Gt | QueryCompare::GtEq, .. } => {
            shape.range.get_or_insert_with(|| column.clone());
        }
        _ => {}
    }
}

/// A `database` block's pool size, which the checker proved is declared
/// for Postgres and from 1 to 1000.
fn declared_connections(database: &corvid_ast::DatabaseDecl) -> Option<u32> {
    database.connections.and_then(|(count, _)| u32::try_from(count).ok())
}
