# Corvid Upgrade And Migration Tools

`corvid upgrade` is the supported entrypoint for source, stdlib, schema, trace-format, and connector-manifest migrations. It is designed for CI first: `check` reports pending changes and exits non-zero when work is required; `apply` performs safe rewrites and prints the same report.

## Source And Stdlib

```bash
corvid upgrade check my_app/
corvid upgrade check my_app/ --json
corvid upgrade apply my_app/
```

The source migrator distinguishes mechanical edits from calls that need a developer decision:

- `syntax.pub_extern_agent_single_line`: joins the split-line export declaration into the accepted `pub extern "c" agent` spelling.
- `stdlib.llm_complete_to_agent_run`: flags a legacy model call for manual migration to a Corvid prompt with the intended input and result types.
- `stdlib.cache_get_or_create_to_remember`: flags a legacy cache call for review of its key, storage and lifetime.

Each finding includes a stable rule id, migration kind, file path, occurrence count, and replacement text. A manual finding has a null replacement. Both check and apply return exit code 1 while a manual finding remains. Apply still writes the mechanical edits. Text inside comments and string literals is preserved.

## CI Contract

For an application repository:

```bash
corvid upgrade check . --json > target/corvid-upgrade-report.json
```

Exit code `0` means no known migration is pending. Exit code `1` means at least one migration is required. A stable release is blocked if a public breaking change exists without a corresponding `corvid upgrade` rule or explicit non-scope note in the migration guide.

## Comparing guarantees across library versions

Export each shared library's embedded guarantees, then compare them alongside the source migration scan. Substitute your platform's shared-library filenames.

```sh
corvid claim --explain --json ./current.so > current-claims.json
corvid claim --explain --json ./candidate.so > candidate-claims.json
corvid upgrade check src --claims-current current-claims.json --claims-target candidate-claims.json --json
```

The comparison reports removed guarantees and weaker enforcement classes. Exit code 1 indicates pending migrations or a regression. To verify the signature and corresponding source while exporting, supply `--key PUBLIC_KEY` and `--source SOURCE_FILE`. Check the export command's exit status before consuming its output.

## Schema, Trace, And Connector Manifests

Schema, trace-format, and connector-manifest migrations use the same command family and report shape:

- `schema.migration_state_v1`: migrates migration-state JSON from `corvid.migration_state.v0` to `corvid.migration_state.v1`.
- `trace.format_v1`: migrates trace envelopes from `corvid.trace.v0` to `corvid.trace.v1`.
- `connector.manifest_v1`: migrates connector manifests from `manifest_version` `0.1` to `1.0`.

The JSON rules match the top-level field and its string value. Spaces, indentation and line endings are preserved. For a trace, only the envelope version changes. A `schema` field inside the payload remains part of the application data. An invalid JSON object produces an error before that file is written.
