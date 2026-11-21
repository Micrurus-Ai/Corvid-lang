# Building and targets

## Running a program

`corvid run src/main.cor` selects an execution tier. It uses native compilation
when the entry point and its calls fit the native CLI boundary. It can select the
interpreter for programs that use executing standard-library tools, parallel
work or other interpreter features. Force a tier when checking portability.

```sh
corvid run src/main.cor --target interp
corvid run src/main.cor --target native
```

The native CLI handles arithmetic, common list operations, string-keyed maps,
supported Option and Result patterns and user-defined sum values within the
program. It also supports a subset of callbacks and captured values. Native
entry points have their own argument and result limits. A forced native run
reports unsupported code. Browser handlers and exported library functions have
separate type and host-call boundaries.

## Building an artifact

Run the command from the project root. `corvid check src/main.cor` checks the
program. `corvid build` prints the paths it writes.

| Target | Command | Output for the example |
|---|---|---|
| Python | `corvid build src/main.cor` | `target/py/main.py` |
| Native executable | `corvid build src/main.cor --target native` | `target/bin/main` (`main.exe` on Windows) |
| WebAssembly | `corvid build src/main.cor --target wasm` | `target/wasm/main.wasm`, JavaScript loader, TypeScript declarations and a manifest |
| HTTP wrapper for a native entry agent | `corvid build src/main.cor --target server` | Server executable under `target/server/` and handler under `target/bin/` |
| Portable web application | `corvid build src/main.cor --target web` | `target/web.tar` |
| Shared library | `corvid build src/api.cor --target cdylib --header` | Platform shared library and C header under `target/release/` |
| Static library | `corvid build src/api.cor --target staticlib --header` | Platform static library and C header under `target/release/` |

Use `corvid serve src/main.cor` for a source application with typed `server`
routes, forms and streaming handlers. Use the web target to distribute that
application with its compiled browser handlers. The server build target wraps
a native entry agent in an HTTP server and uses Cargo to build the wrapper.
The wrapper forwards application requests to the compiled entry agent. A file can also declare HTTP routes. The compiler checks those routes before building the entry agent. Cargo can use a shared compilation cache through `CARGO_TARGET_DIR`. Corvid copies the finished wrapper into the application's `target/server/` directory. Use `corvid serve` to dispatch requests to declared routes.

## Calling a compiled agent from Python

Save this agent in `src/main.cor` and run `corvid build src/main.cor --target python` from the project root.

```corvid
agent twice(value: Int) -> Int:
    return value * 2
```

The build creates an importable Python module. To call it, save the following as `call_agent.py` in the project root. Set `CORVID_HOME` to your Corvid installation directory so Python can find the bundled runtime.

```python
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(os.environ["CORVID_HOME"]) / "runtime-py"))

from corvid_runtime import run_sync
from target.py.main import twice

print(run_sync(twice, 21))
```

Run `python call_agent.py` with Python 3.11 or later. It prints `42`. In an existing async function, import `run` from `corvid_runtime` and use `await run(twice, 21)`.

When choosing the Python target, exercise the generated agent from a Python host. Some Corvid expressions raise `NotImplementedError` when execution reaches them. For example, an agent returning `Ok(42)` passes `corvid check` and builds an importable Python module, but calling that agent raises the error. Use `corvid run src/main.cor --target interp` to execute agents that construct `Result` values.

## Libraries and host calls

A library needs an exported entry point. Save this example as `src/api.cor`
and use either library command in the table.

```corvid
pub extern "c"
agent twice(value: Int) -> Int:
    return value * 2
```

Use `--with-tools-lib PATH` to link native tools. Use `--abi-descriptor` on a
shared-library build for the ABI descriptor, or `--all-artifacts` for the
descriptor and header. To sign a shared library, supply `--sign KEY_PATH`.
Use `corvid build --help` for the options accepted by the installed version.
See the [native interop guide](../guides/ffi-c-rust.md) for the C calling and
ownership conventions.

## Web application bundle

Build a source-backed HTTP application and its compiled browser handlers with `corvid build src/main.cor --target web`. Extract `target/web.tar` and start the application with `corvid serve src/main.cor` from the extracted directory. See [Packaging an interactive application](19-browser-views.md#packaging-an-interactive-application) for the project layout and runtime requirements.

## Reproducible builds

```sh
corvid build src/main.cor --target=native
sha256sum target/bin/main
```

Build the same sources in a second project directory and compare the two hashes. Use the same Corvid compiler, runtime archive, target architecture and system linker for both builds. On Windows the executable is `target/bin/main.exe` and PowerShell's `Get-FileHash` computes its SHA-256.

The Dockerfile generated by `corvid deploy package` selects the same release tag as the CLI that generated it. A CLI built from source uses its nightly tag when build metadata is available and the latest nightly otherwise. You can select a different release with `docker build --build-arg CORVID_VERSION=<tag>`.

The Linux application image includes Python 3.11, the shared library the Corvid CLI loads, and the Python runtime from the selected archive when present. Docker selects the AMD64 or ARM64 archive for the target architecture. To test a locally unpacked candidate, place its contents under `distribution/opt/corvid/` and pass `--build-context candidate=./distribution --build-arg CORVID_DISTRIBUTION=candidate` when building the generated Dockerfile.
