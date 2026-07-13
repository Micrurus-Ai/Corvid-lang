//! Replay-mode LLM quarantine — slice `35V2-P38-C-4`.
//!
//! When a runtime is constructed in `RuntimeMode::Replay(source)` and
//! `source.uses_live_llm()` is false (the default Substitute mode that
//! `corvid jobs replay` and `corvid replay` use), every registered
//! `LlmAdapter` is wrapped in [`QuarantinedLlmAdapter`] before being
//! handed to the runtime. The wrapper's `call` method returns
//! `RuntimeError::QuarantineViolation` instead of delegating to the
//! inner adapter — so even if a future code path reaches the registry
//! directly (bypassing `Runtime::call_llm_ref`'s `replay_llm_call`
//! substitution), no real provider call leaves the process.
//!
//! The interpreter dispatch path (slice C-3 routes through it) already
//! routes LLM calls through `ReplaySource::replay_llm_call`, which
//! substitutes recorded results and surfaces `ReplayDivergence` when
//! the next event doesn't match. The quarantine layer here is
//! defense-in-depth for any caller that grabs an adapter directly from
//! `LlmRegistry::call(&req)` — the registry's normal entry point.
//! `QuarantineViolation` and `ReplayDivergence` are distinct error
//! variants so tests can tell the substitution-mismatch case
//! (`ReplayDivergence`, the existing path) from the
//! bypass-attempt case (`QuarantineViolation`, this layer's promise).
//!
//! `handles(model)` delegates to the wrapped adapter so the registry's
//! dispatch logic (first-match-wins on model prefix) is unaffected —
//! the wrapper participates in dispatch and refuses at the call site.

use crate::errors::RuntimeError;
use crate::llm::{LlmAdapter, LlmRequestRef, LlmResponse};
use futures::future::BoxFuture;
use std::sync::Arc;

pub struct QuarantinedLlmAdapter {
    inner: Arc<dyn LlmAdapter>,
}

impl QuarantinedLlmAdapter {
    pub fn wrap(inner: Arc<dyn LlmAdapter>) -> Self {
        Self { inner }
    }

    pub fn inner_name(&self) -> &str {
        self.inner.name()
    }
}

impl LlmAdapter for QuarantinedLlmAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn handles(&self, model: &str) -> bool {
        self.inner.handles(model)
    }

    fn call<'a>(
        &'a self,
        req: &'a LlmRequestRef<'a>,
    ) -> BoxFuture<'a, Result<LlmResponse, RuntimeError>> {
        let inner_name = self.inner.name().to_string();
        let model = req.model.to_string();
        let prompt = req.prompt.to_string();
        Box::pin(async move {
            Err(RuntimeError::QuarantineViolation {
                surface: "llm".to_string(),
                detail: format!(
                    "adapter `{inner_name}` blocked an unrecorded live call \
                     for model `{model}` (prompt `{prompt}`) during replay-mode \
                     quarantine. A replayed run must consume recorded `LlmResult` \
                     events; reaching the adapter means the recorded sequence has \
                     diverged from the agent's execution."
                ),
            })
        })
    }
}

