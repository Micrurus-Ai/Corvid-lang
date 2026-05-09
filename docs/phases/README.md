# Phase records

Development records, audit reports, refactor logs. **Not user-facing
material** — these are historical engineering documents that pair
with `ROADMAP.md` and `learnings.md`.

If you are reading these as a user, you probably want
[the book](../book/) or [the guides](../guides/) instead.

## What's here

- `phase-NN-*.md` — per-phase implementation briefs and closing
  audits (matching the phase numbers in `ROADMAP.md`).
- `memory-foundation-*.md` — Phase 29 memory primitives dev records.
- `external-trials/` — Phase 42 external-trial feedback notes.

## How to use

Search by phase number when investigating "why did we ship X this
way?". Each phase doc includes:

- The original implementation brief (from when the phase opened).
- A closing audit (appended when the phase closed).
- Pointers to the load-bearing tests, registry rows, and dev-log
  entries that the phase produced.

Phase docs are append-only after a phase closes; corrections happen
in subsequent phases (or in a verification round like Phase 35V),
not by editing closed phase docs in place.
