//! Intermediate representation for the Corvid compiler.
//!
//! Post-typecheck, pre-codegen. Desugared, normalized, and carrying
//! resolved references plus attached types.
//!
//! See `ARCHITECTURE.md` §4.

mod imports;
pub use imports::imported_source_modules;
pub mod lower;
mod expr_visit;
mod secret_types;
pub use secret_types::{holds, holds_secret};
mod type_visit;
pub mod types;

pub use lower::{lower, lower_with_modules};
pub use expr_visit::{for_each_agent_expr, for_each_expr, walk_expr as for_each_subexpr, walk_stmt as for_each_stmt_expr};
pub use type_visit::for_each_compiled_type_mut;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::{Backoff, BackpressurePolicy, Effect};
    use corvid_resolve::resolve;
    use corvid_syntax::{lex, parse_file};
    use corvid_types::typecheck;

    fn lower_src(src: &str) -> IrFile {
        let tokens = lex(src).expect("lex");
        let (file, perr) = parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = resolve(&file);
        assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
        let checked = typecheck(&file, &resolved);
        assert!(checked.errors.is_empty(), "typecheck: {:?}", checked.errors);
        lower(&file, &resolved, &checked)
    }

    #[path = "lower_basic.rs"]
    mod lower_basic;

    #[path = "lower_effects.rs"]
    mod lower_effects;

    #[path = "lower_replay.rs"]
    mod lower_replay;

    #[path = "lower_imports.rs"]
    mod lower_imports;

    #[path = "lower_connector.rs"]
    mod lower_connector;

    #[path = "types.rs"]
    mod types;
}
