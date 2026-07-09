# Language gap audit — 2026-07-09

Four-dimension survey of Corvid's gaps as of commit `73ea143` (33S4 closed):
language surface, type system, stdlib coverage + backend parity, and
AI-native feature coverage. Every finding is evidence-backed with
file:line references gathered from the source, not from docs.

**Status: uncommitted working document.** Findings feed a ROADMAP
amendment discussion; none of this is scheduled work until a pre-phase
chat locks scope.

---

## Headline verdict

Corvid is two languages stapled together at very different maturity
levels.

The **governance layer** — typed effect system, compile-time `approve`
gates, runtime approval queues with TTL tokens, deterministic replay
with quarantine, per-call cost accounting, trace-aware evals with flake
detection, signed cdylib attestation, and the executing io/http/sqlite/
json surfaces with structural guarantees — is real, deep, and
test-backed. Nothing mainstream has this. It is the moat and it holds.

The **expression layer** — the part where a programmer manipulates data
and drives LLM interactions — is far behind the project's own
documentation. `docs/book/` chapters 04, 05, 11, and 13 describe
roughly a full modern language (match, patterns, closures, maps,
methods, `fn` with generics, struct literals, sum types, `while`,
`let`, multi-message prompts). The implemented surface is a much
smaller statement language: declarations + `if`/`for`/assign + a fixed
expression grammar with no user-extensible data manipulation.

The strategic risk: the moat governs a language currently too weak to
write the programs the moat protects. A skeptical reviewer's first
probe — "show me a chat loop with a system prompt, fan out three model
calls, pick the best" — fails at every step today.

---

## Tier 1 — Blockers (nine)

### Core language (six)

**B1. No `match` expression / no pattern matching at all.**
No `KwMatch` token (`crates/corvid-syntax/src/token.rs:23-130`), no
parser arm (`parser/expr.rs:258-364`), no AST node, no IR node.
`docs/book/13-pattern-matching.md` is an entire chapter describing it
as shipped ("match is exhaustive — the compiler refuses to emit…"),
including `s @ Approved(_)` bindings and guards. None of it parses.
`docs/reference/grammar.md:273-284` declares the productions as if
implemented. Confirmed in `dev-log.md` (2026-06-10).

**B2. No user sum types / enums.**
Explicitly deferred: "v0.1 supports struct-like types only. Enum/union
types arrive in v0.2" (`crates/corvid-ast/src/decl.rs:324`).
`parse_field` accepts only `name: type` (`parser/decl/type_field.rs:56-68`)
— no `| Variant` branch despite `grammar.md:68` and book 05:127-137
documenting it. The only sum types are builtin `Option`/`Result`. No
user-modelled state machines are possible, compounding B1.

**B3. No `Map`/`Dict` type.**
Not in the builtin list (`crates/corvid-resolve/src/scope.rs:28-87`),
no `Type` variant (`corvid-types/src/types.rs:20-145`), no `{...}`
literal, indexing typechecks only `List` with `Int`
(`checker/expr.rs:41-73`). Book 04:31 + 05:69-76 document
`Map<String, Int>` literals and `.get`/`.keys`. Post-33R5b, the only
string→value map is the opaque write-only `JsonBuilder`.

**B4. No number↔string conversion, no general string interpolation.**
`"count: " + n` is a type error (`checker/ops.rs:73-88`; runtime
`interp/expr.rs:114-131`). No `to_string`/`str()`/`to_float`/
`to_int_truncated` anywhere (workspace grep: zero hits) despite book
05:50-51 documenting them. An Int cannot become a String in pure
Corvid; the only serialization escape hatch is
`json_object_set_int` + `json_object_finish`. `{param}` interpolation
exists only inside prompt templates (`interp/prompt/mod.rs:324-334`).

**B5. No string/list method library — cannot get a length.**
The checker hard-rejects methods on builtins: "methods currently work
only on user-declared struct types" (`checker/call.rs:704`). No
`length`, `upper`, `lower`, `split`, `trim`, `contains`, `replace`,
`substring`; no `map`/`filter`/`append`/`sort`/`contains` on List.
Book 05:36-66 documents most of these. Out-of-range list indexing is
discoverable only at runtime because there is no bounds-check
primitive. (ROADMAP 33R5c/33R5d cover part of this; they are open.)

