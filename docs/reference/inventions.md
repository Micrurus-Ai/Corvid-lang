# Corvid Inventions

Corvid is a general-purpose language with AI built into the compiler. These are
the shipped language ideas that make it different from Python libraries,
TypeScript frameworks, and ordinary model SDKs.

This page is intentionally independent of build instructions. It answers one
question: what can Corvid express as a language that other ecosystems usually
express as runtime glue?

## 1. Safety At Compile Time

### Approve Before Dangerous

```corvid
tool issue_refund(id: String) -> Receipt dangerous

agent refund(id: String) -> Receipt:
    approve IssueRefund(id)
    return issue_refund(id)
```

Corvid makes irreversible authority visible. A dangerous tool call without a
prior `approve` boundary is rejected by the compiler.

Why it is unique: ordinary languages can only ask a library to remember whether
approval happened. Corvid makes the boundary part of the program's static
contract.

### Dimensional Effects

```corvid
effect llm_call:
    cost: $0.05
    trust: autonomous
    reversible: true

prompt summarize(text: String) -> String uses llm_call:
    "Summarize {text}"
```

Effects in Corvid are structured dimensions, not flat labels. Cost, trust,
reversibility, data, latency, confidence, and custom dimensions compose through
declared algebra.

Why it is unique: AI applications carry money, trust, privacy, reversibility,
and confidence through the same workflow. Corvid lets the compiler reason about
those dimensions together.

### Grounded<T>

```corvid
effect retrieval:
    data: grounded

tool fetch_doc(id: String) -> Grounded<String> uses retrieval

agent answer(id: String) -> Grounded<String>:
    return fetch_doc(id)
```

`Grounded<T>` means a value must be connected to retrieval provenance. The
compiler rejects grounded returns that have no grounded source.

Why it is unique: grounding is usually a prompt convention or a RAG library
habit. Corvid makes it a type.

### Strict Citations

```corvid
prompt answer(ctx: Grounded<String>) -> Grounded<String>:
    cites ctx strictly
    "Answer from {ctx}"
```

A prompt can require citations to a specific grounded parameter. The compiler
checks the parameter's grounded type; runtime checks the model response.

Why it is unique: the citation requirement is not just text inside the prompt.
It is a contract the compiler and runtime both understand.

### Compile-Time Budgets

```corvid
effect cheap_call:
    cost: $0.05

@budget($0.10)
agent bounded(text: String) -> String:
    first = classify(text)
    return classify(first)
```

Corvid can reject workflows whose declared worst-case cost exceeds the agent's
budget.

Why it is unique: most systems discover AI cost after execution. Corvid can
make cost a static bound.

### Confidence Gates

```corvid
effect llm_decision:
    confidence: 0.95

@min_confidence(0.90)
agent bot(query: String) -> String:
    return search(query)
```

Confidence is a dimension that composes by weakest link. Agents can require a
confidence floor before autonomous action.

Why it is unique: confidence stops being a loose telemetry field and becomes a
constraint that can block unsafe autonomy.

## 2. AI-Native Ergonomics

### AI-Native Keywords

```corvid
model local:
    capability: basic

prompt say(name: String) -> String:
    requires: basic
    "Hello {name}"

agent hello(name: String) -> String:
    return say(name)
```

Corvid has syntax for agents, tools, prompts, effects, approvals, models, evals,
replay, and streams.

Why it is unique: the compiler can only protect what it can see. Corvid exposes
the AI boundaries directly in source.

### Trace-Aware Evals

```corvid
eval refund_accuracy:
    result = refund_bot(ticket)
    assert result.should_refund == true
```

Corvid eval declarations are designed to assert on behavior, including trace
events such as calls, approvals, ordering, and cost.

Why it is unique: output-only tests miss agents that get the right answer
through the wrong process. Trace-aware evals target process correctness.

### Replay And Receipts

```corvid
@deterministic
@replayable
agent classify(text: String) -> String:
    return text
```

Corvid executions can become traces, replay artifacts, diffs, signed receipts,
and verification bundles.

Why it is unique: AI behavior changes are usually invisible. Corvid turns them
into artifacts that can be audited and compared.

### Replay Quarantine For Durable Jobs

```corvid
@replayable
agent daily_brief(user_id: String) -> String:
    return "brief for " + user_id
```

```sh
# Original run records a typed JSONL trace at target/trace/jobs/<job_id>.jsonl
corvid jobs run --source app.cor --state queue.db --workers 1 --max-runtime-ms 0

# Replay reproduces the run from the trace. Every side-effect surface refuses
# to escape — recorded calls substitute, unrecorded ones fail closed with
# `QuarantineViolation` naming the surface (`llm`, `http`, `store`, `io`).
corvid jobs replay --source app.cor --job <job_id>
```

`@replayable` on an agent means more than "trace recorded." During replay,
Corvid's runtime quarantines four side-effect surfaces by construction: LLM
adapter calls, outbound HTTP, application store writes, and filesystem writes.
The durable job queue uses raw SQLite (not the application store) and the
trace writer uses a dedicated writer (not the application IO surface), so the
runtime can tell queue-internal persistence apart from application side
effects without any runtime-mode token.

Why it is unique: a "replay" in other ecosystems usually means re-running the
program and hoping nothing leaks. Corvid's replay refuses to leak by
construction. Differential replay is a separate, opt-in mode that intentionally
hits a live LLM for record-vs-live comparison; the default is the closed one.

### Governed Retrieval

- **Status**: shipped (slice 46g, 2026-07-12)
- **Run it**: `corvid tour --topic governed-retrieval`
- **Tests**: `crates/corvid-driver/tests/executing_rag_through_driver.rs`
- **Spec**: `docs/reference/stdlib/rag.md`
- **What it is**: `rag_ingest` / `rag_search` — retrieval with the moat attached: index paths confined by the `[io] root` policy, failures as typed `Err` values, provenance keys on every retrieved chunk, trace/replay substitution (the embedder never fires on replay), and honest lexical degradation when no embedder is configured.
- **Non-scope**: loaders on the tool surface, reranking, effect-level `Grounded<T>` wrapping (waits for cross-module provenance composition, post-v1.0).

### Effect-Audited Skills

