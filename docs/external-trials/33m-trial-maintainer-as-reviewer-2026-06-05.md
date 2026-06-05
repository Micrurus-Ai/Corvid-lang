# [Trial] maintainer-as-reviewer-2026-06-05 — production-shape threat-intel app, friction audit

> Self-administered trial: the maintainer playing reviewer-#2 of
> the 33M friends-and-family round. Built a small production-
> shape cyber-threat-intel app from scratch using the build prompt
> at
> [`docs/external-trials/33m-friends-and-family-prompt.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/33m-friends-and-family-prompt.md).
> Treated as a real trial — every friction point documented, no
> wave-throughs.

## Intake

- **Handle**: maintainer-as-reviewer-2026-06-05
- **Commit SHA tested**: `7a65b38` (33Q5) — debug build of the
  current HEAD at the time of the trial.
- **OS + architecture + shell**: Windows 11, x86_64, git-bash.
- **Time spent**: ~1 hour focused friction-mining.

## What I built

- **Use case**: cyber-threat-intelligence triage agent — accepts
  POSTed indicators of compromise (IOCs), classifies severity +
  confidence, requires human approval before publishing alerts.
- **Surfaces exercised** (5 of 6, not 6 — see Finding 8):
  - ✅ HTTP route with typed body (POST `/ioc/triage`)
  - ✅ Effect declarations with cost/trust/data
  - ✅ Approval-gated dangerous tool (`publish_to_slack`)
  - ✅ `@trust(autonomous_if_confident(0.85))`
  - ✅ `@budget($2.00)`
  - ❌ Persistence (`std.db`) — see Finding 8
  - ❌ Durable job — skipped, see Finding 8
- **Approximate line count of `main.cor`**: ~55 lines.
- **Real provider keys used**: no (no LLM provider needed for
  the trial — exercised the compile + serve + build + deploy
  paths, not the LLM dispatch).

## Findings (ordered by severity)

### P1 — CODE: `corvid_runtime` Python package is not installable

The scaffold's "Next steps" output literally tells the reviewer:

```
pip install corvid-runtime
```

But `corvid-runtime` is **not a published PyPI package**. The
package source lives in this repo at
[`runtime/python/corvid_runtime/`](https://github.com/Micrurus-Ai/Corvid-lang/tree/main/runtime/python/corvid_runtime).
A standalone-app reviewer running `pip install corvid-runtime`
would get either a 404 OR a wrong package (someone else's
`corvid_runtime` on PyPI — supply-chain risk).

Downstream effect: the 33Q1b tools.py autoloader fails at
serve startup with:

```
error: autoload tools.py: python call `tools.<import>` failed: Traceback ...
ModuleNotFoundError: No module named 'corvid_runtime'
```

Without `corvid_runtime` importable, ANY tools.py with the
canonical `from corvid_runtime import tool` decorator crashes
on serve. Surface 3 (the demo the trial-prompt most wants
exercised) is unreachable without a workaround (`PYTHONPATH=`
pointing at the repo's `runtime/python/`, which a friends-and-
family reviewer cloning a release artifact does not have).

**Three plausible fixes**:

1. Publish `corvid_runtime` to PyPI as part of the v1.0 cut
   (the simplest answer; matches the scaffold's directive).
2. Ship `corvid_runtime/` next to the binary in the release
   tarball and have `corvid serve` auto-set `PYTHONPATH` to
   include `<corvid_home>/runtime-py` before embedding Python.
   No PyPI dep; mirrors how `CORVID_HOME=/opt/corvid` already
   ships `std/` alongside the binary.
3. Detect "`corvid_runtime` not importable" in
   `corvid_runtime/src/python_tools.rs::install_python_tools`
   and skip the autoload path with a clear error pointing
   reviewers at one of (1)/(2). Today the user just gets a
   raw Python traceback.

**Disposition**: CODE (general). I'd lean #2 — the binary
already ships `std/`; one more directory is cheap and avoids
the PyPI release-engineering tail.

### P1 — DOCS+CODE: trust-lattice value `bounded` is used by reference apps but absent from the spec

The spec doc at
[`docs/internals/effect-spec/04-builtin-dimensions.md:65-66`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/internals/effect-spec/04-builtin-dimensions.md#L65-L66)
documents the trust lattice as:

```
autonomous  <  supervisor_required  <  human_required
```

But the production reference app
[`examples/backend/customer_support_agent/src/main.cor`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/examples/backend/customer_support_agent/src/main.cor)
uses `trust: bounded`, `trust: workspace`, `trust: grounded`
in its effect declarations — values NOT in the spec. The
reference app has NO `corvid.toml` declaring these as custom
dimensions, so either:

- The spec doc is incomplete (silently accepts more values).
- The reference app is broken (and our `cargo test` doesn't
  exercise the full typecheck on it).
- There's an undocumented dimension-discovery mechanism I
  couldn't find.

A reviewer copy-pasting from the reference apps (which the
trial prompt explicitly recommends) hits this immediately:
I tried `trust: bounded` on my first attempt and got a
typecheck rejection. Worked around by switching to
`trust: autonomous`, but it shouldn't be a guessing game.

**Disposition**: DOCS+CODE. The spec needs to be
authoritative (and either name `bounded` etc., or the
reference apps should be moved to the canonical values).
Adversarial test: a CI step that grep-checks every reference
app's `trust:` declarations against the spec's authoritative
list would catch this drift.

### P1 — CODE: `pub extern "c"` agents reject struct parameters and returns

To sign a cdylib (`corvid build --target=cdylib --sign`), the
exported agent must be `pub extern "c"`. But `pub extern "c"`
accepts **only Int / Float / Bool / String** for parameters,
and **only scalar / Grounded<scalar> / Nothing** for returns.
Any agent whose boundary is a struct (the natural shape for
HTTP request/response bodies) is rejected:

```
error: extern "c" agent `triage_ioc` uses unsupported ABI type
       `struct` in parameter `req`
Help: extern "c" currently accepts Int/Float/Bool/String parameters
       plus scalar, `Grounded<scalar>`, or `Nothing` returns;
       rich structured boundary types still wait for later Phase 22
       FFI slices
```

This kills the signed-cdylib path for any production-shape
app with structured request/response bodies. The Help text
honestly names it as a Phase 22 follow-up — so it's a known
gap, not a regression — but it's the kind of gap that's
load-bearing for the launch claim "signed cdylib for any
backend you can write." The reference apps presumably work
around this somehow (probably by exposing scalar-only
entrypoints and unpacking JSON internally), but that pushes
type discipline OFF the boundary, which is the opposite of
Corvid's pitch.

**Disposition**: CODE. The existing Phase 20n-C struct-return
work landed for internal codegen + prompt bridges; the
`pub extern "c"` boundary still rejects them. A follow-up
slice that lifts this rejection (perhaps reusing 20n-C's
struct decoder/encoder) would close the launch-readiness gap.

### P2 — UX: `corvid serve` route table mislabels routes as `approval-gated`

The startup log shows:

```
POST   /ioc/triage  -> triage_ioc (body; approval-gated -> 202 + queued)
```

But `triage_ioc` has **no `approve` boundary** in its body —
only `publish_alert` (a non-routed agent) does. The label is
misleading: a reviewer reading this would expect
`POST /ioc/triage` to answer 202 + queue an approval, but the
route actually returns 200 directly (when the tool handler is
registered) or 500 (when it isn't).

The label probably fires because the agent's
`@trust(autonomous_if_confident(0.85))` declares a possible
escalation to `human_required`. But that's a runtime
escalation, not a guaranteed queue. The startup table treats
"possibly queueable" as "queued."

**Disposition**: UX (DOCS or CODE). Either rename the label
(e.g. "may queue if trust escalates" vs "always queues") or
omit it for routes whose body has no syntactic `approve`. A
reviewer using this table to plan their client integration
would write the wrong polling logic.

### P2 — CODE: 500 response body leaks internal span ranges

When a tool isn't registered (the natural state during
incremental development), the 500 body is:

```json
{"detail":"[1227..1269] no handler registered for tool `classify_ioc`","error":"handler_failed"}
```

The `[1227..1269]` is the IR byte-span of the call site — an
internal compiler artifact. It leaks to the HTTP client and
is not actionable on the client side. A real client would
need to strip it OR ignore it OR report a confusing range to
the user. Either:

- Strip the span prefix from the 500 body's `detail` (the
  span is in the server's traces; the client doesn't need it).
- Replace it with a source-line reference (`src/main.cor:24`)
  that's at least human-meaningful.

**Disposition**: CODE. Probably a small fix in
`crates/corvid-cli/src/serve_cmd.rs::approve_approval`'s
error-formatting branch (and the equivalent dispatch
handlers' error mapping).

### P2 — CODE: `corvid deploy package` leaves partial artifacts on signing-key failure

Running `corvid deploy package <app> --out deploy/` without
`CORVID_DEPLOY_SIGNING_KEY` set:

- Exits with `error: CORVID_DEPLOY_SIGNING_KEY is required ...`
  (non-zero exit code).
- BUT leaves 6 of the expected 9 files in `deploy/`
  (`Dockerfile`, `oci-labels.json`, `env.schema.json`,
  `health.json`, `migrate.sh`, `startup-checks.md` —
  everything emitted before the attestation step that needs
  the key).

A reviewer would see "error" and either: (a) re-run and get
"create dir failed: file exists" on some path, (b) wonder if
the partial deploy/ is usable, (c) delete deploy/ and retry.
None of these is good. The contract should be all-or-nothing:
either the deploy/ dir is fully populated OR the operation
left no artifacts behind.

**Disposition**: CODE. `run_package` in `deploy_cmd.rs`
should fail-fast on missing CORVID_DEPLOY_SIGNING_KEY before
writing the first file (or write to a tempdir and atomic-
rename on success).

### P3 — DOCS: `CORVID_DEPLOY_SIGNING_KEY` is undiscoverable from `--help`

```
$ corvid deploy package --help
Usage: corvid deploy package [OPTIONS] <APP>
Options:
  --out <DIR>      Output directory for generated artifacts
  --cdylib <PATH>  Path to the signed cdylib ...
  -h, --help       Print help
```

No mention of `CORVID_DEPLOY_SIGNING_KEY`. The error message
when the env is missing also doesn't tell the user what value
shape to set (a 32-byte hex seed; the trial prompt knows but
the CLI itself doesn't). A `--help` entry naming the env var
or a `--signing-key <path>` flag would close the discovery
loop.

**Disposition**: DOCS (with optional CODE follow-up to add a
`--signing-key` flag for parity with `corvid build --sign`).

### P3 — DOCS: `std.db` is types-only, NOT first-class persistence

The build prompt says: *"Persistence through `std.db` — at
least 2 tables and one migration applied through `corvid
migrate up`."* But
[`src/std/db.cor`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/std/db.cor)
is METADATA ENVELOPES (`DbConnection`, `DbQuery`, `DbResult`,
`DbParam`, `DbColumn` …) — types describing a DB interaction,
not actual connection/query primitives. To do real
persistence you need to declare a Corvid `tool` that wraps
your own SQL execution and implement it in `tools.py`
against (say) `sqlite3` or `psycopg`. That's not "first-class
DB access" the way the README implies; it's "you write your
own SQL bridge."

For my trial app I skipped this surface entirely because the
ceremony was too much for an hour-long trial budget. The
reference apps presumably do this elaborate dance (~600+
line `main.cor`s), but a first-time reviewer would expect
something closer to FastAPI + SQLAlchemy ergonomics. The
gap between "Corvid has typed DB access" (the pitch) and "you
write your own SQL bridge through a Corvid tool wrapper" (the
reality) is large.

**Disposition**: DOCS for v1.0 (clarify what `std.db` is and
what it isn't); CODE follow-up (post-v1.0) for first-class
typed `db.query(...)` primitives. The 35V2-P38 phase already
has runtime SQLite/Postgres execution; the gap is the
**source-syntax surface**, which is filed as a post-v1.0
`35V2-P39-I`-style syntax-sugar slice IIRC.

### P3 — UX: Windows path-separator mixing in OCI labels

The rendered `oci-labels.json` (via the attestation payload):

```
"org.opencontainers.image.source":
  "C:/Users/SBW/AppData/Local/Temp/threat_intel_agent\\src\\main.cor"
```

Mixed `/` and `\\` in the same path string. Functionally
works on Windows but reads oddly in OCI metadata that's
typically POSIX-shaped. Probably a `Path::display()` that
mixes the user-supplied path's separators with `Path::join`'s
platform-native separators.

**Disposition**: CODE (low). Normalize the source-path
display to forward-slashes for cross-platform OCI metadata.

### Minor — `pub extern "c"` requirement not surfaced in error helpfulness

The "must have at least one `pub extern "c"` agent" error:

```
error: failed to build `src/main.cor` (cdylib): native codegen
       failed: [0..0] native codegen does not yet support:
       library targets require at least one `pub extern "c"` agent
```

- The `[0..0]` span is a zero-width anchor at the file start —
  meaningless to the user.
- The phrasing "native codegen does not yet support: library
  targets require..." reads awkwardly (the colon's parse).
- No "Help:" line pointing at a doc page that explains how to
  add `pub extern "c"` (the user has to guess the syntax).

**Disposition**: DOCS+CODE polish. The same pattern as
Finding P3-P5 from the previous trial round: errors should
name a doc page when the user is missing the boilerplate.

## What worked (so the signal isn't all negative)

- `corvid new` produced a clean scaffold with std/ vendored
  at `src/std/` (33Q1's fix is real). `import "./std/effects"`
  resolved immediately.
- `corvid check` was fast and the diagnostics generally point
  at the right span. The `effect_constraint_violated` error
  on `@trust(autonomous_if_confident(0.85))` over an
  incompatible tool was precisely what I expected — and the
  "composed value is bounded" message named exactly which
  dimension and which value.
- `corvid serve` started up cleanly once `PYTHONPATH` was
  set, listed every route + admin endpoint, and the
  `/healthz` check answered 200. The post-33Q1a tool-loading
  logic visibly worked (the missing-tool 500 named the right
  tool).
- `corvid deploy package` produced all 9 expected artifacts
  when CORVID_DEPLOY_SIGNING_KEY was set. The 33Q4 + 33Q5
  fixes are visible in the rendered Dockerfile:
  `ARG CORVID_VERSION=nightly-2026-06-05-84e1709` (pinned to
  SHA), `COPY tools.py ./tools.py` (presence-conditional), no
  `COPY migrations/evals/traces` lines for the bare app.
- The attestation chain payload correctly reports
  `chain_status: incomplete` when `--cdylib` isn't provided.

## Production-readiness ceiling

The first thing that would stop me from shipping this app
for real:

- **`pub extern "c"` struct rejection** (Finding P1) — until
  this lifts, signed cdylibs can't expose structured request/
  response APIs, which means the production "signed cdylib +
  ops introspection" story is unusable for any non-trivial
  HTTP backend. Workarounds (scalar-only entrypoints with
  internal JSON unpacking) push type discipline off the
  signed boundary, which defeats the moat.

The second thing:

- **`corvid_runtime` PyPI gap** (Finding P1) — every reviewer
  installing from a release artifact hits the tools.py
  autoload failure. The friends-and-family round starts
  with a broken first-impression.

The third thing:

- **`std.db` ceremony** (Finding P3) — until the source-
  syntax surface for DB queries lands, "Corvid has first-
  class persistence" is overclaiming. Real apps need real
  DB ergonomics.

## Honest moments

- **Spec doc vs reference apps drift** (Finding P1.2). The
  spec is the canonical contract, the reference apps are the
  worked examples. They MUST agree. A reviewer who trusts
  the spec is contradicted by the reference apps; a reviewer
  who trusts the reference apps is contradicted by the spec.
  This is a load-bearing trust signal we're spending without
  noticing.
- **"`corvid_runtime` Python package"** is named in the
  scaffold ("Next steps"), the prompt, the autoloader docs,
  and the worked examples — but it isn't real on PyPI. That
  feels like a missed step in the v1.0 release plan.
- **`pub extern "c"` for cdylib** is the kind of constraint a
  language designer accepts as obvious but a friends-and-
  family reviewer doesn't. The error message names it; the
  build prompt names it; but the *combination* of "you must
  have one" + "it can only have scalar boundaries" + "your
  HTTP routes have struct bodies" forces every reviewer to
  contort their app shape to fit the cdylib path. That
  contortion is the moat — but selling it as ergonomic
  requires either lifting the struct restriction (Phase 22
  follow-up) OR clearly documenting the workaround pattern.

## Suggested disposition (my call, you triage)

| Finding | Class | Owning slice |
|---------|-------|--------------|
| P1.1 `corvid_runtime` not installable | code | new slice — propose 33Q6-corvid-runtime-distribution |
| P1.2 trust lattice spec drift | docs+test | new slice — propose 33Q7-spec-reference-apps-drift-gate |
| P1.3 pub extern "c" rejects structs | code | follow-up to Phase 20n-C; propose 33Q8 OR file under Phase 22 |
| P2.1 serve mislabels routes | code or docs | new slice — propose 33Q9-serve-approval-label-accuracy |
| P2.2 IR span leak in 500 body | code | new slice — propose 33Q10-serve-error-detail-clean |
| P2.3 deploy package partial artifacts | code | new slice — propose 33Q11-deploy-package-atomic |
| P3.1 CORVID_DEPLOY_SIGNING_KEY hidden | docs | small docs slice |
| P3.2 std.db types-only docs | docs | docs slice or post-v1.0 syntax-sugar |
| P3.3 Windows path-separator mixing | code (low) | small slice |
| Minor `pub extern "c"` error UX | docs+code | small slice |

## Repro harness

The threat_intel_agent app I used is at:

- `/tmp/threat_intel_agent/src/main.cor` (the 55-line source)
- `/tmp/threat_intel_agent/tools.py` (scaffold-shape; not
  wired up because of P1.1)
- `/tmp/threat_intel_agent/corvid.toml` (scaffold default)

Happy to commit it as a test fixture under
`crates/corvid-cli/tests/fixtures/maintainer_trial_app/` if
that helps the 33Q* follow-ups.

## Time commitment

~1 hour focused, vs the prompt's "half-day to full-day"
budget. I ran narrow — chose 5-of-6 surfaces (skipped
persistence + jobs because of Finding P3.3's ceremony) and
focused on the breadth of friction rather than depth on any
one feature. A real friends-and-family reviewer with 4-8
hours would likely find more in the depth.