**B6. Option/Result can only be propagated, never inspected.**
The sole consumption path for `Some/Ok/Err/None` values is postfix `?`
(`checker/ops.rs:183-244`; `interp.rs:650-668`), which requires the
enclosing function to itself return Option/Result. No `unwrap_or`,
`is_some`, `is_ok`, no if-let, no default operator — and no match
(B1). A `None` can never be converted to a default at the point of
use. Consequence visible in the stdlib itself: every std surface
except json routes AROUND Result with success-shaped envelope structs
(`std/db.cor:204-207`, `std/http.cor:91-92`, `std/io.cor:77-79`).

### AI-native (three)

**B7. No multi-turn conversation, no system prompts.**
Prompts are single-shot templates — one user-role string.
`docs/book/11-prompts.md:46-55` documents multi-message `system:`/
`user:` prompt blocks; the prompt parser
(`corvid-syntax/src/parser/prompt.rs`) has no role blocks (zero
matches for `PromptMessage` across corvid-ast/corvid-syntax).
`std/ai.cor`'s `AiMessage`/`AiSession` are envelope types nothing
consumes. No context-window management exists (only an
`estimate_tokens` heuristic, `llm_dispatch.rs:1072`). Multi-turn is
the default AI workload.

**B8. No parallel fan-out.**
No `spawn`/`parallel`/`async`/`await` in the language (zero matches in
corvid-syntax). The effect spec defers parallel composition
(`docs/internals/effect-spec/10-interactions.md:84-96`, "Tracked in
Phase 22" — not shipped). Only ensemble voting runs concurrently
internally (`interp/prompt/voting.rs:103`). The runtime is tokio;
the language is strictly sequential.

**B9. No MCP support.**
Explicitly deferred to v0.4 (`FEATURES.md:161`;
`corvid-ast/src/decl.rs:256`). External tools are Rust `#[tool]` FFI,
Python `tools.py`, or connectors. In 2026 an AI-native language that
cannot consume MCP tool servers forfeits the external tool ecosystem.

---

## Tier 2 — Majors

**M1. No user generics.** `Type` enum has no type-variable variant;
`List/Option/Result/…` are compiler-special-cased heads
(`checker/types.rs:264-309` string-matches names; unknown heads →
`Type::Unknown`). No declaration takes type parameters. Book 05:139-146
documents `fn first<T>(xs: List<T>)`.

**M2. No lambdas/closures.** No form in parser/AST/IR. Book 05:61-62
shows `xs.map(fn (x) -> x * 2)`. Function-type annotations
`(Int) -> Int` silently resolve to `Unknown` — unchecked
(`checker/types.rs:47`).

**M3. No `while` loop, no `range`.** Book 04:102-103 lists `while`;
no `KwWhile` exists. Loops are `for x in <List|String|Stream>` only.
Counted iteration requires a pre-existing list or recursion.

**M4. No `let`, no annotated locals.** `let` deliberately demoted to a
plain identifier (`corvid-syntax/src/lib.rs:120-133` test). Assignment
parses only as bare `IDENT = expr` with no annotation
(`parser/stmt.rs:211-233`). Nearly every book example uses `let`;
`grammar.md:227` shows the let-production.

**M5. No field/index assignment, no compound assignment.**
`x.field = v`, `xs[i] = v`, `+=` are all parse errors (same site as M4).

**M6. Streaming is a singleton-chunk fake.** All four provider
adapters defer streaming (`llm/anthropic.rs:13`, `openai.rs:11`,
`gemini.rs:16`, `ollama.rs:89-91` sends `"stream": false`). A prompt
declared `-> Stream<T>` makes ONE blocking call and wraps the complete
response in a one-element stream (`interp/prompt/mod.rs:35-86`). The
stream effect algebra (budget/confidence mid-stream termination,
Partial<T>, ResumeToken) is an invention with nothing real flowing
through it. README markets "Streaming" as a headline.

**M7. No sampling parameters.** `LlmRequest` (`llm/mod.rs:33-51`)
carries no temperature/top_p/max_tokens. `temperature` appears only in
a doc comment. Prompt `max_tokens` truncates the local stream post-hoc.
No Corvid program can set temperature.

**M8. Embeddings/RAG executing in Rust but unreachable from Corvid.**
Real embedders + SQLite cosine index exist
(`rag/embedders.rs:9-68`, `rag/index.rs:191,258`) but `std/rag.cor` is
envelope-only — no `is_stdlib_rag_tool` dispatch exists, unlike
io/http/db/json.

**M9. Effects (and models) cannot be exported/imported via `use`.**
`collect_public_exports` (`corvid-resolve/src/modules.rs:225-253`)
covers Type/Store/Tool/Prompt/Agent only; effects fall through
`_ => continue`. Confirmed workaround (typed-decoder convention):
user code redeclares effects inline.

**M10. Backend parity — the batteries are interpreter-only.**
DbHandle/JsonValue/JsonBuilder: codegen-cl emits `not_supported`
(`corvid-codegen-cl/src/lowering/agent.rs:438-449`); codegen-py
degrades hints to `object` and emits `tool_call("db_open", …)` calls
for which `runtime/python` ships NO implementations (grep for
`io_read_text|http_get|db_open|json_parse` in runtime/python: zero
hits) — a transpiled program using the batteries fails at runtime.
Result/Option/`?`/`try` emit `NotImplementedError` markers in
codegen-py (`codegen.rs:373-392`). This reproduces the Phase 20l
"object-shaped degradation" gap shape. Mitigated by the tier-picker's
auto-fallback to interpreter (`native_ability.rs:27-63`), but any
claim that compiled tiers include batteries is false today.

**M11. `corvid new` hello-world requires Python.** The default
scaffold's `src/main.cor` calls `tool echo` implemented in scaffolded
`tools.py`, executed via embedded PyO3 (`corvid-driver/src/scaffold.rs:64-85`;
`corvid-runtime/src/python_tools.rs:1-40`). The first program a new
user runs contradicts the "no Python required" positioning.

**M12. std vendoring failure modes.** Vendor is a silent no-op when no
std source is found (`scaffold.rs:138-140`) — a dev-built `corvid new`
produces a project where `import "./std/io"` fails at first use with
no scaffold-time warning. Vendored std is a frozen snapshot with no
upgrade path beyond manual copy (`scaffold.rs:135-136`).

**M13. No datetime, no math.** No `now()`, no clock/duration/date
type; no sqrt/abs/min/max/pow/floor/round/random; no Float↔Int
conversion. (ROADMAP 33R5e/f cover this; open.)

**M14. Tool-body `@host` syntax taught in the book doesn't parse.**
Book 04:57-66 shows a tool with a body calling `@host.email.send(...)`;
tools are signature-only declarations (`parser/decl/mod.rs:130-161`).
The quickstart's own refund example (`02-quickstart.md`) uses this
unparseable form.

**M15. grammar.md's drift-gate claim is misleading (meta-gap).**
`grammar.md:3-10` claims a drift-gate cross-checks every production
against the parser. The actual gate
(`corvid-syntax/tests/grammar_drift.rs:1-65`) only checks internal
EBNF consistency and documents that it does NOT check parser
correspondence. This is the mechanism that let `match_expr`,
`map_literal`, `struct_literal`, `pattern`, variants, aliases, and
`let` sit unimplemented in an "authoritative" grammar with a green
gate.

**M16. `schedule` declarations are inert** (parse + typecheck, never
fire). Mitigated by warning W0280 (33Q14) but still a
declared-but-inert surface.

---

## Tier 3 — Minors

- Type aliases (`type CustomerId = String`) documented, unparseable.
- `Unit` (book) vs `Nothing` (impl) type-name mismatch — `-> Unit`
  fails resolution.
- No `elif` / `else if` — chained conditions require nesting.
- Unary `+` in grammar.md, not in parser.
- Doc comments `#:` claimed to render in help/hover; no DocComment
  token in the lexer.
- `break`/`continue`/`pass` encoded as sentinel `Ident` expressions —
  functionally working, structurally fragile.
- Int overflow: book says "saturates in release with an `--overflow`
  flag"; implementation always checks and traps.
- Recursive struct types: representation supports them; zero test
  coverage (risk zone is codegen struct layout).
- Struct literals are positional-only (`TypeName(v1, v2)`); book
  documents named-field literals + `..` update spread.
- Error-code drift: quickstart shows E0301; compiler emits E0101
  (already filed as 33R12).

---

## What genuinely works (calibration)

if/else, for-in over List/String/Stream with break/continue, checked
Int arithmetic + IEEE Float, String/List `+` concat, comparisons,
`?` propagation, `try … on error retry N times backoff …`, list
literals/indexing, positional struct construction with field
typechecking, `extend`-block methods on user structs, prompt `{param}`
templates, and the full declaration surface (agent/tool/prompt/effect/
model/type/store/server/schedule/test/eval/fixture/mock/import with
use-lifts). Plus the entire governance layer: effect dimensions,
approve (compile-time + runtime queues), replay + quarantine, real
cost accounting + OTel, trace-aware evals + flake detection,
structured output (schema-forced tool_use / json_schema + typed
decode + Marshal errors), signed attestation, executing
io/http/sqlite/json with structural guarantees, 36-topic
compiler-checked tour.

---

## Cross-cutting finding: the docs describe an unshipped language

At least 20 distinct features are documented as shipped but do not
exist: match/patterns (all of book ch 13), Map, sum types, `fn` +
generics, lambdas, `while`, `let` + annotated locals, type aliases,
named struct literals + `..` spread, string/list methods, numeric
conversions, multi-message prompts (`system:`), tool bodies with
`@host`, `Unit`, doc comments, overflow flags. Under the project's own
no-shortcuts rule ("if a shipped surface conflicts with the spec, fix
the surface rather than softening the spec"), this is the top-tier
finding: the book is aspirational and nothing enforces it — the
drift gate that claims to (M15) checks the wrong thing.

Two honest resolutions exist per feature: implement it, or move it to
an explicit "planned" section. Leaving chapter 13 as-is fails both.

---

## ROADMAP coverage check

Covered by existing open slices: strings (33R5c), collections (33R5d),
datetime (33R5e), math (33R5f), error-code drift (33R12), registry
(33R4), fmt (33R11), did-you-mean (33R9).

**Not covered anywhere in the ROADMAP as slices:** match / pattern
matching, user sum types, Map type, lambdas, generics, `let` +
annotated locals, field/index/compound assignment, conversation +
system prompts, parallel fan-out, MCP client, real provider streaming,
sampling parameters, RAG stdlib dispatch, effect export via `use`,
doc-honesty pass over book 04/05/11/13 + grammar.md, drift-gate
strengthening.

---

## Recommended sequencing (proposal, pending pre-phase chat)

1. **Doc-honesty pass first** (cheap, protects credibility, applies
   the no-shortcuts rule to ourselves): re-align book 04/05/11/13 and
   grammar.md with reality; move unshipped features to explicit
   "planned" sections; make the drift gate check parser
   correspondence for the productions it can.
2. **Core expression tier** (unblocks everything else): `match` +
   user sum types + `Map` + number↔string conversion + string/list
   builtin methods (subsumes 33R5c/d) + `let`/annotated locals +
   field/index assignment. This is one coherent language-completeness
   phase.
3. **AI-native tier**: system prompts + message history as first-class
   values, sampling params in LlmRequest, real provider streaming,
   a `parallel` construct, MCP client.
4. **Parity decision**: declare compiled backends explicitly
   post-v1.0 for the batteries surfaces (they already auto-fallback),
   or fund the codegen work; either way say it in one place.

Items 2 and 3 are new phases requiring ROADMAP amendment via
pre-phase chat before any code.