- **Status**: shipped (slices 49a/49b, 2026-07-14)
- **Run it**: `corvid add skill <dir|git:|github:> [--publisher-key k.hex]`, `corvid skill sign|update`
- **Tests**: `crates/corvid-driver/src/skills/` (mod: audit/verify/vendor + edited-skill catch + dishonest-label refusal; signing: sign/verify/tamper; source: github/git parsing + real file:// clone; pin: update cycle)
- **Spec**: `docs/guides/capabilities.md`
- **What it is**: the capability nutrition label — a skill's declared ceiling (capability groups, trust, cost, data) verified against a source-computed audit at add time AND on every check/run, so even edited skills cannot silently outgrow what the user consented to. DSSE-signed registry-free; hash-pinned sources; consent-gated updates.
- **Non-scope**: hosted registry (post-v1.0); reach (hosts/paths) is declared on the label, enforced at runtime by the existing policies.

### Typed MCP Add

- **Status**: shipped (slice 49c, 2026-07-14)
- **Run it**: `corvid add mcp <name> --cmd ...|--url ...`, `corvid mcp regen <name>`
- **Tests**: `crates/corvid-driver/src/mcp_codegen.rs` (typed/fallback/empty generation, generated-module-compiles, sanitization) + `McpRuntime::list_tools` over both transports
- **Spec**: `docs/guides/capabilities.md`
- **What it is**: discovery-first MCP onboarding — the server's schemas become one typed agent per tool (json-builder args), config lands untrusted-by-default, and the approval gate rides through the generated wrappers unchanged.
- **Non-scope**: schemas beyond the primitive v1 mapping fall back to `args_json: String`, stated in the generated comment.

### Prompt Injection Is a Compile Error

- **Status**: shipped (slice 50i, 2026-07-15)
- **Run it**: `corvid tour --topic injection-taint`
- **Tests**: `crates/corvid-types/src/tests.rs` (tainted-output-to-dangerous refusal, trusted() boundary, direct-source refusal, concatenation contagion) + the `taint.untrusted_cannot_reach_dangerous` guarantee row
- **Spec**: `docs/meta/50i-injection-taint-design.md`
- **What it is**: an effect's `data: untrusted` marks its results `Tainted<T>`; taint is contagious (concatenation, and prompt-output — an LLM that read untrusted text produces tainted output) and never assignable to `T`; passing it to an approval-requiring call is a compile error. `trusted(expr)` is the sole, greppable unwrap boundary. `Grounded<T>`'s provenance machinery inverted — tracking where untrusted data must NOT go. OWASP LLM #1, answered structurally.
- **Non-scope**: compile-time flow property (content-based jailbreak detection is the complementary `with judged` guard); whole-value granularity in v1; implicit sanitizer typing is v2.

### Replay-Safe Secret Access

- **Status**: shipped (slice 48a, 2026-07-14)
- **Run it**: `corvid tour --topic replay-safe-secrets`
- **Tests**: `crates/corvid-driver/tests/executing_secrets_cache_through_driver.rs` (value-to-program, trace-redaction, missing-is-modeled) + `crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_rereads_secret_from_live_environment`
- **Spec**: `docs/reference/stdlib/secrets.md`
- **What it is**: `secret_read` with the trace problem solved instead of ignored — the program gets the real value, the recorded ToolResult carries a redacted copy (`secrets.trace_never_carries_value`, RuntimeChecked), and replay re-reads the live environment instead of substituting, so rotated credentials diverge honestly.
- **Non-scope**: the residual forwarding channel (a secret passed into another tool's args is recorded by that tool's events) — the structural `SecretHandle` taint is the tracked post-v1.0 deepening.

### Provenance-Keyed Cache

- **Status**: shipped (slice 48a, 2026-07-14)
- **Run it**: `corvid tour --topic provenance-cache`
- **Tests**: `crates/corvid-driver/tests/executing_secrets_cache_through_driver.rs` (roundtrip, invalidation-key eviction, provenance eviction across namespaces, one-entry-per-address overwrite)
- **Spec**: `docs/reference/stdlib/cache.md`
- **What it is**: an in-run cache whose eviction composes with provenance — entries carry the provenance key of their source, and one `cache_invalidate_provenance` call drops everything derived from a changed source. Misses are modeled Ok states; all operations record/replay-substitute deterministically.
- **Non-scope**: in-memory per-run scope (durable caching is a different feature); String values in v1.

### MCP With Governance

| | |
|---|---|
| **Status** | Shipped (slice 46f) |
| **Run it** | `corvid tour --topic mcp` |
| **Tests** | `crates/corvid-runtime/tests/mcp_integration.rs` — trusted round-trip, approval denial (no transport I/O), tool-side errors |
| **Spec** | `docs/reference/stdlib/mcp.md` |
| **Non-scope** | Client only (server is post-v1.0); no compile-time tool introspection; SSE transport streaming |

One governed surface — `mcp_call` — makes external MCP tools
subject to the full moat: untrusted-by-default approval gating,
trace + replay quarantine (replays never contact a server, never
prompt), budget-visible effect rows, and Err-value failures
including denial. A bare MCP client is commodity; MCP that cannot
bypass the governance is the invention.

### Governed Concurrency (`parallel:`)

| | |
|---|---|
| **Status** | Shipped (slice 46e) |
| **Run it** | `corvid tour --topic parallel` |
| **Tests** | `crates/corvid-vm/src/tests/parallel.rs` — arm-ordered trace + identical replay |
| **Spec** | `docs/meta/46e-parallel-design.md` + effect-spec §10.6 (parallel composition operator) |
| **Non-scope** | Racing/select, cancellation, streaming arms, arbitrary arm bodies (post-v1.0) |

`parallel:` arms run concurrently while every moat guarantee
survives: costs SUM into `@budget`, trust maxes, confidence mins;
arm trace buffers flush in ARM ORDER at the join so a concurrent
run replays deterministically through the unchanged sequential
cursor; the error rule is arm-ordered. Concurrency that stays
governed is the invention — no mainstream runtime replays
concurrent LLM calls deterministically.

### Deterministic Time And Randomness

```corvid
import "./std/time" use time_now_utc, time_format_iso
import "./std/random" use random_int

agent schedule_followup(days: Int) -> String:
    now = time_now_utc()
    return time_format_iso(now.epoch_ms + days * 86400000)
```

Clock reads and random draws are tools, not builtins — so they are
traced, substituted under replay (a re-run reads the recorded
instant and draws the recorded value), and rejected inside
`@deterministic` bodies at compile time. Reproducibility comes
from the replay machinery the language already has, not from seed
management conventions.

Why it is unique: ordinary languages treat `now()` and `random()`
as ambient, untracked authority — the two most common reasons a
"deterministic" pipeline isn't. Corvid routes both through the
effect system.

- Shipped: slice 45m. Runnable: `corvid tour --topic deterministic-time`.
- Tests: `crates/corvid-driver/tests/executing_time_through_driver.rs`,
  `crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_substitutes_recorded_time_and_random`.
- Spec: [`std.time`](./stdlib/time.md), [`std.random`](./stdlib/random.md).
- Non-scope: UTC only; no calendar arithmetic; no seeded PRNG surface.

## 3. Adaptive Routing

### Typed Model Routing

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

Models are declarations with capabilities and policy dimensions. Prompt routing
is checked against those facts.

Why it is unique: model selection becomes a typed program decision instead of a
string hidden in runtime glue.

### Progressive Refinement

```corvid
prompt classify(q: String) -> String:
    progressive:
        cheap below 0.80
        medium below 0.95
        expensive
    "Classify {q}"
```

Prompts can try cheaper models first and escalate only when confidence is not
high enough.

Why it is unique: cost-quality tradeoffs are visible in source and can be
reviewed with the rest of the program.

### Ensemble Voting

```corvid
prompt classify(q: String) -> String:
    ensemble [opus, sonnet, haiku] vote majority
    "Classify {q}"
```

One prompt can dispatch to multiple models and fold the responses through a
typed voting strategy.

Why it is unique: consensus becomes a language-level strategy, not an
unreviewed helper function.

### Jurisdiction And Privacy Routing

```corvid
model eu_private:
    jurisdiction: eu_hosted
    compliance: gdpr
    privacy_tier: strict
    capability: expert
```

Model declarations can include privacy, compliance, and jurisdiction facts.

Why it is unique: data-placement policy can be checked before a prompt crosses
the wrong boundary.

## 4. Streaming

### Streaming Effects

```corvid
agent count() -> Stream<Int>:
    yield 1
    yield 2
```

Streams are typed values that can carry provenance, confidence, cost, and
backpressure semantics.

Why it is unique: streaming AI is usually an untyped callback path. Corvid keeps
it inside the language.

### Progressive Structured Streams

```corvid
type Plan:
    title: String
    body: String

agent read(snapshot: Partial<Plan>) -> Option<String>:
    return snapshot.title
```

`Partial<T>` exposes complete fields as they arrive while the rest of a
structured response is still forming.

Why it is unique: users can work with partial structured output safely instead
of parsing incomplete JSON.

### Typed Stream Resumption

```corvid
agent capture(topic: String) -> ResumeToken<String>:
    stream = draft(topic)
    return resume_token(stream)
```

`ResumeToken<T>` preserves the stream element contract across interruption and
continuation.

Why it is unique: resumption is typed. A token for one stream shape cannot be
used as another.

### Declarative Fan-Out / Fan-In

```corvid
agent fanout() -> Stream<Event>:
    groups = source().split_by("kind")
    return merge(groups).ordered_by("fair_round_robin")
```

Streams can split by structured fields and merge back with deterministic
ordering.

Why it is unique: stream topology is declared in the program, so the compiler
and runtime can preserve ordering and effect metadata.

## 5. Verification

### Proof-Carrying Dimension Registry

```corvid
effect local_policy:
    data: pii
    reversible: true

tool read_profile(id: String) -> String uses local_policy
```

Custom effect dimensions can be distributed as signed artifacts with law checks,
proof pointers, and regression programs.

Why it is unique: the effect system can grow without asking users to trust
arbitrary executable packages.

### Adversarial Bypass Testing

```corvid
tool refund(id: String) -> String dangerous uses transfer_money

@trust(human_required)
agent safe_refund(id: String) -> String:
    approve Refund(id)
    return refund(id)
```

Corvid includes a bypass taxonomy so the effect checker can be attacked by a
deterministic adversarial corpus.

Why it is unique: the language uses adversarial testing against its own safety
claims instead of treating them as prose.

## 6. The Application Surface

### Define Once, Get Everything {#the-application-surface}

A Corvid backend describes its whole public interface as a machine-readable
**Application Contract**. From that one artifact the compiler emits a standard
OpenAPI 3.1 document, an AI-native `corvid-ai.json`, a universal console, and
typed client SDKs — no hand-written glue.

```corvid
public type Answer:
    text: String
    score: Int where between(0, 100)

public agent classify(question: String) -> Answer:
    return Answer(question, 90)

public agent chat(message: String) -> Stream<String>:
    return echo_stream(message)
```

```bash
corvid contract app        # the machine-readable Application Contract
corvid contract openapi    # standard OpenAPI 3.1 (any client generator consumes it)
corvid contract ai         # the AI-native event/grounding/cost metadata
corvid contract ts-client  # a typed TypeScript client
corvid generate sdk --language swift|kotlin|python
corvid generate frontend --framework react   # a runnable starter project
corvid dev                 # a universal, contract-driven console
```

Why it is unique: OpenAPI describes ordinary HTTP; it cannot express streaming
event protocols, approvals, grounding, confidence routing, or per-cost budgets.
Corvid's contract does, and every generated client, the console, and the SDKs in
four languages read the SAME contract — so no two platforms can disagree about a
type's shape.

### Typed Errors That Reach The Frontend

An error enum's variants carry an `@status(code)` and `@ui(...)` presentation
defaults; a `Result<T, E>` route projects one OpenAPI response per status, and
the TypeScript generator emits a discriminated union so a frontend handles every
case exhaustively — the compiler-enforced exhaustiveness of a Corvid `match`
extended across the HTTP boundary.

### Identity, Safe By Construction

`identity` declares the sign-in providers and session posture. Every OAuth
safe-default (Authorization Code + PKCE, JWKS verification, secure http-only
cookies, session rotation, CSRF, refresh rotation, encrypted tokens, redacted
logs) is the default AND mandatory: making a session insecure is a **compile
error** unless a loud `insecure_opt_out: true` is present. Account linking runs
an explicit-confirmation flow with no silent email-match merge, and a per-user
connector token is a distinct credential the runtime refuses to accept as a
login session. A local mock IdP plus source-bypass mutators + JWT byte-fuzz
prove the safe-defaults cannot be bypassed.

Why it is unique: identity is usually a library you can misconfigure. In Corvid
the insecure configuration is the one you have to fight the compiler for.

## 7. The Complete Application Runtime

### The Backend Proves Its Own Contract, Or Refuses To Start {#the-complete-application-runtime}

Phase 51 makes a Corvid backend describe its whole public interface. Phase 52
makes the runtime **prove it implements that interface** — the two must never
disagree. Every declared route shape executes through the interpreter tier: a
path parameter (`path.id`), a typed query struct (`query.status`), and a typed
JSON body (`body.item`) each run their handler body through the ordinary agent
machinery, so effects, approval, provenance, and replay apply to route execution
automatically (slice 52a).

The runtime executes the HTTP-boundary types the contract advertises. A
`Stream<T>` route streams end-to-end as Server-Sent Events (`data:` per yield,
`event: done` to close). An `Upload<Format>` body is parsed from the multipart
request — accepted-MIME and max-size enforced at the boundary — and read through
`body.text()` / `body.bytes()` / `body.filename()`. A `Page<Item>` response is
built with `Page(items, next_cursor)` and serialized as the standard
`{items, next_cursor, has_more}` cursor envelope, `has_more` derived. Each falls
straight out of the language's own type with zero glue.

An upload route has no hidden interpreter-wide size default. The source must put
`@upload(max_mb: N)` or `@upload(max_bytes: N)` immediately before a direct
`body Upload<Format>` route; omission is a compile error. The compiler emits
that exact maximum and the resolved MIME set into the Application Contract and
OpenAPI, and `corvid serve` enforces the same values at the multipart boundary.
The contract, generated clients, and running backend therefore cannot disagree
about the accepted file.

**Contract Closure** guarantees the surface and the runtime can never drift.
Before `corvid serve` / `corvid dev` bind a listener, they walk the public HTTP
surface the Application Contract advertises and assert a runtime execution path
exists for every route. A route the contract describes but the runtime cannot
yet serve is a **startup error**, never a silent runtime `501`:

```corvid
identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")

server secure_api:
    route GET "/secret" -> json Secret requires authenticated:
        return Secret("classified")
```

```bash
corvid check main.cor   # ok — the source compiles cleanly
corvid serve main.cor   # STARTS — `requires authenticated` is contract-closed
curl -i .../secret      # 401: `corvid serve` resolves the caller's session to a
                        #   verified `actor` and enforces the policy BEFORE the
                        #   handler runs; the classified body never leaks.
```

The closure surface is driven by a capability snapshot that each Phase 52 slice
flipped as it landed the capability (route execution ✓, streaming ✓, uploads ✓,
pagination ✓, authorization enforcement ✓) — the interpreter tier is now
complete, so the running backend can never advertise more than it delivers. The
refuse-to-start mechanism still guards any future capability and the native tier;
until a capability lands, an advertising route is a startup error (`E5204`),
never a silent runtime `501`.

Why it is unique: every other framework lets a server route return `501` — or
worse, a plausible-but-wrong response — for an endpoint its API docs promise. In
Corvid the developer's own source is the forcing function: writing a route the
runtime can't serve refuses the whole process, loudly, at startup.

### Cancel Fast, But Never Past a Point of No Return {#parallel-cancellation}

A `parallel:` block runs its arms concurrently and fails fast — when one arm
errors, the others are asked to stop. Corvid adds the guarantee that makes
concurrent effects safe: **a branch past a non-reversible effect boundary is
never cancelled.** The moment an arm dispatches an irreversible tool — a write,
a `POST`, any effect whose composed row is `reversible: false` — it is shielded
and runs to completion, even if a sibling has already failed. Only arms that
have done nothing irreversible are cancelled, and they stop at a tool-dispatch
boundary *before* their next effect, so a cancelled arm never leaves a
half-finished irreversible action behind.

```corvid
agent worker() -> Bool:
    parallel:
        a = read_arm()      # reversible — cancelled cleanly if a sibling fails
        b = commit_arm()    # once it commits its write it is SHIELDED, always completes
    return b
```

Cancellation is **cooperative, not preemptive** — each arm checks a shared flag
at each tool-dispatch boundary — precisely so it can hold that line without a
race (a preemptive abort could stop an arm *inside* an irreversible call, after
the effect fired). And because live cancellation is timing-dependent, every
block records each arm's outcome, reversibility, and dispatch boundary, and
Substitute-mode replay **reproduces the exact run deterministically**: a
cancelled arm replays to its recorded boundary and stops, a shielded arm reaches
its recorded terminal, and non-cancelling blocks replay byte-identically. The
replay/trace determinism moat holds through cancellation.

Why it is unique: concurrent frameworks cancel tasks with no notion of what
those tasks have irreversibly done — and none can deterministically replay a
cancelled concurrent run. Corvid's effect system knows exactly which branch has
crossed a point of no return, and its trace pins the one true run.

### First Login Is An Explicit Compile-Time Decision {#first-login-is-an-explicit-compile-time-decision}

Declaring an `identity` block makes `corvid serve` mount the entire login
surface — `/auth/{provider}/login`, `/callback`, `/logout`, `/session` — wired to
Authorization Code + PKCE, a single-use signed state, an OIDC nonce, and JWKS
signature verification, with a Secure/HttpOnly/SameSite session cookie. The
invention is what the compiler forces you to decide *first*: **how an unknown,
verified user becomes an account.** There is no silent default. An identity block
that declares OAuth providers but does not state its first-login policy is a
compile error:

```
E5210 First-login policy required: identity `users` declares OAuth providers but
does not state how an unknown verified subject is provisioned.
Add: provisioning: first_login: open | invited
```

```corvid
identity users:
    provider google
    provider github
    provisioning:
        first_login: invited          # or: open
        tenant: from_invitation        # or: fixed("public") / from_claim("org") allow "acme"
```

Because the choice sets an app's whole registration and tenancy posture, silence
must not pick it — defaulting to auto-provision would let an enterprise app
become open-registration the instant anyone with a matching account hit the
callback. So the posture is declared in source (the "no hidden defaults for
consequential policy" rule), `open` and `invited` are executable today, and
`approval_required` parses but will not compile until the runtime can execute it
completely — a policy is never silently downgraded to a weaker one.

The identity a login resolves to is **always established server-side, keyed on the
provider's own authoritative id** — `(issuer, subject)` from a verified ID token
for OIDC providers, or `(provider, user_id)` from a server-to-server userinfo
fetch for OAuth2-only providers (github/slack/discord). It is never an email and
never a claim the caller controls: Corvid does not identify or merge accounts by
email, and a tenant comes only from fixed config, a verified invitation, or an
allowlisted issuer claim. The callback runs a strict order — validate the
single-use state (PKCE + nonce), exchange the code, verify the token or fetch
userinfo, recognise the subject or provision under the declared policy, and issue
the session only after provisioning succeeds.

Why it is unique: everywhere else, first-login provisioning is a runtime setting
you can leave on its permissive default. In Corvid it is a decision the program
must state to compile at all.

### Complete Approval Policies Are Source, Not Middleware {#complete-approval-policies}

```corvid
server payments:
    @approval(role: "finance_reviewer", risk: "financial_transfer", data: "financial", expires_ms: 600000, max_cost_usd: $2500.0, irreversible: true)
    route POST "/payments" body Payment -> json Receipt requires authenticated:
        return submit_payment(body)
```

When a served route can reach `approve`, Corvid requires the whole operational
decision contract in source: reviewer role, risk, data class, expiry, cost
ceiling, and reversibility. All six values lower through the Application
Contract into the durable queue record. The server cannot substitute defaults.
The compiler also proves that the reviewer role exists and that the identity
model grants `approvals.decide`; a policy on a route with no reachable approval
is rejected as dead configuration.

At the decision boundary the runtime resolves roles fresh and requires the
reviewer to hold both the exact source-declared role and the permission, in the
same tenant, with valid CSRF, while enforcing separation of duties. An actor
with `approvals.decide` through an unrelated role cannot be treated as the
required reviewer.

Why it is unique: frameworks usually split the approval call, reviewer
middleware, queue schema, TTL, audit record, and UI metadata across unrelated
files. Corvid compiles one policy into every layer and refuses any incomplete
path.

### Protocol-Typed Connectors {#protocol-typed-connectors}

```corvid
connector github:
    base_url: "https://api.github.com"
    auth: bearer(secret("GITHUB_TOKEN"))
    retry: 3
    rate_limit: 60 per 60s
    circuit_breaker: 5
    modes: [mock, replay, real]
    operation get_repo(owner: String, repo: String) -> Result<Repo, GithubError> uses http_read:
        GET "/repos/{owner}/{repo}"
        on status 404 -> NotFound
        mock: Ok(Repo("corvid"))
```

An external API is declared in source as a `connector`, and each `operation` is a
callable tool with a declarative HTTP body. Four things the language forces or
composes make this different from a client library:

1. **A credential is a `secret(...)` reference, never a literal** — a bare-string
   credential is a parse error. It resolves at dispatch into a request header and
   never enters the IR, a trace, or an error message.
2. **The execution mode is chosen at the boundary, with no default.** A connector
   declares the `modes` it may run in; the deployment selects one with
   `corvid run --mode`. Omitting `modes` is a compile error, and selecting a mode
   the connector doesn't allow (or one the runtime can't execute) refuses at
   startup — a program can never reach a real provider by silence.
3. **The same unchanged file runs three ways.** `mock` evaluates the compiled
   `mock:` payload; `real` makes the HTTP request; `replay` serves a recorded
   interaction and never falls through to a real call (the credential is absent
   from the recording by construction).
4. **An operation IS a tool, so the moat composes.** A `dangerous` operation still
   needs a prior `approve`; budgets, taint, and provenance still apply; and
   `on status <code> -> Variant` turns an HTTP status into a typed `Result` error
   the compiler makes you handle.

Why it is unique: other ecosystems express base URLs, auth, retries, mocks, and
recorded fixtures as scattered runtime glue and test config. Corvid makes the
integration — and the decision of whether it touches the real world — part of the
program's static contract.

### Verified Provider Protocols {#verified-provider-protocols}

Some provider calls do not finish when the response arrives. You submit, and the
real work happens minutes or hours later. Everywhere else that means a hand-rolled
poll loop, and the poll loop is where the bugs live: the timeout nobody tuned, the
retry that submits a second job, the restart that loses the work, the response
field read before anyone checked it was there.

An `async:` block declares the temporal contract as part of the operation:

```corvid
operation submit_shipment(order: String) -> Job dangerous uses http_write:
    POST "/shipments" body order
    async:
        statuses: [queued, processing, completed, failed]
        initial: queued
        terminal: [completed, failed]
        deadline: 600s
        deadline_target: failed
        idempotency: intent
        poll GET "/shipments/{id}"
        every: 30s
        cancel POST "/shipments/{id}/cancel"
        on_protocol_change: refuse
        state queued:
            on queued -> queued
            on processing -> processing
            on completed -> completed
            on failed -> failed
```

**What the compiler proves.** Every status declared exactly once; transition
tables total over the status universe; every state reaching a terminal; non-zero
deadline and cadence; a non-mutating poll; and a mutating submit passing the
`dangerous` approval boundary. The worst-case observation count
(`deadline / interval`) multiplies the operation's cost, so a protocol cannot
poll its way past a `@budget` — polling is a compile-time bound, not a runtime
surprise.

**What the runtime guarantees.** The intent is checkpointed *before* the submit
request leaves the process, so a crash between "intent recorded" and "submit
acknowledged" cannot lose the work. The provider job id is bound only from the
JSON-decoded response — never guessed, never assumed from the request. Every
observed transition is checkpointed, so a restart resumes at the last observation
instead of re-submitting. The call returns only on a declared terminal state: the
submit response is never mistaken for completion.

**What it does NOT yet guarantee (open work, tracked as 52h-6).** Three claims
that appear elsewhere in this document's history are narrower than they read, and
are corrected here rather than left standing:

1. **Not exactly-once against the provider.** The intent key is a durable
   checkpoint label; it is never transmitted as an idempotency header, body field,
   or path value. A crash in the window between the provider accepting a submit
   and Corvid checkpointing that fact resumes into a *second* submit, and the
   provider has no way to recognise it. Exactly-once needs a provider-visible
   idempotency transport plus a logical invocation identity — the current key is
   `hash(connector, operation, args)`, so two *intentional* identical calls in one
   job also collapse into one intent.
2. **The deadline is not preserved across restarts.** It is measured from process
   start, not intent creation, so a repeatedly restarted protocol receives a fresh
   full window each time. This also weakens the budget bound, which assumes one
   deadline window.
3. **"Typed response" means JSON-decoded, not schema-validated.** Binding happens
   on raw JSON; type decoding occurs after the lifecycle returns. A malformed
   submit response can create a provider job and fail later. Validation at the
   boundary is 52i.

**What happens when things go wrong.** A provider's `Retry-After` can slow the
declared cadence but never speed it past what the source declared. Transient poll
failures are tolerated — the submitted job is still out there — while
`circuit_breaker: N` consecutive failures give up on *observing* without giving up
on the intent. Cancelling is honest about what it can do: exact before submit,
compensated through the declared `cancel` endpoint after it, and explicitly
DETACHED when no endpoint is declared, saying plainly that the provider job is
still running.

**What happens when you edit it.** Changing a protocol with intents in flight is a
consequential decision, so `on_protocol_change: refuse | resume` is required and
omitting it is a compile error. You never bump a version number: Corvid
fingerprints the canonical protocol graph, so re-ordering, reformatting, or
changing the policy itself are not drift, and a real edit is. When a change is
detected the diagnostic says *what* changed (`deadline=600 -> 900`,
`state complete: removed`), not merely that something did.

**What you can learn before deploying.** `corvid connectors simulate` explores what
a provider could do to you — the shortest behaviour reaching each terminal, the
behaviours that never terminate on their own, and the worst-case observation count
the budget is charged for. It drives the same transition engine the runtime uses,
so what it predicts is what runs.

Why it is unique: other ecosystems give you a job queue, or a retry library, or a
state-machine DSL, and leave the correspondence between them to code review.
Corvid makes the provider's *timeline* a checked part of the program — durable
submission, budget-bounded polling, honest cancellation, and replayable history
falling out of one declaration.

## Proof Matrix

| Invention | Status | Runnable command | Test coverage | Spec | Explicit non-scope |
|---|---|---|---|---|---|
| Approve Before Dangerous | Shipped | `corvid tour --topic approve-gates` | `crates/corvid-types/src/lib.rs` | [`03-typing-rules.md`](./effects-spec/03-typing-rules.md) | Proves the approval boundary, not approval quality. |
| Dimensional Effects | Shipped | `corvid tour --topic dimensional-effects` | `crates/corvid-types/src/effects.rs` | [`02-composition-algebra.md`](./effects-spec/02-composition-algebra.md) | Proves declared contracts, not provider honesty. |
| Grounded<T> | Shipped | `corvid tour --topic grounded-values` | `crates/corvid-types/src/effects/grounded.rs` | [`05-grounding.md`](./effects-spec/05-grounding.md) | Proves source linkage, not source truth. |
| Strict Citations | Shipped | `corvid tour --topic strict-citations` | `crates/corvid-vm/src/tests/dispatch.rs` | [`05-grounding.md`](./effects-spec/05-grounding.md) | Checks citation evidence, not factual correctness. |
| Provenance Propagation + `@grounded_pure` | Shipped | `corvid tour --topic provenance-propagation` | `crates/corvid-types/src/tests.rs` (`grounded_pure_*`, `grounded_coercion_*`) + `tests/corpus/combined_all.cor` + `tests/corpus/legacy_grounded_coercion.cor` | [`grounded-propagation-design.md`](../meta/grounded-propagation-design.md) | `@grounded_pure` forbids laundering inside a body; trust in the upstream retrieval source is the operator's responsibility. |
| Compile-Time Budgets | Shipped | `corvid tour --topic cost-budgets` | `crates/corvid-types/src/effects/cost.rs` | [`07-cost-budgets.md`](./effects-spec/07-cost-budgets.md) | Static declared costs, not invoice reconciliation. |
| Confidence Gates | Shipped | `corvid tour --topic confidence-gates` | `crates/corvid-types/src/tests.rs` | [`06-confidence-gates.md`](./effects-spec/06-confidence-gates.md) | Depends on calibrated adapter confidence. |
| AI-Native Keywords | Shipped | `corvid tour --topic language-keywords` | `crates/corvid-syntax/src/parser/tests.rs` | [`01-dimensional-syntax.md`](./effects-spec/01-dimensional-syntax.md) | Does not replace ordinary general-purpose code. |
| Trace-Aware Evals | Shipped | `corvid tour --topic eval-traces` | `crates/corvid-types/src/lib.rs` | [`12-verification.md`](./effects-spec/12-verification.md) | Full eval runner is later workflow tooling. |
| Replay And Receipts | Shipped | `corvid tour --topic replay-receipts` | `crates/corvid-cli/tests/bundle_verify.rs` | [`14-replay.md`](./effects-spec/14-replay.md) | Receipts are observed evidence, not full formal verification. |
| Typed Model Routing | Shipped | `corvid tour --topic model-routing` | `crates/corvid-vm/src/tests/dispatch.rs` | [`13-model-substrate-shipped.md`](./effects-spec/13-model-substrate-shipped.md) | Does not benchmark model quality automatically. |
| Progressive Refinement | Shipped | `corvid tour --topic progressive-routing` | `crates/corvid-vm/src/tests/dispatch.rs` | [`13-model-substrate-shipped.md`](./effects-spec/13-model-substrate-shipped.md#135-progressive-refinement-slice-e) | Thresholds need calibrated confidence. |
| Ensemble Voting | Shipped | `corvid tour --topic ensemble-voting` | `crates/corvid-vm/src/tests/dispatch.rs` | [`13-model-substrate-shipped.md`](./effects-spec/13-model-substrate-shipped.md#137-ensemble-voting-slice-f) | Custom vote functions need future function values. |
| Jurisdiction And Privacy Routing | Shipped | `corvid tour --topic privacy-routing` | `crates/corvid-types/src/effects.rs` | [`13-model-substrate-shipped.md`](./effects-spec/13-model-substrate-shipped.md#134-regulatory--compliance--privacy-dimensions-slice-d) | Legal compliance still needs operations and audits. |
| Streaming Effects | Shipped | `corvid tour --topic streaming-effects` | `crates/corvid-vm/src/tests/stream.rs` | [`08-streaming.md`](./effects-spec/08-streaming.md) | Provider-native continuation depends on providers. |
| Progressive Structured Streams | Shipped | `corvid tour --topic partial-streams` | `crates/corvid-types/src/tests.rs` | [`08-streaming.md`](./effects-spec/08-streaming.md) | Full native parity remains backend work. |
| Typed Stream Resumption | Shipped | `corvid tour --topic stream-resume` | `crates/corvid-vm/src/tests/stream.rs` | [`08-streaming.md`](./effects-spec/08-streaming.md) | Provider-native session continuation is future adapter work. |
| Declarative Fan-Out / Fan-In | Shipped | `corvid tour --topic stream-fanout` | `crates/corvid-types/src/tests.rs` | [`08-streaming.md`](./effects-spec/08-streaming.md) | Lambda extractors wait for function values. |
| Proof-Carrying Dimension Registry | Shipped | `corvid tour --topic effect-registry` | `crates/corvid-driver/src/dimension_registry.rs` | [`dimension-artifacts.md`](./effects-spec/dimension-artifacts.md) | Distributes declarations, not executable code. |
| Adversarial Bypass Testing | Shipped | `corvid tour --topic adversarial-tests` | `crates/corvid-driver/src/adversarial.rs` | [`adversarial-taxonomy.md`](./effects-spec/adversarial-taxonomy.md) | Live LLM generation expands but does not replace deterministic gates. |
| Executing File-I/O Surface | Shipped (33S1) | `corvid tour --topic file-io` | `crates/corvid-runtime/tests/executing_io_tools.rs` + `crates/corvid-runtime/src/io.rs::tests` + `crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_blocks_executing_io_*` | [`stdlib/io.md`](./stdlib/io.md) | Confines paths to the configured `[io] root`; does not police what user code does with the contents. |
| Executing HTTP-Client Surface | Shipped (33S2) | `corvid tour --topic http-client` | `crates/corvid-driver/tests/executing_http_through_driver.rs` + `crates/corvid-runtime/src/http.rs::tests` + `crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_blocks_executing_http_*` | [`stdlib/http.md`](./stdlib/http.md) | Enforces always-on SSRF block + required `[http] allow` allowlist + replay quarantine; does not police response-body content or rewrite request headers. |
| Executing SQLite Surface | Shipped (33S3) | `corvid tour --topic sqlite` | `crates/corvid-driver/tests/executing_sqlite_through_driver.rs` + `crates/corvid-runtime/src/db.rs::tests` + `crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_blocks_executing_db_*` | [`stdlib/db.md`](./stdlib/db.md) | Structural parameter-binding-only (no SQL interpolation path exists; typechecker's `List<DbParam>` + `params_from_iter` together prevent injection) + `[io] root` path confinement reuse + write quarantine on replay + opaque/refcounted `DbHandle` primitive type (user code cannot forge a handle). SQLite only; Postgres path remains envelope-only. |
| Executing JSON Surface | Shipped (33R5b) | `corvid tour --topic json` | `crates/corvid-driver/tests/executing_json_through_driver.rs` + `crates/corvid-runtime/src/json.rs::tests` + `crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_does_not_block_executing_json_*` | [`stdlib/json.md`](./stdlib/json.md) | Ships BOTH the opaque-handle shape (for dynamic JSON) AND the typed-decoder convention (for typed APIs; declare a struct + `decode_X_from_json` tool, runtime decodes generically via serde + `json_to_value`). Two RuntimeChecked guarantees: parse-safety (malformed input returns `Result::Err`, never panics) + field-type-safety (typed-accessor mismatches return `Result::Err`, never coerce). The C-ABI `corvid_json_*` exports exist in `ffi_bridge::json_exports`; cdylib bridging is interpreter-only for v1.0 (follow-up slice). |
| Application Contract → OpenAPI + AI metadata | Shipped (51a-51c) | `corvid tour --topic application-surface` | `crates/corvid-abi/src/app_contract.rs` + `openapi.rs` + `corvid_ai.rs` tests | [`inventions.md`](#the-application-surface) | Describes the public surface (routes, agents, types, capabilities); does not execute it. The AI-native metadata is Corvid-specific and rides alongside a clean standard OpenAPI document. |
| Typed Errors Across The Boundary | Shipped (51e) | `corvid contract openapi` on a `Result<T, E>` route | `crates/corvid-abi/src/app_contract.rs` (`error_enum_variants_*`) + `openapi.rs` (`result_route_projects_per_status_*`) | [`inventions.md`](#typed-errors-that-reach-the-frontend) | `@status`/`@ui` per variant + per-status OpenAPI responses + a TS discriminated union; the frontend's exhaustiveness is the client's, enabled by the contract. |
| Uploads + Cursor Pagination | Shipped (51f) | `corvid contract openapi` | `crates/corvid-abi/src/app_contract.rs` + `openapi.rs` (upload + page tests) | [`inventions.md`](#the-application-surface) | HTTP-boundary types (`Upload<Format>`, `Page<Item>`); native codegen refuses to lower them (serve/interpreter tier). |
| Identity, Safe By Construction | Shipped (51g-51i) | `corvid check` on an `identity` block | `crates/corvid-abi/src/app_contract.rs` (identity + auth-route + linking tests) + `corvid-types` `check_identity` | [`inventions.md`](#identity-safe-by-construction) | Insecure session config is a compile error absent a loud opt-out; account-linking never silently merges by email. Route mounting in `corvid serve` is a serve-integration follow-up; the storage-layer OAuth crypto ships in `corvid-runtime/src/auth`. |
| Connector Token ≠ Login Session | Shipped (51j) | `corvid contract list --kind connector` | `crates/corvid-connector-runtime/src/auth.rs` (`login_session_credential_*`, `per_user_*`) | [`core-semantics.md`](./core-semantics.md) (`connector.per_user_token_separate_from_session`) | Runtime refuses a login-session credential at the connector boundary; per-user connectors require the end-user actor. |
| Auth Safe-Defaults Are Unbypassable | Shipped (51k) | run `crates/corvid-runtime/src/jwt_verify/mock_idp.rs` tests | `mock_idp.rs` (`every_mutated_token_is_refused`, `byte_fuzz_never_panics_and_never_forges`) | [`core-semantics.md`](./core-semantics.md) (`auth.jwt_tamper_and_fuzz_resistant`) | A mock IdP whose only verifiable token is the correct one; six source-bypass mutators + a 2000-input byte-fuzz. |
| Typed SDKs In Four Languages | Shipped (51l, 51o) | `corvid generate sdk --language ts\|swift\|kotlin\|python` | `crates/corvid-abi/src/ts_client.rs` + `sdk_gen.rs` tests | [`inventions.md`](#the-application-surface) | One contract → typed models everywhere; the TS target is a full client (shipped `@corvid/client`), the others are typed models + a transport scaffold. |
| Universal Dev Console | Shipped (51m) | `corvid dev` | `crates/corvid-abi/src/dev_console.rs` tests | [`inventions.md`](#the-application-surface) | One self-contained, contract-driven console for every app; execution targets a running `corvid serve`. |
| React Hooks + Frontend Scaffold | Shipped (51n, 51p, 51q) | `corvid generate frontend --framework react` | `sdk/typescript/react` (tsc-checked) + `crates/corvid-abi/src/frontend_gen.rs` tests | [`inventions.md`](#the-application-surface) | Generic hooks specialized by the generated types + prototype components + a runnable starter; scaffolds you own, not product UI. |
| Route Execution (path/query/body) | Shipped (52a) | `corvid serve examples/reference_app/src/main.cor` | `crates/corvid-cli/src/serve_cmd.rs` tests + `crates/corvid-cli/tests/serve_smoke.rs` | [`inventions.md`](#the-complete-application-runtime) | Every declared route shape runs its handler body through the interpreter (path params, query structs, typed JSON bodies); malformed boundary input is a structured 400. Native-tier parity is later work. |
| Contract Closure (refuse-to-start) | Shipped (52b) | `corvid serve` on a route the tier can't serve | `crates/corvid-driver/src/contract_closure.rs` tests + `crates/corvid-cli/tests/serve_smoke.rs::serve_enforces_a_requires_authenticated_route_instead_of_refusing_to_start` | [`core-semantics.md`](./core-semantics.md) (`contract.runtime_closure`) | The backend refuses to start (E5204) when it advertises a route it cannot execute; grew in lockstep with the runtime (each Phase 52 slice flipped one capability), and the interpreter tier is now complete. The mechanism still guards future capabilities + the native tier. |
| Streaming Route Responses (SSE) | Shipped (52c-1) | `corvid serve` a `Stream<T>` route, then `curl -N` it | `crates/corvid-cli/tests/serve_smoke.rs::serve_streams_a_stream_route_as_server_sent_events` | [`inventions.md`](#the-complete-application-runtime) | A `Stream<T>` route response flushes each yielded value as an SSE `data:` event with an `event: done` terminator; the transport falls straight out of the language's `Stream` type. Provider-native session continuation remains adapter work. |
| File Uploads (multipart) | Shipped (52c-2) | `corvid serve` an `Upload<Format>` route, then `curl -F` it | `crates/corvid-cli/tests/serve_smoke.rs::serve_parses_a_multipart_upload_and_enforces_mime` | [`inventions.md`](#the-complete-application-runtime) | An `Upload<Format>` body is parsed from multipart with accepted-MIME + max-size enforcement (400 on violation), read via `body.text()`/`bytes()`/`filename()`/`content_type()`/`size()` methods. Interpreter tier buffers the whole body; streaming large uploads is later work. |
| Cursor Pagination (Page envelope) | Shipped (52c-2) | `corvid serve` a `Page<Item>` route, then GET it | `crates/corvid-cli/tests/serve_smoke.rs::serve_returns_a_page_cursor_envelope` | [`inventions.md`](#the-complete-application-runtime) | `Page(items, next_cursor)` builds the `{items, next_cursor, has_more}` envelope (`has_more` derived, cursor unwrapped from `Option`); the incoming cursor is read from the route's typed query struct. |
| Reversibility-Guarded Parallel Cancellation | Shipped (52d) | `corvid tour --topic parallel-cancellation` | `crates/corvid-vm/src/tests/parallel.rs` (rule + replay-reproduction + adversarial) | [`core-semantics.md`](./core-semantics.md) (`parallel.cancellation_reversibility`) | A `parallel:` block fails fast, but a branch past a non-reversible effect boundary is never cancelled; live cancellation is recorded per arm and Substitute-mode replay reproduces it deterministically (cancelled arm stops at its recorded boundary, shielded arm reaches its terminal, non-cancelling blocks byte-identical). Cooperative at tool-dispatch boundaries, not preemptive. |
| First-Login Provisioning Is A Compile-Time Decision | Shipped (52e) | `corvid tour --topic oauth-login` | `crates/corvid-cli/src/serve_auth/routes.rs` (`callback_tests`: open provisions+recognises, invited gate, reused-state / nonce-mismatch / tampered-token refused, userinfo) + `crates/corvid-cli/tests/serve_smoke.rs::serve_mounts_the_oauth_login_surface_and_redirects_to_the_provider` + `crates/corvid-abi/src/app_contract.rs::identity_with_oauth_provider_but_no_provisioning_is_rejected` | [`inventions.md`](#first-login-is-an-explicit-compile-time-decision) | An `identity` block auto-mounts the login surface (PKCE + single-use state + nonce + JWKS/userinfo verify + safe cookie); omitting the first-login `provisioning:` policy is a compile error (E5210). Identity is keyed server-side on `(issuer, subject)` / `(provider, user_id)`, never email. Durable `approval_required` provisioning is a later slice. |
| Verified Provider Protocols | Shipped (52h) | `corvid tour --topic verified-provider-protocols` / `corvid connectors simulate <file>` | `crates/corvid-driver/tests/protocol_lifecycle.rs` (submit-once + id-bound-from-typed-response + terminal-only return; resume never re-submits; `Retry-After` slows the declared cadence; breaker tolerates then trips; cancel compensates through the declared endpoint; recorded lifecycle replays with the provider gone; a gapped recording refuses at the gap; a changed protocol refuses or resumes with ZERO re-submits) + `crates/corvid-runtime/src/protocol.rs` (transition engine, binding conventions, resume verdicts, fingerprint stability) + `crates/corvid-runtime/src/protocol_simulate.rs` + `crates/corvid-cli/tests/connectors_simulate.rs` + `crates/corvid-abi/src/ts_client.rs` (typed state union) | [`inventions.md`](#verified-provider-protocols) | A declared `async:` block becomes a durable, budget-bounded, replayable state machine: intent checkpointed before submit, job id bound only from the decoded response, terminal-only return, governed cadence, honest cancellation (compensate/detach), lifecycle replay with strict no-real-fallback, and required `on_protocol_change` with derived graph fingerprints. NOT yet exactly-once against the provider (intent key not transmitted; crash-after-accept can duplicate) and the deadline is not preserved across restarts — both tracked as 52h-6. Live payload-conformance and drift quarantine are 52i; the end-to-end kill→drift→quarantine→repair demonstration is 52h-5b. |
| Protocol-Typed Connectors | Shipped (52g) | `corvid tour --topic connectors` | `crates/corvid-driver/tests/connector_modes.rs` (mock/real/replay, status→typed-error, rate-limit, secret-never-in-trace, real-without-credential) + `crates/corvid-runtime/src/connectors.rs` (request-builder: secret-only-in-header, unresolved-secret-named) + `crates/corvid-types/src/tests.rs` (`on_status_*` coherence) | [`inventions.md`](#protocol-typed-connectors) | Declared connectors run mock/real/replay from one file; credentials are `secret(...)` refs never in IR/trace, mode has no default (omission = compile error / startup refusal), `on status -> Variant` gives typed `Result` errors. Async provider state machines + provider-drift quarantine are later 52h/52i slices. |
| Route Authorization Enforced Before The Handler | Shipped (52f) | `corvid tour --topic route-authorization` | `crates/corvid-runtime/tests/route_authz.rs` (forged cookie, expired/revoked session, cross-tenant, CSRF mismatch, authenticated-but-insufficient, permission-union, stale-role-after-revocation) + `crates/corvid-cli/tests/serve_smoke.rs::a_role_gated_route_allows_the_right_role_and_denies_others` (live: admin 200, plain 403, anonymous 401) + `::serve_enforces_a_requires_authenticated_route_instead_of_refusing_to_start` + `crates/corvid-runtime/src/auth/roles.rs` tests | [`core-semantics.md`](./core-semantics.md) (`contract.runtime_closure`) | A `requires authenticated\|role\|permission` route resolves the session to a verified typed `actor` and enforces tenant + role + permission (set membership + permission union) and CSRF double-submit on mutations BEFORE the handler or any effect runs — 401 unauthenticated, 403 under-privileged. The actor is only ever the authenticated one, never request-supplied. This closed the last Contract Closure gap. |
| Complete Source-Declared Approval Policy | Shipped (52f-4d) | `corvid check` an approval-capable served route | `corvid-syntax::server_approval_route_*` + `corvid-types::approval_route_*` + `corvid-abi::approval_route_surfaces_its_complete_runtime_policy` + `serve_smoke::approval_decisions_reject_every_unauthorized_path` | [`core-semantics.md`](./core-semantics.md) (`approval.policy_clause_static_check`) | All six policy fields are mandatory and compile into the queue. Runtime proves exact role + permission + tenant + CSRF + separation of duties; Corvid proves policy completeness and enforcement, not whether a human's decision is wise. |
