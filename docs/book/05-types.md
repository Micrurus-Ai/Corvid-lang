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

## Numbers

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

Float-to-string rendering always shows a decimal point or exponent
(`42.0` renders as `"42.0"`, never `"42"`) so string output — which
feeds LLM prompts and JSON — stays visibly typed.

Integer overflow, division by zero, and modulo by zero are **checked
in every build mode** and trap with a typed runtime error. There is
deliberately no saturating or wrapping mode: silent saturation
corrupts values that feed LLM calls, and a single arithmetic
semantics keeps deterministic replay byte-identical across the
interpreter and compiled tiers. `to_int_truncated()` follows the
same rule: NaN or out-of-range Floats trap rather than wrapping.

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

`append`, `sort`, and `reverse` mutate the shared list in place and
return `Nothing` (reference semantics — see the callout above).
`sort` is only offered on `Int`/`Float`/`String` element types.
`range(start, end)` is a builtin function producing a half-open
`List<Int>`.

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

## Maps

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

Construct with `Ok(x)` / `Err(e)`; the `?` operator propagates `Err`
early:

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

> **Planned — named-field literals and `..` update syntax land in
> slice 45n:**

```corvid-planned
d = Decision { refund: true, amount: 50.0, reason: "policy match" }
d2 = Decision { ..d, amount: 75.0 }
```

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

## Generics

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

> **Planned — lands in slice 45n:**

```corvid-planned
type CustomerId = String
type Cents = Int
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
