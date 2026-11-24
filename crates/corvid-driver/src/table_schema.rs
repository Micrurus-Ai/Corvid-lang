//! The declared shape of a program's tables (B09), for planning
//! migrations: each table's name, key, checked column types and its
//! `renamed from` and `retired` clauses.

use crate::{compile_to_ir, Diagnostic};
use corvid_ast::Decl;
use corvid_runtime::tables::TableSchema;

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
            Some(TableSchema {
                name: record.name.clone(),
                key: table.key.as_ref()?.name.clone(),
                columns: record.fields.iter().map(|field| (field.name.clone(), field.ty.clone())).collect(),
                renamed: table.renamed.iter().map(|r| (r.field.name.clone(), r.from.name.clone())).collect(),
                retired: table.retired.iter().map(|name| name.name.clone()).collect(),
            })
        })
        .collect())
}
