# Inventions tour

## What this page is

A short index of Corvid's load-bearing language inventions. Each
invention has a runnable demo through `corvid tour --topic <name>`, a
catalog entry in
[`docs/reference/inventions.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/inventions.md),
a spec link, and tests.

## The six headline inventions

### 1. Compile-time approvals

Dangerous tool calls without `approve` do not compile. The token is
lexically scoped, structural, and adversarially tested.

```sh
corvid tour --topic approve-gates
```

### 2. Grounded provenance types

`Grounded<T>` is a different type than `T`. The compiler refuses
silent unwrap; the audit log records every explicit unwrap.

```sh
corvid tour --topic grounded-provenance
```

### 3. Compile-time budgets

`@budget($X)` is a real ceiling. The compiler computes worst-case cost
across the agent's call graph and refuses to emit if the composition
exceeds the budget.

```sh
corvid tour --topic compile-time-budgets
```

### 4. Confidence gates

Effect rows carry confidence; composed agent confidence below a
threshold can be configured to escalate or refuse.

```sh
corvid tour --topic confidence-gates
```

### 5. Strict citations

A model output that should be backed by a citation cannot be passed to
a downstream prompt without one. The grounded type and the citation
metadata enforce it.

```sh
corvid tour --topic strict-citations
```

### 6. Replay and receipts

Every agent run produces a deterministic trace. `corvid replay`
reproduces it byte-for-byte.
`corvid eval --swap-model <m>` diffs the new model's behavior against
the baseline. Receipts are signed via DSSE.

```sh
corvid tour --topic replay-receipts
```

## The full catalog

`corvid tour --list` prints every shipped invention.
[`docs/reference/inventions.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/inventions.md)
is the proof matrix: per-invention shipped status, runnable command,
test coverage, spec link, explicit non-scope.
