# Types

Every fenced code block marked `corvid` compiles through the real
driver in CI. Blocks under a **Planned** marker show designed syntax
that is not yet implemented — each names the roadmap slice that
ships it.

## Built-in primitives

| Type | Description | Literal |
|---|---|---|
| `Int` | signed 64-bit integer | `42`, `-1` |
| `Float` | 64-bit IEEE-754 | `3.14`, `1000000.0` |
| `Bool` | boolean | `true`, `false` |
| `String` | UTF-8 string | `"hello"`, `"""multi-line"""` |
| `Nothing` | the unit type | (returned implicitly when nothing is) |

## Generic built-ins

| Type | Description |
|---|---|
| `List<T>` | ordered sequence |
| `Stream<T>` | incremental sequence — see **[Streaming](/docs/streaming)** |
| `Option<T>` | `Some(T)` or `None` |
| `Result<T,E>` | `Ok(T)` or `Err(E)` |
| `Grounded<T>` | value with provenance — see **[Grounded](/docs/grounded)** |
| `Map<K,V>` | key→value map — `m[k]` reads as `Option<V>`, `m[k] = v` inserts or updates |

## Strings

```corvid
agent greet(name: String) -> String:
    greeting = "hello, " + name + "!"
    return greeting
```

Multi-line strings use triple quotes; all bytes between the quotes are
preserved:

```corvid
agent policy_text() -> String:
    text = """
        multi-line
        raw bytes
    """
    return text
```

Concatenation with `+` requires both sides to be `String` — there is
no implicit number-to-string coercion (explicit conversions are
Planned, see Numbers below).

The string method set (every block below compiles in CI):

```corvid
agent demo(s: String) -> String:
    n = s.length()                        # Int — Unicode scalars, not bytes
    parts = "a,b,c".split(",")            # List<String>
    has = s.contains("ell")               # Bool
    starts = s.starts_with("he")          # Bool
    ends = s.ends_with("!")               # Bool
    cleaned = s.trim()                    # strip Unicode whitespace
    swapped = s.replace("old", "new")     # every occurrence
    piece = s.substring(0, 5)             # scalar indices, clamped
    return cleaned.to_upper() + piece.to_lower()
```

