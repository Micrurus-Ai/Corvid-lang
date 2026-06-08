# Handoff prompt — wire the Corvid docs site (slice 33J3)

This is a self-contained brief for a developer (or coding agent)
who does **not** know the Corvid programming language and is being
asked to add a documentation section to the Corvid website.
Everything you need is here. Read top to bottom.

---

## The two repositories

You will work across two repos:

1. **The Corvid language repo** — the source of truth for the
   docs content.
   - URL: <https://github.com/Micrurus-Ai/Corvid-lang>
   - The docs you will render live at `docs/` in this repo.
   - **Do not push commits here.** Treat it as read-only for this
     slice; any docs edits land upstream and we sync.

2. **The Corvid website repo** — where you will add the docs site.
   - URL: <https://github.com/Micrurus-Ai/corvid-website>
   - Currently contains a single `index.html` (~1,100-line marketing
     landing page), `/logos/`, `/social/`. No build system yet, no
     docs section.
   - **You will add the docs render pipeline here**, without breaking
     the existing marketing landing.

---

## What Corvid is (one paragraph for context)

Corvid is a programming language where dangerous AI actions either
compile with a proof or do not compile at all. The compiler reads
`approve`, `Grounded<T>`, effect rows, and budget annotations the
way other compilers read types, and it refuses to emit a binary if
a tool call that should be gated by an approval is reachable
without one. You don't need to learn the language to do this slice.
You just need to render its documentation.

---

## What's already done

The Corvid main repo has a fully-organized Diataxis-style docs
tree at `docs/` (committed in slice 33J2). The shape:

```
docs/
├── README.md                  ← top-level docs index
├── book/                      ← 18 sequential learning chapters
│   ├── 00-why-corvid.md
│   ├── 01-install.md
│   ├── 02-quickstart.md
│   ├── 03-tutorial-refund-agent.md
│   ├── 04-syntax.md
│   ├── 05-types.md
│   ├── 06-modules.md
│   ├── 07-effects.md
│   ├── 08-approve.md
│   ├── 09-grounded.md
│   ├── 10-agents.md
│   ├── 11-prompts.md
│   ├── 12-errors.md
│   ├── 13-pattern-matching.md
│   ├── 14-project-layout.md
│   ├── 15-testing.md
│   ├── 16-building.md
│   ├── 17-replay.md
│   └── README.md              ← book table of contents
├── guides/                    ← task-focused how-to guides
│   ├── backend.md
│   ├── persistence.md
│   ├── jobs.md
│   ├── auth.md
│   ├── observability.md
│   ├── connectors.md
│   ├── wasm.md
│   ├── ffi-python.md
│   ├── ffi-c-rust.md
│   ├── editor-and-lsp.md
│   ├── debugging.md
│   ├── performance.md
│   └── README.md
├── recipes/
│   ├── README.md              ← contains all the recipe patterns
│   └── personal-executive-agent.md
├── reference/
│   ├── grammar.md             ← formal EBNF
│   ├── lexer-rules.md
│   ├── cli.md
│   ├── guarantees.md
│   ├── inventions.md          ← full proof matrix (356 lines)
│   ├── inventions-tour.md     ← short index
│   ├── core-semantics.md      ← AUTO-GENERATED, do not edit
│   ├── package-imports.md
│   ├── reference-apps.md
│   ├── reference-apps/        ← per-app subtree
│   ├── stdlib/
│   │   ├── README.md          ← stdlib overview
│   │   └── connectors/        ← Gmail, MS365, Slack, Calendar, Tasks, Files
│   └── README.md
├── migration/
│   ├── from-python.md
│   └── README.md
├── developer-production-guide.md   ← promoted from docs/operations/
├── maintainer-runbooks.md          ← promoted from docs/operations/
├── release-policy.md               ← promoted from docs/meta/
├── claim-inventory.md              ← promoted from docs/meta/
├── beta-program.md                 ← promoted from docs/meta/
├── launch-rehearsal.md             ← promoted from docs/meta/
├── phase-43-market-readiness.md    ← promoted from docs/phases/
├── external-trials/                ← promoted from docs/phases/external-trials/
│   ├── phase-42-trial-one.md
│   └── phase-42-feedback-triage.md
├── operations/
│   ├── production-checklist.md
│   ├── receipts-and-signing.md
│   ├── observability-conformance.md
│   ├── ci.md
│   └── README.md
├── security/
│   ├── model.md
│   ├── model-overview.md
│   ├── stability-contract.md
│   └── README.md
├── internals/
│   ├── effect-spec/           ← formal effect spec sub-tree (16 files)
│   ├── wasm-abi.md
│   ├── bundle-format.md
│   ├── host-compliance.md
│   ├── testing-primitives.md
│   ├── package-manager-scope.md
│   └── README.md
├── help/
│   ├── faq.md
│   ├── glossary.md
│   └── README.md
├── meta/
│   ├── conventions.md
│   ├── release-policy.md
│   ├── upgrade-migrations.md
│   ├── beta-program.md
│   ├── launch-claim-audit.md       ← data table (consumed by `corvid claim audit`)
│   ├── v1.0-demo-script.md
│   ├── ai-benchmarks.md
│   ├── reference-apps-launch-status.md
│   ├── website-docs-handoff.md  ← this file
│   └── README.md
└── phases/                    ← historical dev records, NOT user-facing
    └── (skip these — they go in the dev archive, not the docs site)
```

