# Modules and imports

## One file = one module

Every `.cor` file is a module. The module's name is the filename
without the extension. Public declarations are visible to other
modules via `import`.

## Visibility

Three levels:

- (no modifier) — private. Only the same file can see it.
- `public` — visible to any module that imports this one.
- `public(package)` — visible to other modules in the same package
  but not to consumers outside the package.

```corvid
public effect refund_effect:
    cost: $50.00
    trust: supervisor_required

public tool refund(amount: Float, id: String) -> String uses refund_effect:
    @host.payment.refund(id, amount)

# private — only this file can call it
fn validate_refund_amount(amount: Float) -> Bool:
    amount > 0.0 and amount <= 1000.0
```

## Importing local modules

```corvid
# src/main.cor
import refund

agent main():
    approve refund.Refund(50.0, "cust_123")
    return refund.refund(50.0, "cust_123")
```

`import refund` brings in `src/refund.cor` and exposes its public
declarations under the `refund.` prefix.

## Aliasing

```corvid
import refund as r

agent main():
    approve r.Refund(50.0, "cust_123")
    return r.refund(50.0, "cust_123")
```

The alias is local to this file.

## Importing from packages

```corvid
import "@stdlib/db" as db
import "@stdlib/http" as http
import "@scope/connector-gmail" as gmail
```

Package imports go through the package manager. The version in
`corvid.toml` pins the resolved version; `corvid import-summary`
shows the full transitive set.

## Selective import

```corvid
import "@stdlib/io" use { read_file, write_file }

agent main():
    contents = read_file("data.txt")
    ...
```

Only `read_file` and `write_file` are bound; the rest of `@stdlib/io`
is not in scope.

## Re-export

```corvid
public import refund use { Refund, refund }
```

A module that re-exports another's public surface participates in the
adversarial corpus check (`@approve` re-export bypass): if a re-export
chain ends with a dangerous tool, the compile-time approval rule
still applies at the original call site.
