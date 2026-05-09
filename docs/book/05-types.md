# Types

## Built-in primitives

| Type | Description | Literal |
|---|---|---|
| `Int` | signed 64-bit integer | `42`, `-1`, `0x2A` |
| `Float` | 64-bit IEEE-754 | `3.14`, `1.0e6` |
| `Bool` | boolean | `true`, `false` |
| `String` | UTF-8 string | `"hello"`, `"""multi-line"""` |
| `Unit` | the unit type | (returned implicitly when nothing is) |

## Generic built-ins

| Type | Description |
|---|---|
| `List<T>` | ordered, growable sequence |
| `Map<K,V>` | key→value map |
| `Option<T>` | `Some(T)` or `None` |
| `Result<T,E>` | `Ok(T)` or `Err(E)` |
| `Grounded<T>` | value with provenance — see **[Grounded](/docs/grounded)** |

## Strings

```corvid
let s = "hello"                          # String
let multi = """
    multi-line
    raw bytes
"""                                       # String, all bytes preserved

# concatenation with +
let greeting = "hello, " + name + "!"

# methods
s.length()                                # Int
s.contains("ell")                         # Bool
s.split(",")                              # List<String>
s.to_upper()                              # String
s.trim()                                  # String
```

## Numbers

```corvid
let n: Int = 42
let f: Float = 3.14

# conversion is explicit
let f_from_n: Float = n.to_float()
let n_from_f: Int = f.to_int_truncated()  # method names spell out behavior

# integer overflow is checked in debug builds; saturates in release with
# an `--overflow` build flag for explicit semantics
```

## Lists

```corvid
let xs: List<Int> = [1, 2, 3]
let ys = xs.map(fn (x) -> x * 2)         # List<Int>
let zs = xs.filter(fn (x) -> x > 1)
let n = xs.length()
let head = xs.first()                    # Option<Int>

xs.iter()                                # iterable in `for x in ...`
```

## Maps

```corvid
let m: Map<String, Int> = {"a": 1, "b": 2}
let v: Option<Int> = m.get("a")
let exists: Bool = m.contains_key("a")
let keys: List<String> = m.keys()
```

## Option

```corvid
fn find(xs: List<Int>, needle: Int) -> Option<Int>:
    for x in xs:
        if x == needle:
            return Some(x)
    return None

# consume an option
match find(xs, 5):
    Some(x) -> "found: " + x.to_string()
    None    -> "not found"

# default with `unwrap_or`
let x = find(xs, 5).unwrap_or(0)
```

## Result

```corvid
fn parse_int(s: String) -> Result<Int, String>:
    if s.matches_int():
        return Ok(s.to_int())
    return Err("not an integer: " + s)

# the ? operator propagates Err
fn double_parsed(s: String) -> Result<Int, String>:
    let x = parse_int(s)?     # returns Err early if parse_int errors
    return Ok(x * 2)
```

## Structs

```corvid
struct Decision:
    refund: Bool
    amount: Float
    reason: String

let d = Decision { refund: true, amount: 50.0, reason: "policy match" }
let amount = d.amount

# update syntax
let d2 = Decision { ..d, amount: 75.0 }
```

## Sum types (enums)

```corvid
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

```corvid
fn first<T>(xs: List<T>) -> Option<T>:
    if xs.length() > 0:
        return Some(xs[0])
    return None
```

## Type aliases

```corvid
type CustomerId = String
type Cents = Int

fn refund(amount: Cents, id: CustomerId) -> Result<Unit, String>:
    ...
```

## Type inference

Most local bindings don't need annotations:

```corvid
let n = 42                               # inferred Int
let xs = [1, 2, 3]                       # inferred List<Int>
```

Function signatures, struct fields, and effect rows always require
explicit types — they're the boundaries where inference would be
ambiguous and where you want a checker-readable contract.
