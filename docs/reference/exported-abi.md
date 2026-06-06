# Exported ABI — `pub extern "c"` agents

> Quick reference: which Corvid agents become C-ABI entry points in a
> signed cdylib, what parameter/return shapes the v1.0 boundary
> accepts, and where the boundary's current restrictions are tracked.

## What `pub extern "c"` does

A Corvid agent declared `pub extern "c"` is exported as a C-ABI
symbol when you build a library target (`corvid build --target=cdylib`
or `--target=staticlib`). The linker writes it into the binary's
dynamic-symbol table; host code (a Rust/Go/Python binding, a
`dlopen`+`dlsym` invocation, the `corvid_call_agent` C-API) can
locate and call it directly.

Without at least one `pub extern "c"` agent in your source, the build
refuses:

```
error: library targets (cdylib / staticlib) need at least one
       `pub extern "c"` agent — the C-ABI entry point the linker
       exports.
```

There's nothing to export, so there's no point producing the
library.

## Accepted boundary types (v1.0)

| Position | Accepted types |
|---|---|
| **Parameter** | `Int`, `Float`, `Bool`, `String` |
| **Return** | `Int`, `Float`, `Bool`, `String`, `Grounded<Int>`, `Grounded<Float>`, `Grounded<Bool>`, `Grounded<String>`, `Nothing` |

The boundary is scalar-only by design at v1.0 — the C ABI doesn't
have a portable struct layout that round-trips identically across
hosts, so signed boundaries stay in the scalar lane where the ABI is
well-defined.

## What's NOT accepted at v1.0

- **Struct parameters or returns**. An agent like
  `pub extern "c" agent triage(req: IocRequest) -> IocVerdict` is
  rejected at typecheck time with an error naming the unsupported
  type. The internal codegen DOES support struct returns (Phase
  20n-C lifted them for prompt bridges and internal entry
  agents), so the underlying machinery exists; surfacing it at
  the `pub extern "c"` boundary is post-v1.0 work tracked under
  ROADMAP slice **33Q8** (filed by maintainer-as-reviewer-
  2026-06-05 P1.3). When 33Q8 ships, the boundary will accept
  struct shapes via the existing JSON-decoder/encoder family +
  `corvid-prompt-format`'s schema generator.
- **`List<T>`, `Option<T>`, generic parameters**. Same reason —
  no portable C-ABI representation. The post-v1.0 plan threads
  these through the same JSON boundary 33Q8 introduces.
- **Tool calls inside the exported agent's body**. Allowed, but
  the host must supply the tool implementation at runtime via
  the `corvid_register_tool` C-API (the same dispatch path
  `corvid serve --with-tools-cdylib` uses). The cdylib itself
  doesn't carry tool implementations.

## Worked example — scalar boundary

```corvid
@budget($0.50)
@trust(human_required)
pub extern "c"
agent estimate_credit_score(applicant_id: String, balance: Float) -> Float:
    approve EstimateCreditScore(applicant_id, balance)
    return classify_score(applicant_id, balance)
```

This agent:

- Takes two scalar parameters (`String`, `Float`).
- Returns a scalar (`Float`).
- Declares an `@budget` cap (signable under
  `budget.compile_time_ceiling`) and an `@trust` constraint
  (signable under `trust.constraint_enforcement` per slice
  33Q3).
- Has an `approve` boundary (signable under
  `approval.dangerous_call_requires_token`).

`corvid build --target=cdylib --sign <key>` accepts it and emits a
DSSE-signed descriptor whose `claim_guarantees` array carries the
three guarantee ids above. `corvid claim --explain <output.so>`
enumerates them as the binary's enforced surface.

## Working around the struct restriction at v1.0

If your route or RPC naturally takes a struct (`HttpRequest`,
`OrderLine`, etc.), the pattern at v1.0 is:

1. Decompose the struct into scalar parameters at the boundary
   (`String` for opaque-id payloads, multiple `Int`/`Float` for
   structured numerics).
2. OR: serialize the struct to JSON in the host (the binding
   layer that calls into the cdylib) and pass it as a single
   `String` parameter. Have the Corvid agent parse the JSON
   internally — but be aware this pushes type discipline OFF the
   signed boundary, which is the opposite of Corvid's pitch. We
   recommend it as a stopgap, not as the production shape.

When 33Q8 ships, both workarounds become unnecessary — the
boundary natively accepts struct shapes with the same JSON
roundtrip happening at codegen time inside the signed boundary
instead of in the host.

## Related references

- [`cli.md`](./cli.md) — full `corvid build` command surface.
- [`core-semantics.md`](./core-semantics.md) — the full
  guarantee registry, including the three ids the worked example
  above signs against.
- [`inventions.md`](./inventions.md) — Phase 20n-C "native
  struct returns" entry, the internal codegen work this slice
  builds on.
- ROADMAP slice **33Q8** — the planned tightening that lifts
  this v1.0 restriction.
