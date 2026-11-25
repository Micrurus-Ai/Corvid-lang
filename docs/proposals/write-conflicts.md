# Write conflicts (B09)

**Status:** decided; building.
**Owner:** backend (Claude Code). It changes no grammar; the JSON and contract shape of a conflict is recorded for frontend development.
**Roadmap:** [B09](../backend-ai-roadmap.md), the item "a distinct conflict value for writes".

## The problem

A table write that cannot happen returns its reason as a value, but the value does not say which reason:

| Call | Today's error | What it can mean |
| --- | --- | --- |
| `Member.insert(row)` | `Err(holder)` | another row has the key, or a unique value |
| `Member.save(row)` | `Err(Some(row))` | the row changed since it was read, **or** another row has a unique value |
| `Member.save(row)` | `Err(None)` | the row was deleted |
| `Member.delete(row)` | `Err(Some(row))` / `Err(None)` | changed / deleted |

A program tells "changed" from "taken" by comparing keys (`if holder.id == row.id`), which every caller must remember and which a type cannot check. Each application then writes the same translation by hand. The B10 proposal's own example needs a helper for it:

```corvid
fn placed(saved: Result<Card, Option<Card>>) -> Result<Card, MoveRefused>:
    return match saved:
        Ok(card) -> Ok(card)
        Err(Some(current)) -> Err(CardChanged(current))
        Err(None) -> Err(CardMissing)
```

## The decision

One built-in type names why a write did not happen:

```text
Conflict<Member>
    | Changed(current: Member)   # it changed since it was read; `current` is the row now
    | Deleted                    # it is gone
    | Taken(holder: Member)      # another row has its key or one of its unique values
```

Each operation returns the cases it can produce, and no others:

| Call | Result | Its conflicts |
| --- | --- | --- |
| `Member.insert(row)` | `Result<Member, Conflict<Member>>` | `Taken` |
| `Member.save(row)` | `Result<Member, Conflict<Member>>` | `Changed`, `Deleted`, `Taken` |
| `Member.delete(row)` | `Result<Member, Conflict<Member>>` | `Changed`, `Deleted` |

```corvid
agent rename(member: Member, nickname: String) -> String:
    return match Member.save(Member { nickname: Some(nickname), ..member }):
        Ok(saved) -> "saved"
        Err(Changed(current)) -> "someone else edited this member first"
        Err(Deleted) -> "this member was removed"
        Err(Taken(holder)) -> holder.email + " already uses that nickname"

agent join(member: Member) -> String:
    return match Member.insert(member):
        Ok(stored) -> "welcome"
        Err(Taken(holder)) -> "that email is already a member"
```

`join` covers every case `insert` can return, so it needs no `Changed` or `Deleted` arm, and no wildcard that would hide a case added later. The checker knows which cases each operation produces, so a match is exhaustive when it covers those. A value stored in a variable keeps the cases it came with. A written type, `Conflict<Member>`, means all three, and any narrower conflict can be passed where it is expected, so a function that both inserts and saves returns one type. Code can build the cases too (`Err(Changed(current))`), so a test or a fake store produces the same value.

### What each tier gets

- **Backend.** One `match` names each reason; the key comparison and the hand-written translation disappear. `?` passes a conflict out unchanged, and an `atomic:` block left through one undoes its writes as before.
- **UI.** A conflict is a sum type in the public JSON mapping, `{"tag": "variant", "type": "Conflict", "variant": "Changed", "fields": [current]}`, so a form can show "someone else changed this" next to the current row, or name the field whose value is taken, from the value alone. The contract and the generated SDKs name it like any sum type.
- **Assistants.** Under B10 an action can return `Result<Card, Conflict<Card>>` directly. The assistant receives `Changed(current)` with the current row and can decide again; nothing is a bare 409. The B10 example's `placed` helper is no longer needed.

### Composition with the guarantees

- **Replay.** The trace already records each write's outcome; it records the case and its row, and a replay returns the same case.
- **Effects and budgets.** No change: the operations keep their effects, and a conflict costs nothing extra.
- **Provenance.** A row read from the database carries none, so `current` and `holder` carry none either.

## Alternatives considered

| Form | Why not |
| --- | --- |
| Keep `Option<T>` and add a key comparison helper | The ambiguity stays in the type; every caller still has to remember the helper. |
| One `Conflict<T>` without per-operation cases | Every `insert` match would need `Changed` and `Deleted` arms that can never run, which teaches people to write `Err(_)`, and a wildcard hides the next case anyone adds. |
| Three types (`Taken<T>`, `Stale<T>`, `Conflict<T>`) | Three names for one idea, and a function that inserts and saves needs a fourth to return both. |
| A record: `Err(conflict)` with `conflict.reason` and `conflict.row` | A reason field cannot be matched exhaustively, and `Deleted` would carry a row it does not have. |
| `Exists`, `Duplicate` or `Unique` for `Taken` | `Taken(holder)` says what happened and who holds it, for a key and a unique value alike. |
| `Stale` or `Outdated` for `Changed` | "Changed" is what a person is told: someone else changed it. |

## Slices

- [ ] Conflict-1. The type: `Conflict<T>` with `Changed(current)`, `Deleted` and `Taken(holder)`; the cases each value can hold, checked by `match` exhaustiveness; building the cases in code; subsumption into a written `Conflict<T>`; the public JSON mapping, the contract and the SDKs.
- [ ] Conflict-2. The tables: `insert`, `save` and `delete` return it on both engines, recorded and replayed, with the reference, tour, tests and examples moved over.
- [ ] Conflict-3. The B10 proposal's example uses `Conflict<Card>`, dropping `placed`.

## Non-scope

- Naming which unique rule a `Taken` broke. `holder` shows the row; when a table has several `unique` lines, the rule could follow as a field once a program needs it.
- Native code. Tables run on the interpreter until P6.
