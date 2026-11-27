# Native runtime stamp

Status: building, 2026-11-26 (macOS session). Roadmap: the staticlib stamp item under native dispatch in [the backend roadmap](../backend-ai-roadmap.md).

## The weakness

A native build links whatever `libcorvid_runtime.a` it finds on disk, and the
driver reuses a cached binary while its key is unchanged. Neither knows which
runtime sources it holds:

- After a runtime change, a build links an archive from before the change. It
  fails to link a new export (`_corvid_json_encode_recorded` not found, hit
  four times in native dispatch), or links an old runtime that disagrees with
  the code generated for it.
- The cache key hashes the C runtime files and the code generator's own
  sources. An edit to the Rust runtime, or to a crate the code generator
  builds from (`corvid-ir`, `corvid-public-json`), reuses a binary built
  from the old code.
- Linking a binary and linking a library each locate the archive with their
  own copy of the lookup. Only the library build rebuilds a missing one.

## Design

One stamp, one lookup.

1. A build-dependency crate, `corvid-source-stamp`, hashes a crate's sources
   and those of every workspace crate it depends on, transitively, with paths
   relative to the workspace root so the stamp is the same in every checkout.
   It covers each crate's `src`, `Cargo.toml`, `build.rs` and, where present,
   the `runtime` C sources. Tests, examples and benchmarks are left out, so
   editing a test does not rebuild the runtime.
2. `corvid-runtime`'s build script stamps the runtime and exports a symbol
   named after it, `corvid_runtime_stamp_<hex>`. Every archive that holds the
   runtime holds that name in its symbol index: `libcorvid_runtime.a` and a
   tools library that bundles the runtime alike.
3. Before linking, the code generator reads the archive's symbol index and
   checks that it holds the stamp of the runtime the code generator was built
   against. A stale archive in the development target directory is rebuilt,
   then checked again. A stale override or installed archive is refused with
   an error that names the archive and how to rebuild it. It is never linked.
4. The binary and library links share one lookup (`runtime_staticlib.rs`),
   so both rebuild and both check.
5. `NATIVE_BACKEND_ID` becomes the code generator's closure stamp. It covers
   the runtime and every crate the generated code depends on, so the native
   cache key changes whenever the code that builds or runs a binary changes.

## Alternatives

- **Compare file times.** An archive newer than the sources can still come
  from another checkout or profile. Times are not identity.
- **Hash the archive's bytes.** This depends on the profile and the toolchain, so the
  code generator cannot know the expected value without the archive.
- **Check the export list the code generator declares.** This catches a
  missing export but not an export whose behavior changed. The stamp catches
  both.
- **Reference the stamp symbol from generated code**, so the linker fails.
  This gives a raw undefined-symbol error, and the build cannot rebuild and
  retry. The check before the link gives a clear error and can rebuild.

## Three tiers

This is build infrastructure, so it changes no application surface. The
backend benefits directly. A native binary, a served route compiled natively
and an assistant calling that route all run the runtime their code was
generated for.

## Non-scope

- Toolchain and libc versions stay outside the stamp, as before.
- Concurrent tests rebuilding `corvid-test-tools` while another links against it stay as filed.
  Building it once per test process is the fix there.
