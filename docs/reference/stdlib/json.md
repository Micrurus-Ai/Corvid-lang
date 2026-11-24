# `std.json` — Executing JSON surface

> **Status:** Phase 33R5b (closed 2026-06-10) and B01 typed JSON.
> 13 executing tools plus `T.from_json` / `value.to_json()`. Two
> RuntimeChecked guarantees: parse-safety (malformed input is
> recoverable, never panics) and field-type-safety (shape and
> accessor mismatches return Err, never coerce or panic). Corvid
> ships BOTH the opaque-handle shape (for dynamic JSON) AND typed
> decoding (for typed APIs) so a program needs ZERO Python glue to
> consume JSON.

## Quick reference

The umbrella ships two complementary shapes:

### 1. Opaque path — for dynamic JSON

```corvid
import "std:json" use json_parse, json_get_int, json_get_string

agent fetch_user_id(text: String) -> Result<Int, String>:
    parsed = json_parse(text)?
    id = json_get_int(parsed, "id")?
    return Ok(id)
```

Parse text into an opaque `JsonValue`, access fields via typed
getters that return `Result<T, String>`. Use the `?` postfix
operator (TryPropagate) for Result chaining.

### 2. Typed decoding — for typed APIs

```corvid
type User:
    id: Int
    email: String

agent fetch_user(text: String) -> Result<User, String>:
    return User.from_json(text).map_err(fn (e) -> e.message)
```

Declare the record and call `from_json` on its name. No import, tool
or effect is needed. The next section describes the methods.

## Typed decoding and encoding

Every type has `to_json`, and `from_json` is called on a type name.
Neither needs an import:

```corvid
agent read_order(text: String) -> String:
    return match Order.from_json(text):
        Ok(order) -> order.to_json()
        Err(problem) -> problem.message
```

| Call | Result |
| --- | --- |
| `T.from_json(text)` | `Result<T, JsonError>`. Keys `T` does not declare are skipped, because external APIs add fields. |
| `T.from_json_strict(text)` | `Result<T, JsonError>`. The first undeclared key is an error. |
| `value.to_json()` | Compact JSON text with object keys in sorted order, so the same value always encodes to the same text. |

A `JsonError` has four `String` fields: `path` (such as
`lines[0].quantity`, empty when the problem is the whole text),
`expected`, `found`, and `message`, which combines them into one line.
Text that is not JSON fails at the root, and its `found` gives the line
and column. In an agent that returns `Result<_, String>`, write
`T.from_json(text).map_err(fn (e) -> e.message)?` to pass a failure
to the caller.

To decode a list, map or option at the top level, name it with a type
alias:

```corvid
type Orders = List<Order>

agent count(text: String) -> Int:
    return match Orders.from_json(text):
        Ok(orders) -> orders.length()
        Err(problem) -> 0
```

A public type from another module is called the way it is imported:
`Customer.from_json(text)` after `import "./models" use Customer`, or
`m.Customer.from_json(text)` after `import "./models" as m`. An alias
declared in another module is refused for now, because aliases from
other modules are not expanded in any type position yet. Declare the
alias in your own module instead, such as
`type Customers = List<m.Customer>`.

The checker refuses a call when the type has no JSON form in that
direction, naming the field that prevents it. `from_json` refuses a
type that contains `Grounded`, because JSON text cannot prove where a
value came from. Neither method accepts a type that contains a function,
stream, handle, upload or JSON builder. `to_json` on a value that
contains `Tainted` data returns `Tainted<String>`, so encoding never
removes taint. An `extend` block cannot declare a method named
`to_json`, `from_json` or `from_json_strict`.

The methods are pure, so `@deterministic` code may call them and replay
reproduces their results. Encoding a `Float` that is NaN or infinite
stops the program, because JSON has no form for it. The native target
is not supported yet, so programs that use these methods run on the
interpreter.

## Public JSON mapping

The public mapping is the JSON Corvid exchanges with code outside the
program. HTTP request bodies and JSON replies, `T.from_json` and
`to_json` use it, and the published OpenAPI schemas describe it.

