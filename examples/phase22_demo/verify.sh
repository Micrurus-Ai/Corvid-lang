#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/examples/_bundle_demo_common.sh"

BUNDLE="$ROOT/examples/phase22_demo"
BASE="$ROOT/examples/phase22_demo_base"

expect_success_stdout_contains "bundle OK: phase22-demo (x86_64-unknown-linux-gnu)" \
  bundle verify "$BUNDLE"

# The `--rebuild` check below re-runs the full Corvid codegen on
# the bundled source and asserts the produced descriptor/header/
# bindings/library are byte-identical to the committed artifacts.
# This is a stronger reproducibility guarantee than the hash
# check above (which only asserts the on-disk bytes match the
# recorded sha256s), but it's also coupled to the exact compiler
# version: every materially-affecting change to the emitter — a
# new descriptor field, a new claim guarantee row, a new agent
# flag — invalidates the recorded artifacts until the bundle is
# regenerated.
#
# The committed phase22 demo bundle is a *snapshot* of a
# specific Corvid version and is regenerated on demand rather
# than gating every PR on byte-identical rebuild parity. The
# hash check above is the production-grade integrity gate; the
# rebuild check here logs a warning when the snapshot drifts so
# a maintainer can choose when to refresh it.
#
# To regenerate (Linux x86_64):
#   corvid build --target=cdylib --all-artifacts \
#     examples/phase22_demo/src/classify.cor
#   # copy `target/release/{classify.corvid-abi.json,libclassify.so,
#   #     lib_classify.h}` into
#   #   examples/phase22_demo/artifacts/release/
#   # regenerate bindings via `corvid bind rust|python` and copy
#   #   into examples/phase22_demo/bindings_{rust,python}/
#   # re-record the new sha256s in examples/phase22_demo/corvid-bundle.toml
if [[ "$(uname -s)" == "Linux" ]]; then
  rebuild_stderr="$(mktemp)"
  if "$CORVID_BUNDLED_BIN" bundle verify "$BUNDLE" --rebuild >/dev/null 2>"$rebuild_stderr"; then
    : # snapshot still matches current codegen
  else
    echo "warning: bundle verify --rebuild reported drift against current codegen:" >&2
    cat "$rebuild_stderr" >&2 || true
    echo "warning: committed phase22 demo bundle is a snapshot; regenerate when convenient." >&2
  fi
  rm -f "$rebuild_stderr"
fi

expect_success_stdout_contains "\"descriptor_hash_changed\": true" \
  bundle diff "$BASE" "$BUNDLE" --json

expect_success_stdout_contains "approval-gated agents: issue_tag" \
  bundle audit "$BUNDLE" --question "Which agents require approval?" --json

expect_success_stdout_contains "\"trace_count\": 1" \
  bundle explain "$BUNDLE" --json

expect_success_stdout_contains "CC7.2" \
  bundle report "$BUNDLE" --format soc2 --json

expect_success_stdout_contains "agent.replayable_gained:classify" \
  bundle query "$BUNDLE" --delta agent.replayable_gained:classify --json

expect_success_stdout_contains "\"signature_verified\": true" \
  bundle lineage "$BUNDLE" --json
