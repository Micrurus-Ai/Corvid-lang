# Project guidance

Rules that apply to every piece of work in this repo. Machine-enforceable where possible; otherwise honored by convention.

## Design decision principle

**Every design decision is judged on making Corvid great, innovative, inventive, powerful, and easier than any other language available.** Set by the CTO 2026-07-09; ease bar strengthened 2026-07-10; goal restated 2026-11-23.

**The goal.** Corvid is a general-purpose language for building an entire application — UI, backend and AI — in one language that a first-time reader can follow. A small app takes little code; a team can build a large AI application at full scale from day one with the same grammar, types and tools. Anything developers build on the backend and AI side with Python or TypeScript and their libraries must be buildable in Corvid, through built-in capability or typed interop.

**The defining advantage.** A person and an AI agent use the application through the **same typed actions**. An action is defined once — its inputs, who may call it, its validation and its conflict behavior — and the UI, the backend and an authorized assistant all use that one definition. There is no separate agent API, and the author chooses which actions an assistant can discover. Changing an action's input makes the compiler point at every affected UI, backend and agent caller.

**The organizing principle: the application and the AI can always see each other.** The compiler understands every screen, action, type, permission, effect and cost, so an assistant receives typed application state instead of screenshots, a coding AI can ask the compiler what calls what and what a change breaks, and an outside AI can use a deployed app's declared actions. In the other direction, people see what an agent did — which action, why, at what cost, and whether it can be undone — and developers see one timeline across UI, backend, agent and model steps. What an AI may see or call is always declared in source; nothing is exposed to an AI by default. The [platform plan](docs/proposals/corvid-platform-plan.md) applies this principle to the roadmap.

Corvid is not a niche language for agent safety. Effects, provenance, approval, replay and budgets are what make it safe to hand those shared actions to an agent; they serve the goal rather than define it.

How to apply:

1. **For commodity features** (strings, lists, maps, bindings), "inventive" means picking the best known semantics and removing every papercut other languages kept — not exotic syntax. The ease bar dominates; coherence with the Python-flavored, effect-typed surface is what makes Corvid easiest. (Precedents: `x = 42` bindings with optional `x: Int = 42` annotation and no `let` keyword; `m[k] -> Option<V>` instead of KeyError/zero-values; `split("")` traps with a hint; Float→String always shows the decimal point.)
2. **The invention budget goes to what lets a developer build the whole application**: shared typed actions across UI, backend and agents first, then the guarantees that make agent access safe — effects, provenance, approval, replay, budgets. Judge every proposal by what it lets a developer build. Every feature must **compose** with those guarantees — e.g. cost analysis covers assignment-target index expressions so `@budget` stays sound; builtin methods are pure so replay stays deterministic; no method path may silently launder `Grounded<T>`.
3. **Safety choices that remove failure modes count as both "powerful" and "easier"** — checked arithmetic everywhere, compile-time gates over runtime surprises.
4. Record the rationale per slice in `ROADMAP.md` and `dev-log.md`. No per-decision escalation needed; decide against these criteria and write it down.

## Built for the hardest work on earth

**Every decision, every line of code and every piece of syntax must make Corvid the strongest and most powerful language there is: one that every kind of organization, from space and rocket companies to small teams, chooses to solve complicated problems and build powerful, complex applications.** Set by the CTO 2026-11-24.

How to apply:

1. **Hold every feature to the most demanding user.** Ask whether it would be trusted where failure is expensive: flight software, finance, medicine, infrastructure. That means exact semantics, checked arithmetic, deterministic replay, failures that are typed values or loud errors, and nothing that works only in a demo.
2. **It must work at full scale from day one.** The same grammar, types and tools serve a ten-line script and a large codebase owned by many teams. Check that a feature stays fast, verifiable and understandable with thousands of files, many authors and heavy concurrent load.
3. **Power comes from composition, not from size.** Prefer a few features that combine cleanly with effects, provenance, approval, replay and budgets over many special cases. Power and the [simplicity rule](#syntax-as-simple-as-it-can-be) are never traded: the strongest version of a feature is also the easiest to read.
4. **Prove it.** A claim of power, speed or safety ships with a test, benchmark or runnable example that shows it, as the invention shipping contract requires.
5. **No ceiling.** When a design would block a hard but real use (hard real-time limits, very large data, formal verification, embedded or offline targets), name the limit in the roadmap as planned work instead of designing it out.

## Use all three tiers in every step

**For every decision, line of code, task and step, analyze how it can take advantage of Corvid having the backend, the frontend and AI native in one language, and use that to make the work more efficient, more powerful and stronger.** Set by the CTO 2026-11-24. A feature designed as if Corvid were only a backend language, only a UI language or only an AI language throws away the advantage no other language has.

How to apply:

1. **Ask the three questions before designing.** What does the backend know here that the frontend and AI could use? What could the UI give the person for free (forms, validation messages, conflict handling, progress)? What could an assistant do with it, safely, through declared actions? Write the answers in the slice's proposal, even when the answer is "nothing yet, because…".
2. **One definition, every tier.** A type, validation, permission or error declared once must reach every tier from that one declaration, never a copy per tier (see "Single source of truth"). For example, a table's `where` refinement validates the frontend form, the backend write and the agent's tool call alike.
3. **Let the compiler see across tiers.** Prefer designs where the compiler can check a change in one tier against its users in the others: a renamed column, a changed action input or a new error variant points at every affected screen, route and agent call.
4. **Agents get typed state, not scraping.** When a feature produces data or actions, make it available to an authorized assistant as typed values and declared actions, with effects, approval, replay and budgets applied, and nothing exposed to an AI unless the source says so.
5. **Measure the gain.** When the cross-tier design is the point, show it: less code than the separate-stack equivalent, a mistake caught at compile time that a separate stack would ship, or fewer round trips.
6. **Coordinate across sessions.** When a backend step creates something the frontend or an assistant should use, record it for the frontend roadmap and tell the session that owns that tier, rather than waiting for it to be discovered.

## Syntax as simple as it can be

**Every piece of syntax is as simple, readable and understandable as it can possibly be.** Set by the CTO 2026-11-24. Before adding or changing syntax, ask: *if Steve Jobs were shipping this, would he ship it like this?* If the answer is no, simplify until it is yes.

How to apply:

1. **A first-time reader understands it without the docs.** Source reads like a plain description of what happens: `name: String renamed from title`, `retired legacy_note`, `Shelf.where(fn (s) -> s.owner == owner)`. Use plain words over symbols, abbreviations or jargon.
2. **Remove until it breaks.** Every keyword, punctuation mark and required part must earn its place. If the compiler can know something for certain, the source does not repeat it. A consequential policy is still written explicitly (see "No hidden defaults"): simple never means hiding a decision.
3. **One obvious way.** Do not add a second spelling for something the language can already say. A new form must replace or clearly beat the old one.
4. **The common case is the shortest.** What most programs write most often takes the least code; rarer needs may take more.
5. **Check it in context.** Write the new syntax in a realistic program next to existing code before settling it. If it looks foreign, noisy or needs a comment to explain, it is not done.
6. Record which simpler alternatives were considered and why the shipped form won, in the slice's proposal or `dev-log.md` entry.

## File responsibility discipline

**Every source file holds exactly one responsibility.** Line count is a *heuristic for where to look* — it is not the rule.

A file fails this discipline when any of these hold:

1. It mixes unrelated top-level concepts (parsing + lexing; checking + IR lowering; dispatch + recording).
2. It has 2+ public items representing unrelated domains.
3. It has 2+ internal sections that share no state.

A 3,000-line file that does one thing cleanly is fine. A 400-line grab bag is not.

**Three explicit carve-outs (these still count as one responsibility):**

1. **Inline `#[cfg(test)] mod tests`** — co-located unit tests for the file's own type/concept are part of that responsibility, not a second one. Extract the tests when they grow past ~300 lines OR when they cover sibling-module concerns rather than this file's own concept.
2. **A type and its inherent + canonical-derive trait impls** — `struct Foo`, `impl Foo`, `impl Clone for Foo`, `impl Drop for Foo`, `impl PartialEq for Foo` are one responsibility ("the Foo type"). Cross-cutting trait impls implemented for many types (e.g. a `Render` trait implemented for ten record types) get their own file per impl-cluster.
3. **A facade module** that exists to compose siblings is itself one responsibility — "the facade." Re-exports + a thin orchestrator struct + a small dispatcher all count as one concern, even though they're three syntactic items.

**When a file fails the rubric:**

- If you are already modifying the file for a feature, split it in the same branch but as a separate commit that precedes the feature commit.
- If the file is untouched by your current work, file it as a Phase 20i/j/k audit candidate and move on — do not refactor pre-emptively.

**When splitting:**

- One commit per file extraction. No batching.
- Validation gate between every commit: `cargo check --workspace` + targeted `cargo test -p <crate> --lib` + `cargo run -q -p corvid-cli -- verify --corpus tests/corpus` (must still exit `1` only on the two deliberate fixtures).
- Push before starting the next extraction.
- Wait for acknowledgement at slice boundaries in parallel work — no autonomous chaining.
- Zero semantic changes during a refactor commit. Move code, add `pub use` re-exports to preserve the public API, nothing else. Bugs spotted mid-refactor get a separate branch.
- Commit message: `refactor(<crate>): extract <responsibility> from <file>` — body names which rubric criterion failed and how the split resolves it.

## No shortcuts

The hardest rule. No shortcuts anywhere. If a shipped surface conflicts with the spec, fix the surface rather than softening the spec (see `memory/project_phase_20_closed.md` for the canonical example).

## No hidden defaults for consequential policy

**Behavior with a security, privacy, tenancy, cost, or access consequence is NEVER a silently hardcoded default.** When such a choice has more than one reasonable policy, the developer's SOURCE must declare it explicitly, and OMITTING it is a compile error (or refuse-to-start) that names the missing decision and its allowed values — never a default the author didn't choose. The source is the forcing function; a program cannot back into a consequential posture by silence.

Set by the CTO 2026-07-23 (OAuth first-login provisioning). The canonical case: an `identity` block that declares OAuth providers must also declare `provisioning: first_login: open | invited`; omitting it is `E52xx First-login policy required` — so an enterprise app can never *silently* become open-registration. Same shape as the existing precedents: insecure session config is a compile error absent a loud `insecure_opt_out` (51g); a `requires`-policy route refuses to start until authorization is actually enforced (52b); native codegen degrades loudly rather than silently producing `object`-shaped output.

How to apply:

1. **A consequential choice with multiple reasonable policies is declared in source, never inferred.** If picking one silently could surprise a security-conscious author (open vs. invited signup, permissive vs. strict CORS, redact vs. retain, cross-tenant vs. scoped), do not pick — require the declaration.
2. **Omission is a compile error (or refuse-to-start)** that names the decision and the allowed values. Not a warning, not a default.
3. **Only offer policy values the runtime can execute COMPLETELY today.** A declared-but-unimplemented value is rejected with a clear error (e.g. `approval_required` before durable-approval composition exists), never silently degraded to a weaker behavior.
4. **Never derive a consequential identifier from an untrusted signal.** Tenancy, identity, and authority come from fixed configuration, a verified artifact, or an explicitly configured + allowlisted mapping — never a bare token/claim/header the caller controls (e.g. never assign a tenant from an arbitrary ID-token claim; never merge accounts by email).

## Single source of truth — check before you re-implement

**Before writing any logic that mirrors behavior that already exists, search for the canonical implementation and reuse it. Do not hand-copy a mapping, projection, validation, or resolution into a second place.**

This is mandatory because two divergent copies of the "same" logic is a bug that ships silently — the copies drift, and one layer enforces something a different layer never promised. It has bitten this repo more than once:

- `corvid serve` hand-copied the contract's `default_mime_for_format` upload map instead of calling it; the copy omitted `Audio`/`Video` and used the wrong unknown-format default, so the runtime accepted media types the Application Contract told the frontend picker to reject.
- The IR lowerer carried its own `type_ref_to_type` separate from the checker's; the lowerer's copy didn't handle `Upload`/`Page`, so Contract Closure silently passed routes it should have refused.

**The pre-implementation check (do this every time, not just for "big" features):**

1. **Before implementing a mapping / table / format rule / validation / type-or-shape derivation**, grep for an existing one (by the concept name, a sample key, a representative value). Cross-layer pairs are the high-risk zone: contract ↔ runtime, checker ↔ lowerer, checker ↔ codegen, OpenAPI ↔ serve.
2. **If a canonical version exists, call it.** Make it `pub` (or lift it to a shared crate) rather than copy it. A one-line `pub` beats a second copy that drifts.
3. **If you genuinely must duplicate** (e.g. a dependency-cycle blocks reuse), say why in a code comment AND add a test that asserts the two stay in sync — the copies fail loudly the moment they diverge, never silently.
4. **When the shared thing changes**, the single call site updates every consumer for free — that is the whole point.

The forcing question before you type a `match` on strings or a lookup table: *"Does this already exist somewhere the contract, checker, or another tier already relies on?"* If unsure, grep first.

## Invention shipping contract

Every new invention ships with public proof at the same time as the code. A feature counts as an invention when it is a Corvid-specific language/runtime capability that we would name in the README, site, launch material, or HN discussion.

Before an invention slice is complete, it must include:

- A row in the README's invention table and an entry in [`docs/reference/invention-catalog.md`](docs/reference/invention-catalog.md), or an explicit update explaining why an existing entry already covers it.
- A `corvid tour --topic <name>` demo whose source compiles through the normal driver pipeline.
- A `docs/reference/inventions.md` proof-matrix row with shipped status, runnable command, test coverage, spec link, and explicit non-scope.
- A spec or reference-doc link that defines the behavior.
- Tests that validate the behavior named in the catalog entry.

Do not ship hidden inventions. If the feature is important enough to make Corvid extraordinary, it is important enough to be discoverable, runnable, and test-backed.

## Pre-phase chat mandatory

Never start code on a phase or slice until we've chatted and both sides understand the scope. This applies to refactor slices too — a decomposition plan gets agreed before any file moves.

## Commit at slice boundaries

A slice is not done until its commit is on `main`. Dev-log + git history + ROADMAP checklist must all agree before the next slice starts.

## Professional naming

Apply the mandatory [project-wide naming rule](CONTRIBUTING.md#professional-naming) to every new or renamed item. Check names when creating a branch and when reviewing changes for merge.

## Track deferred work in ROADMAP

In-progress phases carry slice-by-slice `- [ ]` checklists so deferred work never falls through the cracks.

## Update `learnings.md` per user-visible slice

Doc-and-feature land together. No drift between shipped behavior and the learnings record.

## Follow ROADMAP order strictly

Work proceeds in the defined sequence. Never skip ahead or reorder without an explicit pre-phase chat that amends the ROADMAP first.
