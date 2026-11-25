# Types

Use `choice` when a value can take one of several forms. Give each option its own indented line. An option can carry named data.

```corvid
choice Action:
    Lookup(shelf: Int)
    Suggest(title: String)
```

For example, `Lookup(2)` carries a shelf number. `Suggest("Persuasion")` carries a title. Use `match` to handle the options. Existing sum types declared with `type` and `|` use the same constructors and matching rules.


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

Integer arithmetic is checked by default. The smallest `Int` divided by `-1` overflows. Its remainder modulo `-1` is zero. A zero divisor is an error for both operators. In an `@wrapping` agent, integer addition, subtraction, multiplication and negation wrap at 64 bits. The Python target follows these integer rules too. For example, `-7 / 3` returns the integer `-2` and `-7 % 3` returns `-1`. In generated Python, checked integer overflow raises `OverflowError` and a zero divisor raises `ZeroDivisionError`.

For `Float`, division by zero gives a signed infinity when the numerator is positive or negative. Zero divided by zero gives NaN. The remainder has the sign of the dividend, so `-7.5 % 2.0` gives `-1.5`. A zero divisor or an infinite dividend gives NaN for remainder. These rules apply in the interpreter and generated Python.

`to_int_truncated()` rounds a float toward zero. `floor()` rounds down, `ceil()` rounds up, and `round()` rounds to the nearest integer with ties away from zero. For example, `(-2.5).round()` returns `-3`. Each conversion requires a finite result from -2^63 inclusive to 2^63 exclusive. `to_float()` converts an integer to a binary64 float, which can lose precision when the integer magnitude exceeds 2^53.

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

Use `first()` or `last()` when the list may be empty. Both return
`Option<T>`. An empty list gives `None`, and a list ending in zero
gives `Some(0)`. A selected string remains usable after the list goes
out of scope.

```corvid
agent last_number(items: List<Int>) -> Int:
    return match items.last():
        Some(number) -> number
        None -> -1
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

A `for` loop visits a snapshot of the list entries taken at loop entry.
Appending or replacing a slot during the loop affects the original list.
The remaining iterations still visit the original entries. A snapshot
of records shares the records themselves. With a list of strings, replacing
a slot through an alias leaves the current loop variable and the remaining
iterations holding their original strings.

List indices in a write are evaluated once, from left to right. Corvid
then evaluates the right-hand side and updates the current list at those
indices. If a function replaces a row, a nested write updates the new row.
Negative indices and indices at or beyond the list length raise an error.

```corvid
agent update(rows: List<List<Int>>) -> Int:
    rows[0][0] += 4
    return rows[0][0]
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

When building for Python, a map literal with `String`, `Int` or `Bool` keys
creates a Python dictionary. A Python caller can use the returned dictionary
directly. Keys retain their first insertion position when a later entry
replaces the value. Use the interpreter for structural keys, Corvid map
methods, map mutation and lookups that need Corvid's `Option` semantics.

For `Map<String, String>`, `length()` counts distinct keys.
`contains_key(key)` also finds keys whose value is the empty string.
`keys()` and `values()` produce new lists in first-insertion order.
Replacing a value preserves its position. A snapshot keeps the strings it
contained when you took it, even if another variable updates the same map.

```corvid
agent snapshot() -> String:
    labels = {"first": "original"}
    saved = labels.values()
    alias = labels
    alias["first"] = "updated"
    return saved[0]
```

Use `table[key]` or `table.get(key)` for a typed lookup. Both return
`Option<V>` for a `Map<String, V>`. The native target supports integer,
string and local record values, as well as lists and options of those
values. A snapshot from `values()` keeps the same element type.

```corvid
agent count(totals: Map<String, Int>, key: String) -> Int:
    return match totals[key]:
        Some(amount) -> amount
        None -> -1
```

For a `Map<String, Option<Int>>`, a missing key gives `None`. A key
mapped to `None` gives `Some(None)`. On integer maps, `totals[key] += 1`
updates an existing entry. Initialize a new counter with `totals[key] = 0`
before incrementing it.

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
typed prompt-output decode, `T.from_json(text)` and HTTP request
bodies. A structurally valid but out-of-range value refuses to decode,
naming the field and the refinement. For prompt output the message
names the value too:

```text
field `age` on `Person`: value 969 violates the declared refinement `between(0, 150)`
```

That exact message is what `with repair N` (see the prompts chapter)
feeds back to the model, so refined prompt outputs heal until they
are structurally AND semantically valid — with every repair attempt
still counted against `@budget`.

A refinement holds for every value, however it is built, so anything
that reads a `Person` can rely on `0 <= age <= 150`. A constant out of
range is a compile error:

```text
`Person.age` must be between(0, 150), but this value is 969
```

