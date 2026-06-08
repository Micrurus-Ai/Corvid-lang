# `std.io` — Executing file-I/O surface

> **Status:** Phase 33S1 (closed 2026-06-08). Three tools execute
> real filesystem operations; three RuntimeChecked guarantees
> govern their behavior. Earlier slices shipped the envelope types
> only; 33S1 promotes the module to executing.

## Quick reference

```corvid
import "./std/io" use io_read_text, io_write_text, io_list_dir

agent demo() -> String:
    io_write_text("notes.txt", "hello from corvid")
    file = io_read_text("notes.txt")
    return file.contents
```

When this agent runs through `corvid run`, both calls flow through
the configured `[io] root` and return typed envelopes:
`FileReadEnvelope`, `FileWriteEnvelope`,
`DirectoryEntryEnvelope`.

## The three tools

### `io_read_text(path: String) -> FileReadEnvelope uses io_read`

Reads a UTF-8 file under the configured root. The envelope carries
`path_value`, `contents`, `bytes`, and an `effect_meta` field for
trace + replay metadata.

### `io_write_text(path: String, content: String) -> FileWriteEnvelope uses io_write`

Writes UTF-8 content to a file under the configured root. The
envelope carries `path_value`, `bytes`, and `effect_meta`. The
effect row marks the operation as `reversible: false` — composes
correctly with `@reversible` constraints elsewhere in the call
graph.

### `io_list_dir(path: String) -> List<DirectoryEntryEnvelope> uses io_list`

Lists immediate children of a directory under the configured root.
Each entry carries `path_value`, `name`, `is_dir`, and `effect_meta`.

## Security model — `[io] root`

The executing file-I/O surface refuses to operate without an
explicit `[io] root` declared in `corvid.toml`. The
`corvid new`-scaffolded project carries `[io] root = "."` so a
default starter project works out of the box, but the security
boundary is **always explicit and signable**.

### corvid.toml

```toml
[io]
root = "."          # relative paths anchor against corvid.toml's dir
# root = "/var/lib/myapp/data"   # absolute paths are taken as-is
```

### Env override

```sh
CORVID_IO_ROOT=/tmp/sandbox corvid run src/main.cor
```

Precedence: `CORVID_IO_ROOT` env wins over `corvid.toml`. Both
forms ultimately produce the same `IoToolPolicy` (see
`crates/corvid-runtime/src/io.rs::IoToolPolicy`).

### What's rejected

- **Missing `[io] root`**: every executing tool call fails closed
  with a structured diagnostic naming the missing config + the
  33S0 security model.
- **Path traversal**: a caller path that resolves OUTSIDE the
  root after `.` / `..` / absolute-prefix normalization is
  refused. The diagnostic names both the offending caller path
  AND the configured root.
- **Absolute-looking caller paths**: `/etc/passwd` is stripped of
  its leading separator and joined under the root — it can NEVER
  escape via path-join behavior.

### What's allowed

- Relative paths under the configured root: `"notes.txt"`,
  `"reports/daily.txt"`.
- The root itself: `"."` lists the root; `"./README.md"` reads
  the README at the root.

## Determinism

All three tools are non-deterministic. The checker rejects calls
inside a `@deterministic` body — this is the existing
`decl_replayability.rs::is_deterministic_compatible` rule that
treats every tool call as non-deterministic by declaration kind.
Programs that need both deterministic logic AND file I/O must
factor I/O into a separate, non-deterministic agent.

## Replay quarantine

When a Corvid program runs in Substitute-mode replay (replaying a
recorded trace), the executing file-I/O surface honors the same
quarantine the existing `IoRuntime::quarantine_writes` hook
provides. Two layers:

1. **Low-level `IoRuntime::io_write_text`** returns
   `QuarantineViolation { surface: "io", .. }` if called during
   replay. Read paths pass through transparently.
2. **Dispatch path** (`Runtime::call_tool("io.io_write_text", ...)`)
   goes through the replay-substitution path FIRST. Writes either
   substitute from the recorded trace OR diverge — they never
   reach the live filesystem.

Together the two layers prove the filesystem is provably untouched
during replay.

## Guarantees

Three RuntimeChecked rows in the canonical guarantee registry
([`docs/reference/core-semantics.md`](../core-semantics.md)):

| id | class | enforces |
|---|---|---|
| `io_source.fs_path_confinement` | RuntimeChecked | Path stays inside `[io] root`; traversal refused; missing root fails closed. |
| `io_source.fs_write_quarantine_on_replay` | RuntimeChecked | Writes during replay don't reach the filesystem. |
| `io_source.fs_read_quarantine_on_replay` | RuntimeChecked | Reads during replay either substitute from trace or diverge. |

The `@deterministic`-rejection property is covered by the existing
`replay.deterministic_pure_path` guarantee — it applies to all
tool calls regardless of effect.

## Worked example — file-backed daily summary

```corvid
import "./std/io" use io_read_text, io_write_text

agent record_summary(date: String, summary: String) -> String:
    path = date + ".txt"
    io_write_text(path, summary)
    return path

agent load_summary(date: String) -> String:
    path = date + ".txt"
    file = io_read_text(path)
    return file.contents
```

With `corvid.toml`:

```toml
[io]
root = "./summaries"
```

`record_summary("2026-06-08", "...")` writes
`./summaries/2026-06-08.txt`; `load_summary("2026-06-08")` reads
it back. Path traversal is impossible — `record_summary("../etc/passwd", ...)`
is rejected at the `IoToolPolicy::resolve` boundary.

## Related references

- [`core-semantics.md`](../core-semantics.md) — full guarantee
  registry, including the three `io_source.*` rows.
- [`inventions.md`](../inventions.md) — invention proof matrix.
- `corvid tour --topic file-io` — runnable demo.
- `crates/corvid-runtime/src/io.rs` — `IoToolPolicy` source +
  inline guarantee anchors.
- `std/io.cor` — the tool declarations + envelope types.