| Corvid | JSON |
| --- | --- |
| `Int` | integer; a fractional or out-of-range number is an error |
| `Float` | number; an integer is accepted |
| `String`, `Bool` | string, boolean |
| `Nothing` | `null` |
| record | object with one key per field |
| `Option<T>` | the `T` value, or `null` for `None`; a record field may also be left out |
| `List<T>` | array |
| `Map<String, V>` | object |
| `Map<K, V>` with another key type | array of `[key, value]` pairs |
| `Result<T, E>` | `{"tag": "ok", "ok": T}` or `{"tag": "err", "err": E}`; `tag` is optional on input but must agree with the key present |
| sum type | `{"tag": "variant", "type": "<Type>", "variant": "<Variant>", "fields": [...]}` with payload fields in declaration order; on input only `variant` and, for a payload, `fields` are required |
| `Grounded<T>` | `{"value": T, "sources": [...]}` on output only |
| `JsonValue` | the JSON value itself |

Decoding never invents provenance. A `Grounded<T>` target is rejected
because JSON text cannot show where a value came from. Functions,
handles, streams and JSON builders have no JSON form.

A decode error names the first problem by path, such as `items[2].price`
or `["user id"]` for a key that is not an identifier, with the expected
type and what was found. Field refinements are checked at the same
path. HTTP routes and `from_json_strict` reject keys a record does not
declare, as the published schemas state `additionalProperties: false`.

Prompt structured output, tool arguments and recorded traces use
Corvid's internal envelope instead, which represents `Option` as
`{"tag": "some", "value": …}` or `{"tag": "none"}`. Records, lists,
scalars and sum types have the same form in both.

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

## Retired: the `decode_<Type>_from_json` tool convention

Before `T.from_json` existed, declaring
`tool decode_<X>_from_json(text: String) -> Result<X, String>` made
the runtime decode JSON in place of calling the tool. That convention
is retired. It decoded through the internal envelope instead of the
public mapping, reported errors as text with no field path, and was
triggered by a tool's name rather than anything the program states.

A tool declared in that shape is now a compile error that names the
replacement. A tool with such a name but another return type was never
decoded by the runtime and is unaffected. To migrate, delete the tool
and its effect, then change each call:

```corvid
# before
user = decode_user_from_json(text)?

# after
user = User.from_json(text).map_err(fn (e) -> e.message)?
```

For a list or map target, name it first with a type alias, such as
`type Users = List<User>`, and call `Users.from_json(text)`.

## Safety properties

### 1. Parse safety — **`json.parse_safety_no_panic`**

`json_parse(text)` against arbitrary bytes returns
`Result::Err(message)` rather than panicking. The runtime's
parse path goes through `serde_json::from_str`. Typed decoding
holds the same property: `T.from_json` on malformed text returns
`Err` with a `JsonError` at the root.

```corvid
import "std:json" use json_parse

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
import "std:json" use json_parse, json_get_int

agent main() -> Result<Int, String>:
    parsed = json_parse("{\"name\": \"alice\"}")?
    id = json_get_int(parsed, "name")?  # Err: field is String, not Int
    return Ok(id)
```

`main()` returns `Err("field 'name' is not an Int (got
String)")`. Typed JSON access is safe at the runtime level even
though the JSON itself is dynamically typed.

Typed decoding holds the same property: `User.from_json(text)` on
JSON with a String where `User.id: Int` is declared returns `Err`
with a `JsonError` whose `path` is `id`.

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
type User:
    id: Int
    email: String

import "std:http" use http_get
import "std:db" use db_open, db_execute, db_param_int, db_param_text

agent ingest_user(url: String, db_path: String) -> Result<Int, String>:
    # Fetch the JSON (Result: transport/policy failures are Err values).
    response = http_get(url)?
    # Decode into the typed record; a mismatch names the field.
    user = User.from_json(response.body).map_err(fn (e) -> e.message)?
    # Persist via parameterised SQL (Result: SQL errors are Err values).
    handle = db_open(db_path)?
    db_execute(handle, "INSERT INTO users(id, email) VALUES (?, ?)", [db_param_int(user.id), db_param_text(user.email)])?
    return Ok(user.id)
```

No Python glue. No string interpolation. Typed decoding
catches shape mismatches; the executing SQLite surface catches
SQL-injection attempts structurally; the executing HTTP surface
gates the URL through `[http] allow`. End-to-end through
`corvid run`.

## Post-v1.0 — what's deliberately NOT in scope

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
  interpreter routing branch.
- `crates/corvid-vm/src/public_json.rs` — the public mapping that
  `from_json` and `to_json` use.
- `std/json.cor` — the tool declarations + envelope types.