Semantics worth knowing: indices and lengths count Unicode scalar
values (like Python's `len(str)`), never UTF-8 bytes; casing is full
Unicode; `split` with an empty separator traps at runtime (iterate
with `for c in s` to walk characters); `substring` clamps
out-of-range indices and returns `""` when `start >= end`.

`String.length()` works on both CLI targets for stored strings and temporary expressions. For example, `("🐧" + "é").length()` returns `2`.

Both targets also support `substring`, `to_upper` and `to_lower`, including inside match arms. Substring bounds are evaluated left to right. For example, `"A🐧B".substring(1, 2)` returns `"🐧"`. Case conversion can change length, as in `"ß".to_upper() == "SS"`.

The interpreter and native CLI both support trimming, searching, splitting and replacing strings. Split keeps leading, adjacent and trailing empty fields. For example, `",a,,".split(",")` produces `["", "a", "", ""]`. Splitting empty input with a nonempty separator gives `[""]`. The search methods match literal text, and `replace` changes all non-overlapping matches.

## Numbers

The native CLI and interpreter both support `Int.to_float()` and the four float-to-integer methods `to_int_truncated()`, `floor()`, `ceil()` and `round()`. Truncation rounds toward zero. Integer results must fit in a signed 64-bit value. For example, converting `9223372036854775808.0` to an integer raises an error, while `(-9223372036854775808.0).to_int_truncated()` succeeds.

```corvid
agent ratio(n: Int) -> Float:
    f = 3.14
    return f
```

`Int` widens to `Float` implicitly where a `Float` is expected.
Every other conversion is explicit, with the method name spelling
out the behavior:

```corvid
agent conversions() -> Result<String, String>:
    n = 42
    f_from_n = n.to_float()                # Int -> Float
    n_from_f = 3.9.to_int_truncated()      # toward zero; traps on NaN/overflow
    count_text = "count: " + n.to_string() # Int -> String
    price_text = 19.5.to_string()          # "19.5" — Floats always show a `.`
    parsed = " 42 ".parse_int()?           # Result<Int, String>; trims whitespace
    ratio = "2.5".parse_float()?           # Result<Float, String>
    return Ok(count_text)
```

`String.parse_int()` works in both native and interpreted programs. It
trims surrounding Unicode whitespace and accepts a signed decimal integer.
Empty input, invalid digits, and values outside the signed 64-bit range
return `Err` with the original input in the message.

Float-to-string rendering always shows a decimal point or exponent
(`42.0` renders as `"42.0"`, never `"42"`) so string output — which
feeds LLM prompts and JSON — stays visibly typed.

The math methods live on the same table, under the same
always-checked rule — `abs` on the minimum Int traps, `pow` traps
on overflow or a negative exponent, `sqrt` of a negative traps
(a silent NaN would poison downstream arithmetic), and the
`Float`→`Int` conversions trap on NaN. `round` is half **away from
zero** (`2.5` → `3`, `-2.5` → `-3`), deliberately not Python's
half-to-even:

```corvid
agent math_tour(x: Float, n: Int) -> Int:
    magnitude = x.abs().sqrt()             # Float -> Float
    clamped = n.min(100).max(0)            # Int bounds
    grown = 2.pow(10)                      # checked: overflow traps
    lo = 3.7.floor()                       # 3 — Float -> Int
    hi = 3.2.ceil()                        # 4
    nearest = (-2.5).round()               # -3 — half away from zero
    return clamped + grown + lo + hi + nearest + magnitude.floor()
```

The **effectful** numeric surfaces — wall-clock time and
randomness — are deliberately NOT methods: they are stdlib tools
(`std/time`, `std/random`) so every read is traced, substituted
under replay, and rejected inside `@deterministic` agents. See the
[`std.time`](../reference/stdlib/time.md) and
[`std.random`](../reference/stdlib/random.md) references.

Ordinary integer arithmetic traps on overflow. Mark an agent with `@wrapping`
to make integer addition, subtraction, multiplication and unary negation wrap
at the signed 64-bit boundary. Division and modulo by zero still trap.
`to_int_truncated()` also traps on NaN or an out-of-range float.

## Lists

```corvid
agent sum(xs: List<Int>) -> Int:
    total = 0
    for x in xs:
        total = total + x
    return total

agent first_of_three() -> Int:
    xs = [1, 2, 3]
    return xs[0]
```

List literals, `Int` indexing, `+` concatenation of two lists, and
`for … in` iteration are shipped. Out-of-range indexing traps at
runtime with a typed error.

Use `xs.append(item)` to grow a list in a loop. The operation updates the
same list, so aliases see the new item. Native lists reserve more space as
they grow; appending takes amortized constant time. Use `xs + ys` when you
want a fresh list containing the elements of both inputs.

Elements are assignable in place, including compound forms:

```corvid
agent bump() -> Int:
    xs = [10, 20, 30]
    xs[1] = 99
    xs[2] += 1
    return xs[1] + xs[2]
```

Lists are shared heap cells — assigning a list to another binding or
storing it in a record field aliases the SAME list; mutation through
any alias is visible through all of them (reference semantics, as in
Python).

The list method set (compiles in CI):

```corvid
agent list_demo() -> Int:
    xs = [3, 1, 2]
    xs.append(4)                          # in place — every alias sees it
    xs.sort()                             # in place; Int/Float/String elements
    xs.reverse()                          # in place
    n = xs.length()                       # Int
    head = xs.first()                     # Option<Int>
    tail = xs.last()                      # Option<Int>
    has = xs.contains(2)                  # Bool
    mid = xs.slice(1, 3)                  # new list, clamped indices
    names = ["a", "b"]
    csv = names.join(",")                 # List<String> only
    total = 0
    for i in range(0, 5):                 # counted iteration: 0,1,2,3,4
        total = total + i
    return n + total

```

`slice(start, end)` copies the selected slots into an independent list. The end is exclusive, and reversed or empty ranges produce an empty list. In the interpreter, the copying cost grows with the number of selected elements.

`first()` and `last()` return an `Option<T>` on both the native and interpreter targets for supported element types. For example, `[3, 0].last()` returns `Some(0)`. Both methods return `None` for an empty list. An extracted string remains usable after its list goes out of scope.

`append`, `sort`, and `reverse` mutate the shared list in place and
return `Nothing` (reference semantics — see the callout above).
`sort` is only offered on `Int`/`Float`/`String` element types.
`range(start, end)` is a builtin function producing a half-open
`List<Int>`.

Both CLI targets support `List<Int>.map` with an integer callback. For example, `[1, 2].map(fn (n) -> n + 3)` returns `[4, 5]`. The method applies the callback in list order, returns an independent list, and skips the callback on an empty list.

On `List<Int>`, both targets also support the following calls.

| Call | Result |
|---|---|
| `[3, 1, 4].filter(fn (n) -> n > 2)` | A new list containing `[3, 4]` in input order |
| `[3, 1, 4].fold(0, fn (acc, n) -> acc * 10 + n)` | `314`, using each item from left to right |
| `[3, 1, 4].any(fn (n) -> n == 1)` | `true`, stopping at the first match |
| `[3, 1, 4].all(fn (n) -> n > 1)` | `false`, stopping at the first failed test |

For an empty list, `filter` returns an empty list, `fold` returns its initial accumulator, `any` returns `false`, and `all` returns `true`. None of these calls invokes the callback on empty input.

The lambda-taking methods take `fn (x) -> expr` lambdas and are
fully checked — `filter`'s predicate must return `Bool`, `map`'s
result element type is inferred from the lambda body, and `fold`'s
accumulator type comes from its `init` argument. `any`/`all`
short-circuit. Lambdas are first-class values: store one in a
variable (with an optional `(Int) -> Int` function-type annotation)
and call it like any function. Captures are by-value snapshots
taken when the lambda is created — heap cells still share, so a
captured list observes later mutations (compiled in CI):

```corvid
agent transform(xs: List<Int>) -> Int:
    ys = xs.map(fn (x) -> x * 2)
    zs = xs.filter(fn (x) -> x > 1)
    total = xs.fold(0, fn (acc, x) -> acc + x)
    has_big = xs.any(fn (x) -> x > 100)
    base = 10
    add_base: (Int) -> Int = fn (n) -> n + base
    if has_big and xs.all(fn (x) -> x > 0):
        return total
    return add_base(ys.length() + zs.length())
```

### Updating list items

A numeric local accepts compound arithmetic such as `total *= 2` and `total -= 1`. The same operators work on numeric list items.

Use `items[i] = value` to replace an item or `items[i] += amount` to add to a numeric item. Other variables referring to the same list see the update.

```corvid
agent main() -> Int:
    scores = [4, 6]
    scores[1] += 3
    return scores[1]
```

This returns `9`. In `items[position()] += amount()`, Corvid calls `position()` first and `amount()` second, once each. It then checks the index and reads the current item, including any change made by `amount()`. The index must be at least zero and less than the list length at the time of the write. The native target supports one index on a local list. Nested indexed writes such as `rows[i][j] += 1` currently use the interpreter.

## Maps

Both the native compiler and interpreter support creating a `Map<String, String>`, passing it to a function, returning it, and looking up a key with `names[id]` or `names.get(id)`. Both reads return `Option<String>`. The last value for a repeated key wins. A stored empty string produces `Some("")`, and an absent key produces `None`.

```corvid
agent display_name(names: Map<String, String>, id: String) -> String:
    return match names[id]:
        Some(name) -> name
        None -> "Unknown user"
```

Assign to a key to insert or replace a value. A function that receives the map can update it, and every alias sees the change. A string you read before replacement keeps its original value.

```corvid
agent rename(names: Map<String, String>, id: String, name: String) -> Nothing:
    names[id] = name

agent main() -> String:
    names: Map<String, String> = {}
    names["owner"] = "Ada"
    rename(names, "owner", "Grace")
    return match names["owner"]:
        Some(name) -> name
        None -> "Unknown user"
```

This program returns `"Grace"` on both targets. Native keyed reads and writes take linear time in the worst case.

For named counts, use `Map<String, Int>`. Indexing and `.get()` return `Option<Int>`, and `.values()` returns a `List<Int>` snapshot in insertion order. Both targets run this example and return `1`.

```corvid
agent main() -> Int:
    counts: Map<String, Int> = {}
    counts["files"] = 0
    return match counts["files"]:
        Some(n) -> n + 1
        None -> -1
```

Native maps support string keys with string, integer or record values, and lists or options of those types. A lookup returns the stored type inside `Option`. A missing key returns `None` even when the stored type is itself optional. This program returns `"unset"` on both targets.

```corvid
agent describe(limit: Option<Int>) -> String:
    return match limit:
        Some(_) -> "set"
        None -> "unset"

agent main() -> String:
    limits: Map<String, Option<Int>> = {"daily": None}
    return match limits["daily"]:
        Some(limit) -> describe(limit)
        None -> "missing"
```

If a value is a list, retrieving it preserves its identity. Appending through the retrieved list updates the same list held by the map. A `values()` snapshot keeps the original records and lists alive when a key is overwritten. For `Map<String, Int>`, both targets support `+=`, `-=`, `*=`, `/=` and `%=`. In `counts[key()] += amount()` the key runs first, followed by the amount. The update reads the current counter after both calls. A missing key raises a runtime error. Use plain assignment to initialize it. Earlier lookups and value snapshots keep the previous integer. Other key or value types outside the native subset use the interpreter.

For a string map, `length() -> Int` counts distinct keys and `contains_key(key: String) -> Bool` tests presence even if the stored value is empty. `keys() -> List<String>` and `values() -> List<String>` return snapshots in matching insertion order on both targets. An overwrite retains the key's original position. For example, `{"a": "old", "b": "", "a": "new"}.keys()` gives `["a", "b"]`, and its values are `["new", ""]`. Use `for key in names.keys():` to visit each snapshotted key once.

Maps are Python-style literals with typed reads that can never
throw a KeyError or hand back a silent zero-value (every block
compiles in CI):

```corvid
agent map_demo() -> Int:
    m = {"a": 1, "b": 2}
    m["c"] = 3                        # insert-or-update place assignment
    m["b"] += 5                       # compound on an existing key
    v = m["a"]                        # Option<Int> — absence is None, never a trap
    exists = m.contains_key("a")      # Bool
    gone = m.remove("a")              # Option<Int>
    total = 0
    for k in m.keys():                # insertion-order List<String>
        total = total + 1
    return m.length() + total
```

Semantics worth knowing: `m[k]` READS as `Option<V>` (handle absence
with `?` or `==`), while `m[k] = v` WRITES as insert-or-update — the
safest read and the easiest write. Duplicate keys in a literal: the
last one wins. Maps are shared heap cells (reference semantics, like
lists) with structural key equality and insertion-order iteration.
`Map<String, V>` round-trips with JSON objects. Values with
non-comparable types work as keys too (structural equality).

## Option

Construct with `Some(x)` / `None`; consume with postfix `?`, which
propagates `None` to a caller that itself returns `Option`:

```corvid
agent find_positive(xs: List<Int>) -> Option<Int>:
    for x in xs:
        if x > 0:
            return Some(x)
    return None

agent double_positive(xs: List<Int>) -> Option<Int>:
    x = find_positive(xs)?
    return Some(x * 2)
```

`match` consumes an Option exhaustively — defaulting at the point of
use no longer requires propagation:

```corvid
agent count_or_zero(o: Option<Int>) -> Int:
    return match o:
        Some(x) -> x
        None -> 0
```

> **Planned — the `unwrap_or` / `is_some` method shorthands land in
> slice 45l:**

```corvid-planned
x = find_positive(xs).unwrap_or(0)
```

## Result

Construct a result with `Ok(x)` or `Err(e)`. Postfix `?` unwraps an
`Ok` or returns the error from the enclosing function. Inside a loop,
an error exits the whole function. In an expression such as
`first() + text.parse_int()? + last()`, `first()` runs before the parse.
If parsing fails, Corvid skips `last()` and releases the function's locals.
In `enabled and text.parse_int()? > 0`, a false `enabled` skips the
parse entirely. An `or` expression skips its right operand when its left
operand is true.

```corvid
agent check_amount(amount: Float) -> Result<Float, String>:
    if amount > 100.0:
        return Err("amount exceeds the auto-approve limit")
    return Ok(amount)

agent apply_fee(amount: Float) -> Result<Float, String>:
    approved = check_amount(amount)?
    return Ok(approved + 2.5)
```

For retry semantics around a fallible call, `try … on error retry N
times backoff linear|exponential` wraps any expression.

## Record types

Declare with `type`, construct positionally (arguments in field
order), access with `.field`:

```corvid
type Decision:
    refund: Bool
    amount: Float
    reason: String

agent decide() -> Float:
    d = Decision(true, 50.0, "policy match")
    return d.amount
```

Fields are assignable in place, including through nested paths and
compound operators:

```corvid
type Wallet:
    balance: Float

type Account:
    wallet: Wallet

agent adjust() -> Float:
    acct = Account(Wallet(100.0))
    acct.wallet.balance = 250.0
    acct.wallet.balance += 50.0
    return acct.wallet.balance
```

Records are shared heap cells: `alias = acct` aliases the SAME
account, and `alias.wallet.balance *= 2.0` is visible through
`acct` too (reference semantics, as in Python). A compound
assignment evaluates its target path exactly once.

Named-field literals construct records with every field labeled
(any order), and `..base` copies the remaining fields from an
existing value into a NEW cell — the base is untouched. A bare
field name is shorthand for `field: field`. The spread goes last
(compiled in CI):

```corvid
type Decision:
    refund: Bool
    amount: Float
    reason: String

agent build(base: Decision) -> Float:
    amount = 50.0
    d = Decision { refund: true, amount, reason: "policy match" }
    d2 = Decision { amount: 75.0, ..d }
    Decision { refund, amount: final_amount, .. } = d2
    if refund:
        return final_amount
    return base.amount
```

The last line is a **destructuring binding** — the same
`Type { ... }` surface, on the left of `=`, binding fields to
locals (`amount: final_amount` renames; `..` ignores the rest).
Destructuring must be irrefutable: literal or nested patterns
belong in `match`.

### Updating a record

Use a field name to update a record. Other variables referring to the same record see the change.

```corvid
type Counter:
    value: Int
    label: String

agent main() -> Int:
    counter = Counter(2, "draft")
    alias = counter
    counter.label = "ready"
    counter.value += 3
    return alias.value
```

This returns `5`. Updates can follow nested fields, as in `state.counter.value += step()`. Corvid evaluates `step()` once before reading the field for the addition. If the call replaces `state.counter`, the update applies to the replacement.

## Field refinements

Record fields can declare VALUE constraints, not just shapes:

```corvid
type Person:
    age: Int where between(0, 150)
    name: String where len_between(1, 80)
```

`between(min, max)` refines `Int` fields; `len_between(min, max)`
refines `String` fields (character count); bounds are inclusive. A
refinement whose form doesn't fit the field's type is a compile
error.

Refinements are enforced where structured data enters the program —
typed prompt-output decode and the typed-decoder convention. A
structurally valid but out-of-range value refuses to decode with a
message naming the field, the value, and the refinement:

```text
field `age` on `Person`: value 969 violates the declared refinement `between(0, 150)`
```

That exact message is what `with repair N` (see the prompts chapter)
feeds back to the model, so refined prompt outputs heal until they
are structurally AND semantically valid — with every repair attempt
still counted against `@budget`.

## Sum types (enums)

Sum types declare "one of N shapes" — unit variants are bare
values, payload variants construct like calls, and equality is
structural (this block compiles in CI):

```corvid
type Status:
    | Pending
    | Approved(approver: String)
    | Denied(reason: String, code: Int)

agent triage() -> Bool:
    s = Approved("alice")
    p = Pending
    d = Denied("policy", 42)
    return s == Approved("alice") and not (s == p)
```

A type declaration is a record XOR a sum — mixing field lines and
variant lines is a parse error. Variant names are file-scope
constructors, so two sum types cannot share a variant name (v1
limitation, diagnosed as a duplicate declaration).

`match` consumes variants with compiler-checked exhaustiveness (see
**[Pattern matching](/docs/pattern-matching)**):

```corvid
type Verdict:
    | Waiting
    | Cleared(officer: String)

agent report(v: Verdict) -> String:
    return match v:
        Waiting -> "waiting"
        Cleared(who) -> "cleared by " + who
```

An imported function can return a sum value and another function can accept it. The interpreter and native CLI both preserve record payloads across those calls. Use the interpreter when the CLI entry point itself takes or returns a sum as JSON.

Use `==` or `!=` to compare two values of the same record or sum type. For example, `Denied("policy", 42) == Denied("policy", 42)` is true, and `Pending != Approved("alice")` is true. Both CLI targets compare the active variant and its fields. List fields compare in order. String-keyed map fields compare by keys and values regardless of insertion order. A later comparison sees any mutations made through an alias.

These operators also work directly on typed lists, string-keyed maps, options and results of supported types. Two separately constructed `Some(0)` values compare equal. An `Ok` value differs from an `Err` value, even if their payloads are equal.

### Sum values in HTTP JSON

A route can accept and return the same sum type directly.

```corvid
type Status:
    | Pending
    | Approved(approver: String)

server status_api:
    route POST "/echo" body Status -> json Status:
        return body
```

Posting either of these JSON objects to `/echo` returns the same object.

```json
{"tag":"variant","type":"Status","variant":"Pending","fields":[]}
```

```json
{"tag":"variant","type":"Status","variant":"Approved","fields":["alice"]}
```

Payload fields follow declaration order. When the request type is known,
a client may omit `type` and `tag`. An empty variant may also omit `fields`.
An unknown variant, conflicting type or tag, or incorrect payload receives
HTTP 400 with the error code `invalid_body`. For a public sum type,
`/openapi.json` describes each variant and its positional payload types.

## Generics

For built-in collections, the element type follows the value through imports. If `items.labels()` returns `List<String>`, then `label = items.labels().last()` infers `Option<String>`, and `Some(s)` binds a string. The same method on `List<Int>` infers `Option<Int>`.

> **Planned — post-v1.0.** User-declared generics need a
> type-variable representation in the checker and monomorphization
> through all backends; the decision and scope live in the ROADMAP's
> post-v1.0 section (45p resolution). The built-in generic heads
> (`List`, `Option`, `Result`, `Stream`, `Grounded`) cover the v1.0
> workload.

```corvid-planned
fn first<T>(xs: List<T>) -> Option<T>:
    if xs.length() > 0:
        return Some(xs[0])
    return None
```

## Type aliases

Aliases name a type without wrapping it — `CustomerId` IS
`String`, everywhere (transparent, not a newtype). Alias cycles
are a compile error, and an alias is not a constructor (compiled
in CI):

```corvid
type CustomerId = String
type Cents = Int

agent describe(id: CustomerId, price: Cents) -> String:
    return id.to_upper() + ": " + price.to_string()

agent main() -> String:
    return describe("c-42", 1999)
```

## Type inference and annotations

Local bindings don't need annotations — the type is inferred from the
initializer:

```corvid
agent inference_demo() -> Int:
    n = 42                               # inferred Int
    xs = [1, 2, 3]                       # inferred List<Int>
    return n + xs[0]
```

When you want the contract explicit, annotate the binding — the same
`name: Type` shape fields and params use. The checker verifies the
initializer agrees with the annotation (a mismatch is a compile
error), and the annotation becomes the binding's type:

```corvid
agent annotated_demo() -> Float:
    n: Int = 42
    xs: List<Int> = [1, 2, 3]
    f: Float = 42                        # Int widens into the Float slot
    return f
```

There is deliberately no `let` keyword — one binding form, coherent
with the rest of the surface.

Function signatures, record fields, and effect rows always require
explicit types — they're the boundaries where inference would be
ambiguous and where you want a checker-readable contract.

### Releasing a stream

Consumers of a `Stream<T>` share the receiver. A producer can continue
while another consumer holds a reference. Dropping the last receiver
wakes a producer waiting to yield into a full buffer. Its next send
ends the producer's yield loop. Resumable prompt streams keep delivered
chunks for their resume token.