Each section has a `README.md` index that lists its pages with
descriptions. The top-level `docs/README.md` is the docs site
landing page content.

---

## What you will build

A docs section on the Corvid website rendering this tree as a
navigable site at `https://corvid-lang.org/docs/...`. Concrete
deliverables:

1. **A docs site build pipeline** in the
   `Micrurus-Ai/corvid-website` repo that:
   - Pulls the markdown source from `Micrurus-Ai/Corvid-lang`
     under `docs/` (excluding `docs/phases/` which is dev
     archive, not user-facing).
   - Renders each markdown file as a navigable page.
   - Generates a left-nav matching the directory structure
     (each section's `README.md` becomes that section's index
     page).
   - Renders fenced code blocks with the language tag ` ```corvid `
     using a Corvid syntax highlighter (see "Corvid syntax for
     the highlighter" below).
   - Resolves relative cross-links between markdown files
     (`[link](../guides/backend.md)` → `/docs/guides/backend`).
   - Provides full-text search.

2. **Integration with the existing landing page**:
   - The current `index.html` continues to serve at `/`.
   - The docs site serves at `/docs/...`.
   - Visual styling roughly matches the landing's design tokens
     (Source Serif 4 for headings, Source Sans 3 for body,
     JetBrains Mono for code, the ink/paper/red/green palette
     in `index.html`'s CSS).

3. **Deploy hook** that rebuilds the docs site when either repo
   pushes to `main`.

---

## Recommended framework: Astro Starlight

[Astro Starlight](https://starlight.astro.build/) is the obvious
pick for this slice. It is:

- **Static by default** — fast cold loads, deployable to any CDN
  (Vercel, Netlify, Cloudflare Pages).
- **MDX-first** — renders the existing markdown without
  modification.
- **Built-in left-nav, search, dark mode, mobile menu** —
  saves you a week of UI work.
- **Sidecar config** — you control the nav structure via a
  TypeScript config file, no need to write a CMS.
- **Custom syntax highlighting** — you can register a Corvid
  grammar (Shiki / Prism, both supported) without forking the
  framework.
- **Theming** — the landing page's design tokens map cleanly to
  Starlight's CSS variables.

If you have a strong preference for another tool (Docusaurus,
Nextra, MkDocs, VitePress), the requirements above all map
cleanly. Just keep the same shape.

---

## How to source the docs from the main repo

Three viable approaches; pick one and document the choice in
the website repo's README:

### Option A — git submodule (simplest)

Add `Micrurus-Ai/Corvid-lang` as a submodule under
`packages/corvid-source/` in the website repo. Configure the
docs site to read markdown from `packages/corvid-source/docs/`.
The submodule pin updates with each website-repo commit.

Pros: explicit version pinning, single deploy artifact.
Cons: submodules are a known footgun; updating the pin is a
manual step.

### Option B — build-time fetch (recommended)

In the website repo, write a `scripts/sync-docs.ts` (or `.sh`)
that runs at build time:

```sh
# scripts/sync-docs.ts (pseudocode)
git clone --depth=1 https://github.com/Micrurus-Ai/Corvid-lang.git .corvid-source
mkdir -p src/content/docs
cp -r .corvid-source/docs/* src/content/docs/
rm -rf src/content/docs/phases   # exclude dev archive
```

Wire it into `package.json`'s build script:

```json
{
  "scripts": {
    "predev": "tsx scripts/sync-docs.ts",
    "prebuild": "tsx scripts/sync-docs.ts"
  }
}
```

Pros: single-repo workflow, easy to review, no submodule pain.
Cons: every build clones; cache the clone for incremental dev.

### Option C — monorepo move

Move the website source into the main Corvid repo as a
top-level `website/` directory. Out of scope for this slice;
mention only if you think it's the right call.

---

## Cross-link convention

The markdown files use **relative links** between siblings, e.g.:

```md
See [the moat](00-why-corvid.md) and [the security model](../security/model.md).
```

The renderer must resolve these to website paths:

- `00-why-corvid.md` (sibling) → `/docs/book/00-why-corvid`
- `../security/model.md` → `/docs/security/model`
- `README.md` (in any directory) → that directory's index page,
  e.g. `docs/book/README.md` → `/docs/book`

External links to GitHub are fully-qualified URLs and should
pass through untouched, e.g.:

```md
The parser at [`crates/corvid-syntax/src/parser/`](https://github.com/Micrurus-Ai/Corvid-lang/tree/main/crates/corvid-syntax/src/parser/)
is the source of truth.
```

Some pages from the original `website-content.md` mega-file used
website-style absolute paths (e.g. `[link](/docs/the-moat)`).
These are correct already — pass them through as
`/docs/the-moat`.

---

## Corvid syntax for the highlighter

Code blocks tagged ` ```corvid ` should highlight these tokens.

**Keywords (reserved):**
```
agent, tool, prompt, effect, type, struct, fn, let,
if, else, for, in, while, return, yield, break, continue, pass,
match, import, as, public, package, pub, extern, extend,
approve, await_approval, server, route, schedule, zone,
eval, test, fixture, mock, model, requires, replay, when,
true, false, Nothing, and, or, not, dangerous, uses,
assert, assert_snapshot, session, memory, try, on, error,
retry, times, backoff, linear, exponential,
progressive, below, rollout, ensemble, vote, adversarial,
propose, challenge, adjudicate
```

**Built-in types (highlight as types):**
```
Int, Float, Bool, String, Unit, List, Map, Option, Result, Grounded
```

**Annotations** — start with `@`, followed by a name, optionally
arguments in parens:
```
@budget($0.50)
@retry(max_attempts: 3, backoff: exponential(base: 30s))
@idempotency(key: brief.user_id)
@replayable
@max_steps(10)
@max_wall_time(30s)
```

**Operators:**
```
+ - * / % == != < > <= >=  =  ?  .  ->  +=  -= |
```

**String literals:**
- Double-quoted: `"..."` (single-line, with `\n`, `\t`, `\r`,
  `\\`, `\"`, `\0` escapes).
- Triple-quoted: `"""..."""` (multi-line, raw).

**Comments:**
- `# single-line comment`
- `#: doc comment` (renders in `--help` and LSP hover)

**Identifiers:** `[A-Za-z_][A-Za-z0-9_]*`. Type names are
PascalCase; identifiers/effect names are snake_case.

**Numeric literals:**
- Integer: `42`, `-1`, `0x2A`
- Float: `3.14`, `1.0e6`
- Money: `$0.005`, `$50.00` (used in `cost:` dimensions)

**Special syntactic shapes worth highlighting distinctly:**
- `effect` declarations with `: INDENT` body of `name: value` pairs.
- `agent` / `tool` / `prompt` declarations with `(...) -> Type uses
  effect_a, effect_b:` signatures.
- `approve <PascalCaseAction>(args)` — the compile-time approval
  token. The PascalCase action name is structurally significant.
- `Grounded<T>` — a type wrapper.
- `@host.namespace.method(...)` — host FFI calls, used inside `tool`
  bodies.

A reference set of code blocks to test the highlighter against
lives in `docs/book/03-tutorial-refund-agent.md`,
`docs/book/07-effects.md`, and `docs/recipes/README.md`.

---

## Visual / theme alignment with the landing page

The existing `index.html` defines its design tokens in a `<style>`
block. Match these in the docs site's theme config:

- **Fonts**: Source Serif 4 (headings), Source Sans 3 (body),
  JetBrains Mono (code).
- **Colors**: `--ink` (near-black foreground), `--paper`
  (near-white background), `--red` (accent), `--green`
  (success/highlight).
- **Layout**: max content width should match the landing's hero
  area for visual continuity.

Read the `<style>` block in `index.html` for the exact CSS
custom-property values.

---

## Constraints (no-shortcut rules)

This project takes documentation honesty seriously. Honor these:

1. **Do not edit upstream docs source.** All markdown edits land
   in `Micrurus-Ai/Corvid-lang` upstream. The website repo only
   renders. If you spot a doc bug, file an issue at
   <https://github.com/Micrurus-Ai/Corvid-lang/issues>.

2. **Do not edit the auto-generated spec doc.**
   `docs/reference/core-semantics.md` is regenerated from the
   in-code registry by `corvid contract regen-doc
   docs/reference/core-semantics.md`. Treat it as read-only.

3. **Exclude `docs/phases/` from the rendered site.** That tree
   is historical engineering records, not user-facing material.

4. **Do not break the landing page.** The current `index.html`
   stays at `/`; the docs site lives at `/docs/...`.

5. **Search must work without a third-party tracking dependency
   in the critical path.** Local search via Lunr or Pagefind is
   preferred over Algolia DocSearch unless you can confirm
   Algolia's privacy posture.

---

## Acceptance criteria

The slice is done when:

- [ ] Visiting `corvid-lang.org/docs` shows the docs landing page
      (rendered from `docs/README.md`).
- [ ] Left-nav lists every section (Book, Guides, Recipes,
      Reference, Migration, Operations, Security, Internals,
      Help, Meta) and renders each section's pages in the
      defined order.
- [ ] Every internal markdown link resolves correctly.
- [ ] Code blocks with ` ```corvid ` are highlighted with the
      keyword/type/annotation/string/operator categories above.
- [ ] Search returns results for terms appearing in any rendered
      page (e.g. searching "approve" returns at least
      `docs/book/08-approve.md`, `docs/reference/grammar.md`,
      and `docs/help/glossary.md`).
- [ ] Mobile breakpoints are usable (hamburger nav at <768px).
- [ ] Build pipeline runs in <2 minutes on a clean checkout.
- [ ] A push to `main` on either repo rebuilds and redeploys.
- [ ] The landing page at `/` is unchanged in behavior.

---

## Out of scope (file as separate slices)

- **WASM playground** — slice 33J7 (separate). The playground
  uses Corvid's WASM target; you will not need to integrate it
  in this slice.
- **Benchmark page** — slice 33J4 (separate). Renders
  `benches/moat/RESULTS.md` from the main repo at a separate URL.
- **Blog shell** — slice 33J5 (separate). Just the empty shell;
  the launch post itself is slice 33L.
- **Grammar drift gate** — slice 33J6 (separate). A CI test that
  cross-checks `docs/reference/grammar.md` against the parser's
  test corpus.
- **Authoring new docs** — happens upstream. You are the
  rendering pipeline, not a content author.

---

## Where to start

1. Clone both repos side-by-side.
2. Inside `corvid-website`, scaffold an Astro Starlight project
   under a subdirectory (so the existing `index.html` is
   undisturbed at the repo root).
3. Wire the build-time sync script (Option B above) to copy
   `docs/*` from the main repo into the Starlight content
   directory.
4. Configure the left-nav in Starlight's config to reflect the
   directory tree.
5. Add the Corvid syntax highlighter (Shiki grammar JSON or
   Prism extension; reference set in
   `docs/book/03-tutorial-refund-agent.md`).
6. Theme the Starlight CSS variables to match the landing page's
   design tokens.
7. Configure the deploy target so `/docs/*` routes to the
   Starlight build and `/` continues to serve `index.html`.
8. Validate against the acceptance criteria.
9. Open a PR on `Micrurus-Ai/corvid-website` titled
   `feat(33J3): docs site build with Starlight`.

---

## Questions to ask before starting

If anything below is unclear, ask the project maintainers (open
an issue on `Micrurus-Ai/Corvid-lang` with the `33J3` label):

- Which deploy target does `corvid-website` currently use?
  (Vercel? Netlify? Cloudflare Pages? GitHub Pages?)
- Is there a CI workflow already wired in `corvid-website`, or
  do we need to add one?
- Confirm the canonical domain: `corvid-lang.org` is the assumption
  (locked in the 33R market-readiness track 2026-06-08);
  if it's something else, that affects link resolution.
- Confirm that the existing `index.html` should remain at `/`
  versus being absorbed into the Starlight site as the home
  page.

---

## Reference reading (skim, don't memorize)

- The Corvid Book: <https://github.com/Micrurus-Ai/Corvid-lang/tree/main/docs/book>
- The CLI reference: <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/cli.md>
- The grammar (formal EBNF):
  <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/grammar.md>
- The project's file-responsibility rule (so you understand the
  docs-tree shape's "why"):
  <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/CLAUDE.md>
- The phase 33 ROADMAP entry (the slice you're closing):
  <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md>
  (search for `33J-website-playground`).

Good luck. Treat this as a normal frontend infra slice — the docs
content is already done, you're just wiring the rendering. Ping
back with questions.
