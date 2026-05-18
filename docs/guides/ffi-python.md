# FFI: Calling Python

Phase 30 ships PyO3-backed Python FFI. The runtime side (PyO3
bridge, type conversion, sandbox enforcement, replay
quarantine) is shipped and tested; the source-level
`py.import(...)` / `module.call(...)` ergonomic surface is
post-v1.0 — today Python interop enters Corvid through `tool`
declarations whose host-side implementation wraps the PyO3
runtime, the same pattern persistence + connectors follow.

## When to use it

- A numpy / scipy / sklearn calculation Corvid doesn't ship as
  stdlib.
- A trained model loaded via PyTorch or HuggingFace.
- An existing Python function you don't want to rewrite.

## Tool-declaration entry path (shipped today)

The host-side Rust glue calls into Python via PyO3 (Phase 30);
the Corvid source declares the typed interface as a `tool` that
the host implementation wraps.

```corvid
effect py_call_effect:
    cost: $0.0001
    trust: model_only

tool score_features(features: List<Float>) -> Float uses py_call_effect

agent rank_one(features: List<Float>) -> Float uses py_call_effect:
    return score_features(features)
```

The host-side implementation of `score_features` loads the
Python module and invokes the function; the typechecker enforces
the effect row + cost ceiling at the Corvid layer.

## Type bridging (runtime contract)

The PyO3 bridge converts these types in both directions:

| Corvid | Python |
|---|---|
| `Int` | `int` |
| `Float` | `float` |
| `Bool` | `bool` |
| `String` | `str` |
| `List<T>` | `list` |
| `Nothing` | `None` |
| `Option<T>` | `Optional[T]` (None ↔ None) |
| `type` (struct) | `dict` (or typed `dataclass` when the Python side declares one) |

Type mismatches at the bridge surface as typed runtime errors,
not panics. Python exceptions become `Result::Err` at the Corvid
layer.

## Effect rows + the typechecker

A Python call carries the `py_call_effect` effect (or whichever
effect the host declares on the tool). If the Python code does
dangerous things (filesystem writes, network calls, money-moving
operations), the wrapping Corvid tool MUST declare the
corresponding effects on its own row — the FFI does not infer
them transitively. Mark the tool `dangerous` if the Python code
mutates state irreversibly, and the typechecker will refuse to
call it without a matching `approve` boundary (the existing
`approval.dangerous_call_requires_token` rule applies).

## Sandboxing

Python calls run in a host-side sandbox: filesystem-read allow-
list, network deny-list, max wall time, max memory. Configure
via environment variables on the running agent:

```sh
export CORVID_PY_ALLOW_FS_READ=data/,models/
export CORVID_PY_DENY_NETWORK=true
export CORVID_PY_MAX_WALL_TIME_SECONDS=30
export CORVID_PY_MAX_MEMORY_MB=1024
```

The sandbox is a runtime check, not a compile-time guarantee.
The `py_call_effect` row carries `trust: model_only` by default;
declare the tool `dangerous` (and require `approve`) if the
Python code does anything the operator should gate on.

## Replay

Python calls are recorded by the same lineage path LLM calls
use. `corvid replay <trace>` serves the cached return values
without re-executing the Python — the replay-quarantine property
(no real provider call leaks out of a replay session) applies to
Python tool calls the same way it applies to LLM calls.

## Pointers to the registry contracts

| Property | Registry id | Class | Where |
|---|---|---|---|
| Dangerous Python tool requires approve | `approval.dangerous_call_requires_token` | Static | `crates/corvid-types/src/checker/` |
| Lexical-scope approve enforcement | `approval.token_lexical_only` | Static | `crates/corvid-types/src/checker/` |
| Effect rows propagate through call graph | `effect_row.body_completeness` | Static | `crates/corvid-types/src/effects.rs` |
| Replay quarantine for tool calls | `jobs.replayable_side_effects` | OutOfScope | gated on `35V2-P38-C-deferred` (replay-quarantine cross-layer wiring) |
| Source-level `py.import(...)` / `module.call(...)` ergonomic surface | n/a | post-v1.0 | filed as ergonomic improvement; today Python interop enters via tool declarations |
