# Invention catalog

Each Corvid invention with its example, specification, tour, roadmap entry, proof and non-scope. The README keeps a short table of these entries; this page holds the full text. The [proof matrix](./inventions.md) lists the same inventions with their tests.

## Verifiable Launch Surface

Corvid's strongest production claim is not prose; it is a signed cdylib
workflow that emits externally checkable artifacts:

```bash
cargo run -q -p corvid-cli -- build app.cor --target=cdylib --sign=key.hex
cargo run -q -p corvid-cli -- claim --explain target/release/libapp.so --key pub.hex --source app.cor
cargo run -q -p corvid-abi-verify -- --source app.cor target/release/libapp.so
cargo run -q -p corvid-cli -- receipt verify-abi target/release/libapp.so --key pub.hex
```

Those commands are the public claim boundary:

- `corvid build --sign` refuses to sign when source-declared contracts are not covered by registered, non-`out_of_scope` guarantee ids.
- The cdylib embeds `CORVID_ABI_DESCRIPTOR` and, when signed, `CORVID_ABI_ATTESTATION`.
- `corvid claim --explain` prints the descriptor-carried guarantee ids, signing-key fingerprint, and source/binary descriptor agreement when `--key` and `--source` are supplied.
- `corvid-abi-verify` rebuilds the ABI descriptor from source through a separate process and byte-compares it with the cdylib's embedded `CORVID_ABI_DESCRIPTOR` (catches post-link tampering and build-cache drift; full second-implementation TCB shrinkage is post-v1.0).
- `corvid receipt verify-abi` verifies the DSSE attestation and descriptor match.

For the exact trust boundary and non-goals, read [docs/security/model.md](../../docs/security/model.md). For the canonical guarantee table, read [docs/reference/core-semantics.md](../../docs/reference/core-semantics.md).

Run the shipped invention tour:

```bash
cargo run -q -p corvid-cli -- tour --list
cargo run -q -p corvid-cli -- tour --topic approve-gates
```

The tour demos are compiler-checked in CI-style tests, so the catalog below is not detached marketing copy.

## Invention Catalog

Each entry has a runnable tour topic, a spec link, a roadmap pointer, a test pointer, and an explicit non-scope. Corvid should be ambitious, but every claim below is tied to shipped source.

### Safety At Compile Time

#### Approve Before Dangerous

Dangerous tools are not hidden behind decorators. The compiler requires an explicit approval boundary before irreversible actions.

This makes "approval happened before action" a type/effect property instead of a runtime best effort.

```corvid
tool issue_refund(id: String) -> Receipt dangerous

agent refund(id: String) -> Receipt:
    approve IssueRefund(id)
    return issue_refund(id)
```

Spec: [typing rules](../../docs/internals/effect-spec/03-typing-rules.md)
Tour: `corvid tour --topic approve-gates`
Roadmap: [Phase 20 safety wave](../../ROADMAP.md)
Proof: [approval checker tests](../../crates/corvid-types/src/lib.rs)
Non-scope: Corvid proves the approval boundary exists; it does not decide whether approval is morally or legally correct.

For served applications, an approval boundary also requires a verified requester and a complete route policy. If a route can reach `approve`—directly or through called agents—but omits `requires authenticated` or any `@approval(...)` field, the compiler rejects it. Reviewer role, risk, data class, expiry, cost ceiling, and reversibility come from source and travel through the Application Contract into the queue; `serve` invents none of them. At decision time Corvid verifies that the reviewer holds the exact declared role plus `approvals.decide`, in the same tenant, with valid CSRF, and is not the requester.

```corvid
server payments:
    @approval(role: "finance_reviewer", risk: "financial_transfer", data: "financial", expires_ms: 600000, max_cost_usd: $2500.0, irreversible: true)
    route POST "/payments" body Payment -> json Receipt requires authenticated:
        return submit_payment(body)
```

#### Dimensional Effects

Effects are not flat tags. Cost, trust, reversibility, data, latency, confidence, and user-defined dimensions compose through their own algebra.

That lets the compiler reason about AI workflows as resource- and authority-carrying programs, not just function calls.

```corvid
effect llm_call:
    cost: $0.05
    trust: autonomous

prompt summarize(text: String) -> String uses llm_call:
    "Summarize {text}"

@budget($0.10)
@trust(autonomous)
agent summarize_twice(text: String) -> String:
    first = summarize(text)
    return summarize(first)
```

Spec: [composition algebra](../../docs/internals/effect-spec/02-composition-algebra.md)
Tour: `corvid tour --topic dimensional-effects`
Roadmap: [Phase 20a and Phase 20g](../../ROADMAP.md)
Proof: [effect composition tests](../../crates/corvid-types/src/effects.rs)
Non-scope: Declared effects are compiler contracts; external providers still need operational verification.

#### Grounded<T> Provenance

`Grounded<T>` means the value must flow from a retrieval source. The typechecker rejects grounded returns without a provenance chain.

At runtime the value carries its provenance, so later prompts and traces can inspect where the answer came from.

```corvid
effect retrieval:
    data: grounded

tool fetch_doc(id: String) -> Grounded<String> uses retrieval

agent research(id: String) -> Grounded<String>:
    return fetch_doc(id)
```

Spec: [grounding](../../docs/internals/effect-spec/05-grounding.md)
Tour: `corvid tour --topic grounded-values`
Roadmap: [Phase 20b](../../ROADMAP.md)
Proof: [grounded effect tests](../../crates/corvid-types/src/effects/grounded.rs)
Non-scope: Grounding proves source linkage, not that the source itself is true.

#### Strict Citation Contracts

A prompt can name the grounded context it must cite. The compiler proves the cited parameter is grounded, and runtime checks the model response.

That turns "please cite your sources" from prompt text into a checked contract.

```corvid
prompt answer(ctx: Grounded<String>) -> Grounded<String>:
    cites ctx strictly
    "Answer from {ctx}"
```

Spec: [grounding and citation contracts](../../docs/internals/effect-spec/05-grounding.md)
Tour: `corvid tour --topic strict-citations`
Roadmap: [Phase 20b cites ctx strictly](../../ROADMAP.md)
Proof: [VM citation tests](../../crates/corvid-vm/src/tests/dispatch.rs)
Non-scope: Citation checks textual evidence, not the truth of the cited document.

#### Provenance Propagation + `@grounded_pure`

Grounded values stay grounded as they flow through ordinary code. Add a `Grounded<String>` to a plain `String`, pass it as a call argument, return it through a branch — the wrapper carries through and the runtime delivers what the type promises.

Mark an agent `@grounded_pure` and the compiler refuses to launder. Every silent `Grounded<T> -> T` coercion, every `.unwrap_discarding_sources()`, every call into an agent that isn't itself `@grounded_pure` — all rejected at compile time. The moat composes through the call graph the same way `@deterministic` does.

```corvid
effect retrieval:
    data: grounded

prompt audit() -> String uses retrieval:
    "Audit"

# `+` lifts `String + Grounded<String>` to `Grounded<String>`
# via the contagion law; the wrapper rides through to return.
@grounded_pure
agent run() -> Grounded<String>:
    head = "Summary: "
    tail = audit()
    return head + tail
```

Spec: [grounded propagation design](../../docs/meta/grounded-propagation-design.md)
Tour: `corvid tour --topic provenance-propagation`
Roadmap: [Provenance Propagation phase](../../ROADMAP.md)
Proof: [proof tests + corpus fixtures](../../crates/corvid-types/src/tests.rs) (`grounded_pure_*` + `tests/corpus/combined_all.cor`, `tests/corpus/legacy_grounded_coercion.cor`)
Non-scope: `@grounded_pure` forbids laundering inside an agent's body; it does not validate that the cited source is truthful (citation contracts handle one layer; trust in the upstream retrieval is the operator's responsibility).

#### Compile-Time Budgets

