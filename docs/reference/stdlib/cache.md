# `std.cache` — Executing provenance-keyed cache

> **Status:** executing. Four tools drive the runtime's shared
> in-memory `cache::CacheRuntime`.

## Quick reference

```corvid
import "./std/cache" use cache_put, cache_get, cache_invalidate_provenance

agent summarize_cached(doc_id: String, body: String) -> Result<String, String>:
    found = cache_get("summaries", doc_id)?
    if found.hit:
        return Ok(found.value)
    summary = body.substring(0, 80)
    cache_put("summaries", doc_id, summary, "", "doc:" + doc_id)?
    return Ok(summary)
```

## The four tools

- `cache_put(namespace, subject, value, invalidation_key, provenance_key) -> Result<CacheEntryEnvelope, String> uses cache_write`
  — stores `value` at the `(namespace, subject)` address. One entry
  per address: a second put overwrites. Pass `""` for keys you don't
  use.
- `cache_get(namespace, subject) -> Result<CacheLookupEnvelope, String> uses cache_read`
  — a MISS is `Ok` with `hit: false` (absence is a modeled state).
- `cache_invalidate(invalidation_key) -> Result<Int, String> uses cache_write`
  — evicts every entry stored with that invalidation key; returns
  the count.
- `cache_invalidate_provenance(provenance_key) -> Result<Int, String> uses cache_write`
  — evicts every entry DERIVED from that source. This is the
  provenance composition: when a source document changes, one call
  drops everything computed from it, across namespaces.

## Semantics

- **Scope**: in-memory, shared across the run (worker pools and
  tracer-swapped clones see one cache). Not persisted across
  processes — durable caching is a different feature.
- **Values**: Strings in v1. JSON-encode richer shapes via
  `std/json`.
- **Replay**: all four tools record and replay-substitute as
  ordinary tool events, so replayed runs observe identical cache
  behavior (hits stay hits) regardless of live cache state.

## Related references

- `corvid tour --topic provenance-cache` — runnable demo.
- `crates/corvid-runtime/src/cache.rs` — `CacheRuntime`.
- `std/cache.cor` — tool declarations + envelopes.
