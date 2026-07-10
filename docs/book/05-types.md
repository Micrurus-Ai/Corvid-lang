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
| `Map<K,V>` | key→value map — **Planned, slice 45g** |

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

> **Planned — the lambda-taking methods land in slice 45j:**

```corvid-planned
ys = xs.map(fn (x) -> x * 2)             # 45j — needs lambdas
zs = xs.filter(fn (x) -> x > 1)          # 45j
```

## Maps

> **Planned — the `Map<K,V>` type, `{...}` literals, and map methods
> land in slice 45g.** Until then, the typed key→value shapes are
> user-declared record types, and dynamic string→value data goes
> through the `std/json` surface.

```corvid-planned
m: Map<String, Int> = {"a": 1, "b": 2}
v: Option<Int> = m.get("a")
exists: Bool = m.contains_key("a")
keys: List<String> = m.keys()
```

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

> **Planned — `match` consumption lands in slice 45i and
> `unwrap_or` / `is_some` in slice 45l:**

```corvid-planned
match find_positive(xs):
    Some(x) -> "found"
    None    -> "not found"

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

> **Planned — user-declared sum types land in slice 45h, and `match`
> over them in slice 45i.** Until then, the built-in `Option` and
> `Result` are the sum types, and "one of N shapes" is modelled with
> a record carrying a tag field.

```corvid-planned
type Status:
    | Pending
    | Approved(approver: String)
    | Denied(reason: String)

match status:
    Pending           -> "waiting"
    Approved(who)     -> "approved by " + who
    Denied(reason)    -> "denied: " + reason
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
