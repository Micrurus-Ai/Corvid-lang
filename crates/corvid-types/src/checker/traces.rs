//! The `traces:` declaration (lineage L3).
//!
//! A served program keeps what people sent it on disk when it records
//! its runs: a privacy and a cost choice. So a file with a `server`
//! block must declare `traces:`, and leaving it out is a compile error
//! that offers `traces: none`. A program declares it once.
//!
//! A block that records requests says where (`location:`) and how long
//! (`keep:`, read by `crate::traces::keep_seconds`, the rule `corvid
//! serve` deletes old traces by). Neither has a default.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use corvid_ast::{Decl, File};

impl<'a> Checker<'a> {
    pub(super) fn check_traces(&mut self, file: &File) {
        let declared: Vec<&corvid_ast::TracesDecl> = file
            .decls
            .iter()
            .filter_map(|decl| match decl {
                Decl::Traces(traces) => Some(traces),
                _ => None,
            })
            .collect();
        let server = file.decls.iter().find_map(|decl| match decl {
            Decl::Server(server) => Some(server),
            _ => None,
        });
        let mut invalid = |problem: String, span| {
            self.errors.push(TypeError::new(TypeErrorKind::TracesPolicyInvalid { problem }, span));
        };
        if let (Some(server), true) = (server, declared.is_empty()) {
            invalid(
                format!(
                    "server `{}` serves people, but the program does not say whether their requests are recorded; declare `traces: none` to record nothing",
                    server.name.name
                ),
                server.span,
            );
        }
        for extra in declared.iter().skip(1) {
            invalid("`traces:` is declared twice; a program declares it once".into(), extra.span);
        }
        for traces in &declared {
            let Some(recording) = &traces.recording else { continue };
            // Where the traces go and how long they stay are both
            // consequential, so both are written.
            if recording.location.is_none() {
                invalid(
                    "`traces:` records requests, so it says where: `location: env(\"TRACE_DIR\")` or a path".into(),
                    recording.span,
                );
            }
            match &recording.keep {
                None => invalid(
                    "`traces:` records requests, so it says how long a trace is kept, as in `keep: 720 hours`; there is no default".into(),
                    recording.span,
                ),
                Some((count, unit, span)) => {
                    if let Err(problem) = crate::traces::keep_seconds(count, unit) {
                        invalid(problem, *span);
                    }
                }
            }
        }
    }
}
