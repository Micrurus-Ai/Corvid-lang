# `std.json` — Executing JSON surface

> **Status:** Phase 33R5b (closed 2026-06-10). 13 executing
> tools + the typed-decoder convention. Two RuntimeChecked
> guarantees: parse-safety (malformed input is recoverable,
> never panics) and field-type-safety (typed-accessor mismatches
> return Err, never coerce or panic). The umbrella ships BOTH
> the opaque-handle shape (for dynamic JSON) AND the typed-
> decoder convention (for typed APIs) so a Corvid program needs
> ZERO Python glue to consume JSON.

## Quick reference

The umbrella ships two complementary shapes:

### 1. Opaque path — for dynamic JSON

```corvid
import "./std/json" use json_parse, json_get_int, json_get_string

agent fetch_user_id(text: String) -> Result<Int, String>:
    parsed = json_parse(text)?
    id = json_get_int(parsed, "id")?
    return Ok(id)
```

Parse text into an opaque `JsonValue`, access fields via typed
getters that return `Result<T, String>`. Use the `?` postfix
operator (TryPropagate) for Result chaining.

### 2. Typed-decoder convention — for typed APIs

```corvid
effect json_decode_eff:
    reversible: true

type User:
    id: Int
    email: String

tool decode_user_from_json(text: String) -> Result<User, String> uses json_decode_eff

agent fetch_user(text: String) -> Result<User, String>:
    return decode_user_from_json(text)
```

The user declares the target struct + a tool matching the
`decode_<X>_from_json` pattern. The runtime intercepts the call
(via `is_typed_json_decoder_tool_call`) and routes through
`serde_json::from_str` + `json_to_value` against the declared
target type. **No per-type runtime handler exists** — the
dispatch is generic over the declared signature.

## The opaque shape

### `JsonValue` — opaque parsed JSON

`JsonValue` is an opaque, refcounted primitive type. The
typechecker carries it through agent and tool signatures, but
unlike `DbHandle` there is NO opacity gate at the
`json_to_value` boundary — `Value::JsonValue`'s payload IS the
JSON shape, not a key into a registry. The conversion
`json_to_value(json, Type::JsonValue, _)` produces
`Value::JsonValue(Arc::new(json))` as the natural identity
wrap.

The Arc lets multiple references share the same parsed JSON
without copying. PartialEq is STRUCTURAL — two JSON values
with the same shape are equal even if they were parsed from
different sources.

### `JsonBuilder` — mutable JSON object builder

`JsonBuilder` is an opaque, mutable builder for assembling
JSON objects. Returned by `json_object_new` and consumed by
`json_object_set_*` (fluent — mutates the inner Arc<Mutex<...>>
and returns the SAME builder) and `json_object_finish`
(snapshots the current state and serialises; the builder
remains usable).

The Arc<Mutex<...>> design lets multiple references to the same
builder all see each other's mutations. Snapshot semantics on
finish means there is no "consumed builder" lifecycle to
track — calling finish twice yields two independent strings
reflecting the builder's state at each call.

PartialEq is IDENTITY (`Arc::ptr_eq`) — structural equality
would race against concurrent mutations.

### The 13 executing tools

#### Parse path

`json_parse(text: String) -> Result<JsonValue, String> uses json_egress_read`

Parses `text` into a JsonValue. Returns `Err(message)` on
malformed input — never panics.

#### Typed accessors

```
json_get_int(value: JsonValue, field: String) -> Result<Int, String>
json_get_float(value: JsonValue, field: String) -> Result<Float, String>
json_get_string(value: JsonValue, field: String) -> Result<String, String>
json_get_bool(value: JsonValue, field: String) -> Result<Bool, String>
json_get_object(value: JsonValue, field: String) -> Result<JsonValue, String>
json_get_array(value: JsonValue, field: String) -> Result<List<JsonValue>, String>
```

Each accessor:
- Returns `Err` if `value` is not an object.
- Returns `Err` if the named field is missing.
- Returns `Err` if the field's JSON kind doesn't match the requested type.
- Otherwise returns `Ok(typed_value)`.

The error messages name the property violated so user code can
route a useful diagnostic up to its callers.

#### Builder path

```
json_object_new() -> JsonBuilder
json_object_set_int(builder: JsonBuilder, key: String, value: Int) -> JsonBuilder
json_object_set_float(builder: JsonBuilder, key: String, value: Float) -> JsonBuilder
json_object_set_string(builder: JsonBuilder, key: String, value: String) -> JsonBuilder
json_object_set_bool(builder: JsonBuilder, key: String, value: Bool) -> JsonBuilder
json_object_finish(builder: JsonBuilder) -> String
```

