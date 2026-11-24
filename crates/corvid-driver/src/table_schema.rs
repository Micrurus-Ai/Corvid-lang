//! The declared shape of a program's tables (B09), for planning
//! migrations: each table's name, key, checked column types and its
//! `renamed from` and `retired` clauses, and the engine its database runs on.

use crate::{compile_to_ir, Diagnostic};
use corvid_ast::Decl;
use corvid_runtime::tables::{TableEngine, TableSchema};

/// A `database` block as `corvid migrate` connects to it.
#[derive(Debug, Clone)]
pub struct DeclaredDatabase {
    pub name: String,
    pub engine: TableEngine,
    pub location: corvid_ast::DatabaseLocation,
    pub tables: Vec<String>,
}

/// The databases `source` declares. The source must compile, so each
/// names a running engine and a location.
pub fn declared_databases(source: &str) -> Result<Vec<DeclaredDatabase>, Vec<Diagnostic>> {
    compile_to_ir(source)?;
    let tokens = corvid_syntax::lex(source).map_err(|_| Vec::new())?;
    let (file, _) = corvid_syntax::parse_file(&tokens);
    Ok(file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Database(db) => Some(db),
            _ => None,
        })
        .filter_map(|db| {
            Some(DeclaredDatabase {
                name: db.name.name.clone(),
                engine: TableEngine::from_declaration(&db.engine.as_ref()?.name, db.tls.as_ref().map(|t| t.name.as_str()))?,
                location: db.location.clone()?,
                tables: db.tables.iter().map(|t| t.name.clone()).collect(),
            })
        })
        .collect())
}

/// The tables `source` declares, with their columns' checked types and
/// `revision` last. The source must compile.
pub fn table_schemas(source: &str) -> Result<Vec<TableSchema>, Vec<Diagnostic>> {
    let ir = compile_to_ir(source)?;
    let tokens = corvid_syntax::lex(source).map_err(|_| Vec::new())?;
    let (file, _) = corvid_syntax::parse_file(&tokens);
    Ok(file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Table(table) => Some(table),
            _ => None,
        })
        .filter_map(|table| {
            let record = ir.types.iter().find(|ty| ty.name == table.name().name)?;
            // The checker proved the table lives in exactly one database
            // whose engine and settings are stated.
            let database = file.decls.iter().find_map(|decl| match decl {
                Decl::Database(db) if db.tables.iter().any(|t| t.name == record.name) => Some(db),
                _ => None,
            })?;
            let engine = TableEngine::from_declaration(
                &database.engine.as_ref()?.name,
                database.tls.as_ref().map(|tls| tls.name.as_str()),
            )?;
            Some(TableSchema {
                name: record.name.clone(),
                keys: table.keys.iter().map(|key| key.name.clone()).collect(),
                columns: record.fields.iter().map(|field| (field.name.clone(), field.ty.clone())).collect(),
                renamed: table.renamed.iter().map(|r| (r.field.name.clone(), r.from.name.clone())).collect(),
                retired: table.retired.iter().map(|name| name.name.clone()).collect(),
                engine,
            })
        })
        .collect())
}
