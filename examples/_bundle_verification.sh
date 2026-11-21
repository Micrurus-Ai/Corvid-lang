#!/usr/bin/env bash
set -euo pipefail

# Resolve the corvid binary once at source-time. The previous
# `cargo run -q -p corvid-cli -- "$@"` re-entered cargo on every
# `run_corvid` call, which caused the bundle_integration tests to
# fail under `cargo test`: those tests run in parallel by default
# (one thread per test), so multiple `cargo run` invocations
# raced on cargo's package-cache file lock. One eventually lost
# the race, returned non-zero, and `set -e` exited the script
# before any of the diagnostic output reached the test runner —
# producing the empty-stdout/empty-stderr panic the CI log
# showed. Resolving the binary up-front fixes this at the root.
#
# Resolution order: `$CORVID_BIN` (explicit override) →
# `$ROOT/target/debug/corvid[.exe]` →
# `$ROOT/target/release/corvid[.exe]` → fail with a clear
# message that names how to build. We never fall back to
# `cargo run` because that re-introduces the lock contention.

_corvid_resolve_bin() {
  if [ -n "${CORVID_BIN-}" ]; then
    if [ -x "$CORVID_BIN" ]; then
      printf '%s\n' "$CORVID_BIN"
      return 0
    fi
    echo "CORVID_BIN points at non-executable path: $CORVID_BIN" >&2
    return 1
  fi
  local root="${ROOT-${BUNDLE_DEMO_ROOT-}}"
  if [ -z "$root" ]; then
    # Best-effort: walk up from this script to find the workspace root.
    root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  fi
  local candidate
  for candidate in \
      "$root/target/debug/corvid" \
      "$root/target/debug/corvid.exe" \
      "$root/target/release/corvid" \
      "$root/target/release/corvid.exe"; do
    if [ -x "$candidate" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  cat >&2 <<EOF
no corvid binary found.

Tried:
  CORVID_BIN env var (not set, or path not executable)
  $root/target/debug/corvid[.exe]
  $root/target/release/corvid[.exe]

Build first, then re-run:
  cargo build -p corvid-cli            # debug build
  cargo build -p corvid-cli --release  # release build
or set CORVID_BIN=/abs/path/to/corvid to an existing binary.
EOF
  return 1
}

CORVID_BUNDLED_BIN="$(_corvid_resolve_bin)"

run_corvid() {
  "$CORVID_BUNDLED_BIN" "$@"
}

expect_fail_contains() {
  local needle="$1"
  shift

  local stdout_file stderr_file
  stdout_file="$(mktemp)"
  stderr_file="$(mktemp)"
  if run_corvid "$@" >"$stdout_file" 2>"$stderr_file"; then
    echo "expected failure containing: $needle" >&2
    cat "$stdout_file" >&2 || true
    cat "$stderr_file" >&2 || true
    rm -f "$stdout_file" "$stderr_file"
    exit 1
  fi
  if ! grep -Fq "$needle" "$stderr_file"; then
    echo "stderr did not contain expected marker: $needle" >&2
    cat "$stderr_file" >&2 || true
    rm -f "$stdout_file" "$stderr_file"
    exit 1
  fi
  rm -f "$stdout_file" "$stderr_file"
}

expect_success_stdout_contains() {
  local needle="$1"
  shift

  local stdout_file stderr_file
  stdout_file="$(mktemp)"
  stderr_file="$(mktemp)"
  # NOTE: the previous formulation was `run_corvid "$@" >"$stdout_file"
  # 2>"$stderr_file"` without checking the exit code. Under `set -e`,
  # a non-zero exit from corvid (e.g. `bundle verify` returning
  # `BundleHashMismatch`) aborted the script with both captured files
  # still on disk and NEVER displayed — the test runner saw an empty
  # stdout/stderr panic. Wrapping in `if ! ... ; then ...` checks the
  # exit code AND dumps the captured output so the test surfaces the
  # actual failure.
  if ! run_corvid "$@" >"$stdout_file" 2>"$stderr_file"; then
    echo "expected success, got non-zero exit from \`corvid $*\`:" >&2
    echo "--- stdout ---" >&2
    cat "$stdout_file" >&2 || true
    echo "--- stderr ---" >&2
    cat "$stderr_file" >&2 || true
    rm -f "$stdout_file" "$stderr_file"
    exit 1
  fi
  if ! grep -Fq "$needle" "$stdout_file"; then
    echo "stdout did not contain expected marker: $needle" >&2
    echo "--- stdout ---" >&2
    cat "$stdout_file" >&2 || true
    echo "--- stderr ---" >&2
    cat "$stderr_file" >&2 || true
    rm -f "$stdout_file" "$stderr_file"
    exit 1
  fi
  rm -f "$stdout_file" "$stderr_file"
}
