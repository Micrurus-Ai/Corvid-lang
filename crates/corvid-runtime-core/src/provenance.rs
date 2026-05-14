//! Shared provenance-chain types.
//!
//! These live in `corvid-runtime-core` so the interpreter, native FFI
//! boundary, browser playground, and future host-mint/query slices
//! all speak the same provenance shape across the wasm/native split.
//! `corvid-runtime` re-exports them so existing native consumers see
//! no API change.

use serde::{Deserialize, Serialize};

/// A value paired with an explicit provenance chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundedValue<T> {
    pub value: T,
    pub provenance: ProvenanceChain,
}

/// The provenance chain: every retrieval source, prompt transformation,
/// and agent handoff that a value passed through.
///
/// `Eq` is derived (not just `PartialEq`) because `ProvenanceKind::Derived`
/// nests `Vec<ProvenanceChain>` and needs the whole tree to be `Eq`. The
/// chain types carry no floats — confidence lives on the runtime's
/// `GroundedValue`, not here — so total equality is sound.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceChain {
    pub entries: Vec<ProvenanceEntry>,
}

/// One step in the provenance chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceEntry {
    pub kind: ProvenanceKind,
    pub name: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvenanceKind {
    /// Data retrieved from an external source (tool with `data: grounded`).
    Retrieval,
    /// Data transformed by an LLM prompt.
    PromptTransform,
    /// Data passed through an agent call.
    AgentHandoff,
    /// Provenance deliberately severed by `.unwrap(reason: ...)`.
    Severed { reason: String },
    /// A value produced by an operator (arithmetic, concat, comparison)
    /// with at least one `Grounded` operand — the Provenance Propagation
    /// contagion law (see `docs/meta/grounded-propagation-design.md` D1).
    ///
    /// `op` names the operator (`"add"`, `"concat"`, `"lt"`, …); `inputs`
    /// holds the operand provenance chains in operand order. An
    /// ungrounded operand contributes an empty chain. This is
    /// how-provenance: the entry records the operation *tree*, not just
    /// the leaf sources, so a renderer can reconstruct the computation
    /// that produced the value.
    Derived {
        op: String,
        inputs: Vec<ProvenanceChain>,
    },
}

impl ProvenanceKind {
    pub fn label(&self) -> &str {
        match self {
            ProvenanceKind::Retrieval => "retrieval",
            ProvenanceKind::PromptTransform => "prompt",
            ProvenanceKind::AgentHandoff => "agent",
            ProvenanceKind::Severed { .. } => "severed",
            ProvenanceKind::Derived { .. } => "derived",
        }
    }
}

impl ProvenanceChain {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn with_retrieval(tool_name: &str, timestamp_ms: u64) -> Self {
        Self {
            entries: vec![ProvenanceEntry {
                kind: ProvenanceKind::Retrieval,
                name: tool_name.to_string(),
                timestamp_ms,
            }],
        }
    }

    pub fn add_prompt_transform(&mut self, prompt_name: &str, timestamp_ms: u64) {
        self.entries.push(ProvenanceEntry {
            kind: ProvenanceKind::PromptTransform,
            name: prompt_name.to_string(),
            timestamp_ms,
        });
    }

    pub fn add_agent_handoff(&mut self, agent_name: &str, timestamp_ms: u64) {
        self.entries.push(ProvenanceEntry {
            kind: ProvenanceKind::AgentHandoff,
            name: agent_name.to_string(),
            timestamp_ms,
        });
    }

    /// Build the chain for an operator result under the contagion law:
    /// a single `Derived` entry recording the operator and its operand
    /// chains. `op` names the operator (`"add"`, `"concat"`, `"lt"`, …);
    /// `inputs` are the operand provenance chains in operand order — an
    /// ungrounded operand contributes an empty chain. The interpreter,
    /// native runtime, and replay all mint operator results through
    /// this constructor so the four tiers produce byte-identical
    /// provenance.
    pub fn derived(op: &str, inputs: Vec<ProvenanceChain>, timestamp_ms: u64) -> Self {
        Self {
            entries: vec![ProvenanceEntry {
                kind: ProvenanceKind::Derived {
                    op: op.to_string(),
                    inputs,
                },
                name: op.to_string(),
                timestamp_ms,
            }],
        }
    }