Fluent — `json_object_set_*` returns the same builder for
chaining. `json_object_finish` snapshots and serialises; the
builder remains usable for further set+finish cycles.

## The typed-decoder convention

A user declares a tool with the signature:

```corvid
tool decode_<X>_from_json(text: String) -> Result<X, String> uses <effect>
```

where:

- `<X>` is any Corvid type the runtime can convert from JSON
  (a user-declared struct, a primitive, a list, etc.).
- `<effect>` is any effect the user declares inline. Effects
  don't export via `use` — the runtime dispatch keys on the
  tool name pattern + return type, not the effect.

### How the dispatch fires

The interpreter's tool-call site recognises the pattern via
`is_typed_json_decoder_tool_call(callee_name, result_decode_ty)`,
which checks TWO conditions simultaneously:

1. Name matches `decode_*_from_json` (where `*` is non-empty).
2. Return type matches `Result<T, String>` for some T.

Both conditions together prevent the dispatch from silently
intercepting an unrelated user tool that happens to have one
or the other property.

When the gate fires:

1. The text argument is extracted.
2. `Type::Result(ok_ty, _err_ty)` is unpacked to get the target
   type T.
3. `serde_json::from_str(text)` is called. Failure → wrap in
   `Result::Err("malformed JSON in `<tool>`: ...")`.
4. `json_to_value(parsed, ok_ty, &types_by_id)` converts the
   parsed JSON to the typed Corvid Value. Type-shape mismatches
   → wrap in `Result::Err("JSON shape mismatch in `<tool>`: ...")`.
5. Success → wrap in `Result::Ok(typed_value)`.

The conversion uses the same `json_to_value` path that the io /
http / db dispatch surfaces use — handles structs, lists,
options, results, nested types.

### Worked example

```corvid
effect json_decode_eff:
    reversible: true

type Address:
    street: String
    city: String

type User:
    id: Int
    email: String
    address: Address

tool decode_user_from_json(text: String) -> Result<User, String> uses json_decode_eff

agent main(text: String) -> Result<String, String>:
    user = decode_user_from_json(text)?
    return Ok(user.email)
```

A single tool declaration handles arbitrary JSON shapes that
match the `User` type — including nested objects (the runtime
recursively converts JSON objects to Corvid structs).

## Safety properties

### 1. Parse safety — **`json.parse_safety_no_panic`**

`json_parse(text)` against arbitrary bytes returns
`Result::Err(message)` rather than panicking. The runtime's
parse path goes through `serde_json::from_str`; the typed-
decoder convention inherits the same property.

```corvid
import "./std/json" use json_parse

agent main(text: String) -> Result<JsonValue, String>:
    return json_parse(text)
```

`main("definitely not json")` returns `Err("malformed JSON:
...")`. The runtime never panics, never escapes — the caller
routes the error via `?` propagation or pattern-matching.

### 2. Field-type safety — **`json.field_type_safety_at_access_boundary`**

Each typed accessor returns `Result<T, String>` where the Err
branch fires on missing fields AND on type mismatches.

```corvid
import "./std/json" use json_parse, json_get_int

agent main() -> Result<Int, String>:
    parsed = json_parse("{\"name\": \"alice\"}")?
    id = json_get_int(parsed, "name")?  # Err: field is String, not Int
    return Ok(id)
```

`main()` returns `Err("field 'name' is not an Int (got
String)")`. Typed JSON access is safe at the runtime level even
though the JSON itself is dynamically typed.

The typed-decoder convention inherits this property — a
`decode_user_from_json("...")` call where the JSON has a String
where `User.id: Int` is declared returns `Result::Err` with a
structured diagnostic.

## Determinism

All 13 executing tools are non-deterministic from the
typechecker's perspective. The checker rejects calls inside a
`@deterministic` body — this is the existing
`decl_replayability.rs::is_deterministic_compatible` rule that
treats every tool call as non-deterministic by declaration kind.

The decoupling is intentional: `@deterministic` is the
language-level "this computes from declared inputs only"
annotation; tool calls are excluded by the existing rule
regardless of effect, even though JSON parse is deterministic
given the same input bytes.

Programs that need both deterministic logic AND JSON access
must factor the JSON calls into a separate, non-deterministic
agent.

## Replay quarantine

