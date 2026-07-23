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
        name: "replay-quarantine",
        title: "Replay Quarantine For Durable Jobs",
        category: "Verification",
        pitch: "An `@replayable` durable job records a typed JSONL trace on its first run. `corvid jobs replay --source <path>.cor --job <id>` reproduces the run from the trace — and during that replay every side-effect surface (LLM, HTTP, application store writes, file writes) refuses to escape the process. Recorded calls substitute from the trace; unrecorded ones fail closed with a typed `QuarantineViolation` naming the surface. Differential replay can opt into live LLM calls; the default closes everything.",
        spec: "docs/phases/phase-38-replay-quarantine.md",
        roadmap: "Phase 38 audit-correction track 35V2-P38-C-replay-quarantine",
        test: "crates/corvid-runtime/tests/replay_quarantine_corpus.rs",
        non_scope: "Quarantine ensures no real side effect escapes during a Substitute-mode replay; it does not verify that the original recording itself was correct, and it does not extend to surfaces the runtime does not own (e.g. raw process spawns outside `IoRuntime`).",
        source: r#"@replayable
agent daily_brief(user_id: String) -> String:
    return "brief for " + user_id
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
    TourTopic {
        name: "file-io",
        title: "Executing File-I/O Surface",
        category: "Executing I/O",
        pitch: "Corvid's std/io tools execute real filesystem operations through a runtime-enforced [io] root confinement. Paths that escape the root are refused; calls inside @deterministic agents are rejected at typecheck; replay-mode writes never reach the live filesystem. Recoverable failures — a policy refusal, a missing file, an OS error — return honest Err values naming the cause instead of trapping, so agents can branch on them. The security boundary is declared in corvid.toml and signable through the cdylib's claim manifest.",
        spec: "docs/reference/stdlib/io.md",
        roadmap: "Phase 33S1 executing file-I/O surface",
        test: "crates/corvid-runtime/tests/executing_io_tools.rs + crates/corvid-runtime/src/io.rs IoToolPolicy tests + crates/corvid-runtime/tests/replay_quarantine_corpus.rs replay_blocks_executing_io_* fixtures",
        non_scope: "Confines paths to the declared [io] root; does not police what user code does with the read contents.",
        source: r#"import "./std/io" use io_read_text, io_write_text, FileReadEnvelope, FileWriteEnvelope

agent persist_summary(date: String, body: String) -> Result<String, String>:
    written: FileWriteEnvelope = io_write_text(date + ".txt", body)?
    return Ok(date)

agent load_summary(date: String) -> Result<String, String>:
    file: FileReadEnvelope = io_read_text(date + ".txt")?
    return Ok(file.contents)
"#,
    },
    TourTopic {
        name: "injection-taint",
        title: "Prompt Injection Is a Compile Error",
        category: "Trusted Agents",
        pitch: "OWASP's #1 LLM risk, answered by the type system. An effect declared `data: untrusted` marks its results (retrieved documents, user messages, untrusted MCP output) as `Tainted<T>`. Taint is contagious — concatenation preserves it, and a prompt that reads tainted content produces tainted output (the LLM read attacker-controlled text). `Tainted<T>` is never assignable to `T`, and passing it to an approval-requiring call (a `dangerous` tool, or one at supervisor/human trust) is a COMPILE ERROR. The only way through is the explicit, greppable `trusted(expr)` boundary — one reviewable place a human asserts the value was constrained. It is `Grounded<T>`'s provenance machinery inverted: instead of tracking where trusted data came from, it tracks where untrusted data must not go.",
        spec: "docs/meta/50i-injection-taint-design.md",
        roadmap: "Slice 50i injection-taint-v1",
        test: "crates/corvid-types/src/tests.rs (tainted_prompt_output_cannot_reach_dangerous_tool, trusted_boundary_unwraps_taint_to_reach_dangerous, direct_untrusted_source_cannot_reach_dangerous_tool, untrusted_concatenation_stays_tainted)",
        non_scope: "Compile-time flow property only; content-based injection DETECTION (is this text a jailbreak?) is the complementary `with judged` runtime guard. v1 taints whole values, not struct fields; implicit sanitizer typing (recognizing a guard cleared the taint without `trusted(...)`) is v2.",
        source: r#"effect web_content:
    data: untrusted

tool fetch_page(url: String) -> String uses web_content

effect send_money:
    trust: human_required
    reversible: false

tool pay(recipient: String, amount: Float) -> String dangerous uses send_money

prompt extract_recipient(page: String) -> String:
    "Who should be paid, per this page? {page}"

agent assistant(url: String) -> Result<String, String>:
    page = fetch_page(url)
    recipient = extract_recipient(page)
    safe = trusted(recipient)
    approve Pay(safe, 100.0)
    return Ok(pay(safe, 100.0))
"#,
    },
    TourTopic {
        name: "replay-safe-secrets",
        title: "Replay-Safe Secret Access",
        category: "Executing I/O",
        pitch: "secret_read is secret access with the trace problem actually solved: the program receives the real value, but the recorded ToolResult carries a redacted copy (traces NEVER persist secret values — a RuntimeChecked guarantee), and Substitute-mode replay re-reads the live environment instead of substituting, so a rotated credential diverges honestly instead of replaying a value the trace never stored. A missing secret is Ok with present: false — absence is a modeled state, not a crash. The residual channel is stated, not hidden: forwarding a secret into another tool's arguments records it there; the structural SecretHandle taint is the tracked deepening.",
        spec: "docs/reference/stdlib/secrets.md",
        roadmap: "Slice 48a executing-secrets-and-cache",
        test: "crates/corvid-driver/tests/executing_secrets_cache_through_driver.rs + crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_rereads_secret_from_live_environment",
        non_scope: "Env-backed reads only; the opaque SecretHandle taint (a value that never serializes, accepted by consuming surfaces) is post-v1.0.",
        source: r#"import "./std/secrets" use secret_read, SecretReadEnvelope

agent api_key() -> Result<String, String>:
    key: SecretReadEnvelope = secret_read("ANTHROPIC_API_KEY")?
    if not key.present:
        return Err("set ANTHROPIC_API_KEY")
    return Ok(key.value)
"#,
    },
    TourTopic {
        name: "provenance-cache",
        title: "Provenance-Keyed Cache",
        category: "Executing I/O",
        pitch: "cache_put / cache_get / cache_invalidate / cache_invalidate_provenance — an in-run cache whose eviction composes with Corvid's provenance story: every entry can carry the provenance key of the source it was derived from, and one cache_invalidate_provenance call drops everything computed from a source the moment that source changes, across namespaces. Addressed by (namespace, subject), one entry per address, misses are Ok values with hit: false. All four tools record and replay-substitute as ordinary tool events, so replayed runs observe identical cache behavior.",
        spec: "docs/reference/stdlib/cache.md",
        roadmap: "Slice 48a executing-secrets-and-cache",
        test: "crates/corvid-driver/tests/executing_secrets_cache_through_driver.rs",
        non_scope: "In-memory, per-run scope; durable cross-process caching is a different feature. String values in v1 (JSON-encode richer shapes via std/json).",
        source: r#"import "./std/cache" use cache_put, cache_get, CacheLookupEnvelope

agent cached_summary(doc_id: String, body: String) -> Result<String, String>:
    found: CacheLookupEnvelope = cache_get("summaries", doc_id)?
    if found.hit:
        return Ok(found.value)
    summary = body.substring(0, 80)
    cache_put("summaries", doc_id, summary, "", "doc:" + doc_id)?
    return Ok(summary)
"#,
    },
    TourTopic {
        name: "governed-retrieval",
        title: "Governed Retrieval",
        category: "Executing I/O",
        pitch: "rag_ingest and rag_search are retrieval with the moat attached: index paths resolve through the same [io] root policy as file I/O (fails closed), every failure is a typed Err value instead of a trap, every retrieved chunk carries its provenance key, and the calls are traced + replay-substituted like every executing tool — the embedder never fires on replay. With no embedder configured, search degrades honestly to term-scored lexical matching over the same index, so programs behave identically with lower recall.",
        spec: "docs/reference/stdlib/rag.md",
        roadmap: "Slice 46g rag-stdlib-dispatch",
        test: "crates/corvid-driver/tests/executing_rag_through_driver.rs",
        non_scope: "No PDF/HTML loaders on the tool surface (runtime loaders exist for embedders); no reranking; effect-level Grounded<T> wrapping waits for cross-module provenance composition (post-v1.0) — provenance travels explicitly in the envelope values.",
        source: r#"import "./std/rag" use rag_ingest, rag_search, RagChunkEnvelope

agent remember(note: String) -> Result<Int, String>:
    return rag_ingest("index.sqlite", "note-1", "notes", note, 200)

agent recall(question: String) -> String:
    found: Result<List<RagChunkEnvelope>, String> = rag_search("index.sqlite", question, 3)
    hits: List<RagChunkEnvelope> = found.unwrap_or([])
    if hits.length() > 0:
        return hits[0].text
    return "nothing indexed yet"
"#,
    },
    TourTopic {
        name: "mcp",
        title: "MCP With Governance",
        category: "Executing I/O",
        pitch: "Corvid consumes Model Context Protocol tool servers through ONE governed surface: mcp_call(server_name, tool_name, args_json). A bare MCP client is commodity — the invention is that every MCP call arrives GOVERNED: servers declared in corvid.toml are UNTRUSTED BY DEFAULT, so their calls go through the runtime approver before any transport I/O (mark trust = \"autonomous\" to loosen explicitly); every call is traced and replay-substituted (a replayed run never contacts a server and never prompts); costs ride the effect row into @budget; and every failure — unknown server, transport error, JSON-RPC error, tool-side isError, APPROVAL DENIAL — is an Err value, never a trap. stdio (spawned process, cached connection) and HTTP transports.",
        spec: "docs/reference/stdlib/mcp.md",
        roadmap: "Slice 46f mcp-client",
        test: "crates/corvid-runtime/tests/mcp_integration.rs",
        non_scope: "Client only — serving Corvid tools over MCP is post-v1.0. No compile-time tool introspection (per-tool typed imports); pair mcp_call with std/json typed accessors to decode structured output. SSE server transport streaming not in v1.",
        source: r#"import "./std/mcp" use mcp_call

agent list_notes() -> String:
    found: Result<String, String> = mcp_call("notes", "list", "{}")
    return found.unwrap_or("server unavailable or denied")
"#,
    },
    TourTopic {
        name: "parallel",
        title: "Governed Concurrency",
        category: "AI-native ergonomics",
        pitch: "parallel: runs two or more named arms CONCURRENTLY and joins when all complete — and every governance guarantee survives the concurrency. Arm costs SUM into the enclosing @budget (parallelism hides latency, not money). Each arm's trace events buffer and flush IN ARM ORDER at the join, so the recorded trace reads like sequential execution and corvid replay reproduces a concurrent run deterministically with the ordinary sequential cursor — zero trace-schema changes. Failures are arm-ordered too: the first failed arm by position, not by completion time. No other language replays concurrent LLM calls deterministically.",
        spec: "docs/meta/46e-parallel-design.md",
        roadmap: "Slice 46e parallel-construct",
        test: "crates/corvid-vm/src/tests/parallel.rs::parallel_trace_is_arm_ordered_and_replays_identically",
        non_scope: "Racing/select, timeouts, cancellation, streaming arms, and arbitrary statement bodies per arm are post-v1.0 — each arm is one call; wrap richer logic in an agent.",
        source: r#"agent fetch_weather(city: String) -> String:
    return "sunny in " + city

agent fetch_news(city: String) -> String:
    return "quiet in " + city

agent brief(city: String) -> String:
    parallel:
        weather = fetch_weather(city)
        news = fetch_news(city)
    return weather + " / " + news
"#,
    },
    TourTopic {
        name: "deterministic-time",
        title: "Deterministic Time & Randomness",
        category: "Executing I/O",
        pitch: "Corvid's std/time and std/random surfaces make clock reads and random draws TOOLS, not builtins. That single decision buys the whole reproducibility story: tool calls are traced and substituted from the recorded trace under replay, so an agent that read 2026-07-11T08:30:00Z or drew 0.42 reads and draws exactly those values on every re-run — and the checker rejects clock/dice reads inside @deterministic bodies at compile time with zero extra machinery. Durations are plain Int milliseconds (ordinary checked arithmetic IS the duration API). The pure math methods (abs, min, max, pow, sqrt, floor, ceil, round) live on the builtin-method table and never touch the trace.",
        spec: "docs/reference/stdlib/time.md",
        roadmap: "Slice 45m datetime-and-math-builtins",
        test: "crates/corvid-driver/tests/executing_time_through_driver.rs + crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_substitutes_recorded_time_and_random",
        non_scope: "UTC only — no timezone database or calendar arithmetic surface; no seeded PRNG (reproducibility comes from replay, not seed management).",
        source: r#"import "./std/time" use time_now_utc, time_format_iso
import "./std/random" use random_int

agent schedule_followup(days: Int) -> String:
    now = time_now_utc()
    return time_format_iso(now.epoch_ms + days * 86400000)

agent roll() -> Int:
    return random_int(1, 6)
"#,
    },
    TourTopic {
        name: "json",
        title: "Executing JSON Surface (Opaque + Typed-Decoder)",
        category: "Executing I/O",
        pitch: "Corvid's std/json tools ship BOTH shapes a v1.0 batteries language needs: an opaque JsonValue + typed accessors for dynamic JSON (LLM responses, polymorphic APIs, debug tooling) AND a typed-decoder convention for typed APIs (declare a struct, declare decode_X_from_json, the runtime decodes generically via serde + json_to_value against the target type). Two RuntimeChecked guarantees hold structurally — parse-safety (malformed input returns Result::Err, never panics) and field-type-safety (typed-accessor mismatches return Result::Err, never coerce or panic). Calls from @deterministic agents are rejected at typecheck; JSON parse/build are deterministic and process-internal so replay-mode dispatch runs identically to live. The tour parses JSON, accesses fields via the opaque path, AND demonstrates the typed-decoder convention by declaring a User struct + decode_user_from_json. NO Python glue required.",
        spec: "docs/reference/stdlib/json.md",
        roadmap: "Phase 33R5b executing JSON surface",
        test: "crates/corvid-driver/tests/executing_json_through_driver.rs + crates/corvid-runtime/src/json.rs tests + crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_does_not_block_executing_json_*",
        non_scope: "Opaque + typed-decoder shapes ship; cdylib codegen for JsonValue / JsonBuilder is interpreter-only in 33R5b (the corvid_json_* C-ABI exports already exist; the cdylib wire-up is plumbing for a follow-up slice). No JSON Path / JSONata / JMESPath query language; nested access via json_get_object chains.",
        source: r#"effect json_decode_eff:
    reversible: true

type User:
    id: Int
    email: String

import "./std/json" use json_parse, json_get_int

tool decode_user_from_json(text: String) -> Result<User, String> uses json_decode_eff

agent opaque_path(text: String) -> Result<Int, String>:
    parsed = json_parse(text)?
    id = json_get_int(parsed, "id")?
    return Ok(id)

agent typed_decoder_path(text: String) -> Result<Int, String>:
    user = decode_user_from_json(text)?
    return Ok(user.id)
"#,
    },
    TourTopic {
        name: "sqlite",
        title: "Executing SQLite Surface",
        category: "Executing I/O",
        pitch: "Corvid's std/db tools perform real SQLite operations through three load-bearing structural properties: SQL injection is prevented STRUCTURALLY (the typechecker's List<DbParam> signature + the runtime's rusqlite::params_from_iter binding path together make string interpolation impossible — a literal `\"'; DROP TABLE users; --\"` placed in db_param_text survives as data); path confinement REUSES [io] root from the file-I/O surface (db_open is structurally as narrow as io_write_text); replay quarantine refuses db_execute regardless of SQL contents. The DbHandle returned by db_open is an opaque, refcounted language primitive — user code cannot construct or forge one. Calls from @deterministic agents are rejected at typecheck. Open/SQL/binding failures return honest Err values naming the cause instead of trapping. The tour uses `:memory:` so it runs offline; production programs configure persistent paths through corvid.toml's [io] root.",
        spec: "docs/reference/stdlib/db.md",
        roadmap: "Phase 33S3 executing SQLite surface",
        test: "crates/corvid-driver/tests/executing_sqlite_through_driver.rs + crates/corvid-runtime/src/db.rs DbHandleRegistry tests + crates/corvid-runtime/tests/replay_quarantine_corpus.rs replay_blocks_executing_db_* fixtures",
        non_scope: "SQLite only; the Postgres path remains envelope-only (declare a Postgres tool in user code). Path confinement reuses [io] root; no separate [db] allowlist.",
        source: r#"import "./std/db" use db_open, db_execute, db_query, db_param_int, db_param_text

agent record_user(email: String) -> Result<Int, String>:
    handle = db_open(":memory:")?
    db_execute(handle, "CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT NOT NULL)", [])?
    db_execute(handle, "INSERT INTO users(id, email) VALUES (?, ?)", [db_param_int(1), db_param_text(email)])?
    rows = db_query(handle, "SELECT id FROM users WHERE email = ?", [db_param_text(email)])?
    return Ok(rows[0].rows_affected)
"#,
    },
    TourTopic {
        name: "http-client",
        title: "Executing HTTP-Client Surface",
        category: "Executing I/O",
        pitch: "Corvid's std/http tools perform real HTTP requests through a two-layer security boundary: an always-on structural SSRF block that refuses private / loopback / link-local hosts regardless of allowlist, plus a required [http] allow allowlist that fails closed when unconfigured. Calls from @deterministic agents are rejected at typecheck; Substitute-mode replay refuses every executing HTTP call regardless of allowlist contents. Transport failures and policy refusals return honest Err values (error HTTP statuses like 404 are still Ok envelopes — inspect `status`). The allowlist is declared in corvid.toml (or overridden by CORVID_HTTP_ALLOW) and signable through the cdylib's claim manifest. The same source compiles, type-checks, and runs identically whether the configured network endpoint is real or a loopback test responder — production behavior never branches on a test-only flag.",
        spec: "docs/reference/stdlib/http.md",
        roadmap: "Phase 33S2 executing HTTP-client surface",
        test: "crates/corvid-driver/tests/executing_http_through_driver.rs + crates/corvid-runtime/src/http.rs HttpEgressPolicy tests + crates/corvid-runtime/tests/replay_quarantine_corpus.rs replay_blocks_executing_http_* fixtures",
        non_scope: "Enforces SSRF + allowlist + replay quarantine on the URL host; does not police what user code does with the response body, and does not inspect or rewrite request headers.",
        source: r#"import "./std/http" use http_get, http_post_json, http_ok, HttpResponseEnvelope

agent fetch_status(url: String) -> Result<Int, String>:
    response: HttpResponseEnvelope = http_get(url)?
    return Ok(response.status)

agent ship_event(url: String, body: String) -> Result<Bool, String>:
    response: HttpResponseEnvelope = http_post_json(url, body)?
    return Ok(http_ok(response))
"#,
    },
    TourTopic {
        name: "application-surface",
        title: "Define Once, Get Everything",
        category: "The application surface",
        pitch: "A Corvid backend describes its whole public interface as a machine-readable Application Contract — every public agent/prompt with its typed inputs and AI-native capabilities (streaming events, grounding, approvals, confidence, cost, latency), every exchanged type with its field refinements, typed error enums with @status codes, uploads, cursor pagination, and the identity providers with their guaranteed OAuth safe-defaults. From that ONE contract the compiler emits a standard OpenAPI 3.1 document, an AI-native `corvid-ai.json`, a universal `corvid dev` console, and typed client SDKs in TypeScript / Swift / Kotlin / Python (plus React hooks + a runnable frontend scaffold). Define the backend once; the frontend gets types, methods, streaming, auth, errors, and pagination for free, and no two platforms can disagree about a type's shape.",
        spec: "docs/reference/inventions.md#the-application-surface",
        roadmap: "Phase 51 application surface (51a-51r)",
        test: "crates/corvid-abi/src/app_contract.rs + ts_client.rs + sdk_gen.rs + frontend_gen.rs + dev_console.rs generator tests",
        non_scope: "Describes the AI-backend↔frontend boundary precisely enough that existing frontends consume it safely; it does not make Corvid a frontend language or design your app's UI.",
        source: r#"public type Answer:
    text: String
    score: Int where between(0, 100)

public type RefundError:
    @status(404)
    @ui(message: "We could not find this payment.")
    | PaymentNotFound
    | RefundWindowExpired(expired_at: String)

identity app_users:
    provider google
    provider github
    provisioning:
        first_login: open
        tenant: fixed("public")
    session:
        lifetime: 24h
        same_site: strict

public agent classify(question: String) -> Answer:
    return Answer(question, 90)

public agent chat(message: String) -> Stream<String>:
    return echo_stream(message)

tool echo_stream(m: String) -> Stream<String>
"#,
    },
    TourTopic {
        name: "contract-closure",
        title: "The Backend Proves Its Own Contract, Or Refuses To Start",
        category: "The complete application runtime",
        pitch: "Corvid closes the gap every other backend framework leaves open: the running server can advertise a public interface it does not actually implement. Before `corvid serve` binds a listener it walks the Application Contract's public HTTP surface and asserts a runtime execution path exists for EVERY route it advertises. A route the contract describes but the runtime cannot yet serve is a startup error (`E5204 Contract not executable`) naming the offending route and the capability it needs — never a silent runtime 501. Route execution, `Stream<T>` (Server-Sent Events), `Upload<Format>` (multipart), and `Page<Item>` (cursor envelope) all serve today; the source below COMPILES cleanly, but its `requires authenticated` route makes `corvid serve` refuse to start until the authorization runtime lands — the developer's own source is the forcing function.",
        spec: "docs/reference/inventions.md#the-complete-application-runtime",
        roadmap: "Phase 52 contract closure (52b)",
        test: "crates/corvid-driver/src/contract_closure.rs tests + crates/corvid-cli/tests/serve_smoke.rs::serve_refuses_to_start_when_a_route_is_not_contract_closed",
        non_scope: "Closure asserts a runtime path EXISTS for every advertised element; it grows in lockstep with the runtime (each Phase 52 slice flips one capability on). It does not itself implement the capabilities — it refuses to start until each lands, so the backend can never advertise more than it delivers.",
        source: r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")

type Secret:
    value: String

server secure_api:
    # This route COMPILES, but `corvid serve` refuses to start with
    # E5204 until authorization enforcement (slice 52h) exists — the
    # contract must never advertise an authenticated endpoint the
    # runtime does not actually guard.
    route GET "/secret" -> json Secret requires authenticated:
        return Secret("classified")
"#,
    },
    TourTopic {
        name: "parallel-cancellation",
        title: "Cancel Fast — But Never Past a Point of No Return",
        category: "The complete application runtime",
        pitch: "A `parallel:` block runs its arms concurrently and fails fast: when one arm errors, the others are asked to stop. But Corvid adds the guarantee that makes concurrent effects safe — a branch PAST A NON-REVERSIBLE EFFECT BOUNDARY is never cancelled. The moment an arm dispatches an irreversible tool (a write, a POST — any effect whose composed row is `reversible: false`) it is shielded and runs to completion, even if a sibling has already failed; only arms that have done nothing irreversible are cancelled, and they stop at a tool-dispatch boundary BEFORE their next effect, so a cancelled arm never leaves a half-finished irreversible action behind. Cancellation is cooperative, not a preemptive abort, precisely so it can hold that line without a race. And because live cancellation is timing-dependent, every block records each arm's outcome + reversibility + dispatch boundary, and Substitute-mode replay REPRODUCES the exact run deterministically — a cancelled arm replays to its recorded boundary and stops, a shielded arm reaches its recorded terminal, and non-cancelling blocks replay byte-identically.",
        spec: "docs/reference/core-semantics.md (parallel.cancellation_reversibility)",
        roadmap: "Phase 52 effect-aware scheduling (52d)",
        test: "crates/corvid-vm/src/tests/parallel.rs (arm_past_irreversible_boundary_is_not_cancelled, replay_reproduces_a_recorded_cancellation, + adversarial cases)",
        non_scope: "Cancellation is cooperative at tool-dispatch boundaries (an arm in a tight pure loop is not preempted); reversibility comes from the composed effect row, so a tool is shielded exactly when it declares an irreversible effect.",
        source: r#"effect risky:
    cost: $0.0
    trust: autonomous
    reversible: false

tool commit_write() -> Bool uses risky
tool read_data() -> Bool

agent commit_arm() -> Bool:
    return commit_write()

agent read_arm() -> Bool:
    return read_data()

agent worker() -> Bool:
    parallel:
        # If a sibling fails first, this reversible read is cancelled
        # cleanly before its next effect.
        a = read_arm()
        # Once this commits its irreversible write it is SHIELDED — it
        # always runs to completion, and replay reproduces that exactly.
        b = commit_arm()
    return b
"#,
    },
    TourTopic {
        name: "oauth-login",
        title: "OAuth Login That's Safe By Construction",
        category: "The complete application runtime",
        pitch: "Declare an `identity` block and `corvid serve` mounts the whole login surface for you — `/auth/{provider}/login`, `/callback`, `/logout`, `/session` — wired to Authorization Code + PKCE, a single-use signed state, an OIDC nonce, and JWKS signature verification, with a Secure/HttpOnly/SameSite session cookie. The invention is what the compiler forces you to decide FIRST: how an unknown, verified user becomes an account. There is NO silent default — an identity block that declares OAuth providers but does not state its first-login policy is a compile error (`E5210 First-login policy required`), so an enterprise app can never accidentally ship open registration. You choose `open` (public signup) or `invited` (only against a pre-existing invitation); `approval_required` won't compile until the runtime can execute it completely. Identity is ALWAYS established server-side and keyed on the provider's own authoritative id — `(issuer, subject)` from a verified ID token, or `(provider, user_id)` from a server-to-server userinfo fetch for OAuth2-only providers — never an email and never a claim the caller controls. A tenant comes from fixed config, a verified invitation, or an allowlisted issuer claim, never a bare token value. The source below COMPILES; omit the `provisioning:` block and it does not.",
        spec: "docs/reference/inventions.md#first-login-is-an-explicit-compile-time-decision",
        roadmap: "Phase 52 identity runtime (52e)",
        test: "crates/corvid-cli/src/serve_auth/routes.rs (callback_tests: open provisions+recognises, invited gate, reused-state / nonce-mismatch / tampered-token refused, userinfo login) + crates/corvid-cli/tests/serve_smoke.rs::serve_mounts_the_oauth_login_surface_and_redirects_to_the_provider + crates/corvid-abi/src/app_contract.rs identity_with_oauth_provider_but_no_provisioning_is_rejected",
        non_scope: "52e mounts the login/session ROUTES and provisions the actor; route-level enforcement of `requires authenticated|role|permission` policies (and durable `approval_required` provisioning) lands in a following slice. `approval_required` parses but is rejected until the runtime can execute it.",
        source: r#"identity users:
    provider google
    provider github
    provisioning:
        # No silent default: omit this block and the program does not
        # compile (E5210). `invited` provisions an unknown verified
        # subject only against a pre-existing invitation.
        first_login: invited
        tenant: from_invitation

public agent whoami(handle: String) -> String:
    return handle
"#,
    },
];

/// Look up a topic by its stable kebab-case `name`.
pub fn find_topic(name: &str) -> Option<&'static TourTopic> {
    TOPICS.iter().find(|topic| topic.name == name)
}
