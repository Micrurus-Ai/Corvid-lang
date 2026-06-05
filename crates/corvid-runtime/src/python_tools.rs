//! `tools.py` autoloader — slice 33Q1b.
//!
//! Companion to slice 33Q1a's `corvid serve --with-tools-cdylib`. When
//! a Corvid project ships a `tools.py` file next to the source (the
//! shape `corvid new` scaffolds), this module locates it, embeds the
//! system Python interpreter via PyO3, walks the registered
//! `@tool("<name>")` implementations in
//! `corvid_runtime.registry._TOOL_IMPLS`, and returns a
//! [`crate::ToolRegistry`] whose handlers bridge each Corvid-side
//! tool call back into the user's Python coroutine via
//! `asyncio.run(coro)`.
//!
//! Compiled only with `feature = "python"`; corvid-cli enables it by
//! default so the shipped CLI binary always includes the autoloader.
//! The binary picks up `libpython` at runtime via PyO3's
//! `auto-initialize` (workspace pyo3 config). Users running
//! `corvid serve` against a project that has a `tools.py` are
//! already committed to having Python available; users whose project
//! is pure-cdylib pay only the libpython-link cost at load time and
//! never enter the autoloader path.
//!
//! Why `runtime/python/corvid_runtime/registry.py`'s `_TOOL_IMPLS` is
//! the right introspection target: when a user writes
//! `from corvid_runtime import tool` and decorates their async
//! function with `@tool("send_message")`, the decorator stores the
//! function in that module-level dict (per
//! `runtime/python/corvid_runtime/registry.py:36-43`). Importing
//! `tools.py` runs its top-level code and populates the dict; we
//! then iterate the dict to materialize Rust `ToolHandler`s.
//!
//! Why precedence is "cdylib wins": `corvid serve --with-tools-cdylib
//! <path>` is an explicit operator flag; `tools.py` autoload is
//! implicit (just file presence). Explicit beats implicit, so
//! `cmd_serve` registers the Python handlers FIRST and the cdylib
//! handlers SECOND, letting cdylib registrations overwrite tools.py
//! entries with the same name.

#![cfg(feature = "python")]
#![allow(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use serde_json::Value;

use crate::errors::RuntimeError;
use crate::tools::ToolRegistry;

/// Search results from [`find_tools_py`]: either the discovered
/// `tools.py` path or the not-found shape ("project has no
/// `tools.py`; autoloader is a no-op").
#[derive(Debug, Clone)]
pub enum ToolsPyDiscovery {
    Found {
        tools_py_path: std::path::PathBuf,
        project_root: std::path::PathBuf,
    },
    NotFound,
}

/// Locate `tools.py` for a given Corvid source file. The convention
/// `corvid new` scaffolds is `<project_root>/tools.py` alongside
/// `<project_root>/src/main.cor`. We walk up from the source file
/// looking for `tools.py`:
///
/// 1. Source's own directory (e.g. someone keeps `tools.py` next to a
///    single-file experiment).
/// 2. Source's parent directory's parent (project root when source is
///    under `src/`).
///
/// The walk stops at the first hit OR after one level up. We don't
/// walk all the way to the filesystem root because that risks picking
/// up a `tools.py` belonging to a different project sitting higher in
/// the tree.
pub fn find_tools_py(source_path: &Path) -> ToolsPyDiscovery {
    // Candidate 1: same directory as the source file.
    if let Some(parent) = source_path.parent() {
        let candidate = parent.join("tools.py");
        if candidate.is_file() {
            return ToolsPyDiscovery::Found {
                tools_py_path: candidate,
                project_root: parent.to_path_buf(),
            };
        }
        // Candidate 2: one directory up (the `corvid new` shape:
        // tools.py lives next to src/main.cor's PARENT, i.e. the
        // project root).
        if let Some(grandparent) = parent.parent() {
            let candidate = grandparent.join("tools.py");
            if candidate.is_file() {
                return ToolsPyDiscovery::Found {
                    tools_py_path: candidate,
                    project_root: grandparent.to_path_buf(),
                };
            }
        }
    }
    ToolsPyDiscovery::NotFound
}