JSON parse / build are deterministic and process-internal —
there's no I/O, no network, no filesystem touch, no SQLite
mutation. A Substitute-mode replay runtime runs the JSON
dispatch IDENTICALLY to live mode: parse the same text →
produce the same value; build the same object → produce the
same string.

This is unlike the io / http / db surfaces, which all flip
quarantine flags during replay. JSON has no recorded side
effect to substitute against and no escape to block — the
property is structural and the fixture
`replay_does_not_block_executing_json_parse_or_builder_dispatch`
in `replay_quarantine_corpus.rs` pins it so a future refactor
that silently added a JSON quarantine flag would break this
test.

## Guarantees

Two RuntimeChecked rows in the canonical guarantee registry
([`docs/reference/core-semantics.md`](../core-semantics.md)):

| id | class | enforces |
|---|---|---|
| `json.parse_safety_no_panic` | RuntimeChecked | Malformed input returns `Result::Err`, never panics. |
| `json.field_type_safety_at_access_boundary` | RuntimeChecked | Typed-accessor mismatches return `Result::Err`, never coerce or panic. |

The `@deterministic`-rejection property is covered by the
existing `replay.deterministic_pure_path` guarantee — it
applies to all tool calls regardless of effect.

## Worked example — typed user store pipeline

The 33S4 batteries-quickstart tutorial weaves the JSON, HTTP,
and SQLite surfaces into a single pipeline:

```corvid
effect json_decode_eff:
    reversible: true

type User:
    id: Int
    email: String

import "./std/http" use http_get
import "./std/db" use db_open, db_execute, db_param_int, db_param_text

tool decode_user_from_json(text: String) -> Result<User, String> uses json_decode_eff

agent ingest_user(url: String, db_path: String) -> Result<Int, String>:
    # Fetch the JSON.
    response = http_get(url)
    # Decode into the typed struct (typed-decoder convention).
    user = decode_user_from_json(response.body)?
    # Persist via parameterised SQL.
    handle = db_open(db_path)
    db_execute(handle, "INSERT INTO users(id, email) VALUES (?, ?)", [db_param_int(user.id), db_param_text(user.email)])
    return Ok(user.id)
```

No Python glue. No string interpolation. The typed-decoder
catches shape mismatches; the executing SQLite surface catches
SQL-injection attempts structurally; the executing HTTP surface
gates the URL through `[http] allow`. End-to-end through
`corvid run`.

## Post-v1.0 — what's deliberately NOT in scope

- **Encoder for JsonValue.** Today the builder side produces
  JSON Strings; there's no `json_encode(value: JsonValue) ->
  String` for an already-parsed JsonValue. The pattern is to
  build a new `JsonBuilder` rather than re-encoding a parsed
  value. A follow-up slice may add `json_encode` if real
  programs need it.
- **Polymorphic typed decoder.** The typed-decoder convention
  fires on `decode_<X>_from_json` only. A `decode_either_from_json`
  shape (for sum types of JSON shapes) isn't in scope; users
  declare per-variant decoders and dispatch in user code.
- **JSON Path / JSONata / JMESPath.** The opaque-path accessors
  are field-level only (`json_get_int(value, "field")`). Nested
  access requires `json_get_object` + chained `json_get_*`. A
  query-language layer is post-v1.0.
- **cdylib codegen.** `JsonValue` / `JsonBuilder` are
  interpreter-only today (the codegen-cl backend emits a
  structured "interpreter-only in 33R5b; cdylib bridging lands
  in a follow-up slice" diagnostic). The C-ABI exports already
  exist in `crates/corvid-runtime/src/ffi_bridge/json_exports.rs`,
  so the cdylib wire-up is plumbing rather than primitives.

## Related references

- [`core-semantics.md`](../core-semantics.md) — full guarantee
  registry, including the two `json.*` rows.
- [`inventions.md`](../inventions.md) — invention proof matrix.
- `corvid tour --topic json` — runnable demo (offline; parses +
  builds without network or filesystem touch).
- `crates/corvid-runtime/src/json.rs` — `parse` / `get_*` /
  `object_*` source + inline guarantee anchors
  (`GUARANTEE_ID_JSON_*`).
- `crates/corvid-runtime/src/runtime/llm_dispatch.rs` —
  `Runtime::json_parse_tool` / `json_get_*_tool` /
  `json_object_*_tool` typed-Value dispatch methods.
- `crates/corvid-vm/src/interp.rs` — `dispatch_stdlib_json_tool`
  + `dispatch_typed_json_decoder` interpreter routing branches.
- `std/json.cor` — the tool declarations + envelope types.