// `LlmRegistry::quarantine_all` lives in `llm/mod.rs` because the
// `adapters` field is private to that module; an impl block in a
// child module cannot reach the parent's private fields. The
// `QuarantinedLlmAdapter` type itself stays here so the quarantine
// boundary is one file the reader can audit end-to-end.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::MockAdapter;
    use crate::llm::{LlmRegistry, LlmRequestRef};

    fn dummy_req<'a>() -> LlmRequestRef<'a> {
        LlmRequestRef {
            prompt: "test_prompt",
            model: "mock-1",
            rendered: "Hello",
            args: &[],
            output_schema: None,
            sampling: Default::default(),
            messages: &[],
        }
    }

    /// Slice 35V2-P38-C-4 positive: wrapping a registered adapter in
    /// `QuarantinedLlmAdapter` makes `.call(&req)` return
    /// `RuntimeError::QuarantineViolation { surface: "llm", .. }`
    /// regardless of the inner adapter's behavior. The wrap is the
    /// fail-closed boundary that protects against future callers that
    /// reach the registry directly without going through
    /// `Runtime::call_llm_ref`'s `replay_llm_call` substitution.
    #[tokio::test]
    async fn quarantined_adapter_rejects_call_with_typed_violation() {
        let inner: Arc<dyn LlmAdapter> = Arc::new(
            MockAdapter::new("mock-1").reply("test_prompt", serde_json::json!("ok")),
        );
        let wrap = QuarantinedLlmAdapter::wrap(inner);
        let req = dummy_req();
        let err = wrap.call(&req).await.expect_err("quarantined wrap must error");
        match err {
            RuntimeError::QuarantineViolation { surface, detail } => {
                assert_eq!(surface, "llm");
                assert!(
                    detail.contains("mock-1"),
                    "detail should name the model: {detail}"
                );
                assert!(
                    detail.contains("test_prompt"),
                    "detail should name the prompt: {detail}"
                );
            }
            other => panic!("expected QuarantineViolation, got {other:?}"),
        }
    }

    /// Slice 35V2-P38-C-4 positive: `LlmRegistry::quarantine_all`
    /// replaces every registered adapter with its quarantined wrap.
    /// After the call, the registry's normal dispatch path
    /// (`call_with_adapter_name`) refuses with `QuarantineViolation`
    /// instead of delegating to the wrapped adapter.
    #[tokio::test]
    async fn registry_quarantine_all_wraps_every_registered_adapter() {
        let mut registry = LlmRegistry::default();
        registry.register(Arc::new(
            MockAdapter::new("mock-1").reply("test_prompt", serde_json::json!("ok")),
        ));
        registry.quarantine_all();
        let req = dummy_req();
        let err = registry.call(&req).await.expect_err("registry must refuse");
        assert!(
            matches!(err, RuntimeError::QuarantineViolation { ref surface, .. } if surface == "llm"),
            "registry call after quarantine_all must surface llm QuarantineViolation: {err:?}"
        );
    }

    /// Slice 35V2-P38-C-4 adversarial: registering an adapter AFTER
    /// `quarantine_all` does NOT magically quarantine the new one —
    /// quarantine is installed once at `RuntimeBuilder::build` time
    /// and is not re-applied to later registrations. This test locks
    /// the contract so a caller who registers post-build (which the
    /// production paths don't do, but a future caller might) doesn't
    /// silently get a non-quarantined adapter expecting otherwise.
    /// If we ever need post-build quarantine, this test fails and
    /// forces a deliberate redesign.
    #[tokio::test]
    async fn quarantine_all_does_not_cover_adapters_registered_later() {
        let mut registry = LlmRegistry::default();
        registry.register(Arc::new(
            MockAdapter::new("first").reply("p", serde_json::json!("ok")),
        ));
        registry.quarantine_all();
        registry.register(Arc::new(
            MockAdapter::new("late").reply("p", serde_json::json!("ok")),
        ));
        // The late-registered adapter wins dispatch (its model matches
        // first, since the first-registered adapter is now a
        // quarantined `first` model wrap). Calling the registry with
        // model `late` exercises the late adapter directly.
        let req = LlmRequestRef {
            prompt: "p",
            model: "late",
            rendered: "x",
            args: &[],
            output_schema: None,
            sampling: Default::default(),
            messages: &[],
        };
        let result = registry.call(&req).await;
        // Either succeeds (mock returns the canned response) or fails
        // with NoAdapter. Either way, must NOT be QuarantineViolation.
        if let Err(err) = result {
            assert!(
                !matches!(err, RuntimeError::QuarantineViolation { .. }),
                "late-registered adapter must not be quarantined: {err:?}"
            );
        }
    }
}
