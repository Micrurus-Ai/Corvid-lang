# FFI: C/Rust

## When to use it

You have:
- An existing Rust/C/C++ service you want to add Corvid agents into.
- A Go/Java/JS/Swift service that can dlopen a cdylib.
- A signed-build requirement.

Build Corvid as a cdylib with `pub extern "c"` agent exports, then
call it like any other shared library.

## Exporting an agent

```corvid
public effect refund_effect:
    cost: $50.00
    trust: supervisor_required

pub extern "c" agent refund_handler(amount: Float, customer_id: String) -> String uses refund_effect:
    approve Refund(amount, customer_id)
    return refund_logic(amount, customer_id)
```

`pub extern "c"` makes the agent visible to C callers. The agent's
parameter and return types use the bare-pointer ABI from Phase 20n-B.

## Building

```sh
corvid build src/lib.cor --target=cdylib --sign
```

Outputs:
```
target/cdylib/
├── libmy_app.so          # the binary
├── libmy_app.so.dsse     # DSSE-signed receipt
├── libmy_app.so.claim    # signed claim block
└── my_app.h              # generated C header
```

## Generated C header

```c
// my_app.h (auto-generated)
typedef struct CorvidString { const char* ptr; size_t len; } CorvidString;

// effect: refund_effect (trust: supervisor_required)
CorvidString refund_handler(double amount, CorvidString customer_id);

// metadata
extern const char* CORVID_ABI_DESCRIPTOR;     // embedded JSON
```

`corvid generate c-header src/lib.cor` regenerates the header at
will. The ABI descriptor in the header (and embedded in the cdylib)
is what `corvid-abi-verify` cross-checks.

## Calling from C

```c
#include "my_app.h"

int main(void) {
    CorvidString customer_id = { .ptr = "cust_123", .len = 8 };
    CorvidString result = refund_handler(50.0, customer_id);
    printf("%.*s\n", (int)result.len, result.ptr);
}
```

## Calling from Rust

```rust
extern "C" {
    fn refund_handler(amount: f64, customer_id: CorvidString) -> CorvidString;
}

fn main() {
    let id = CorvidString::from_str("cust_123");
    unsafe {
        let result = refund_handler(50.0, id);
        println!("{}", result.as_str());
    }
}
```

## Verifying the binary

Before loading the cdylib, verify it:

```sh
corvid receipt verify libmy_app.so
```

This catches post-link tampering and build-cache drift before the
host runs the binary.

## Memory ownership

The bare `(ptr, len)` ABI passes UTF-8 byte slices; Corvid owns the
returned slice's memory. Free returned slices with
`corvid_free_string(s)` (declared in the generated header).

## Multi-value returns

Struct returns use multi-value WASM-style returns. The generated
header surfaces them as out-parameters or as packed structs depending
on the target's calling convention. See the per-target shape in
[`docs/internals/wasm-abi.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/internals/wasm-abi.md).
