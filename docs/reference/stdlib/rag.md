# `std/rag` — governed retrieval

Slice 46g. The fifth executing stdlib surface (after io, http,
db+json, and time+random): retrieval that composes with the moat —
path-confined indexes, effect-tagged calls, honest `Result`
returns, provenance on every retrieved chunk, and trace/replay
substitution like every other executing tool.

## Tools

```corvid-fragment
public tool rag_ingest(index_path: String, doc_id: String, source: String, text: String, chunk_chars: Int) -> Result<Int, String> uses rag_write
public tool rag_search(index_path: String, query: String, limit: Int) -> Result<List<RagChunkEnvelope>, String> uses rag_read
```

- `rag_ingest` chunks `text` into `chunk_chars`-sized windows
  (20% overlap), stores the document + chunks in a SQLite index at
  `index_path`, and embeds every chunk when an embedder is
  configured. Returns the chunk count.
- `rag_search` returns the `limit` best chunks — cosine similarity
  over embeddings when an embedder is configured, term-scored
  lexical matching otherwise (an honest degradation, not an
  error). Every retrieved chunk carries its `provenance_key` and
  an `effect_meta` envelope.

## Governance

- **Paths**: `index_path` resolves through the same `[io] root`
  policy as file I/O — fails closed when unconfigured, rejects
  traversal and absolute escapes. A retrieval index is a file the
  program writes; it gets the same confinement.
- **Failures** are `Err` values, never traps: missing index,
  malformed arguments (`chunk_chars < 1`, `limit < 1`), embedder
  errors, and path-policy rejections all return
  `Result::Err(message)`.
- **Replay**: `rag_ingest`/`rag_search` calls are recorded as
  ordinary tool events; Substitute-mode replay returns the
  recorded result — the embedder never fires and the index is
  never touched on replay.

## Embedding configuration

`corvid.toml`:

```toml
[rag]
embedder = "openai"          # or "ollama"
model = "text-embedding-3-small"
# endpoint = "http://localhost:11434"   # ollama only
```

`embedder = "openai"` reads `OPENAI_API_KEY` from the environment.
With no `[rag]` table, retrieval is lexical-only over the same
index — programs behave identically, with lower recall.

## Provenance note (v1 scope)

`rag_read` deliberately does **not** carry `data: grounded`: the
`Grounded<T>` wrapper at an import boundary depends on the
cross-module provenance-composition work tracked in the post-v1.0
roadmap. Provenance travels EXPLICITLY instead — every chunk's
`provenance_key` and `effect_meta` are ordinary values your
program can check (`chunk_is_grounded`), thread, and assert on.
