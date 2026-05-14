//! Playground examples catalog surface.
//!
//! The playground's examples picker renders the curated demo
//! corpus; the terminal panel runs the picked example. Both the
//! native `corvid tour` command and this browser surface read the
//! same `corvid-tour-catalog` crate — one source of truth, no
//! parallel corpus.
//!
//! This module maps the catalog's internal `TourTopic` shape onto
//! the playground-facing `ExampleMeta` wire format described in
//! `docs/meta/playground-examples-contract.md`. The mapping is
//! deliberate: `ExampleMeta` is the stable contract the website
//! builds against, so the catalog's internal shape can evolve
//! without breaking the playground.
//!
//! Tier 1 (typecheck / analyze) is what this module serves — every
//! example routes through the existing [`crate::check`] entry.
//! Tier 2 (actual agent execution) needs the wasm-clean runtime
//! that the 33J7b/c/d slices are still building; `run_example`
//! lands in a contract addendum once that exists.

use serde::Serialize;

use corvid_tour_catalog::{find_topic, TOPICS};

use crate::{BrowserSpan, CheckResult, Diagnostic, Severity};

/// Schema version for the [`ExampleCatalog`] wire format.
/// Independent of [`crate::SCHEMA_VERSION`] (the `CheckResult`
/// version) — the two wire formats version separately. Bump on a
/// non-additive change; coordinate with the website per the
/// schema-change protocol in `crates/corvid-browser/README.md`.
const EXAMPLE_SCHEMA_VERSION: &str = "v1";

/// The curated examples the playground picker renders.
#[derive(Clone, Debug, Serialize)]
pub struct ExampleCatalog {
    pub version: &'static str,
    pub examples: Vec<ExampleMeta>,
}

/// One example in the picker. Derived from a `corvid-tour-catalog`
/// `TourTopic`; see the module doc for why the shapes differ.
#[derive(Clone, Debug, Serialize)]
pub struct ExampleMeta {
    /// Stable kebab-case id, e.g. `"approve-gates"`. The key the
    /// playground passes back to [`check_example`].
    pub name: &'static str,
    /// Human title, e.g. `"Approve Before Dangerous"`.
    pub title: &'static str,
    /// Grouping label, e.g. `"Safety at compile time"`. The picker
    /// groups entries by this.
    pub category: &'static str,
    /// One-paragraph why-this-matters, shown under the title.
    pub pitch: &'static str,
    /// The baked `.cor` program. The playground shows it and — for
    /// the approve-refusal demo — lets the user edit it in-place,
    /// routing edits through [`crate::check`].
    pub source: &'static str,
    /// Docs link into the spec, e.g.
    /// `"docs/internals/effect-spec/03-typing-rules.md"`.
    pub spec_path: &'static str,
    /// What the demo deliberately does NOT prove — keeps the
    /// playground honest about scope.
    pub non_scope: &'static str,
    /// `1` = typecheck-demo (works today). `2` = needs browser
    /// execution. Every example is tier 1 until the runtime + vm
    /// split land wasm execution; tier 2 gets encoded when there
    /// is actually a distinction to draw.
    pub tier: u8,
}

/// Return the full curated examples catalog for the picker.
pub fn list_examples() -> ExampleCatalog {
    ExampleCatalog {
        version: EXAMPLE_SCHEMA_VERSION,
        examples: TOPICS
            .iter()
            .map(|topic| ExampleMeta {
                name: topic.name,
                title: topic.title,
                category: topic.category,
                pitch: topic.pitch,
                source: topic.source,
                spec_path: topic.spec,
                non_scope: topic.non_scope,
                tier: 1,
            })
            .collect(),
    }
}

/// Typecheck one example by name. Thin wrapper over [`crate::check`]
/// using the topic's baked source. Returns the standard
/// [`CheckResult`] wire format unchanged.
///
/// An unknown name returns a `CheckResult` with `ok: false` and a
/// single error diagnostic rather than panicking — the playground
/// surfaces it like any other error. An edited example does NOT
/// come back through here; the playground calls [`crate::check`]
/// directly with the edited source for the approve-refusal demo.
pub fn check_example(name: &str) -> CheckResult {
    match find_topic(name) {
        Some(topic) => crate::check(topic.source),
        None => CheckResult {
            version: crate::SCHEMA_VERSION,
            ok: false,
            diagnostics: vec![Diagnostic {
                guarantee_id: None,
                severity: Severity::Error,
                message: format!(
                    "unknown example `{name}`. Call `list_examples()` for \
                     the valid names."
                ),
                span: BrowserSpan {
                    start_line: 1,
                    start_col: 1,
                    end_line: 1,
                    end_col: 1,
                },
                help: None,
                path: None,
            }],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_non_empty_and_versioned() {
        let catalog = list_examples();
        assert_eq!(catalog.version, "v1");
        assert!(
            !catalog.examples.is_empty(),
            "the curated catalog should never be empty"
        );
    }

    #[test]
    fn every_example_name_round_trips_through_check_example() {
        // The marquee property: every name the picker can show is a
        // name the terminal panel can check. If the catalog and the
        // checker ever disagree, the playground would offer an
        // example it cannot run.
        for example in list_examples().examples {
            let result = check_example(example.name);
            // Tour sources are test-backed to compile, so a known
            // example must never come back as the unknown-name
            // error. We assert the diagnostic shape rather than
            // `ok` because some tour topics legitimately carry
            // warnings.
            assert!(
                !result.diagnostics.iter().any(|d| d
                    .message
                    .starts_with(&format!("unknown example `{}`", example.name))),
                "example `{}` did not resolve through check_example",
                example.name
            );
        }
    }

    #[test]
    fn unknown_example_fails_closed_with_a_diagnostic() {
        let result = check_example("no-such-example");
        assert!(!result.ok);
        assert_eq!(result.diagnostics.len(), 1);
        assert!(result.diagnostics[0]
            .message
            .starts_with("unknown example `no-such-example`"));
    }

    #[test]
    fn approve_gates_is_the_marquee_demo_and_compiles_clean() {
        // The approve-refusal demo rides on `approve-gates`. It must
        // be present and must typecheck clean in its shipped form —
        // the "delete approve, watch it refuse" interaction depends
        // on the baseline being green.
        let result = check_example("approve-gates");
        assert!(
            result.ok,
            "approve-gates must compile clean as shipped: {:?}",
            result.diagnostics
        );
    }
}

// -----------------------------------------------------------------
// wasm-bindgen entries — only compiled for wasm32-unknown-unknown.
// Native callers use the rlib functions above directly.
// -----------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = listExamples)]
pub fn list_examples_wasm() -> JsValue {
    serde_wasm_bindgen::to_value(&list_examples()).unwrap_or(JsValue::NULL)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = checkExample)]
pub fn check_example_wasm(name: &str) -> JsValue {
    serde_wasm_bindgen::to_value(&check_example(name)).unwrap_or(JsValue::NULL)
}
