# Calling Python from a Corvid service

## Direct imports in an interpreted program

```corvid interp
import python "math" as numbers effects: pure:
    fn gcd(left: Int, right: Int) -> Int

agent main() -> Int:
    return numbers.gcd(18, 12)
```

Run `corvid run main.cor` with a Python-enabled CLI and a compatible Python installation. The example prints `6`. Direct imports currently run on the interpreter target. Native compilation, WebAssembly and Python source generation report this target limitation. Install third-party modules on the deployment machine in the embedded interpreter's import path. Direct imports use synchronous Python functions. Values cross the boundary by copying, and Corvid checks the returned data against the declared type. A Python exception includes the function name and traceback. The import effect list applies to each member. To deploy your own Python module, install it or include its directory on the embedded interpreter's `PYTHONPATH`. Replay can supply the recorded result even when the module is absent. For an operation whose declared trust level requires approval, use the imported function's qualified name.

```corvid interp
effect reviewed:
    trust: human_required

import python "math" as numbers effects: reviewed:
    fn gcd(left: Int, right: Int) -> Int

agent main() -> Int:
    approve numbers.gcd(18, 12)
    return numbers.gcd(18, 12)
```

The approval statement shows the module, function and argument values to the reviewer. The following call executes after acceptance. Each approval covers one invocation with those argument values in the enclosing block. Bind computed arguments to locals first when both statements should use the same result.

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

## Host execution

Direct imports run inside the host Python interpreter. The declared effect list is used by the Corvid checker and by the runtime's policy checks for recognized host calls. Use trusted Python code in this process.

## Shipping a Python tool with a Corvid service

Declare the return type in Corvid and register the implementation by name in the project's root `tools.py`. For a Corvid tool declared as `tool measure(value: String) -> Stats uses calculate`, where `Stats` has a field `largest: Int`, a synchronous implementation is:

```python
from math import gcd
from corvid_runtime import tool

@tool("measure")
def measure(value):
    return {"largest": gcd(18, int(value))}
```

`corvid build src/main.cor --target web` includes the root `tools.py`. Extract the archive and run `corvid serve src/main.cor` with a Python-enabled Corvid installation. Install any third-party Python packages on the destination host. The host owns the registered callable for the service lifetime. Each call exchanges values: a Python dictionary becomes a Corvid record after its fields have been checked. A mismatched field or a Python exception fails that request. The same server can handle another call afterward.

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
| Replay quarantine for tool calls | `jobs.replayable_side_effects` | RuntimeChecked | shipped via audit-correction track `35V2-P38-C-replay-quarantine`; cross-surface corpus at `crates/corvid-runtime/tests/replay_quarantine_corpus.rs` |
| Source-level `py.import(...)` / `module.call(...)` ergonomic surface | n/a | post-v1.0 | filed as ergonomic improvement; today Python interop enters via tool declarations |
