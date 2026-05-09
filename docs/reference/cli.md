# CLI reference

## Overview

`corvid <command> [args]`. Every command supports `--help`. The
canonical help output lives in `corvid --help`; this page is the
overview.

## Project commands

```sh
corvid new <name>                       # scaffold a new project
corvid init                             # add corvid.toml to existing dir
corvid build <file>                     # compile to target (default native)
corvid build <file> --target=wasm       # WASM target
corvid build <file> --target=server     # rendered axum binary
corvid build <file> --sign              # sign the cdylib + receipts
corvid run <file>                       # build and run an agent
corvid check <file>                     # typecheck only, no codegen
corvid test                             # run tests in tests/
corvid bench <file>                     # run a benchmark
```

## Diagnostics & audit

```sh
corvid doctor                           # environment health check
corvid audit <file>                     # static report: approvals, replay, budgets, secrets
corvid contract list                    # registry of compile-time guarantees
corvid contract list --format=json
corvid claim --explain <id>             # human-readable rationale for a guarantee
```

## Tour & examples

```sh
corvid tour --list                      # every shipped invention demo
corvid tour --topic <name>              # run a specific demo
corvid examples list                    # canonical reference apps
```

## Replay & traces

```sh
corvid trace list                       # all recorded traces
corvid trace dag <trace-id>             # provenance DAG for one trace
corvid replay <trace-id>                # deterministic re-execution
corvid receipt verify <path>            # verify a signed run receipt
```

## Eval & model upgrade

```sh
corvid eval <file>                      # rerun against a saved trace set
corvid eval --swap-model <m> --source <file> <trace-dir>
corvid bench compare python|js         # published-archive comparison
```

## Jobs (Phase 38)

```sh
corvid jobs run --queue=default --workers=4
corvid jobs schedule list
corvid jobs inspect <id>
corvid jobs explain <id>                # AI-assisted root-cause from typed trace
corvid jobs dlq triage                  # AI-assisted DLQ pattern clustering
corvid jobs retry <id>
corvid jobs cancel <id>
corvid jobs export-trace <id>
corvid jobs pause --queue=default
corvid jobs drain --workers=all
```

## Auth (Phase 39)

```sh
corvid auth keys generate
corvid auth keys rotate
corvid auth jwt verify --jwks <url>
corvid approvals pending
corvid approvals approve <id>
corvid approvals deny <id>
```

## Connectors (Phase 41)

```sh
corvid connectors list
corvid connectors test <connector>      # mock + replay + real-mode self-test
corvid connectors auth <connector>      # OAuth or API-key setup
```

## Migrations (Phase 37)

```sh
corvid migrate status
corvid migrate up [--dry-run]
corvid migrate down [--dry-run]
```

## Package management (Phase 25)

```sh
corvid package metadata <file> --name <n> --version <v>
corvid package publish
corvid import-summary <file>
```

## Self-management

```sh
corvid --version
corvid self update
corvid self uninstall
```