A value computed at run time is checked where the record is built, by
a constructor, a record literal or a `{ ..base }` update, or where a
field is assigned. A value that breaks the refinement stops the run
with the same message, as an overflowing `+` does: it came from the
program, so the line that built it is the bug. A program that builds a
refined value from a number it does not trust checks it first:

```corvid
type Person:
    age: Int where between(0, 150)
    name: String where len_between(1, 80)

agent aged(years: Int) -> Person:
    if years < 0 or years > 150:
        return Person(0, "unknown")
    return Person(years, "Ada")
```

A table row read from the database is checked too, so a row written
outside the program cannot hand on a value its column forbids. Native
code checks refinements the same way and stops with the same message,
so a refined program compiles with `corvid build`.

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

An alias declared `public` in another module is just as transparent.
After `import "./models" use Customers`, where the module declares
`public type Customers = List<Customer>`, a `Customers` parameter is a
list of `Customer` records, and so is a record field typed
`m.Customers` after `import "./models" as m`.

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

### Calling an agent from a match arm

```corvid
agent choose(index: Option<Int>, items: List<Int>) -> Int:
    return match index:
        Some(n) if n >= 0 -> items[n]
        _ -> -1
```

Here the caller must supply an index below the list length. To check both bounds before indexing, use a guard that calls an agent returning Bool. A failed guard continues to the next arm. The selected arm can call other agents and pass values bound by the pattern.

### Transforming an integer list

Both CLI targets accept a named integer function or a closure as a callback on a `List<Int>`.

```corvid
agent main() -> Int:
    values = [3, 1, 4]
    doubled = values.map(fn (n) -> n * 2)
    large = values.filter(fn (n) -> n >= 3)
    total = values.fold(0, fn (acc, n) -> acc + n)
    positive = values.all(fn (n) -> n > 0)
    has_four = values.any(fn (n) -> n == 4)
    return total
```

Here `doubled` is `[6, 2, 8]`, `large` is `[3, 4]`, and `total` is `8`. Both predicates return true. Callbacks visit elements in order. An empty list yields an empty map or filter result and the initial fold value. `any` stops at the first true result and returns false on an empty list. `all` stops at the first false result and returns true on an empty list.

### Returning a record from a sum type

```corvid
public type Entry:
    value: Int
    label: String

public type Reading:
    | Empty
    | Found(item: Entry, weight: Int)

public agent entry(reading: Reading) -> Entry:
    return match reading:
        Found(item, weight) if weight > 1 -> item
        Found(_, _) -> Entry(0, "zero")
        Empty -> Entry(-1, "empty")
```

Construct the cases with `Empty` or `Found(Entry(7, "found"), 8)`. The guard sees both payload fields. A caller in another module can use the returned record and read its fields after the match finishes.

### Comparing values

Use `==` and `!=` to compare records and collections by their current contents. Lists compare elements in order. String-keyed maps compare keys and values regardless of insertion order. Options, results and sum types compare the case first and then its payload. Two independently created records can compare equal, including recursive records that link back to themselves.

```corvid
agent main() -> Bool:
    first = {"a": 0, "b": 2}
    second = {"b": 2, "a": 0}
    same = first == second
    second["a"] = 1
    changed = first != second
    return same and changed
```

Both `same` and `changed` are true. A comparison observes values at the time it runs. Assigning a collection to another variable still shares that collection.

### Working with Unicode strings

`text.length()` counts Unicode scalar values. A letter followed by a combining mark counts as two. `text.substring(start, end)` includes the scalar at `start` and stops before `end`. It clamps indices to the string bounds and returns an empty string for a reversed range.

`to_upper()` and `to_lower()` use Unicode case mapping, which can change the length. `trim()` removes surrounding Unicode whitespace. `contains(needle)`, `starts_with(prefix)` and `ends_with(suffix)` compare exact text and accept an empty pattern. `replace(old, new)` replaces every match. An empty `old` inserts `new` at each scalar boundary.

`"a,,b,".split(",")` returns `["a", "", "b", ""]`. Splitting requires a nonempty separator. `" 42 ".parse_int()` returns `Ok(42)`. Invalid text returns `Err`. Use `is_ok()` or `is_err()` to inspect the result, or use `?` to forward the error.

### Returning early with `?`

In an agent returning a `Result`, `text.parse_int()?` produces the parsed integer on success. On failure, it immediately returns the error from the agent. An `Option`-returning agent can use the same syntax to return `None` when an input is absent.

```corvid
agent replace_first(items: List<Int>, text: String) -> Result<Int, String>:
    items[0] = text.parse_int()?
    return Ok(items[0])
```

For a nonempty list, a successful parse updates the first item. If parsing fails, the assignment never writes to the list. Expressions before `?` keep their side effects. Later operands run only if evaluation reaches them. A skipped operand of `and` or `or` cannot propagate an error. Owned local values and temporaries are released when execution leaves their scope.
