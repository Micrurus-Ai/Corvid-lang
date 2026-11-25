# CLI reference

## Overview

`corvid <command> [args]`. Every command supports `--help`. The
canonical help output lives in `corvid --help`; this page is the
overview.

## Project commands

```sh
corvid new <name>                       # scaffold a new project
corvid new <name> --web                 # scaffold a page, GET / route and rendering test
corvid init                             # add corvid.toml to existing dir
corvid build <file>                     # compile to target (default native)
corvid build <file> --target=wasm       # WASM target
corvid build <file> --target=server     # rendered axum binary
corvid build src/main.cor --target web  # package routes and browser assets in target/web.tar
corvid build <file> --sign              # sign the cdylib + receipts
corvid run <file>                       # build and run an agent
corvid check [file]                     # typecheck file or project src/main.cor
corvid test                             # run tests in tests/
corvid bench <file>                     # run a benchmark
```

### Web packages

Extract `target/web.tar` and run `corvid serve src/main.cor` in the extracted directory. The archive contains server sources for the Corvid process. The browser receives the rendered page and its public assets. The build orders component source paths before compiling their handlers and sorts the archive entries.

The server adds subresource integrity to the standard interactive page footer. Unversioned compiled assets use ETags and require revalidation. A matching SHA-256 version query permits immutable caching. A stale version receives HTTP 404 with `Cache-Control: no-store`. The split quantity module pins its lazy runtime import to that runtime's digest.

The event JavaScript includes inline version 3 source maps for generated export wrappers and for direct calculations in the quantity module. Generated component handlers map to the component's source file. Mappings refer to project-relative source files and omit source contents. Browser compilation of `src/events.cor` checks its resolved import graph for Python and the server modules `std:db`, `std:env`, `std:io` and `std:secrets`. A rejected web build preserves the previous archive.

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
corvid jobs run --source app.cor --queue=default --workers=4    # --source is required
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

## Web package assets

For a web project with entry point `src/main.cor`, `corvid build src/main.cor --target web` includes the files from the project root's optional `assets/` directory in `target/web.tar`. Running `corvid serve src/main.cor` in the project or extracted archive serves each at `/assets/<filename>` with its media type. These files are public. A declared GET route at the same URL causes a startup error.

Use regular files directly inside `assets/` with lowercase extensions `.png`, `.jpg`, `.jpeg`, `.webp`, `.ttf`, `.woff`, `.woff2`, or `.txt` (for accompanying licenses). Names may contain ASCII letters, digits, hyphens, underscores and dots and must start with a character other than a dot. Linked directories, linked files and subdirectories are rejected.

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