`@budget` is a static constraint over composed declared cost. The compiler rejects workflows whose worst-case cost exceeds the bound.

Cost becomes part of the program contract instead of a surprise on the provider invoice.

```corvid
effect cheap_call:
    cost: $0.05

prompt classify(text: String) -> String uses cheap_call:
    "Classify {text}"

@budget($0.10)
agent bounded(text: String) -> String:
    first = classify(text)
    return classify(first)
```

Spec: [cost budgets](../../docs/internals/effect-spec/07-cost-budgets.md)
Tour: `corvid tour --topic cost-budgets`
Roadmap: [Phase 20d](../../ROADMAP.md)
Proof: [cost analysis tests](../../crates/corvid-types/src/effects/cost.rs)
Non-scope: Static budgets use declared costs; provider billing reconciliation is still an operational concern.

#### Confidence Gates

Confidence is a first-class dimension with weakest-link composition. Agents can require a floor before acting autonomously.

Low-confidence paths can route into approval instead of silently pretending every model answer is equally reliable.

```corvid
effect llm_decision:
    confidence: 0.95

@min_confidence(0.90)
agent bot(query: String) -> String:
    return search(query)
```

Spec: [confidence gates](../../docs/internals/effect-spec/06-confidence-gates.md)
Tour: `corvid tour --topic confidence-gates`
Roadmap: [Phase 20e](../../ROADMAP.md)
Proof: [minimum confidence tests](../../crates/corvid-types/src/tests.rs)
Non-scope: Confidence only means something when model adapters report calibrated signals.

#### Secrets That Never Reach a Trace, a Prompt or a Log

A `Secret` is sensitive text, such as a webhook signing key, a password or an API token, that the program can use but never reveal. `secret(name)` from `std/secrets` reads one from the environment. Two secrets compare with `==` in constant time, a secret keys an HMAC (`body.utf8().hmac_sha256(key)`) without its text becoming a value, and a secret can be passed to the tool that needs it, alone or inside a record.

Every other way the text could leave the program is a compile error that says why: no method returns it (`to_string`, `to_json`), it never mixes with text, it has no JSON form, and no route, store field, table column, prompt, connector operation, `ask`, `choose` or `main` signature may hold it, directly or in a record field. A tool that takes a `Secret` receives the text, while every record of the call holds `<redacted>` in that position: the trace, approvals, the step debugger, replay and the exports built from the trace. Replay re-reads the secret from the environment and matches the redacted argument, so a recorded run replays although the trace never held the text. The application contract leaves every secret field out, so no client or SDK has one to read.

A key the program issues is a `Secret` from the moment it exists: `random_token(32)` from `std/crypto` records `<redacted>`, the owner receives it through a tool that takes a `Secret`, and the program keeps only `hash_password(key, owasp_2023())`, an Argon2id hash at a declared cost. `presented.verify_password(hash)` checks one in constant time and is pure. Replay has no token to substitute, so it draws a fresh one and warns.

```corvid
import "./std/secrets" use secret
import "./std/crypto" use random_token, hash_password, owasp_2023

tool sign_webhook(key: Secret, body: String) -> String
tool deliver_key(owner: String, key: Secret) -> Bool

agent signature(body: String) -> Result<String, String>:
    key = secret("WEBHOOK_KEY")?
    return Ok(sign_webhook(key, body))

agent issue_api_key(owner: String) -> String:
    key = random_token(32)
    delivered = deliver_key(owner, key)
    return hash_password(key, owasp_2023())
```

