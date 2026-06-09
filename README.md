# Corvid

Corvid is a general-purpose programming language built for AI-native software.

Python, TypeScript, Rust, Go, and JavaScript can call models through libraries. Corvid makes the AI parts visible to the compiler: agents, tools, prompts, approvals, grounding, budgets, confidence, model routing, streaming, replay, and verification are language constructs.

The goal is not to be a RAG DSL. The goal is a real language for CLIs, servers, data pipelines, automation, embedded hosts, and AI agents where the compiler understands the parts of the program that spend money, call models, cross approval boundaries, stream partial results, and act on a user's behalf.

```corvid
effect transfer_money:
    cost: $0.50
    trust: human_required
    reversible: false

tool issue_refund(id: String) -> Receipt dangerous uses transfer_money

@budget($1.00)
@trust(human_required)
agent refund(id: String) -> Receipt:
    approve IssueRefund(id)
    return issue_refund(id)
```

Remove the `approve` line and the program does not compile. Increase the composed cost above `$1.00` and the program does not compile. Return `Grounded<T>` without retrieval provenance and the program does not compile. That is the point: AI safety is not an SDK convention; it is part of the language.

## Quickstart

Install ([Windows PowerShell](#install) — macOS / Linux below):

```sh
curl -fsSL https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.sh | sh
```

First program:

```bash
corvid new hello
cd hello
corvid run
```

Then explore:

- [The 5-minute quickstart in the book](./docs/book/02-quickstart.md)
- The shipped invention tour: `corvid tour --list`
- [The full book index](./docs/book/README.md)

For the full install options (Windows PowerShell, custom paths,
env overrides, `cargo install` from source, planned package-
manager paths), see [Install](#install) below.

## Contents

- [Verifiable launch surface](#verifiable-launch-surface) — signed cdylib + `claim --explain`
- [Invention catalog](#invention-catalog) — every shipped invention with proof matrix
- [Architecture](#architecture)
- [Status](#status)
- [Install](#install)
- [Install from source](#install-from-source)
- [Developer commands](#developer-commands)
- [Documentation](#documentation)
- [License](#license)

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

For the exact trust boundary and non-goals, read [docs/security/model.md](./docs/security/model.md). For the canonical guarantee table, read [docs/reference/core-semantics.md](./docs/reference/core-semantics.md).

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

Spec: [typing rules](./docs/internals/effect-spec/03-typing-rules.md)
Tour: `corvid tour --topic approve-gates`
Roadmap: [Phase 20 safety wave](./ROADMAP.md)
Proof: [approval checker tests](./crates/corvid-types/src/lib.rs)
Non-scope: Corvid proves the approval boundary exists; it does not decide whether approval is morally or legally correct.

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

Spec: [composition algebra](./docs/internals/effect-spec/02-composition-algebra.md)
Tour: `corvid tour --topic dimensional-effects`
Roadmap: [Phase 20a and Phase 20g](./ROADMAP.md)
Proof: [effect composition tests](./crates/corvid-types/src/effects.rs)
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

Spec: [grounding](./docs/internals/effect-spec/05-grounding.md)
Tour: `corvid tour --topic grounded-values`
Roadmap: [Phase 20b](./ROADMAP.md)
Proof: [grounded effect tests](./crates/corvid-types/src/effects/grounded.rs)
Non-scope: Grounding proves source linkage, not that the source itself is true.

#### Strict Citation Contracts

A prompt can name the grounded context it must cite. The compiler proves the cited parameter is grounded, and runtime checks the model response.

That turns "please cite your sources" from prompt text into a checked contract.

```corvid
prompt answer(ctx: Grounded<String>) -> Grounded<String>:
    cites ctx strictly
    "Answer from {ctx}"
```

Spec: [grounding and citation contracts](./docs/internals/effect-spec/05-grounding.md)
Tour: `corvid tour --topic strict-citations`
Roadmap: [Phase 20b cites ctx strictly](./ROADMAP.md)
Proof: [VM citation tests](./crates/corvid-vm/src/tests/dispatch.rs)
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

Spec: [grounded propagation design](./docs/meta/grounded-propagation-design.md)
Tour: `corvid tour --topic provenance-propagation`
Roadmap: [Provenance Propagation phase](./ROADMAP.md)
Proof: [proof tests + corpus fixtures](./crates/corvid-types/src/tests.rs) (`grounded_pure_*` + `tests/corpus/combined_all.cor`, `tests/corpus/legacy_grounded_coercion.cor`)
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

Spec: [cost budgets](./docs/internals/effect-spec/07-cost-budgets.md)
Tour: `corvid tour --topic cost-budgets`
Roadmap: [Phase 20d](./ROADMAP.md)
Proof: [cost analysis tests](./crates/corvid-types/src/effects/cost.rs)
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

Spec: [confidence gates](./docs/internals/effect-spec/06-confidence-gates.md)
Tour: `corvid tour --topic confidence-gates`
Roadmap: [Phase 20e](./ROADMAP.md)
Proof: [minimum confidence tests](./crates/corvid-types/src/tests.rs)
Non-scope: Confidence only means something when model adapters report calibrated signals.

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

Spec: [dimensional syntax](./docs/internals/effect-spec/01-dimensional-syntax.md)
Tour: `corvid tour --topic language-keywords`
Roadmap: [Phase 20 language surface](./ROADMAP.md)
Proof: [parser tests](./crates/corvid-syntax/src/parser/tests.rs)
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

Spec: [verification](./docs/internals/effect-spec/12-verification.md)
Tour: `corvid tour --topic eval-traces`
Roadmap: [Phase 20c](./ROADMAP.md)
Proof: [eval assertion tests](./crates/corvid-types/src/lib.rs)
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

Spec: [replay](./docs/internals/effect-spec/14-replay.md) and [bundle format](./docs/internals/bundle-format.md)
Tour: `corvid tour --topic replay-receipts`
Roadmap: [Phase 21 and Phase 22](./ROADMAP.md)
Proof: [bundle verification tests](./crates/corvid-cli/tests/bundle_verify.rs)
Non-scope: Receipts are evidence of observed behavior, not full formal verification of every possible run.

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

Spec: [phase-38 replay-quarantine design](./docs/phases/phase-38-replay-quarantine.md)
Tour: `corvid tour --topic replay-quarantine`
Roadmap: [Phase 38 audit-correction track `35V2-P38-C-replay-quarantine`](./ROADMAP.md)
Proof: [replay quarantine corpus](./crates/corvid-runtime/tests/replay_quarantine_corpus.rs)
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

Spec: [typed model substrate](./docs/internals/effect-spec/13-model-substrate-shipped.md)
Tour: `corvid tour --topic model-routing`
Roadmap: [Phase 20h](./ROADMAP.md)
Proof: [dispatch tests](./crates/corvid-vm/src/tests/dispatch.rs)
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

Spec: [progressive refinement](./docs/internals/effect-spec/13-model-substrate-shipped.md#135-progressive-refinement-slice-e)
Tour: `corvid tour --topic progressive-routing`
Roadmap: [Phase 20h slice E](./ROADMAP.md)
Proof: [progressive dispatch tests](./crates/corvid-vm/src/tests/dispatch.rs)
Non-scope: Thresholds depend on calibrated adapter confidence.

#### Ensemble Voting

One prompt can dispatch to several models concurrently and fold answers through a typed voting strategy.

The strategy is source-level, so reviewers can see when consensus is required instead of guessing from runtime code.

```corvid
prompt classify(q: String) -> String:
    ensemble [opus, sonnet, haiku] vote majority
    "Classify {q}"
```

Spec: [ensemble voting](./docs/internals/effect-spec/13-model-substrate-shipped.md#137-ensemble-voting-slice-f)
Tour: `corvid tour --topic ensemble-voting`
Roadmap: [Phase 20h slice F](./ROADMAP.md)
Proof: [ensemble tests](./crates/corvid-vm/src/tests/dispatch.rs)
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

Spec: [regulatory dimensions](./docs/internals/effect-spec/13-model-substrate-shipped.md#134-regulatory--compliance--privacy-dimensions-slice-d)
Tour: `corvid tour --topic privacy-routing`
Roadmap: [Phase 20h slice D](./ROADMAP.md)
Proof: [dimension law tests](./crates/corvid-types/src/effects.rs)
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

Spec: [streaming](./docs/internals/effect-spec/08-streaming.md)
Tour: `corvid tour --topic streaming-effects`
Roadmap: [Phase 20f](./ROADMAP.md)
Proof: [stream tests](./crates/corvid-vm/src/tests/stream.rs)
Non-scope: Provider-native continuation depends on provider APIs; local typed fallback tokens are the shipped boundary.

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

Spec: [streaming](./docs/internals/effect-spec/08-streaming.md)
Tour: `corvid tour --topic partial-streams`
Roadmap: [Phase 20f Stream<Partial<T>>](./ROADMAP.md)
Proof: [partial stream tests](./crates/corvid-types/src/tests.rs)
Non-scope: Full native parity for every partial-stream path remains backend work.

#### Typed Stream Resumption

`ResumeToken<T>` captures the typed stream element contract so continuation cannot resume the wrong prompt shape.

This gives interrupted streams a language-level recovery boundary instead of an ad hoc provider session string.

```corvid
agent capture(topic: String) -> ResumeToken<String>:
    stream = draft(topic)
    return resume_token(stream)
```

Spec: [streaming](./docs/internals/effect-spec/08-streaming.md)
Tour: `corvid tour --topic stream-resume`
Roadmap: [Phase 20f resumption tokens](./ROADMAP.md)
Proof: [stream resume tests](./crates/corvid-vm/src/tests/stream.rs)
Non-scope: Provider-native session continuation waits on provider APIs; local fallback is shipped.

#### Declarative Fan-Out / Fan-In

Streams can split by structured fields and merge back with deterministic ordering.

The stream topology is visible in the program, so effects and ordering can be preserved through orchestration.

```corvid
agent fanout() -> Stream<Event>:
    groups = source().split_by("kind")
    return merge(groups).ordered_by("fair_round_robin")
```

Spec: [streaming](./docs/internals/effect-spec/08-streaming.md)
Tour: `corvid tour --topic stream-fanout`
Roadmap: [Phase 20f fan-out/fan-in](./ROADMAP.md)
Proof: [stream type tests](./crates/corvid-types/src/tests.rs)
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

Spec: [dimension artifacts](./docs/internals/effect-spec/dimension-artifacts.md)
Tour: `corvid tour --topic effect-registry`
Roadmap: [Phase 20g invention 9](./ROADMAP.md)
Proof: [dimension registry tests](./crates/corvid-driver/src/dimension_registry.rs)
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

Spec: [adversarial taxonomy](./docs/internals/effect-spec/adversarial-taxonomy.md)
Tour: `corvid tour --topic adversarial-tests`
Roadmap: [Phase 20g adversarial generator](./ROADMAP.md)
Proof: [adversarial tests](./crates/corvid-driver/src/adversarial.rs)
Non-scope: Live LLM generation expands the corpus; deterministic seeds remain the safety gate.

#### Executing File-I/O Surface

Corvid's `std/io` module ships three executing tools — `io_read_text`, `io_write_text`, `io_list_dir` — that flow through typed effect rows (`io_read`, `io_write`, `io_list`) and a runtime-enforced `[io] root` confinement boundary.

The security boundary is **declared in `corvid.toml`** and signable: a signed cdylib carries `io_source.fs_path_confinement` + the two replay-quarantine guarantees in its claim manifest, so a host can verify the binary refuses to operate outside the declared root.

```corvid
import "./std/io" use io_read_text, io_write_text

agent persist_summary(date: String, body: String) -> String:
    io_write_text(date + ".txt", body)
    return date
```

Calls outside the configured `[io] root` are refused with a structured diagnostic naming the offending path AND the root. Calls inside a `@deterministic` agent are rejected at typecheck. Calls during replay either substitute from the recorded trace or diverge — the filesystem is provably untouched.

Spec: [`std.io` reference](./docs/reference/stdlib/io.md)
Tour: `corvid tour --topic file-io`
Roadmap: [Phase 33S1](./ROADMAP.md)
Proof: [executing I/O tests](./crates/corvid-runtime/tests/executing_io_tools.rs) + [replay-quarantine corpus](./crates/corvid-runtime/tests/replay_quarantine_corpus.rs) + [path-confinement tests](./crates/corvid-runtime/src/io.rs)
Non-scope: Confines paths to the declared root; does not police what user code does with the contents.

#### Executing HTTP-Client Surface

Corvid's `std/http` module ships two executing tools — `http_get`, `http_post_json` — that flow through typed effect rows (`http_egress_get`, `http_egress_post`) and a **two-layer security boundary**: an always-on structural SSRF block that refuses RFC1918 / loopback / link-local hosts regardless of allowlist, plus a required `[http] allow` allowlist that fails closed when unconfigured.

The security boundary is **declared in `corvid.toml`** and signable: a signed cdylib carries `io_source.http_ssrf_structural_block`, `io_source.http_allowlist_enforcement`, and `io_source.http_quarantine_on_replay` in its claim manifest, so a host can verify the binary refuses to reach private networks AND can only reach explicitly approved hosts AND cannot escape replay quarantine.

```corvid
import "./std/http" use http_get, http_post_json, http_ok

agent ship_event(url: String, body: String) -> Bool:
    response = http_post_json(url, body)
    return http_ok(response)
```

```toml
[http]
allow = ["hooks.example.com"]
```

A URL whose host is not in `[http] allow` is refused with a structured diagnostic naming the host AND the configured allowlist. A URL whose host resolves to a private range is refused by the structural SSRF block before the allowlist is consulted — even a misconfigured `[http] allow = ["127.0.0.1"]` cannot reach loopback. Calls inside a `@deterministic` agent are rejected at typecheck. Calls during Substitute-mode replay either substitute from the recorded trace or diverge — the network is provably untouched.

Spec: [`std.http` reference](./docs/reference/stdlib/http.md)
Tour: `corvid tour --topic http-client`
Roadmap: [Phase 33S2](./ROADMAP.md)
Proof: [end-to-end HTTP tests](./crates/corvid-driver/tests/executing_http_through_driver.rs) + [policy tests](./crates/corvid-runtime/src/http.rs) + [replay-quarantine corpus](./crates/corvid-runtime/tests/replay_quarantine_corpus.rs)
Non-scope: Enforces SSRF + allowlist + replay quarantine on the URL host; does not police response-body content or rewrite request headers.

## Architecture

```text
source .cor
  -> lex / parse
  -> resolve names
  -> typecheck
  -> effect, budget, confidence, grounding, approval, routing checks
  -> typed IR
  -> interpreter, native Cranelift backend, Python backend, WASM backend
  -> traces, replay, receipts, bundles
```

The language is designed as one compiler pipeline with multiple execution tiers. Safety properties belong in the shared frontend and IR, not in one backend's runtime glue.

## Status

Corvid is pre-v1.0 and under active development. The compiler, interpreter, effect system, model substrate, streaming substrate, replay/bundle infrastructure, native backend, signed cdylib attestation, separate-binary ABI descriptor verifier, and claim explanation workflow are in the repository today. Some backend paths intentionally reject newer high-level features until parity work lands; signed builds fail closed when contract-like syntax is not mapped to a registered guarantee.

A 2026-04-29 internal audit of Phases 35-41 found four phase-done bullets in Phases 38-41 that were structurally absent (multi-worker job runner + crash-recovery / DST tests, real JWT verification + `corvid auth/approvals` CLI, OTel SDK conformance, connector real mode + `corvid connectors` CLI). The ROADMAP now carries audit-correction tracks (35-N, 38K-M, 39K-L, 40J-K, 41K-M) that close the gaps end-to-end. Slice checkmarks before those tracks land are honest only at the layer the slice named — composition with the surfaces above stays disabled.

Use the roadmap for source-of-truth status:

```bash
rg -n "^- \\[ \\]" ROADMAP.md
```

## Install

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.ps1 | iex
```

**macOS / Linux**:

```sh
curl -fsSL https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.sh | sh
```

The installer downloads a prebuilt `corvid` for your OS/arch into `~/.corvid/`, adds `~/.corvid/bin` to your PATH, and runs `corvid doctor`. If a prebuilt archive is not available for your platform, it falls back to a `cargo install` from source.

Override defaults with `CORVID_REPO`, `CORVID_VERSION` (e.g. `v0.1.0`), or `CORVID_HOME`.

### Or via your package manager (planned, post-v1.0)

> **Status:** the install scripts above (`install/install.{sh,ps1}`)
> are the **canonical install path** for Corvid v1.0. The package-
> manager manifests below are tracked as Phase 33P slices in
> [`ROADMAP.md`](./ROADMAP.md#L1924) and ship **post-v1.0** — none of
> them is the "installer," each is additive metadata that a
> language-installer manager's central repo consumes and routes back
> to the GitHub Release artifacts `release.yml` produces. They're
> filed but not built yet; running them today will report
> "formula/manifest not found."

| Manager | End-user command (post-v1.0) | Filed slice |
|---|---|---|
| Homebrew (macOS / Linux) | `brew install Micrurus-Ai/corvid/corvid` | `33P1-homebrew-tap` |
| Scoop (Windows) | `scoop bucket add corvid https://github.com/Micrurus-Ai/scoop-corvid && scoop install corvid` | `33P2-scoop-bucket` |
| winget (Windows) | `winget install Micrurus-Ai.Corvid` | `33P3-winget-manifest` |
| Chocolatey (Windows) | `choco install corvid` | `33P4-chocolatey-package` |
| AUR (Arch Linux) | `yay -S corvid-bin` (or equivalent AUR helper) | `33P5-aur-package` |
| APT / RPM (Debian / Fedora) | `apt install corvid` / `dnf install corvid` after one-time repo add | `33P6-apt-rpm-repo` |

If you want to **track or contribute** any of these,
[`ROADMAP.md`'s Phase 33P block](./ROADMAP.md#L1924) names each
filed slice with what it actually does. Manifest contributions are
welcome at any time — the GitHub Release artifacts they consume
(`release.yml`) are already shipping; the manifests are
post-v1.0 only because Path A defers public packaging-manager
listings until launch by deliberate design.

## Install From Source

```bash
cargo install --path crates/corvid-cli
corvid doctor
```

Python runtime pieces are only needed when using the Python backend. Native and interpreter work do not require a Python deployment target.

## Developer Commands

```bash
cargo check --workspace
cargo test --workspace
cargo run -q -p corvid-cli -- tour --list
cargo run -q -p corvid-cli -- check examples/refund_bot_demo/refund.cor
```

If `cargo fmt --check` fails because `cargo-fmt` is not installed, install the Rust formatter for the active toolchain before treating formatting as validated.

## Documentation

- [ROADMAP.md](./ROADMAP.md): build plan and shipped slices.
- [docs/reference/inventions.md](./docs/reference/inventions.md): standalone invention catalog and proof matrix.
- [docs/internals/effect-spec/](./docs/internals/effect-spec/): AI-native effect system, grounding, budgets, confidence, streaming, model substrate, replay, and verification specs.
- [docs/reference/core-semantics.md](./docs/reference/core-semantics.md): generated guarantee registry with ids, classes, phases, and test references.
- [docs/security/model.md](./docs/security/model.md): signed artifact trust boundary, host acceptance workflow, and explicit non-goals.
- [docs/operations/ci.md](./docs/operations/ci.md): CI matrix, including optional Python FFI feature coverage.
- [docs/internals/bundle-format.md](./docs/internals/bundle-format.md): signed bundle and receipt format.
- [ARCHITECTURE.md](./ARCHITECTURE.md): compiler design and repo structure.
- [CONTRIBUTING.md](./CONTRIBUTING.md): project rules and contribution expectations.
- [docs/internals/effect-spec/bounty.md](./docs/internals/effect-spec/bounty.md): public submission process for effect-system bypasses and false positives. Accepted reports are credited to the reporter and added to [docs/internals/effect-spec/counterexamples/](./docs/internals/effect-spec/counterexamples/) as permanent regression fixtures.
- [docs/internals/package-manager-scope.md](./docs/internals/package-manager-scope.md): what the package manager does today vs what would require a hosted registry service.
  Corvid ships package format and local/self-hosted registry tooling; no Corvid-hosted package registry service runs yet.
- [dev-log.md](./dev-log.md): chronological build journal.
- [learnings.md](./learnings.md): durable engineering lessons.

## License

Corvid is released under the [MIT License](LICENSE).

Contributions are licensed under the same MIT terms (inbound = outbound)
unless the contributor marks otherwise.
