# Migration

To update an existing Corvid project, run `corvid upgrade check .` to see which files need attention. Apply mechanical changes with `corvid upgrade apply .`, port any calls marked for manual review, then run `corvid check` and your project tests. See the [upgrade and migration guide](../meta/upgrade-migrations.md) for the report format and exit codes.

Coming from another language? Start with the guide below.

## Available guides

- [From Python](from-python.md) — for LangChain / DSPy / LlamaIndex /
  Pydantic-AI users.

## Forthcoming

- From TypeScript (LangChain.js / Vercel AI / Mastra)
- From Go

If you want a "from X" guide for a language not listed, open an issue.
