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
| **Parameter** | `Int`, `Float`, `Bool`, `String`, **user-declared structs whose fields are all `Int` / `Float` / `Bool` / `String`** |
| **Return** | `Int`, `Float`, `Bool`, `String`, `Grounded<Int>`, `Grounded<Float>`, `Grounded<Bool>`, `Grounded<String>`, `Nothing`, **user-declared structs whose fields are all `Int` / `Float` / `Bool` / `String`** |

Slice 33Q8 lifted the struct boundary (filed by maintainer-as-reviewer-
2026-06-05 P1.3). The lift reuses Phase 20n-C's per-struct JSON
decoder + encoder so a struct parameter arrives at the cdylib as a
**caller-owned `const char*` JSON buffer** and a struct return leaves
as a **Corvid-owned `const char*` JSON buffer** the caller frees via
`corvid_free_string(...)`. The generated C header documents the JSON
schema for each struct boundary as a block comment so a C caller knows
the exact field shape without reading the `.cor` source.

### Struct boundary contract

When a struct parameter or return uses the JSON wire:

- **Parameter**: the C caller passes UTF-8, null-terminated JSON
  matching the schema in the generated `.h`. The cdylib's wrapper
  decodes the JSON; if the JSON is malformed (parse failure or
  missing required field), the wrapper **traps** the process. The C
  caller is responsible for sending well-formed JSON; richer
  error-out-parameter wiring is filed for a follow-up FFI slice.
- **Return**: the cdylib serializes the struct to UTF-8 JSON and
  hands the C caller a pointer to a Corvid-owned buffer. Free it
  via `corvid_free_string(...)`.

### What's still NOT accepted at v1.0

- **Struct fields that aren't scalars.** Nested structs, `List<T>`,
  `Option<T>`, and other rich field shapes are rejected at typecheck
  time because Phase 20n-C's encoder/decoder doesn't yet support
  them at the wire. Promoting these is a follow-up FFI slice.
- **Top-level `List<T>`, `Option<T>`, generic parameters**. Same
  reason — the JSON encoder/decoder family currently supports
  scalar fields only. Wrap them in a one-field struct as a stopgap.
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

## Worked example — struct boundary (33Q8)

```corvid
type Ticket:
    id: String
    amount: Int

type Receipt:
    ok: Bool
    note: String

@budget($0.20)
pub extern "c"
agent finalize_ticket(ticket: Ticket @borrowed) -> Receipt:
    return Receipt(true, ticket.id)
```

`corvid build --target=cdylib` produces a library exporting:

```c
// agent finalize_ticket(ticket: struct) -> struct
// JSON shape for parameter `ticket`:
//   {
//     "type": "object",
//     "properties": { "id": {"type": "string"}, "amount": {"type": "integer"} },
//     "required": ["id", "amount"],
//     "additionalProperties": false
//   }
// JSON shape for return value `return`:
//   {
//     "type": "object",
//     "properties": { "ok": {"type": "boolean"}, "note": {"type": "string"} },
//     "required": ["ok", "note"],
//     "additionalProperties": false
//   }
const char* finalize_ticket(const char* ticket, uint64_t* out_observation_handle);
```

The C caller passes a JSON string matching the parameter schema and
receives a JSON string matching the return schema. Both buffers are
UTF-8 + null-terminated; the return must be freed via
`corvid_free_string(...)`.

## Related references

- [`cli.md`](./cli.md) — full `corvid build` command surface.
- [`core-semantics.md`](./core-semantics.md) — the full
  guarantee registry, including the three ids the worked example
  above signs against.
- [`inventions.md`](./inventions.md) — Phase 20n-C "native
  struct returns" entry, the internal codegen work this slice
  builds on.
- ROADMAP slice **33Q8** — the closed slice that shipped the
  struct boundary.
