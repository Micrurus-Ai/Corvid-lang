# FFI: Calling Python

## When to use it

You have:
- A numpy/scipy/sklearn calculation Corvid doesn't ship as stdlib.
- A trained model loaded via PyTorch or HuggingFace.
- An existing Python function you don't want to rewrite.

Call it from Corvid via the Phase 30 PyO3-backed FFI.

## Example

```corvid
import "@stdlib/python" as py

agent score(features: List<Float>) -> Float uses py_call_effect:
    let np = py.import("numpy")
    let model = py.import_user_module("scoring")
    return model.call("score", [features])
```

`py.import("numpy")` resolves a top-level Python module. `py.import_user_module("scoring")` resolves a `.py` file in the
project's `python/` directory. `.call(name, args)` invokes a function;
the return value is auto-converted via the typed bridge.

## Type bridging

| Corvid | Python |
|---|---|
| `Int` | `int` |
| `Float` | `float` |
| `Bool` | `bool` |
| `String` | `str` |
| `List<T>` | `list` |
| `Map<K,V>` | `dict` |
| `Option<T>` | `Optional[T]` (None ↔ None) |
| `Result<T,E>` | (Python exceptions become `Err(...)`) |
| struct | typed `dataclass` or `dict` |

Mismatches at the bridge produce typed `Err(BridgeError)`, not panics.

## Effect rows

A Python call carries the `py_call_effect` effect by default, with
configurable cost and latency. If the Python code does dangerous
things (filesystem writes, network calls), the wrapping Corvid agent
must declare the appropriate effects on its own row — the FFI doesn't
infer them transitively.

## Sandboxing

Python calls run in a configurable sandbox: filesystem-read allow-list,
network deny-list, max wall time, max memory. Configure in
`corvid.toml`:

```toml
[ffi.python]
allow_fs_read = ["data/", "models/"]
deny_network = true
max_wall_time_seconds = 30
max_memory_mb = 1024
```

The sandbox is a host-side runtime check, not a compile-time
guarantee. The `py_call_effect` row carries `trust: model_only` by
default; mark it `supervisor_required` if your Python code does
anything you want gated by `approve`.

## Replay

Python calls are recorded the same way LLM calls are. `corvid replay`
serves the cached return values without re-executing the Python.
Tests can mock Python modules.
