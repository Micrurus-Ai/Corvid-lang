//! The check pipeline: lex, parse, resolve, typecheck and lower, with
//! every diagnostic collected.
//!
//! `analyze_with_config_at_path` is the production entry point; it can
//! resolve sibling `.cor` imports because it has a real file path.
//! `compile_with_config` is the embedded variant for callers that have
//! only a string, and `compile` the no-config wrapper. None of them
//! emits code: a program runs on the interpreter (`corvid run`,
//! `corvid serve`) or builds natively (`corvid build`). The Python
//! transpile target was retired on 2026-11-25.

use std::path::Path;

use corvid_ir::lower;
use corvid_syntax::{lex, parse_file};
use corvid_resolve::resolve;
use corvid_types::{typecheck_with_config, CorvidConfig};

use crate::diagnostic::Diagnostic;

use super::{lower_driver_file, typecheck_with_sources};

/// Outcome of a check: the diagnostics found, and warnings apart.
pub struct CompileResult {
    pub diagnostics: Vec<Diagnostic>,
    /// Self-trial round 4 (Gap A): non-blocking warnings surfaced
    /// during typecheck — e.g. `ScheduleNotExecutable` flagging
    /// that the scheduler runner doesn't fire crons in v1.0. Kept
    /// SEPARATE from `diagnostics` so the existing error-flow
    /// (`compile_with_config_at_path` callers checking `ok()`)
    /// stays unchanged; CLIs that want to surface warnings read
    /// this field explicitly (see `crates/corvid-cli/src/commands/misc.rs::cmd_check`).
    pub warnings: Vec<Diagnostic>,
}

impl CompileResult {
    pub fn ok(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// Check `source` through the whole front end, lowering included.
pub fn compile(source: &str) -> CompileResult {
    compile_with_config(source, None)
}

/// Compile with an explicit `corvid.toml` configuration (for user-
/// defined effect dimensions). Callers with a source-file path
/// usually prefer `compile_with_config_at_path` which walks for
/// `corvid.toml` automatically.
pub fn compile_with_config(source: &str, config: Option<&CorvidConfig>) -> CompileResult {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // 1. Lex
    let tokens = match lex(source) {
        Ok(t) => t,
        Err(errs) => {
            diagnostics.extend(errs.into_iter().map(Diagnostic::from));
            return CompileResult {
                diagnostics,
                warnings: Vec::new(),
            };
        }
    };

    // 2. Parse (collects errors, may still produce a partial AST)
    let (file, parse_errs) = parse_file(&tokens);
    diagnostics.extend(parse_errs.into_iter().map(Diagnostic::from));

    // 3. Resolve (collects errors)
    let resolved = resolve(&file);
    diagnostics.extend(resolved.errors.iter().cloned().map(Diagnostic::from));

    // 4. Typecheck (collects errors — this is where the killer feature lives)
    let checked = typecheck_with_config(&file, &resolved, config);
    diagnostics.extend(checked.errors.iter().cloned().map(Diagnostic::from));
    // Self-trial round 4 Gap A: thread warnings through so the CLI can
    // surface them at `corvid check` time. Previously dropped silently.
    let warnings: Vec<Diagnostic> = checked
        .warnings
        .iter()
        .cloned()
        .map(Diagnostic::from)
        .collect();

    if !diagnostics.is_empty() {
        return CompileResult {
            diagnostics,
            warnings,
        };
    }

    // 5. Lower, so a program that checks also lowers.
    let _ir = lower(&file, &resolved, &checked);

    CompileResult {
        diagnostics: Vec::new(),
        warnings,
    }
}

/// Check a source string that came from `source_path` (`corvid check`).
/// Unlike [`compile_with_config`], this path resolves sibling `.cor`
/// imports, because the driver has a filesystem anchor.
pub fn analyze_with_config_at_path(
    source: &str,
    source_path: &Path,
    config: Option<&CorvidConfig>,
) -> CompileResult {
    analyze_with_open_sources(source, source_path, config, &|_| None)
}

/// Analyze a document with editor buffers overriding local imports before disk lookup.
pub fn analyze_with_open_sources(
    source: &str,
    source_path: &Path,
    config: Option<&CorvidConfig>,
    open_source: &dyn Fn(&Path) -> Option<Vec<u8>>,
) -> CompileResult {
    compile_front_at_path(source, source_path, config, open_source).0
}

/// The check pipeline for a file. Returns the lowered IR alongside the
/// result; `None` IR means
/// diagnostics stopped the pipeline before lowering.
fn compile_front_at_path(
    source: &str,
    source_path: &Path,
    config: Option<&CorvidConfig>,
    open_source: &dyn Fn(&Path) -> Option<Vec<u8>>,
) -> (CompileResult, Option<corvid_ir::IrFile>) {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let tokens = match lex(source) {
        Ok(t) => t,
        Err(errs) => {
            diagnostics.extend(errs.into_iter().map(Diagnostic::from));
            return (
                CompileResult {
                        diagnostics,
                    warnings: Vec::new(),
                },
                None,
            );
        }
    };
    let (file, parse_errs) = parse_file(&tokens);
    diagnostics.extend(parse_errs.into_iter().map(Diagnostic::from));
    let resolved = resolve(&file);
    diagnostics.extend(resolved.errors.iter().cloned().map(Diagnostic::from));

    let typechecked = typecheck_with_sources(&file, &resolved, source_path, config, open_source);
    diagnostics.extend(typechecked.diagnostics);
    diagnostics.extend(
        typechecked
            .result
            .checked
            .errors
            .iter()
            .cloned()
            .map(Diagnostic::from),
    );
    // Self-trial round 4 Gap A: surface warnings (separate from
    // errors) for `corvid check`. Pre-fix these were silently
    // dropped — including the new ScheduleNotExecutable warning that
    // tells reviewers their cron won't fire on v1.0.
    let warnings: Vec<Diagnostic> = typechecked
        .result
        .checked
        .warnings
        .iter()
        .cloned()
        .map(Diagnostic::from)
        .collect();

    if !diagnostics.is_empty() {
        return (
            CompileResult {
                diagnostics,
                warnings,
            },
            None,
        );
    }

    let ir = lower_driver_file(&file, &resolved, &typechecked.result);
    (
        CompileResult {
            diagnostics: Vec::new(),
            warnings,
        },
        Some(ir),
    )
}
