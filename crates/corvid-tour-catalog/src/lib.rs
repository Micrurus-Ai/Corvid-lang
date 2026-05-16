//! Curated Corvid demo catalog.
//!
//! `TOPICS` is the canonical, test-backed, spec-linked set of
//! `.cor` demos. The CLAUDE.md invention-shipping contract requires
//! every Corvid-specific capability to ship a `corvid tour --topic
//! <name>` demo, so this list IS the curated example corpus — there
//! is no parallel one.
//!
//! Two consumers share this crate:
//! - `corvid-cli` renders the catalog for `corvid tour` (native,
//!   text + REPL).
//! - `corvid-browser` renders it for the playground examples picker
//!   (wasm). That is why this crate is wasm-clean: pure `&'static
//!   str` data plus a lookup, no native deps.
//!
//! See `docs/meta/playground-examples-contract.md` for how the
//! playground maps `TourTopic` onto its `ExampleMeta` wire format.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TourTopic {
    pub name: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub pitch: &'static str,
    pub spec: &'static str,
    pub roadmap: &'static str,
    pub test: &'static str,
    pub non_scope: &'static str,
    pub source: &'static str,
}

pub const TOPICS: &[TourTopic] = &[
    TourTopic {
        name: "approve-gates",
        title: "Approve Before Dangerous",
        category: "Safety at compile time",
        pitch: "Dangerous actions are not library conventions. The compiler requires an explicit approval boundary before irreversible tools run. The approve label is the tool's name in either casing — `approve IssueRefund(id)` and `approve issue_refund(id)` both authorise tool `issue_refund`, since the checker normalises label and tool name to snake_case before comparing. Per-tool greppability is preserved: pick a casing convention and grep for it.",
        spec: "docs/internals/effect-spec/03-typing-rules.md",
        roadmap: "Phase 20 safety wave",
        test: "crates/corvid-types/src/lib.rs approval checker tests",
        non_scope: "Does not decide whether a human should approve; it proves the approval boundary exists.",
        source: r#"type Receipt:
    id: String

tool issue_refund(id: String) -> Receipt dangerous

agent refund(id: String) -> Receipt:
    approve IssueRefund(id)
    return issue_refund(id)
"#,
    },
    TourTopic {
        name: "dimensional-effects",
        title: "Dimensional Effects",
        category: "Safety at compile time",
        pitch: "Effects are not flat tags. Cost, trust, reversibility, data, latency, confidence, and user dimensions compose with their own algebra.",
        spec: "docs/internals/effect-spec/02-composition-algebra.md",
        roadmap: "Phase 20a and Phase 20g",
        test: "crates/corvid-types/src/effects.rs composition tests",
        non_scope: "Does not make external providers honest; it proves the declared Corvid contract.",
        source: r#"effect llm_call:
    cost: $0.05
    trust: autonomous

prompt summarize(text: String) -> String uses llm_call:
    "Summarize {text}"

@budget($0.10)
@trust(autonomous)
agent summarize_twice(text: String) -> String:
    first = summarize(text)
    return summarize(first)
"#,
    },
    TourTopic {
        name: "grounded-values",
        title: "Grounded<T> Provenance",
        category: "Safety at compile time",
        pitch: "A grounded return must flow from a retrieval source. At runtime the value carries the provenance chain that made it grounded.",
        spec: "docs/internals/effect-spec/05-grounding.md",
        roadmap: "Phase 20b",
        test: "crates/corvid-types/src/effects/grounded.rs tests",
        non_scope: "Does not prove the retrieved document is true; it proves the answer is sourced.",
        source: r#"effect retrieval:
    data: grounded

tool fetch_doc(id: String) -> Grounded<String> uses retrieval

prompt summarize(doc: Grounded<String>) -> Grounded<String>:
    "Summarize {doc}"

agent research(id: String) -> Grounded<String>:
    doc = fetch_doc(id)
    return summarize(doc)
"#,
    },
    TourTopic {
        name: "strict-citations",
        title: "Strict Citation Contracts",
        category: "Safety at compile time",
        pitch: "A prompt can name the grounded context it must cite. The compiler proves the cited parameter is grounded; runtime checks the response.",
        spec: "docs/internals/effect-spec/05-grounding.md",
        roadmap: "Phase 20b cites ctx strictly",
        test: "crates/corvid-vm/src/tests/dispatch.rs citation tests",
        non_scope: "Strict citation checks text evidence; they do not judge source truth.",
        source: r#"effect retrieval:
    data: grounded

tool fetch_doc(id: String) -> Grounded<String> uses retrieval

prompt answer(ctx: Grounded<String>) -> Grounded<String>:
    cites ctx strictly
    "Answer from {ctx}"

agent cited(id: String) -> Grounded<String>:
    return answer(fetch_doc(id))
"#,
    },
    TourTopic {
        name: "provenance-propagation",
        title: "Provenance Propagation + @grounded_pure",
        category: "Safety at compile time",
        pitch: "Grounded values stay grounded as they flow through ordinary code: the contagion law lifts Grounded<T> through operators and call sites; the runtime delivers Value::Grounded wherever the type promises it. Mark an agent @grounded_pure and the compiler refuses every laundering shape — silent Grounded<T> -> T coercion, explicit .unwrap_discarding_sources(), and calls into agents that aren't themselves @grounded_pure. The moat composes through the call graph the same way @deterministic does.",
        spec: "docs/meta/grounded-propagation-design.md",
        roadmap: "Provenance Propagation phase",
        test: "crates/corvid-types/src/tests.rs grounded_pure_* tests + tests/corpus/combined_all.cor + tests/corpus/legacy_grounded_coercion.cor",
        non_scope: "@grounded_pure forbids laundering inside an agent body; it does not validate the truth of the cited source. Trust in the upstream retrieval is the operator's responsibility.",
        source: r#"effect retrieval:
    data: grounded

prompt audit() -> String uses retrieval:
    "Audit"

@grounded_pure
agent run() -> Grounded<String>:
    head = "Summary: "
    tail = audit()
    return head + tail
"#,
    },
    TourTopic {
        name: "cost-budgets",
        title: "Compile-Time Budgets",
        category: "Safety at compile time",
        pitch: "Budget annotations are static constraints over the composed cost tree, not billing dashboards after the model call has run.",
        spec: "docs/internals/effect-spec/07-cost-budgets.md",
        roadmap: "Phase 20d",
        test: "crates/corvid-types/src/effects/cost.rs tests",
        non_scope: "Static budgets use declared costs; provider invoices still need operational reconciliation.",
        source: r#"effect cheap_call:
    cost: $0.05

prompt classify(text: String) -> String uses cheap_call:
    "Classify {text}"

@budget($0.10)
agent bounded(text: String) -> String:
    first = classify(text)
    return classify(first)
"#,
    },
    TourTopic {
        name: "confidence-gates",
        title: "Confidence As A Dimension",
        category: "Safety at compile time",
        pitch: "Confidence composes by weakest link. Agents can require a floor, and trust gates can escalate when confidence drops.",
        spec: "docs/internals/effect-spec/06-confidence-gates.md",
        roadmap: "Phase 20e",
        test: "crates/corvid-types/src/tests.rs min_confidence tests",
        non_scope: "Confidence is only meaningful when model adapters provide calibrated signals.",
        source: r#"effect llm_decision:
    confidence: 0.95

tool search(query: String) -> String uses llm_decision

@min_confidence(0.90)
agent bot(query: String) -> String:
    return search(query)
"#,
    },
    TourTopic {
        name: "eval-traces",
        title: "Trace-Aware Evals",
        category: "AI-native ergonomics",
        pitch: "Evals can assert process, not just output. Corvid checks whether the agent called, approved, ordered, and spent as intended.",
        spec: "docs/internals/effect-spec/12-verification.md",
        roadmap: "Phase 20c",
        test: "crates/corvid-types/src/lib.rs eval assertion tests",
        non_scope: "This is language and checker support; the full eval runner is later workflow tooling.",
        source: r#"agent always_refund() -> Bool:
    return true

eval refund_accuracy:
    result = always_refund()
    assert result == true
"#,
    },
    TourTopic {
        name: "language-keywords",
        title: "AI-Native Keywords",
        category: "AI-native ergonomics",
        pitch: "Agents, tools, prompts, effects, approvals, models, evals, and streams are syntax the compiler can reason about.",
        spec: "docs/internals/effect-spec/01-dimensional-syntax.md",
        roadmap: "Phase 20 language surface",
        test: "crates/corvid-syntax/src/parser/tests.rs",
        non_scope: "Keywords do not replace ordinary general-purpose code; they make AI boundaries visible.",
        source: r#"model local:
    capability: basic

prompt say(name: String) -> String:
    requires: basic
    "Hello {name}"

agent hello(name: String) -> String:
    return say(name)
"#,
    },
    TourTopic {
        name: "streaming-effects",
        title: "Streaming Effects",
        category: "Streaming",
        pitch: "Streams are typed values that carry effects mid-flight. Budgets, confidence, provenance, and backpressure are not after-the-fact logs.",
        spec: "docs/internals/effect-spec/08-streaming.md",
        roadmap: "Phase 20f",
        test: "crates/corvid-vm/src/tests/stream.rs",
        non_scope: "Provider-native continuation depends on provider APIs; local typed fallback tokens are the shipped boundary.",
        source: r#"agent count() -> Stream<Int>:
    yield 1
    yield 2
"#,
    },
    TourTopic {
        name: "partial-streams",
        title: "Progressive Structured Streams",
        category: "Streaming",
        pitch: "Partial<T> lets the program read complete fields as they arrive while the rest of a structured stream is still forming.",
        spec: "docs/internals/effect-spec/08-streaming.md",
        roadmap: "Phase 20f Stream<Partial<T>>",
        test: "crates/corvid-types/src/tests.rs partial stream tests",
        non_scope: "Native codegen support for every partial-stream path is still bounded by backend parity work.",
        source: r#"type Plan:
    title: String
    body: String

agent read(snapshot: Partial<Plan>) -> Option<String>:
    return snapshot.title
"#,
    },
    TourTopic {
        name: "stream-resume",
        title: "Typed Stream Resumption",
        category: "Streaming",
        pitch: "A ResumeToken<T> captures the typed stream element contract, so continuation cannot resume the wrong prompt shape.",
        spec: "docs/internals/effect-spec/08-streaming.md",
        roadmap: "Phase 20f resumption tokens",
        test: "crates/corvid-vm/src/tests/stream.rs resume tests",
        non_scope: "Provider-native session continuation waits on provider APIs; local fallback is shipped.",
        source: r#"prompt draft(topic: String) -> Stream<String>:
    "Draft {topic}"

agent capture(topic: String) -> ResumeToken<String>:
    stream = draft(topic)
    return resume_token(stream)

agent continue_it(token: ResumeToken<String>) -> Stream<String>:
    return resume(draft, token)
"#,
    },
    TourTopic {
        name: "stream-fanout",
        title: "Declarative Fan-Out / Fan-In",
        category: "Streaming",
        pitch: "Streams can split by structured fields and merge back with deterministic ordering, preserving typed stream effects.",
        spec: "docs/internals/effect-spec/08-streaming.md",
        roadmap: "Phase 20f fan-out/fan-in",
        test: "crates/corvid-types/src/tests.rs stream_split_merge_ordered_by_typechecks",
        non_scope: "Field-keyed split is shipped; first-class lambda extractors wait for function values.",
        source: r#"type Event:
    kind: String
    body: String

agent source() -> Stream<Event>:
    yield Event("b", "two")
    yield Event("a", "one")

agent fanout() -> Stream<Event>:
    groups = source().split_by("kind")
    return merge(groups).ordered_by("fair_round_robin")
"#,
    },
    TourTopic {
        name: "model-routing",
        title: "Typed Model Routing",
        category: "Adaptive routing",
        pitch: "Models are typed declarations with capability and policy dimensions. Prompt dispatch is checked against those model contracts.",
        spec: "docs/internals/effect-spec/13-model-substrate-shipped.md",
        roadmap: "Phase 20h",
        test: "crates/corvid-vm/src/tests/dispatch.rs",
        non_scope: "Does not benchmark model quality automatically; routing reports use recorded eval history.",
        source: r#"model fast:
    capability: basic

model deep:
    capability: expert

prompt answer(q: String) -> String:
    route:
        q == "hard" -> deep
        _ -> fast
    "Answer {q}"
"#,
    },
    TourTopic {
        name: "progressive-routing",
        title: "Progressive Refinement",
        category: "Adaptive routing",
        pitch: "A prompt can try cheap models first and escalate only when confidence falls below a typed threshold.",
        spec: "docs/internals/effect-spec/13-model-substrate-shipped.md#135-progressive-refinement-slice-e",
        roadmap: "Phase 20h slice E",
        test: "crates/corvid-vm/src/tests/dispatch.rs progressive tests",
        non_scope: "Thresholds are only meaningful when adapters report calibrated confidence.",
        source: r#"model cheap:
    capability: basic

model medium:
    capability: standard

model expensive:
    capability: expert

prompt classify(q: String) -> String:
    progressive:
        cheap below 0.80
        medium below 0.95
        expensive
    "Classify {q}"
"#,
    },
    TourTopic {
        name: "ensemble-voting",
        title: "Ensemble Voting",
        category: "Adaptive routing",
        pitch: "One prompt can dispatch to several models concurrently and fold the answers through a typed voting strategy.",
        spec: "docs/internals/effect-spec/13-model-substrate-shipped.md#137-ensemble-voting-slice-f",
        roadmap: "Phase 20h slice F",
        test: "crates/corvid-vm/src/tests/dispatch.rs ensemble tests",
        non_scope: "Majority voting is shipped; arbitrary custom vote functions are future language work.",
        source: r#"model opus:
    capability: expert

model sonnet:
    capability: expert

model haiku:
    capability: standard

prompt classify(q: String) -> String:
    ensemble [opus, sonnet, haiku] vote majority
    "Classify {q}"
"#,
    },
    TourTopic {
        name: "privacy-routing",
        title: "Jurisdiction And Privacy Dimensions",
        category: "Adaptive routing",
        pitch: "Model selection can carry regulatory dimensions such as jurisdiction, compliance, and privacy tier as typed model facts.",
        spec: "docs/internals/effect-spec/13-model-substrate-shipped.md#134-regulatory--compliance--privacy-dimensions-slice-d",
        roadmap: "Phase 20h slice D",
        test: "crates/corvid-types/src/effects.rs dimension law tests",
        non_scope: "The compiler enforces declared routing facts; legal compliance still requires operational controls.",
        source: r#"model eu_private:
    jurisdiction: eu_hosted
    compliance: gdpr
    privacy_tier: strict
    capability: expert

model us_fast:
    jurisdiction: us_hosted
    privacy_tier: standard
    capability: basic
"#,
    },
    TourTopic {
        name: "replay-receipts",
        title: "Replay And Receipts",
        category: "Verification",
        pitch: "Executions become evidence. Traces, replay, trace-diff, and signed receipts turn behavior changes into reviewable artifacts.",
        spec: "docs/internals/effect-spec/14-replay.md",
        roadmap: "Phase 21 and Phase 22",
        test: "crates/corvid-cli/tests/bundle_verify.rs",
        non_scope: "A receipt is cryptographic evidence of observed behavior, not a formal proof of all possible runs.",
        source: r#"@deterministic
@replayable
agent classify(text: String) -> String:
    return text
"#,
    },
    TourTopic {
        name: "effect-registry",
        title: "Proof-Carrying Dimension Registry",
        category: "Verification",
        pitch: "Corvid can distribute pieces of the effect algebra as signed artifacts with law checks, proofs, and regression programs.",
        spec: "docs/internals/effect-spec/dimension-artifacts.md",
        roadmap: "Phase 20g invention #9",
        test: "crates/corvid-driver/src/dimension_registry.rs tests",
        non_scope: "The registry distributes declarations, not executable code or unverified trust.",
        source: r#"effect local_policy:
    data: pii
    reversible: true

tool read_profile(id: String) -> String uses local_policy

agent profile(id: String) -> String:
    return read_profile(id)
"#,
    },
    TourTopic {
        name: "adversarial-tests",
        title: "Adversarial Bypass Testing",
        category: "Verification",
        pitch: "The compiler ships with a bypass-attempt taxonomy so AI can attack Corvid's own effect system in CI.",
        spec: "docs/internals/effect-spec/adversarial-taxonomy.md",
        roadmap: "Phase 20g adversarial generator",
        test: "crates/corvid-driver/src/adversarial.rs tests",
        non_scope: "Live LLM generation expands the corpus; deterministic seeds remain the safety gate.",
        source: r#"effect transfer_money:
    trust: human_required
    reversible: false

tool refund(id: String) -> String dangerous uses transfer_money

@trust(human_required)
agent safe_refund(id: String) -> String:
    approve Refund(id)
    return refund(id)
"#,
    },
];

/// Look up a topic by its stable kebab-case `name`.
pub fn find_topic(name: &str) -> Option<&'static TourTopic> {
    TOPICS.iter().find(|topic| topic.name == name)
}