/// Locate the bundled `corvid_runtime` Python package so the
/// autoloader doesn't require operators to set PYTHONPATH manually.
/// Returns the directory that should be prepended to `sys.path` (the
/// PARENT of the `corvid_runtime/` directory, so `import corvid_runtime`
/// resolves it).
///
/// Search order:
///
/// 1. **Install layout** — `<binary_parent>/../runtime-py/`. This is
///    where `release.yml` stages the package alongside the binary in
///    the release tarball (slice 33Q6's release-side change). The
///    install script extracts the tarball under `/opt/corvid/`, so a
///    binary at `/opt/corvid/bin/corvid` resolves to
///    `/opt/corvid/runtime-py/` here.
/// 2. **Dev layout** — `<exe_dir>/../../runtime/python/` (the
///    workspace's `runtime/python/` relative to `target/<profile>/`).
///    Matches `cargo run -p corvid-cli` from a source clone.
///
/// Returns `None` when neither path resolves — the autoloader then
/// falls back to whatever's on the operator's PYTHONPATH (the pre-33Q6
/// behaviour). That fallback is what dev environments with
/// system-wide `corvid_runtime` would use.
fn find_bundled_corvid_runtime() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    // Install layout: <binary_parent>/../runtime-py/
    // (binary at /opt/corvid/bin/corvid -> /opt/corvid/runtime-py/)
    if let Some(parent) = exe_dir.parent() {
        let candidate = parent.join("runtime-py");
        if candidate.join("corvid_runtime").is_dir() {
            return Some(candidate);
        }
    }

    // Dev layout: target/<profile>/../../runtime/python/
    // (target/debug/corvid -> ../../runtime/python -> runtime/python)
    if let Some(workspace_root) = exe_dir.parent().and_then(|p| p.parent()) {
        let candidate = workspace_root.join("runtime").join("python");
        if candidate.join("corvid_runtime").is_dir() {
            return Some(candidate);
        }
    }

    None
}

/// Embed Python via PyO3, import `tools` from `project_root` (which
/// triggers the `@tool("...")` decorators), and return a
/// [`ToolRegistry`] whose handlers dispatch through PyO3 to the
/// user's coroutines.
///
/// The Python module name we import is literally `tools` — that's the
/// path-less form `import tools` resolves via `sys.path`, which we
/// prepend `project_root` to. PyO3's `auto-initialize` ensures the
/// interpreter is ready before we acquire the GIL.
///
/// Errors propagate through [`RuntimeError::PythonFailed`] with the
/// full traceback included — failure modes include "tools.py raised
/// at import" (user error in tools.py) and
/// "`corvid_runtime` package not importable" (typically a missing
/// `PYTHONPATH` entry, or the project's `tools.py` started fresh
/// without going through `corvid new`).
pub fn install_python_tools(
    source_path: &Path,
) -> Result<ToolRegistry, RuntimeError> {
    let ToolsPyDiscovery::Found {
        tools_py_path: _,
        project_root,
    } = find_tools_py(source_path)
    else {
        return Ok(ToolRegistry::default());
    };

    Python::with_gil(|py| {
        let result: PyResult<ToolRegistry> = (|| {
            // Prepend the project root to sys.path so `import tools`
            // resolves to <project_root>/tools.py.
            let sys = py.import_bound("sys")?;
            let path = sys.getattr("path")?;
            let project_root_str = project_root
                .to_str()
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err(
                    "project root path is not valid UTF-8",
                ))?;
            path.call_method1("insert", (0, project_root_str))?;

            // Slice 33Q6: prepend the bundled `corvid_runtime`
            // package's parent directory to sys.path so user
            // tools.py files can do `from corvid_runtime import
            // tool` without operators having to set PYTHONPATH.
            // `find_bundled_corvid_runtime` checks (in order):
            //
            // 1. Install layout: `<binary_parent>/../runtime-py/`
            //    (where `release.yml` stages the package alongside
            //    the binary). This is the friends-and-family-
            //    reviewer path — they `curl ... | sh` install
            //    script and just-works.
            // 2. Dev layout: `<workspace_root>/runtime/python/`
            //    (where the source lives during development). This
            //    is the maintainer path — `cargo run -p corvid-cli`
            //    without a release tarball.
            //
            // Without 33Q6, a fresh `corvid new` project's tools.py
            // crashed at `from corvid_runtime import tool` with
            // `ModuleNotFoundError`, because `corvid_runtime` is
            // NOT on PyPI and a release-installed reviewer had no
            // PYTHONPATH for it. The scaffold's "Next steps" output
            // even told them `pip install corvid-runtime` — broken
            // before they could exercise Surface 3 of the trial.
            // Filed by maintainer-as-reviewer-2026-06-05 P1.1.
            if let Some(runtime_py_dir) = find_bundled_corvid_runtime() {
                if let Some(s) = runtime_py_dir.to_str() {
                    path.call_method1("insert", (0, s))?;
                }
            }

            // Importing `tools` runs the user's module top-level code,
            // which registers every decorated implementation via
            // `corvid_runtime.registry._TOOL_IMPLS`.
            py.import_bound("tools")?;

            // Read the registry the decorators populated.
            let registry_mod = py.import_bound("corvid_runtime.registry")?;
            let impls_obj = registry_mod.getattr("_TOOL_IMPLS")?;
            let impls = impls_obj.downcast::<PyDict>().map_err(|err| {
                pyo3::exceptions::PyTypeError::new_err(format!(
                    "corvid_runtime.registry._TOOL_IMPLS is not a dict: {err}"
                ))
            })?;

            let mut registry = ToolRegistry::default();
            for (name_obj, fn_obj) in impls.iter() {
                let tool_name: String = name_obj.extract()?;
                // Promote the borrowed Python callable to a long-lived
                // `Py<PyAny>` so the registered handler can re-acquire it
                // outside this `with_gil` scope.
                let callable: Py<PyAny> = fn_obj.unbind();
                register_python_tool(&mut registry, tool_name, callable);
            }
            Ok(registry)
        })();
        result.map_err(|err| python_import_error(py, err))
    })
}

