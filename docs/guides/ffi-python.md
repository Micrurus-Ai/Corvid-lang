# Calling Python from a Corvid service

## Direct imports in an interpreted program

```corvid
import python "math" as numbers effects: pure:
    fn gcd(left: Int, right: Int) -> Int

agent main() -> Int:
    return numbers.gcd(18, 12)
```

Run `corvid run main.cor` with a Python-enabled CLI and a compatible Python installation. The example prints `6`. Direct imports currently run on the interpreter target. Native compilation, WebAssembly and Python source generation report this target limitation. Install third-party modules on the deployment machine in the embedded interpreter's import path. Direct imports use synchronous Python functions. Values cross the boundary by copying, and Corvid checks the returned data against the declared type. A Python exception includes the function name and traceback. The import effect list applies to each member. To deploy your own Python module, install it or include its directory on the embedded interpreter's `PYTHONPATH`. Replay can supply the recorded result even when the module is absent. For an operation whose declared trust level requires approval, use the imported function's qualified name.

```corvid
effect reviewed:
    trust: human_required

import python "math" as numbers effects: reviewed:
    fn gcd(left: Int, right: Int) -> Int

agent main() -> Int:
    approve numbers.gcd(18, 12)
    return numbers.gcd(18, 12)
```

The approval statement shows the module, function and argument values to the reviewer. The following call executes after acceptance. Each approval covers one invocation with those argument values in the enclosing block. Bind computed arguments to locals first when both statements should use the same result.

## Python tools in a service

Declare a tool with the Corvid types the service expects. Put its implementation in `tools.py` beside the entry file or at the project root. With the Python-enabled CLI, `corvid serve src/main.cor` loads the decorated functions at startup. The same decorator works with ordinary Python functions and async functions.

```corvid
type Summary:
    value: Int

tool summarize(mode: String) -> Summary

server app:
    route GET "/summary" -> json Summary:
        return summarize("sample")
```

```python
from statistics import median
from corvid_runtime import tool

@tool("summarize")
def summarize(mode):
    if mode == "sample":
        return {"value": median([4, 8, 9])}
    raise ValueError("unknown sample")
```

Opening `/summary` returns `{"value":8}`. The decorator name matches the Corvid declaration. The server retains the registered callable for subsequent requests. Arguments and return values cross the boundary as data copies. Mutating a Python list passed into the tool changes only the Python copy.

## Data at the tool boundary

| Corvid value | Python representation |
|---|---|
| Int, Float, Bool, String | int, float, bool, str |
| List | list |
| Record | dict with the declared field names |
| Some(value), None | `{"tag": "some", "value": value}`, `{"tag": "none"}` |
| Ok(value), Err(error) | `{"tag": "ok", "ok": value}`, `{"tag": "err", "err": error}` |

Return plain values in these shapes. The server converts the result using the declared Corvid return type. For example, returning `{"value": "eight"}` from the tool above fails that request with HTTP 500 and a diagnostic naming the tool and expected field type. A Python exception also fails the request and can expose its traceback in the error detail. To let Corvid callers handle an expected failure, declare a Result return type and return the corresponding tagged value.

## Packaging

Run `corvid build src/main.cor --target web`, extract `target/web.tar` into a new directory, then run `corvid serve src/main.cor` there. The bundle includes root-level `tools.py` and Python source under `src/`. Supply a compatible Python installation, the matching Corvid Python runtime and any third-party Python packages on the deployment machine.

Python tool implementations execute in the server process with its operating-system permissions. Declare the effects of the host operation on the Corvid tool. Use a dangerous tool declaration and approval when the operation requires it.
