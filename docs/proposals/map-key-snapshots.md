# Map keys are snapshots

Status: building, 2026-11-27 (macOS session). Roadmap: the compound map key item in [the backend roadmap](../backend-ai-roadmap.md).

## The weakness

Records and lists are shared by reference, and a field assignment changes
every alias. A map holding a record key holds that same record, so
changing the record afterwards changes the key inside the map:

```corvid
p = P(1)
m: Map<P, Int> = {p: 10}
p.x = 2
m.get(P(1))   # None: the key is now P(2)
m.get(P(2))   # Some(10)
```

A map whose keys change under it gives answers that depend on unrelated
code elsewhere in the program. It is also why compound keys can't be hashed:
their hash would change after insertion. Python refuses mutable keys
outright; Corvid accepts records, lists and other compound values as keys,
so it needs a rule of its own.

## Design

A map keeps a **snapshot** of each compound key:

1. When a key is inserted, by a literal, `insert` or `m[k] = v`, and it is a
   record, list, map, variant, option or result, the map stores a
   detached copy. Nothing else in the program holds that copy, so nothing can
   change it.
2. A key leaving the map, through `keys()` or iterating entries, is copied
   again, so a key the program reads is its own value and changing it never
   reaches the map.
3. Because a stored key can no longer change, compound keys hash
   structurally. A record hashes its type and fields, a list its elements in
   order, a map its entries in any order, and an option, result or variant
   its tag and payload. A map of record keys is then as fast as a map of text
   keys.

The copy is deep, shares nothing with the original, and keeps cycles intact.
Values are still compared with `==`, so which keys are equal does not
change. Only whether a later mutation leaks into the map changes.

## Alternatives

- **Refuse compound keys.** It's simpler, but it takes away a useful form
  (`Map<Point, Cell>`, grouping by a record) that other languages support.
- **Refuse mutating a value while it is a key.** It needs a per-value flag
  that every field assignment checks, and the error lands far from the map.
- **Copy on insert only.** Then `keys()` hands out the map's own key, and
  mutating it leaks back in. Copying on the way out closes that.

## Three tiers

Native maps take only text-held scalar keys today, which are immutable
values, so native behavior is unchanged. When native maps take compound keys,
they follow this rule.

## Non-scope

- Copy-on-write for records in general. Only map keys are snapshots.
- Native compound keys.