/// Build a Rust handler that dispatches through PyO3 to the user's
/// Python coroutine and register it under `name` on `registry`.
///
/// The handler captures `callable` (a `Py<PyAny>` referencing the
/// user's `async def` function). On each tool call:
///
/// 1. Acquire the GIL.
/// 2. Convert `Vec<serde_json::Value>` args to a Python tuple via
///    [`crate::python_ffi::json_to_py`].
/// 3. Call the function (returns a coroutine).
/// 4. Run the coroutine to completion via `asyncio.run(coro)`.
/// 5. Convert the result back to `serde_json::Value`.
///
/// All steps run synchronously while holding the GIL. That's the
/// honest cost of embedded-Python tool dispatch: each call blocks
/// until the user's coroutine returns. `corvid serve` is a
/// demonstration / development path; high-throughput production
/// uses the cdylib path (`--with-tools-cdylib`) where Python is
/// not involved at runtime.
fn register_python_tool(registry: &mut ToolRegistry, name: String, callable: Py<PyAny>) {
    // `Py<PyAny>` does NOT impl `Clone` — clone requires the GIL via
    // `clone_ref(py)`. Wrap in an `Arc` so the registered Rust handler
    // closure (a `Fn`, called from many tokio tasks) can cheaply
    // produce a new reference per invocation without entering the
    // GIL just to refcount-bump.
    let callable: Arc<Py<PyAny>> = Arc::new(callable);
    let captured_name = name.clone();
    registry.register(name, move |args: Vec<Value>| {
        let captured_name = captured_name.clone();
        let callable = Arc::clone(&callable);
        async move {
            let dispatch_name = captured_name.clone();
            // GIL-acquiring sync block executed on the tokio worker thread.
            tokio::task::spawn_blocking(move || {
                Python::with_gil(|py| {
                    dispatch_python_tool(py, callable.as_ref(), &dispatch_name, &args)
                })
            })
            .await
            .map_err(|join_err| RuntimeError::ToolFailed {
                tool: captured_name.clone(),
                message: format!("python tool join error: {join_err}"),
            })?
        }
    });
}

/// Sync core of the Python tool dispatch — runs inside `with_gil`
/// on a tokio blocking thread. Doing this on a blocking thread
/// (not the async worker thread that called .await) keeps the
/// runtime's other tasks unblocked while the Python coroutine
/// executes.
fn dispatch_python_tool(
    py: Python<'_>,
    callable: &Py<PyAny>,
    tool_name: &str,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let result: PyResult<Value> = (|| {
        let py_args = args
            .iter()
            .map(|arg| crate::python_ffi::json_to_py(py, arg))
            .collect::<PyResult<Vec<_>>>()?;
        let tuple = PyTuple::new_bound(py, py_args);

        // Bind the callable to the current GIL scope and invoke it.
        // A user's `async def echo(message)` returns a coroutine
        // object here, not the awaited result.
        let coro = callable.bind(py).call1(tuple)?;

        // Drive the coroutine to completion synchronously via
        // `asyncio.run(coro)`. This blocks the GIL-holding thread
        // until the coroutine yields its final result.
        let asyncio = py.import_bound("asyncio")?;
        let result = asyncio.call_method1("run", (coro,))?;
        crate::python_ffi::py_to_json(&result)
    })();
    result.map_err(|err| {
        let traceback = crate::python_ffi::format_python_error(py, &err)
            .unwrap_or_else(|| err.to_string());
        RuntimeError::ToolFailed {
            tool: tool_name.to_string(),
            message: format!("python dispatch error:\n{traceback}"),
        }
    })
}

/// Convert a PyO3 error that surfaced during tools.py import (sys.path
/// manipulation, `import tools`, registry lookup) into a
/// [`RuntimeError`] with the Python traceback preserved so the
/// operator sees the same diagnostic they would in a Python REPL.
fn python_import_error(py: Python<'_>, err: PyErr) -> RuntimeError {
    let traceback = crate::python_ffi::format_python_error(py, &err)
        .unwrap_or_else(|| err.to_string());
    RuntimeError::PythonFailed {
        module: "tools".to_string(),
        function: "<import>".to_string(),
        traceback,
    }
}
