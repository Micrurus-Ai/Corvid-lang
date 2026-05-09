# Project conventions

## Naming

- `snake_case` — file names, identifiers, effect names, agent names,
  prompt names, tool names.
- `PascalCase` — type names, sum-type variants, struct names,
  `approve` action names.
- `SCREAMING_SNAKE_CASE` — constants.
- Effect names suffix-with `_effect`: `refund_effect`,
  `retrieval_effect`, `email_effect`.

## File organization

- One agent per file is a good default.
- Multi-agent files are fine when agents share state or test surface.
- `tests/` mirrors `src/` directory structure.
- Examples under `examples/`, benchmarks under `benches/`,
  reference apps top-level (`refund_bot.cor`, `rag_qa_bot.cor`).
- Per-feature docs live under `docs/` next to the source they document.

## Commit messages

Slice numbers (`20m-A`, `33J2`) appear only in commit messages,
`ROADMAP.md`, `dev-log.md`, `learnings.md`, and phase-specific docs.
Source code, doc comments, and docstrings use behavioral names for
features.

## Pre-phase chat

A slice does not start writing code until both sides have agreed on
its scope. Decomposition plans for refactors get the same treatment.

## Slice closure

A slice is not done until its commit is on `main`. The dev log + git
history + ROADMAP checklist all agree before the next slice starts.

## Updating learnings

Doc-and-feature land together. No drift between shipped behavior and
the learnings record.

## Following ROADMAP order

Work proceeds in the defined sequence. Skipping ahead requires an
explicit pre-phase chat that amends the ROADMAP first.

---

# Implementation notes for the website build (33J2)

These are not docs pages — they are notes for whoever wires up the
docs site rendering pipeline.

## Source format

Each `# Page:` heading starts a new page. The fields immediately under
each heading (`slug:`, `nav:`, `section:`) are the page's frontmatter.
The build pipeline reads them, strips them, and renders the rest as
the page body.

## Cross-link convention

Internal cross-links use `/docs/<slug>` form. The build pipeline
resolves these against the slug map. External links to the corvid
GitHub repo use full URLs to
`https://github.com/Corvid-lang/Corvid-lang/blob/main/<path>`.

## Code-block highlighting

Corvid code blocks use the language tag ```corvid```. The website's
syntax highlighter (Prism + a Corvid grammar contributed in 33J2)
recognizes the tag.

## Drift gate

A drift-gate test in CI compares this file's headline guarantee list
against the registry's `Static` rows. The build refuses if a guarantee
mentioned here doesn't exist in the registry, or if a Static
guarantee in the registry isn't mentioned here. Same shape as
Phase 35V's other drift gates.

## Open work for 33J2 (docs site rendering)

This file is the source content. Still TODO under slice 33J2:

- Pick the static-site framework (Astro Starlight is the obvious
  choice; one pre-phase-chat away).
- Wire the build pipeline (parse this file, render each page, emit
  the docs site under `docs/` on the website repo).
- Write the Corvid syntax grammar for Prism.
- Write the drift-gate CI test.
- Add a left-nav structure file derived from `nav:` and `section:`.
- Decide search behavior (Algolia DocSearch vs. local Lunr).