Spec: [Cryptography: secrets](../../docs/reference/cryptography.md#secrets)
Tour: `corvid tour --topic secrets`
Roadmap: [B04 cryptography, slices 2 to 4](../../docs/backend-ai-roadmap.md)
Proof: [checker refusals](../../crates/corvid-types/src/tests/secrets.rs), [interpreter tests](../../crates/corvid-vm/src/tests/secrets.rs), [trace and replay end to end](../../crates/corvid-driver/tests/secret_values.rs), [webhook verification with a secret signing key](../../crates/corvid-driver/tests/webhook_verification.rs), [tokens and passwords end to end](../../crates/corvid-driver/tests/tokens_and_passwords.rs)
Non-scope: in native code, a secret cannot yet reach a tool of the program's own, and a library's exported agents never take or return one; secrets come from the environment and `random_token`, not from a request body; a tool's own handling of the text it receives is outside the program. Showing a new key once in a route response is a roadmap item, and an encrypted table column is tracked in B09.

### AI-Native Ergonomics

#### AI-Native Keywords

`agent`, `tool`, `prompt`, `effect`, `approve`, `model`, `eval`, and streaming constructs are syntax the compiler understands.

The language stays general-purpose, but the AI boundaries are visible instead of buried in framework calls.

```corvid
model local:
    capability: basic

prompt say(name: String) -> String:
    requires: basic
    "Hello {name}"

agent hello(name: String) -> String:
    return say(name)
```

Spec: [dimensional syntax](../../docs/internals/effect-spec/01-dimensional-syntax.md)
Tour: `corvid tour --topic language-keywords`
Roadmap: [Phase 20 language surface](../../ROADMAP.md)
Proof: [parser tests](../../crates/corvid-syntax/src/parser/tests.rs)
Non-scope: Keywords do not replace ordinary application code; they expose AI-specific boundaries to the compiler.

#### Trace-Aware Evals

Corvid evals can assert process, not just output. They can check that an agent called, approved, ordered, and spent as intended.

This targets the failure mode where an AI system gets the right answer through the wrong process.

```corvid
agent always_refund() -> Bool:
    return true

eval refund_accuracy:
    result = always_refund()
    assert result == true
```

Spec: [verification](../../docs/internals/effect-spec/12-verification.md)
Tour: `corvid tour --topic eval-traces`
Roadmap: [Phase 20c](../../ROADMAP.md)
Proof: [eval assertion tests](../../crates/corvid-types/src/lib.rs)
Non-scope: This is language/checker support; the full eval runner is later workflow tooling.

#### Replay And Receipts

Executions become evidence. Traces, deterministic replay, trace-diff, lineage, and signed receipts make behavior changes reviewable.

That gives AI-native programs an audit trail developers can diff, verify, and bundle.

```corvid
@deterministic
@replayable
agent classify(text: String) -> String:
    return text
```

Spec: [replay](../../docs/internals/effect-spec/14-replay.md) and [bundle format](../../docs/internals/bundle-format.md)
Tour: `corvid tour --topic replay-receipts`
Roadmap: [Phase 21 and Phase 22](../../ROADMAP.md)
Proof: [bundle verification tests](../../crates/corvid-cli/tests/bundle_verify.rs)
Non-scope: Receipts are evidence of observed behavior, not full formal verification of every possible run.

#### Effect-Audited Skills

The only agent skill system where you read a skill's permissions like a nutrition label — and the compiler holds it to them. A skill's `skill.toml` declares its capability ceiling; `corvid add skill` COMPUTES the audit from the source (a dishonest label refuses to install), renders it for consent, and vendors visible, git-diffable code. Every `corvid check`/`corvid run` re-verifies: edit a vendored skill past its label and the next check fails naming the exceeded dimension. DSSE-signed without a registry (one verification proves publisher identity AND content integrity), hash-pinned sources (`local`, `git:`, `github:`), and `corvid skill update` with fresh consent on change.

```sh
corvid add skill github:acme/skills/summarize-repo@v1.2 --publisher-key acme.hex
```

```text
capability label (verified against the source):
  uses:       http, llm
  max trust:  supervisor_required
  max cost:   $0.2500 per call
  reach:      hosts api.github.com (enforced at runtime by [http] allow)
  signed — publisher key `bc7cbcb5636375fa` verified; content hashes match.
```

Guide: [Extending your agent](../../docs/guides/capabilities.md)
Proof: [skill audit + signing + pin tests](../../crates/corvid-driver/src/skills/)
Non-scope: hosted registry (post-v1.0); reach hosts/paths are enforced at runtime by `[http] allow` / `[io] root`, declared on the label.

#### Typed MCP In One Command

`corvid add mcp <name> --cmd ...|--url ...` discovers the server's tools and generates a typed module — one typed agent per tool from the server's own JSON schemas, arguments built with the std/json builder so escaping is never string concatenation. Untrusted servers stay approval-gated through the generated wrappers; `corvid mcp regen` refreshes the module when the server changes. `corvid add connector <provider>` does the same for the shipped connectors, rendering each manifest into scope effects + operation tools with `dangerous` on quarantined writes.

Guide: [Extending your agent](../../docs/guides/capabilities.md)
Proof: [MCP codegen tests](../../crates/corvid-driver/src/mcp_codegen.rs) + [connector scaffold tests](../../crates/corvid-cli/src/commands/connector_scaffold.rs)
Non-scope: nested schema shapes fall back to one `args_json` parameter, stated in the generated comment.

#### Prompt Injection Is a Compile Error

OWASP's #1 LLM risk, caught by the type system instead of runtime hope. An effect declared `data: untrusted` marks its results — retrieved documents, user messages, untrusted MCP output — as `Tainted<T>`. Taint is contagious: concatenation preserves it, and a prompt that reads tainted content produces tainted output (the LLM read attacker-controlled text). `Tainted<T>` is never assignable to `T`, and passing it to an approval-requiring call (a `dangerous` tool, or one at supervisor/human trust) is a **compile error**.

```corvid
effect web_content:
    data: untrusted

tool fetch_page(url: String) -> String uses web_content
tool pay(recipient: String, amount: Float) -> String dangerous uses send_money

agent assistant(url: String) -> String:
    page = fetch_page(url)
    recipient = extract_recipient(page)   # Tainted<String>
    return pay(recipient, 100.0)          # error: untrusted content
                                          # cannot parameterize pay
```

The only way through is the explicit, greppable `trusted(expr)` boundary — one reviewable place a human asserts the value was constrained. It is `Grounded<T>`'s provenance machinery inverted: instead of tracking where trusted data came from, it tracks where untrusted data must not go.

Spec: [injection-taint design](../../docs/meta/50i-injection-taint-design.md)
Tour: `corvid tour --topic injection-taint`
Guarantee: `taint.untrusted_cannot_reach_dangerous` (Static)

#### Replay-Safe Secret Access

`secret_read` solves the secrets-in-traces problem instead of ignoring it: the program receives the real value, the recorded trace event carries a redacted copy (`<redacted:XY>` + `value_redacted: true` — a RuntimeChecked guarantee, `secrets.trace_never_carries_value`), and Substitute-mode replay re-reads the live environment instead of substituting, so a rotated credential diverges honestly instead of replaying a value the trace never stored. A missing secret is `Ok` with `present: false`.

```corvid
import "std:secrets" use secret_read

agent api_key() -> Result<String, String>:
    key = secret_read("ANTHROPIC_API_KEY")?
    if not key.present:
        return Err("set ANTHROPIC_API_KEY")
    return Ok(key.value)
```

Spec: [`std.secrets` reference](../../docs/reference/stdlib/secrets.md)
Tour: `corvid tour --topic replay-safe-secrets`
Proof: [secrets + cache e2e tests](../../crates/corvid-driver/tests/executing_secrets_cache_through_driver.rs)
Non-scope: `secret_read` returns the text as a `String`, so forwarding it into another tool's arguments records it in that tool's events; [`Secret`](#secrets-that-never-reach-a-trace-a-prompt-or-a-log), read with `secret(name)`, closes that channel.

#### Provenance-Keyed Cache

`cache_put` / `cache_get` / `cache_invalidate` / `cache_invalidate_provenance` — an in-run cache whose eviction composes with provenance: entries carry the provenance key of the source they were derived from, and one call drops everything computed from a changed source, across namespaces. Misses are modeled `Ok` states (`hit: false`); all four tools record and replay-substitute as ordinary tool events, so replayed runs see identical cache behavior.

Spec: [`std.cache` reference](../../docs/reference/stdlib/cache.md)
Tour: `corvid tour --topic provenance-cache`
Proof: [secrets + cache e2e tests](../../crates/corvid-driver/tests/executing_secrets_cache_through_driver.rs)
Non-scope: in-memory, per-run scope; String values in v1.

#### Governed Cron

Language-level `schedule "<cron>" zone "<tz>" -> agent(args)` declarations execute under the scheduler runner: `corvid schedule run --source app.cor` registers each declaration as a durable schedule manifest and fires due jobs through the durable-jobs worker pool. Scheduled agents inherit the whole durable story — per-job tracing (JSONL traces for `@replayable` agents), retries with backoff, dead-letters, idempotent fires, DST-aware timezone handling, and missed-fire recovery policies — because scheduling rides the same queue everything else does, instead of a side mechanism with weaker guarantees.

Guide: [Jobs and schedules](../../docs/guides/jobs.md)
Proof: [scheduler runner e2e tests](../../crates/corvid-cli/tests/schedule_run.rs)
Non-scope: schedule arguments are literals (the manifest is a durable artifact); computation belongs in the target agent.

#### Replay Quarantine For Durable Jobs

`@replayable` durable jobs record a typed JSONL trace on their first run. `corvid jobs replay --source <path>.cor --job <id>` reproduces the run from the trace, and during replay every side-effect surface refuses to escape: LLM adapter calls, outbound HTTP, application store writes, and filesystem writes. Recorded calls substitute from the trace; unrecorded ones fail closed with a typed `QuarantineViolation` naming the surface.

This makes "replay didn't leak" a runtime invariant instead of a property of the test harness.

```corvid
@replayable
agent daily_brief(user_id: String) -> String:
    return "brief for " + user_id
```

```bash
corvid jobs run --source app.cor --state queue.db --workers 1 --max-runtime-ms 0
corvid jobs replay --source app.cor --job <job_id>
```

Spec: [phase-38 replay-quarantine design](../../docs/phases/phase-38-replay-quarantine.md)
Tour: `corvid tour --topic replay-quarantine`
Roadmap: [Phase 38 audit-correction track `35V2-P38-C-replay-quarantine`](../../ROADMAP.md)
Proof: [replay quarantine corpus](../../crates/corvid-runtime/tests/replay_quarantine_corpus.rs)
Non-scope: Quarantine ensures no real side effect escapes a Substitute-mode replay; it does not verify the original recording was correct, and it does not cover surfaces the runtime does not own (e.g. raw process spawns outside `IoRuntime`).

### Adaptive Routing

#### Typed Model Routing

Models are declarations with capabilities and policy dimensions. Prompt dispatch is checked against those contracts.

Instead of stringly selecting models in application code, routing becomes part of the typed program.

```corvid
model fast:
    capability: basic

model deep:
    capability: expert

prompt answer(q: String) -> String:
    route:
        q == "hard" -> deep
        _ -> fast
    "Answer {q}"
```

Spec: [typed model substrate](../../docs/internals/effect-spec/13-model-substrate-shipped.md)
Tour: `corvid tour --topic model-routing`
Roadmap: [Phase 20h](../../ROADMAP.md)
Proof: [dispatch tests](../../crates/corvid-vm/src/tests/dispatch.rs)
Non-scope: Routing declarations do not automatically benchmark model quality.

#### Progressive Refinement

A prompt can try cheap models first and escalate only when confidence falls below a typed threshold.

That makes cost-quality tradeoffs explicit and reviewable rather than hidden in orchestration glue.

```corvid
prompt classify(q: String) -> String:
    progressive:
        cheap below 0.80
        medium below 0.95
        expensive
    "Classify {q}"
```

Spec: [progressive refinement](../../docs/internals/effect-spec/13-model-substrate-shipped.md#135-progressive-refinement-slice-e)
Tour: `corvid tour --topic progressive-routing`
Roadmap: [Phase 20h slice E](../../ROADMAP.md)
Proof: [progressive dispatch tests](../../crates/corvid-vm/src/tests/dispatch.rs)
Non-scope: Thresholds depend on calibrated adapter confidence.

#### Ensemble Voting

One prompt can dispatch to several models concurrently and fold answers through a typed voting strategy.

The strategy is source-level, so reviewers can see when consensus is required instead of guessing from runtime code.

```corvid
prompt classify(q: String) -> String:
    ensemble [opus, sonnet, haiku] vote majority
    "Classify {q}"
```

Spec: [ensemble voting](../../docs/internals/effect-spec/13-model-substrate-shipped.md#137-ensemble-voting-slice-f)
Tour: `corvid tour --topic ensemble-voting`
Roadmap: [Phase 20h slice F](../../ROADMAP.md)
Proof: [ensemble tests](../../crates/corvid-vm/src/tests/dispatch.rs)
Non-scope: Majority voting is shipped; arbitrary custom vote functions require future function-value work.

#### Jurisdiction And Privacy Routing

Model declarations can carry regulatory dimensions such as jurisdiction, compliance, and privacy tier.

That lets the compiler enforce declared routing constraints before data crosses a boundary.

```corvid
model eu_private:
    jurisdiction: eu_hosted
    compliance: gdpr
    privacy_tier: strict
    capability: expert
```

Spec: [regulatory dimensions](../../docs/internals/effect-spec/13-model-substrate-shipped.md#134-regulatory--compliance--privacy-dimensions-slice-d)
Tour: `corvid tour --topic privacy-routing`
Roadmap: [Phase 20h slice D](../../ROADMAP.md)
Proof: [dimension law tests](../../crates/corvid-types/src/effects.rs)
Non-scope: The compiler enforces declared facts; legal compliance still requires operations, contracts, and audits.

### Streaming

#### Streaming Effects

Streams are typed values that carry effects while data is still arriving. Budgets, confidence, provenance, and backpressure are not after-the-fact logs.

This makes streaming AI workflows safer without forcing users into untyped callback systems.

```corvid
agent count() -> Stream<Int>:
    yield 1
    yield 2
```

Spec: [streaming](../../docs/internals/effect-spec/08-streaming.md)
Tour: `corvid tour --topic streaming-effects`
Roadmap: [Phase 20f](../../ROADMAP.md)
Proof: [stream tests](../../crates/corvid-vm/src/tests/stream.rs)
Non-scope: Provider token streaming is REAL as of slice 46d — a plain `-> Stream<String>` prompt streams SSE deltas from all four provider adapters through this algebra (mid-stream token limits, confidence floors, budget termination), and the trace records chunk boundaries so replay re-chunks identically. Structured `Stream<T>` and prompts using multi-model dispatch clauses (route/progressive/rollout/ensemble/adversarial) still make one whole call and yield it as a single chunk; provider-native continuation additionally depends on provider APIs.

#### Progressive Structured Streams

`Partial<T>` lets a program read complete fields as they arrive while the rest of a structured response is still forming.

The type system exposes incomplete state safely instead of asking users to parse half-valid JSON.

```corvid
type Plan:
    title: String
    body: String

agent read(snapshot: Partial<Plan>) -> Option<String>:
    return snapshot.title
```

Spec: [streaming](../../docs/internals/effect-spec/08-streaming.md)
Tour: `corvid tour --topic partial-streams`
Roadmap: [Phase 20f Stream<Partial<T>>](../../ROADMAP.md)
Proof: [partial stream tests](../../crates/corvid-types/src/tests.rs)
Non-scope: Full native parity for every partial-stream path remains backend work. Until slice 46d wires provider token streaming, `Partial<T>` values from LLM prompts arrive complete rather than progressively (see the Streaming Effects non-scope above).

#### Typed Stream Resumption

`ResumeToken<T>` captures the typed stream element contract so continuation cannot resume the wrong prompt shape.

This gives interrupted streams a language-level recovery boundary instead of an ad hoc provider session string.

```corvid
agent capture(topic: String) -> ResumeToken<String>:
    stream = draft(topic)
    return resume_token(stream)
```

Spec: [streaming](../../docs/internals/effect-spec/08-streaming.md)
Tour: `corvid tour --topic stream-resume`
Roadmap: [Phase 20f resumption tokens](../../ROADMAP.md)
Proof: [stream resume tests](../../crates/corvid-vm/src/tests/stream.rs)
Non-scope: Provider-native session continuation waits on provider APIs; local fallback is shipped. Live mid-stream interruption of provider tokens requires 46d (see the Streaming Effects non-scope above).

#### Declarative Fan-Out / Fan-In

Streams can split by structured fields and merge back with deterministic ordering.

The stream topology is visible in the program, so effects and ordering can be preserved through orchestration.

```corvid
agent fanout() -> Stream<Event>:
    groups = source().split_by("kind")
    return merge(groups).ordered_by("fair_round_robin")
```

Spec: [streaming](../../docs/internals/effect-spec/08-streaming.md)
Tour: `corvid tour --topic stream-fanout`
Roadmap: [Phase 20f fan-out/fan-in](../../ROADMAP.md)
Proof: [stream type tests](../../crates/corvid-types/src/tests.rs)
Non-scope: Field-keyed split is shipped; first-class lambda extractors wait for function values.

### Verification

#### Proof-Carrying Dimension Registry

Corvid can distribute pieces of the effect algebra as signed artifacts with law checks, proofs, and regression programs.

This is how the language can grow new policy dimensions without turning the compiler into a trust bottleneck.

```corvid
effect local_policy:
    data: pii
    reversible: true

tool read_profile(id: String) -> String uses local_policy
```

Spec: [dimension artifacts](../../docs/internals/effect-spec/dimension-artifacts.md)
Tour: `corvid tour --topic effect-registry`
Roadmap: [Phase 20g invention 9](../../ROADMAP.md)
Proof: [dimension registry tests](../../crates/corvid-driver/src/dimension_registry.rs)
Non-scope: The registry distributes declarations, not executable code or unverified trust.

#### Adversarial Bypass Testing

The compiler ships with a bypass-attempt taxonomy so AI can attack Corvid's own effect system in CI.

That turns "AI safety" into a regression target instead of a slogan.

```corvid
tool refund(id: String) -> String dangerous uses transfer_money

@trust(human_required)
agent safe_refund(id: String) -> String:
    approve Refund(id)
    return refund(id)
```

Spec: [adversarial taxonomy](../../docs/internals/effect-spec/adversarial-taxonomy.md)
Tour: `corvid tour --topic adversarial-tests`
Roadmap: [Phase 20g adversarial generator](../../ROADMAP.md)
Proof: [adversarial tests](../../crates/corvid-driver/src/adversarial.rs)
Non-scope: Live LLM generation expands the corpus; deterministic seeds remain the safety gate.

#### Executing File-I/O Surface

Corvid's `std/io` module ships three executing tools — `io_read_text`, `io_write_text`, `io_list_dir` — that flow through typed effect rows (`io_read`, `io_write`, `io_list`) and a runtime-enforced `[io] root` confinement boundary.

The security boundary is **declared in `corvid.toml`** and signable: a signed cdylib carries `io_source.fs_path_confinement` + the two replay-quarantine guarantees in its claim manifest, so a host can verify the binary refuses to operate outside the declared root.

```corvid
import "std:io" use io_read_text, io_write_text

agent persist_summary(date: String, body: String) -> Result<String, String>:
    io_write_text(date + ".txt", body)?
    return Ok(date)
```

All three tools return `Result` — a missing file, an OS error, or a policy refusal is an Err VALUE naming the cause, never a crash. Calls outside the configured `[io] root` return Err with a structured diagnostic naming the offending path AND the root. Calls inside a `@deterministic` agent are rejected at typecheck. Calls during replay either substitute from the recorded trace or diverge — the filesystem is provably untouched.

Spec: [`std.io` reference](../../docs/reference/stdlib/io.md)
Tour: `corvid tour --topic file-io`
Roadmap: [Phase 33S1](../../ROADMAP.md)
Proof: [executing I/O tests](../../crates/corvid-runtime/tests/executing_io_tools.rs) + [replay-quarantine corpus](../../crates/corvid-runtime/tests/replay_quarantine_corpus.rs) + [path-confinement tests](../../crates/corvid-runtime/src/io.rs)
Non-scope: Confines paths to the declared root; does not police what user code does with the contents.

#### Declared HTTP Client

A program calls another service through a `connector`, and every consequential choice about the request is written in source: how long it may take (`timeout: 10s`), which redirects it follows (`redirects: never | same_host | allowed_hosts`), and how its failures are typed (`on failure -> Unreachable`). Omitting the timeout or the redirect policy is a compile error. A `hosts: allowed_hosts` connector takes the URL as a parameter, for pages and callbacks a program learns at run time.

Under every connector sits a **two-layer security boundary**: an always-on structural SSRF block that refuses RFC1918, loopback and link-local hosts regardless of allowlist, and the project's `[http] allow` list, which fails closed when unconfigured. A redirect never carries the credential to another host, and a `POST` is never silently re-sent as a `GET`. A signed cdylib carries `io_source.http_ssrf_structural_block`, `io_source.http_allowlist_enforcement` and `io_source.http_quarantine_on_replay` in its claim manifest.

```corvid
type FetchError:
    | Unreachable(reason: String)

connector web:
    hosts: allowed_hosts
    timeout: 10s                          # omit → compile error
    redirects: allowed_hosts              # never | same_host | allowed_hosts
    published: no                         # yes lists it at /.well-known/corvid
    modes: [real, replay]
    operation page(url: String) -> Result<Response<String>, FetchError> uses http_read:
        GET url
        response: text
        on failure -> Unreachable         # refused, timed out, unreachable → a typed value
```

```toml
[http]
allow = ["hooks.example.com"]
```

A URL whose host is not in `[http] allow` is refused with a diagnostic naming the host and the allowlist; a host in a private range is refused before the allowlist is consulted, so even `[http] allow = ["127.0.0.1"]` cannot reach loopback. The trace records every attempt and redirect with header names only, and replay answers from the recording without touching the network. The `http_get` and `http_post_json` tools this entry once described are replaced by connectors.

Spec: [HTTP client proposal](../../docs/proposals/http-client.md) · [connector grammar](../../docs/reference/grammar.md#connector-declarations)
Tour: `corvid tour --topic http-client`
Roadmap: [B05 complete HTTP client](../../docs/backend-ai-roadmap.md)
Proof: [any-host connectors and `on failure`](../../crates/corvid-driver/tests/connector_any_host.rs) + [redirect, timeout and policy tests](../../crates/corvid-runtime/src/http.rs) + [replay-quarantine corpus](../../crates/corvid-runtime/tests/replay_quarantine_corpus.rs)
Non-scope: Enforces the declared timeout, redirect policy, SSRF block, allowlist and replay quarantine; does not police what the program does with a response, resolve hostnames before the SSRF check, or run on the native target yet.

#### Executing SQLite Surface

Corvid's `std/db` module ships three executing tools — `db_open`, `db_query`, `db_execute` — that flow through typed effect rows (`db_egress_open`, `db_egress_read`, `db_egress_write`) and three **load-bearing structural properties**:

1. **SQL injection is prevented structurally**, not by escaping. The `db_query` / `db_execute` tools' `List<DbParam>` signature forces every value through the typed constructors (`db_param_int`, `db_param_text`, etc.), and the runtime binds via `rusqlite::params_from_iter` over typed `DbValue`s. There is no `format!` call anywhere on the dispatch path; a literal `"'; DROP TABLE users; --"` placed in `db_param_text` survives as text data and never reaches SQLite's parser. The structural argument is pinned by a runtime unit test AND an end-to-end driver test against a real Corvid program.

2. **`[io] root` path confinement reuses the file-I/O boundary.** `db_open` routes through the same `IoToolPolicy` the io tools use; SQLite is structurally as narrow as `io_write_text`. No separate `[db]` allowlist exists. The documented special case `":memory:"` bypasses path resolution.

3. **`DbHandle` is an opaque, refcounted primitive type.** Constructed only by `db_open`'s typed-Value dispatch path; user code cannot forge a handle through JSON round-trip (the VM's `json_to_value` REFUSES to mint a `Type::DbHandle` from any payload, including the trace-debug sentinel shape).

The security boundary is **declared in `corvid.toml`** (via `[io] root`) and signable: a signed cdylib carries `io_source.sqlite_parameter_binding_only`, `io_source.sqlite_write_quarantine_on_replay`, `io_source.sqlite_read_passthrough_on_replay`, and the reused `io_source.fs_path_confinement` in its claim manifest.

```corvid
import "std:db" use db_open, db_execute, db_query, db_param_int, db_param_text

agent record_user(email: String) -> Result<Int, String>:
    handle = db_open(":memory:")?
    db_execute(handle, "CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT NOT NULL)", [])?
    db_execute(handle, "INSERT INTO users(id, email) VALUES (?, ?)", [db_param_int(1), db_param_text(email)])?
    rows = db_query(handle, "SELECT id FROM users WHERE email = ?", [db_param_text(email)])?
    return Ok(rows[0].rows_affected)
```

All three tools return `Result` — open failures, SQL errors, and confinement refusals are Err VALUES naming the cause.

Calls inside a `@deterministic` agent are rejected at typecheck. Calls during Substitute-mode replay refuse `db_execute` with `QuarantineViolation { surface: "db", .. }` — the database is provably untouched. SQLite only; the Postgres path remains envelope-only (declare a Postgres tool in user code and reach `corvid-runtime::PostgresDbRuntime` from a tool wrapper).

Spec: [`std.db` reference](../../docs/reference/stdlib/db.md)
Tour: `corvid tour --topic sqlite`
Roadmap: [Phase 33S3](../../ROADMAP.md)
Proof: [end-to-end SQLite tests](../../crates/corvid-driver/tests/executing_sqlite_through_driver.rs) + [DbHandleRegistry tests](../../crates/corvid-runtime/src/db.rs) + [replay-quarantine corpus](../../crates/corvid-runtime/tests/replay_quarantine_corpus.rs)
Non-scope: SQLite only — the Postgres path remains envelope-only. Path confinement reuses `[io] root`; no separate `[db]` allowlist exists.

#### Executing JSON Surface (Opaque + Typed)

Corvid handles JSON in TWO complementary shapes — the load-bearing "no Python required for JSON" promise of the batteries umbrella:

1. **Opaque path** — the `std/json` module's 13 executing tools: `json_parse(text) -> Result<JsonValue, String>`, typed accessors (`json_get_int`, `json_get_string`, ...), fluent builder (`json_object_new` → `json_object_set_*` → `json_object_finish`). For dynamic JSON: LLM responses of unknown shape, debug tooling, polymorphic APIs.

2. **Typed decoding** — `User.from_json(text)` decodes into a declared type and returns `Result<User, JsonError>`, where a `JsonError` names the failing field's path, what was expected and what was found. `from_json_strict` also rejects undeclared keys, and `value.to_json()` encodes any value. They use the same public mapping as HTTP routes and the published OpenAPI schemas, need no import, and are pure, so `@deterministic` code may call them. For typed APIs and the 33S4 HTTP→JSON→SQLite tutorial.

The surface enforces two **load-bearing structural safety properties**:

1. **Parse safety** — `json_parse` or `from_json` on arbitrary bytes returns `Result::Err` rather than panicking.
2. **Field-type safety** — `json_get_int` against a String field returns `Result::Err`, never coerces; a shape mismatch in `from_json` is an `Err` naming the field.

The security boundary is **structural in the typechecker**: a signed cdylib carries `json.parse_safety_no_panic` and `json.field_type_safety_at_access_boundary` in its claim manifest. `from_json` refuses at compile time any type containing `Grounded`, because JSON text cannot prove provenance, and `to_json` on tainted data returns `Tainted<String>`.

```corvid
type User:
    id: Int
    email: String

import "std:json" use json_parse, json_get_int

agent opaque(text: String) -> Result<Int, String>:
    parsed = json_parse(text)?
    return json_get_int(parsed, "id")

agent typed(text: String) -> Result<Int, String>:
    user = User.from_json(text).map_err(fn (e) -> e.message)?
    return Ok(user.id)
```

The `std/json` tools are rejected inside a `@deterministic` agent at typecheck, like every tool call. JSON parse/build are deterministic and process-internal so replay-mode dispatch runs IDENTICALLY to live (no quarantine flag needed). The cdylib `corvid_json_*` C-ABI exports already exist in `corvid-runtime::ffi_bridge::json_exports`; the native target does not compile `JsonValue` / `JsonBuilder` or the typed methods yet.

Spec: [`std.json` reference](../../docs/reference/stdlib/json.md)
Tour: `corvid tour --topic json`
Roadmap: [Phase 33R5b](../../ROADMAP.md) + [B01 typed JSON](../../docs/backend-ai-roadmap.md)
Proof: [end-to-end JSON tests](../../crates/corvid-driver/tests/executing_json_through_driver.rs) + [typed JSON tests](../../crates/corvid-driver/tests/typed_json_e2e.rs) + [runtime JSON tests](../../crates/corvid-runtime/src/json.rs) + [replay-quarantine corpus](../../crates/corvid-runtime/tests/replay_quarantine_corpus.rs)
Non-scope: No JSON Path / JSONata / JMESPath query language (nested access via `json_get_object` chains). Native-target support is a follow-up slice.

#### Governed Retrieval

`std/rag`'s `rag_ingest` / `rag_search` are retrieval **with the moat attached** — the part every RAG framework leaves to convention, Corvid enforces:

1. **Path confinement**: index paths resolve through the same `[io] root` policy as file I/O — fails closed when unconfigured, rejects traversal and absolute escapes.
2. **Honest failures**: missing index, bad arguments, embedder errors, and policy rejections are `Result::Err` values, never traps.
3. **Provenance on every chunk**: each retrieved `RagChunkEnvelope` carries its `provenance_key` + `effect_meta` — checkable, threadable values.
4. **Replay substitution**: the calls are traced like every executing tool; on replay the recorded results return and the embedder never fires.
5. **Honest degradation**: with no `[rag]` embedder in corvid.toml, search falls back to term-scored lexical matching over the same index — identical program behavior, lower recall.

Embedders (OpenAI / Ollama) configure in `corvid.toml` `[rag]`; ingestion chunks with overlap and embeds in one call.

Spec: [`std.rag` reference](../../docs/reference/stdlib/rag.md)
Tour: `corvid tour --topic governed-retrieval`
Roadmap: [Slice 46g](../../ROADMAP.md)
Proof: [end-to-end RAG tests](../../crates/corvid-driver/tests/executing_rag_through_driver.rs)
Non-scope: No loaders on the tool surface (PDF/HTML loaders exist runtime-side); no reranking; effect-level `Grounded<T>` wrapping waits for cross-module provenance composition (post-v1.0) — provenance travels explicitly in the envelope values.

#### MCP With Governance

Consume any Model Context Protocol tool server through `mcp_call` — with the moat intact: servers are **untrusted by default** (calls go through the runtime approver before any transport I/O; `trust = "autonomous"` loosens explicitly), every call is traced and **replay-substituted** (replays never contact a server), and every failure including approval denial is an `Err` value. stdio + HTTP transports. Try `corvid tour --topic mcp`.

Roadmap: [Slice 46f](../../ROADMAP.md)
Proof: [MCP integration tests](../../crates/corvid-runtime/tests/mcp_integration.rs)
Non-scope: client only (MCP server is post-v1.0); no compile-time tool introspection — pair with `std/json` typed accessors.

#### Governed Concurrency

`parallel:` runs named arms concurrently and joins when all complete — with the governance intact: arm costs **sum** into `@budget`, arm traces flush in **arm order** so `corvid replay` reproduces a concurrent run deterministically (zero trace-schema changes), and failures are arm-ordered, not completion-ordered. Try `corvid tour --topic parallel`.

Roadmap: [Slice 46e](../../ROADMAP.md)
Proof: [parallel tests](../../crates/corvid-vm/src/tests/parallel.rs)
Non-scope: racing/select, cancellation, streaming arms (post-v1.0); each arm is one call — wrap richer logic in an agent.

#### Executing Time & Randomness Surface

Corvid's `std/time` and `std/random` modules make the two most common sources of hidden nondeterminism — clock reads and random draws — **visible to the effect system**. `time_now_utc`, `time_monotonic_ms`, `random_float`, and `random_int` are tools flowing through typed effect rows (`time_wall`, `time_monotonic`, `rand_draw`), which means:

1. **Replay substitutes them.** Tool calls are traced; in Substitute-mode replay the recorded instant / draw is returned instead of touching the live clock or entropy source. A time-dependent agent re-runs deterministically with no seed-management convention.
2. **`@deterministic` rejects them at compile time.** The declaration-kind classifier already rejects tool calls inside `@deterministic` bodies — an agent that secretly reads the clock or rolls dice is a compile error.

The pure math methods (`abs`, `min`, `max`, `pow`, `sqrt`, `floor`, `ceil`, `round`) live on the builtin-method table under the always-checked rule (Int overflow traps; `sqrt` of a negative traps; `floor`/`ceil`/`round` return `Int` and trap on NaN). Durations are plain `Int` milliseconds — checked arithmetic IS the duration API.

```corvid
import "std:time" use time_now_utc, time_format_iso
import "std:random" use random_int

agent schedule_followup(days: Int) -> String:
    now = time_now_utc()
    return time_format_iso(now.epoch_ms + days * 86400000)

agent roll() -> Int:
    return random_int(1, 6)
```

Spec: [`std.time` reference](../../docs/reference/stdlib/time.md) + [`std.random` reference](../../docs/reference/stdlib/random.md)
Tour: `corvid tour --topic deterministic-time`
Roadmap: slice 45m in [ROADMAP.md](../../ROADMAP.md)
Proof: [end-to-end time/math/random test](../../crates/corvid-driver/tests/executing_time_through_driver.rs) + [replay substitution test](../../crates/corvid-runtime/tests/replay_quarantine_corpus.rs)
Non-scope: UTC only (no timezone database or calendar arithmetic); no seeded PRNG surface — reproducibility comes from replay.

### The Application Surface

#### Define Once, Get Everything

A Corvid backend describes its whole public interface as a machine-readable Application Contract. From that one artifact the compiler emits a standard OpenAPI 3.1 document, an AI-native `corvid-ai.json` (streaming events, grounding, approvals, confidence, cost, latency — the behavior OpenAPI cannot express), a universal `corvid dev` console, and typed client SDKs in TypeScript / Swift / Kotlin / Python, plus React hooks and a runnable frontend scaffold — all reading the SAME contract, so no two platforms disagree about a type.

Typed error enums carry `@status`/`@ui` per variant (exhaustive frontend handling); uploads and cursor pagination are first-class HTTP-boundary types. `identity` declares sign-in providers where every OAuth safe-default is mandatory — an insecure session is a compile error absent a loud opt-out, account-linking never silently merges by email, and a per-user connector token is a distinct credential from the login session.

```corvid
public type Answer:
    text: String
    score: Int where between(0, 100)

identity app_users:
    provider google
    provider github
    provisioning:
        first_login: open
        tenant: fixed("public")

public agent classify(question: String) -> Answer:
    return Answer(question, 90)

public agent chat(message: String) -> Stream<String>:
    return echo_stream(message)
```

```bash
corvid contract app | openapi | ai        # the contract + its projections
corvid generate sdk --language ts|swift|kotlin|python
corvid generate frontend --framework react
corvid dev                                # a universal, contract-driven console
```

A running `corvid serve` also advertises its own surface at `/.well-known/corvid` and `/openapi.json`.

Spec: [Application Surface](../../docs/reference/inventions.md#the-application-surface)
Tour: `corvid tour --topic application-surface`
Roadmap: [Phase 51 application surface](../../ROADMAP.md)
Proof: [contract + OpenAPI + SDK generators](../../crates/corvid-abi/src/app_contract.rs) + [core-semantics `contract.matches_compiled_surface`](../../docs/reference/core-semantics.md)
Non-scope: Corvid owns the AI-backend↔frontend boundary — it describes the surface precisely enough that existing frontends consume it safely; it does not become a frontend language or design your app's UI.

### The Complete Application Runtime

#### The Backend Proves Its Own Contract, Or Refuses To Start

Phase 51 makes a Corvid backend describe its interface; Phase 52 makes the runtime prove it implements it. Every declared route shape executes through the interpreter — a path parameter (`path.id`), a typed query struct (`query.status`), and a typed JSON body (`body.item`) each run their handler body through the ordinary agent machinery, so effects, approval, provenance, and replay apply to route execution automatically. Malformed boundary input is a structured `400`, never a `500`. And the HTTP-boundary types execute: a `Stream<T>` route streams as Server-Sent Events (a `chunk` event per item, `completed` to close, `failed` with a sanitized `{error, detail}` on a server failure); an `Upload<Format>` body is parsed from multipart under its required source-declared `@upload(max_mb: N, mime: "...")` policy and read via `body.text()`/`bytes()`/`filename()`; a `Page<Item>` response built with `Page(items, next_cursor)` serves the `{items, next_cursor, has_more}` cursor envelope. Omitting an upload maximum is a compile error—there is no hidden runtime limit.

```corvid
server imports:
    @upload(max_mb: 25, mime: "text/csv")
    route POST "/imports" body Upload<Csv> -> json ImportReceipt:
        return import_rows(body)
```

**Contract Closure** keeps the advertised surface and the runtime from ever drifting: before `corvid serve` binds a listener it asserts a runtime execution path exists for every route the contract advertises. A route it cannot yet serve is a startup error (`E5204`), never a silent runtime `501`. That mechanism carried the runtime to completion — route execution, streaming, uploads, pagination, and now **authorization enforcement** all serve. A `requires authenticated|role|permission` route resolves the caller's session to a verified typed `actor` and enforces tenant + role + permission (and CSRF double-submit on mutations) *before* the handler or any effect runs; an unauthenticated request is a `401` and an under-privileged one a `403`.

```bash
corvid serve examples/orders_api/src/main.cor   # path/query/body/stream/upload/page routes all execute
corvid serve secure_app.cor                        # STARTS — a `requires authenticated` route now serves
curl -i secure_app/secret                          # 401: the session is resolved and enforced before the handler
```

Spec: [The Complete Application Runtime](../../docs/reference/inventions.md#the-complete-application-runtime)
Tour: `corvid tour --topic contract-closure`
Roadmap: [Phase 52 the complete application runtime](../../ROADMAP.md)
Proof: [route execution + authorization enforcement](../../crates/corvid-cli/src/serve_cmd.rs) + [contract closure](../../crates/corvid-driver/src/contract_closure.rs) + [core-semantics `contract.runtime_closure`](../../docs/reference/core-semantics.md)
Non-scope: Closure grew in lockstep with the runtime — each Phase 52 slice flipped one capability on, and the interpreter tier is now complete; the refuse-to-start mechanism still guards any future capability and the native tier. Native-tier route execution is later work.

#### Cancel Fast, But Never Past a Point of No Return

A `parallel:` block runs its arms concurrently and fails fast — when one arm errors, the others are asked to stop. Corvid adds the guarantee that makes concurrent effects safe: **a branch past a non-reversible effect boundary is never cancelled.** The moment an arm dispatches an irreversible tool (a write, a `POST` — any effect whose composed row is `reversible: false`) it is shielded and runs to completion, even if a sibling failed; only arms that have done nothing irreversible are cancelled, and they stop at a tool-dispatch boundary *before* their next effect. Cancellation is cooperative, not a preemptive abort, so it holds that line without a race. And because live cancellation is timing-dependent, every block records each arm's outcome + reversibility + dispatch boundary, and Substitute-mode replay reproduces the exact run deterministically — a cancelled arm replays to its recorded boundary, a shielded arm reaches its recorded terminal, and non-cancelling blocks replay byte-identically.

Spec: [Cancel Fast, But Never Past a Point of No Return](../../docs/reference/inventions.md#parallel-cancellation)
Tour: `corvid tour --topic parallel-cancellation`
Roadmap: [Phase 52 effect-aware scheduling](../../ROADMAP.md)
Proof: [the parallel scheduler](../../crates/corvid-vm/src/interp/stmt.rs) + [replay reproduction](../../crates/corvid-runtime/src/replay/mod.rs) + [core-semantics `parallel.cancellation_reversibility`](../../docs/reference/core-semantics.md)
Non-scope: Cooperative cancellation at tool-dispatch boundaries (an arm in a tight pure loop is not preempted); a tool is shielded exactly when it declares an irreversible effect.

#### OAuth Login That's Safe By Construction

Declare an `identity` block and `corvid serve` mounts the whole login surface — `/auth/{provider}/login`, `/callback`, `/logout`, `/session` — wired to Authorization Code + PKCE, a single-use signed state, an OIDC nonce, and JWKS verification, with a Secure/HttpOnly/SameSite session cookie. The invention is what the compiler makes you decide first: **how an unknown, verified user becomes an account.** An identity block that declares OAuth providers but omits its first-login policy is a compile error (`E5210 First-login policy required`) — no silent default, so an enterprise app can never accidentally ship open registration. You pick `open` (public signup) or `invited` (only against a pre-existing invitation); `approval_required` won't compile until the runtime can execute it. Identity is always established server-side and keyed on the provider's own authoritative id — `(issuer, subject)` from a verified ID token, or `(provider, user_id)` from a server-to-server userinfo fetch for OAuth2-only providers — never an email, never a caller-controlled claim.

```corvid
identity users:
    provider google
    provider github
    provisioning:
        first_login: invited          # omit this block → E5210, does not compile
        tenant: from_invitation
```

Spec: [First Login Is An Explicit Compile-Time Decision](../../docs/reference/inventions.md#first-login-is-an-explicit-compile-time-decision)
Tour: `corvid tour --topic oauth-login`
Roadmap: [Phase 52 identity runtime](../../ROADMAP.md)
Proof: [the login routes](../../crates/corvid-cli/src/serve_auth/routes.rs) + [the provisioning executor](../../crates/corvid-runtime/src/auth/provisioning.rs) + [the E5210 checker gate](../../crates/corvid-types/src/checker/decl.rs)
Non-scope: The login/session and route-authorization runtime is complete; durable `approval_required` first-login provisioning remains a later slice.

#### Protocol-Typed Connectors

Declare an external API in source as a `connector` — base URL, `secret(...)` credentials (a bare-string credential is a parse error), and a reliability posture — and each `operation` is a callable tool with a declarative HTTP body. The mode is chosen at the boundary with **no default**: a connector declares the `modes` it may run in, the deployment picks one with `corvid run --mode`, and omitting `modes` (or selecting one the connector doesn't allow) is a compile error / startup refusal — a program can never reach a real provider by silence. How long a request may take and where a redirect may send it are declared the same way: `timeout:` and `redirects:` are required, and a redirect never carries the credential to another origin. The **same unchanged file** runs three ways: `mock` evaluates the compiled `mock:` payload, `real` makes the HTTP request (the credential resolves at dispatch into a header and never enters the IR, a trace, or an error), and `replay` serves a recorded interaction and never falls through to a real call. Because an operation IS a tool, the moat composes: a `dangerous` operation still needs `approve`, and `on status <code> -> Variant` turns an HTTP status into a typed `Result` error the compiler makes you handle.

```corvid
connector github:
    base_url: "https://api.github.com"
    auth: bearer(secret("GITHUB_TOKEN"))
    timeout: 10s                          # omit → compile error; no default
    redirects: never                      # never | same_host | allowed_hosts
    published: no                         # omit → compile error; no default
    rate_limit: 60 per 60s
    modes: [mock, replay, real]           # omit → compile error; no default
    operation get_repo(owner: String, repo: String) -> Result<Repo, GithubError> uses http_read:
        GET "/repos/{owner}/{repo}"
        on status 404 -> NotFound          # HTTP status → typed Err variant
        mock: Ok(Repo("corvid"))
```

Spec: [Protocol-Typed Connectors](../../docs/reference/inventions.md#protocol-typed-connectors)
Tour: `corvid tour --topic connectors`
Roadmap: [Phase 52 connectors](../../ROADMAP.md)
Proof: [the connector modes](../../crates/corvid-driver/tests/connector_modes.rs) + [the request builder](../../crates/corvid-runtime/src/connectors.rs) + [status→error coherence](../../crates/corvid-types/src/checker/decl.rs)
Non-scope: The runtime enforces the Corvid-declared contract (modes, credentials, egress, typed errors, reliability), not provider honesty; provider-drift quarantine is a later 52i slice.

#### Verified Provider Protocols

Some provider calls don't finish when the response arrives — you submit, and the work happens later. Everywhere else that's a hand-rolled poll loop, and the poll loop is where the bugs live: the timeout nobody tuned, the retry that submits a second job, the restart that loses the work. An `async:` block declares the **temporal contract** instead, and the compiler proves it: statuses declared once, transition tables total, every state reaching a terminal, non-zero bounds, a non-mutating poll, and a mutating submit passing `dangerous` approval. The worst-case observation count multiplies the operation's cost, so **a protocol cannot poll its way past a `@budget`**. At runtime the intent is checkpointed *before* the submit leaves the process and the provider job id binds only from the JSON-decoded response, so **a crash cannot lose the work** and a job that already recorded its submit is never re-submitted on resume. The call returns only on a declared terminal state; a submit response is never mistaken for completion.

Cancelling is honest: exact before submit, compensated through a declared `cancel` endpoint after it, and explicitly *detached* when none is declared. Editing a protocol with intents in flight requires `on_protocol_change` — and you never bump a version number, because Corvid fingerprints the protocol graph and tells you *what* changed.

Exactly-once needs the provider's help, so Corvid makes you declare how to ask for it: `idempotency: intent via header "Idempotency-Key"` sends the intent key on the submit, and omitting it is a compile error. Durability alone cannot cover the window where the provider accepted a submit but the process died before recording it — only the provider can recognise the repeat, and only if it was given something to recognise.

```corvid
operation submit_shipment(order: String) -> Job dangerous uses http_write:
    POST "/shipments" body order
    async:
        statuses: [queued, processing, completed, failed]
        initial: queued
        terminal: [completed, failed]
        deadline: 600s                     # bounds the poll loop AND the budget
        deadline_target: failed
        idempotency: intent                # checkpointed before submit
        poll GET "/shipments/{id}"         # {id} binds from the DECODED response
        every: 30s
        cancel POST "/shipments/{id}/cancel"
        on_protocol_change: refuse         # omit → compile error; no default
        state queued:
            on processing -> processing
            on completed -> completed
            on failed -> failed
```

`corvid connectors simulate <file>` explores what a provider could do to you *before* you deploy — including the behaviours that never terminate on their own:

```
[non_terminating] reporting `processing` forever holds the intent in `processing`;
  after 600s (20 observations) the declared deadline forces `failed`
[deadline_reachable] `failed` is reachable without the provider ever failing —
  a slow provider is enough
```

Spec: [Verified Provider Protocols](../../docs/reference/inventions.md#verified-provider-protocols)
Tour: `corvid tour --topic verified-provider-protocols`
Roadmap: [Phase 52 verified provider protocols](../../ROADMAP.md)
Proof: [the durable lifecycle](../../crates/corvid-driver/tests/protocol_lifecycle.rs) + [the transition engine](../../crates/corvid-runtime/src/protocol.rs) + [the simulator](../../crates/corvid-runtime/src/protocol_simulate.rs)
Non-scope: Validating that a provider's payload matches its declared shape once it arrives — live conformance and drift quarantine are a later 52i slice. The deadline and cadence are declared and checked, never defaulted.

#### Typed Tables With Revision-Checked Writes

A `table` is a record type stored in a database the program declares.
The `database` block must name its engine, location and tables, so where
data lives is always stated in source. Rows come back as typed records,
so renaming a column is a compile error at every read, and SQL text never
appears in source. Every row carries a database-maintained `revision`:
`Shelf.save(row)` writes only if the stored row still has the revision
the row was read at, and a conflict is an ordinary typed value holding
the row stored now. A query, `Shelf.where(fn (s) -> s.owner == owner)`
with optional `order_by`, `descending` and `take`, is a `List<Shelf>`
run as one parameterized query, and a condition the database cannot run
is a compile error. `corvid migrate plan` writes the migrations that
create and change declared tables. A rename (`renamed from`) and a drop
(`retired`) are declared, so the planner never mistakes one for the
other, and a change that would lose data is refused by name.

Money, time and bytes are columns the database compares exactly as
Corvid does, on SQLite and Postgres: `10 == 10.0` for a `Decimal`, and
instants in time order to the nanosecond. **Indexes follow the queries**:
a table is private to its module, so the compiler sees every query it
gets, and the planner creates the index each one needs (its equality
columns, then the column it orders or compares by), replaces it when the
query changes, and drops it when no query needs it. The migration names
the agents each index serves, so nobody maintains an index list beside
the code, and none goes stale.

```corvid
database library:
    engine: sqlite
    location: env("LIBRARY_DATABASE")

table Shelf:
    key shelf: Int
    title: String

agent store(renamed: Shelf) -> String:
    return match Shelf.save(renamed):
        Ok(saved) -> "saved revision " + saved.revision.to_string()
        Err(Changed(current)) -> "someone else changed the shelf first"
        Err(Deleted) -> "the shelf was removed"
```

Try it: `corvid tour --topic tables`. Reference: [Tables](./tables.md).

#### One Action for People and Assistants {#shared-actions}

An action is declared once: its typed input, who may call it, and whether
an assistant is offered it. The page's form, a one-click button, the JSON
API, the TypeScript client and an authorized assistant all use that one
definition, so there is no separate agent API:
- **The form is checked.** `form_for(move_card, MoveCard(...), "Move")`
  is checked against the action's input, so renaming a field of
  `MoveCard` is a compile error at the form, the client and the
  assistant's tool schema alike.
- **The assistant's line.** `assistants: confirm` makes each assistant
  call wait for the person. `assistants: allow` lets it run inside the
  person's own session, or through a grant the person made.
- **Grants.** `delegation: up_to 8 hours` caps how long a grant lasts.
  Each grant is checked at every call against what the person may do at
  that moment, so it never exceeds their authority, and revoking it
  stops it at once.

```corvid
action move_card(move: MoveCard, by: Caller) -> Moved requires permission("board.edit"):
    assistants: allow "Move a card to another column."
    return Moved(move.card, move.position)
```

Try it: `corvid tour --topic shared-actions`. Reference: [`corvid serve` actions](./cli.md), [Shared actions](../proposals/shared-actions.md), [Assistant authority](../proposals/assistant-authority.md).