    pub fn merge(&mut self, other: &ProvenanceChain) {
        for entry in &other.entries {
            if !self
                .entries
                .iter()
                .any(|candidate| candidate.name == entry.name && candidate.kind == entry.kind)
            {
                self.entries.push(entry.clone());
            }
        }
    }

    pub fn has_retrieval(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.kind == ProvenanceKind::Retrieval)
    }

    pub fn has_source(&self, name: &str) -> bool {
        self.entries.iter().any(|entry| entry.name == name)
    }
}

impl<T> GroundedValue<T> {
    pub fn new(value: T, provenance: ProvenanceChain) -> Self {
        Self { value, provenance }
    }

    pub fn has_retrieval(&self) -> bool {
        self.provenance.has_retrieval()
    }

    pub fn map<U, F>(self, map: F) -> GroundedValue<U>
    where
        F: FnOnce(T) -> U,
    {
        GroundedValue {
            value: map(self.value),
            provenance: self.provenance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(chain: &ProvenanceChain) -> ProvenanceChain {
        let json = serde_json::to_string(chain).expect("serialize chain");
        serde_json::from_str(&json).expect("deserialize chain")
    }

    #[test]
    fn derived_entry_round_trips() {
        let chain = ProvenanceChain::derived(
            "add",
            vec![
                ProvenanceChain::with_retrieval("search_tool", 10),
                ProvenanceChain::with_retrieval("audit_tool", 20),
            ],
            30,
        );
        assert_eq!(chain, round_trip(&chain));
    }

    #[test]
    fn derived_chain_is_a_tree_that_survives_round_trip() {
        // A `Derived` whose input is itself a `Derived` chain — the
        // recursive how-provenance structure. The whole tree must
        // serialize and come back identical.
        let inner = ProvenanceChain::derived(
            "concat",
            vec![
                ProvenanceChain::with_retrieval("a", 1),
                ProvenanceChain::with_retrieval("b", 2),
            ],
            3,
        );
        let outer = ProvenanceChain::derived(
            "add",
            vec![inner.clone(), ProvenanceChain::with_retrieval("c", 4)],
            5,
        );
        let restored = round_trip(&outer);
        assert_eq!(outer, restored);
        // Confirm the nesting actually survived, not just a flat blob.
        match &restored.entries[0].kind {
            ProvenanceKind::Derived { op, inputs } => {
                assert_eq!(op, "add");
                assert_eq!(inputs.len(), 2);
                assert_eq!(inputs[0], inner);
            }
            other => panic!("expected Derived, got {other:?}"),
        }
    }

    #[test]
    fn ungrounded_operand_is_an_empty_input_chain() {
        // `Grounded<T> + T` — the plain operand contributes an empty
        // chain. It must round-trip as empty, not vanish from `inputs`.
        let chain = ProvenanceChain::derived(
            "add",
            vec![
                ProvenanceChain::with_retrieval("grounded_src", 1),
                ProvenanceChain::new(),
            ],
            2,
        );
        let restored = round_trip(&chain);
        assert_eq!(chain, restored);
        match &restored.entries[0].kind {
            ProvenanceKind::Derived { inputs, .. } => {
                assert_eq!(inputs.len(), 2);
                assert!(inputs[1].entries.is_empty(), "ungrounded operand stays empty");
            }
            other => panic!("expected Derived, got {other:?}"),
        }
    }

    #[test]
    fn derived_constructor_shapes_a_single_entry() {
        let chain = ProvenanceChain::derived("lt", vec![ProvenanceChain::new()], 7);
        assert_eq!(chain.entries.len(), 1);
        assert_eq!(chain.entries[0].name, "lt");
        assert_eq!(chain.entries[0].timestamp_ms, 7);
        assert_eq!(chain.entries[0].kind.label(), "derived");
    }

    #[test]
    fn provenance_chain_is_eq() {
        // D3: the `Eq` derive must hold across the recursive tree.
        let a = ProvenanceChain::derived("add", vec![ProvenanceChain::new()], 1);
        let b = ProvenanceChain::derived("add", vec![ProvenanceChain::new()], 1);
        let c = ProvenanceChain::derived("sub", vec![ProvenanceChain::new()], 1);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}

