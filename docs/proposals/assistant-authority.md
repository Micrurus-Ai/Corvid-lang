# Assistant authority (B11)

**Status:** proposal, posted for frontend review 2026-11-26. Under the grammar law, built one working day after posting unless frontend development objects. The non-grammar parts (B11-2 onward) follow the grammar slice.
**Owner:** backend and AI (Claude Code); the in-page grant and confirmation UI is frontend development's.
**Roadmap:** [B11](../backend-ai-roadmap.md), widening what [shared actions](shared-actions.md) (B10) narrowed.

## The problem

B10 lets an assistant call an action only inside its person's signed-in browser session, and every call waits for that person to confirm. That is safe, but it stops three things a real application needs:

1. An assistant that is not in the person's browser (a desktop or hosted assistant reaching `POST /mcp`) has no way to act for them.
2. An action the person is happy for their assistant to run without asking each time (`assistants: allow`) has no safe meaning yet.
3. `corvid act` and background jobs run as no one, so they refuse every action that requires a signed-in caller.

The rule B11 must keep: **an assistant acting for a person never exceeds that person's authority, never outlives what the person granted, and stops the moment either is withdrawn.**

## The proposal

### One declaration: whether a program issues delegations, and for how long

```corvid
identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        editor: "board.edit"
    delegation: up_to 8 hours
```

`delegation: up_to <duration>` is the only new line in the identity block. It says the program lets a signed-in person grant an assistant authority, and caps how long any grant may last. A token's lifetime is a consequential security policy, so there is no default: without the line, no grant can be issued and assistants act only inside the person's session, as in B10. The cap is a duration literal, written as a number and a unit as in `30 seconds`; it must be between `1 minute` and `30 days`, so a grant can never be permanent.

### A grant, made by the person

A signed-in person grants an assistant authority from the page (`POST /__delegations`, with session and CSRF):

| Field | Meaning |
| --- | --- |
| `label` | What the person calls the assistant, shown in the grant list and the trace. |
| `actions` | The offered actions the grant covers, each with `confirm` or `allow` (no more than the action declares). |
| `expires_in` | How long it lasts, at most the declared cap. |

The answer carries a bearer token, shown once. It is stored only as a hash in the auth store beside sessions, bound to the person's actor and tenant. The person lists their grants (`GET /__delegations`) and revokes one (`DELETE /__delegations/<id>`).

### What a call with a grant may do

`POST /mcp`, `POST /actions/<name>` and `corvid act --server` accept `Authorization: Bearer <token>`. Each call is authorized as the intersection of, read fresh at the call:

1. the actions the grant names, at the mode it names;
2. the actions the source offers with an `assistants:` line, at the mode the line declares;
3. what the person may do now: their current roles and permissions against the action's `requires`.

So a grant cannot widen what the source offers, the source cannot widen what the person may do, and a role removed from the person removes it from every grant at once. An action's `Caller` is the person; the trace records the grant id and label as `surface: "assistant"`, so "which assistant did this, for whom" is answered from the trace.

### `assistants: allow`

`assistants: allow "Add a label to a card."` means an authorized assistant may run the action without asking each time. It runs without confirmation only through a grant that names it with `allow`, or inside the person's own session; every other call still waits for confirmation. `confirm` keeps its B10 meaning. An action whose body reaches `approve` keeps its `@approval` reviewer whatever the mode: allowing an assistant to start an action never skips the approval that action needs.

### Revocation

Revoking a grant, its expiry, or a change to the person's roles takes effect at the next call. A pending confirmation made under a revoked or expired grant is dropped and recorded as `revoked`; confirming it answers `410`. A call already running finishes or fails as it would for the person; stopping a running call between its writes is durable runs' work (B13), recorded there.

### The command line and workers

- `corvid act --server https://app.example --token-file grant.txt move_card --card 3` calls a running server's action endpoint with a grant, so an operator acts as themselves with the authority they granted. The local form without `--server` keeps refusing a signed-in action.
- A job enqueued by a signed-in person carries the grant id; the worker checks the grant and the person's authority when the job runs, and a revoked grant fails the job without running it (B10-4b).

### Not changed: an action without an `assistants:` line

The roadmap's earlier B11 line said omitting the exposure should be a compile error. B10 shipped the opposite, deliberately: without the line an action is people only and no assistant is told it exists. That is the repository's rule that nothing is exposed to an AI unless the source says so, and the restrictive posture needs no declaration of its own, unlike a choice between two open ones. B11 keeps it, and the roadmap line is corrected.

## Three tiers

- **Backend:** the grant store, bearer authentication, the intersection check, revocation, the trace fields.
- **Frontend:** a page to make, list and revoke grants, generated from the contract's offered actions, and the confirmation prompt (frontend roadmap).
- **AI:** an assistant receives only the tools its grant covers, typed by the contract, and every call it makes is attributable to it in the trace.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| OAuth 2.0 authorization server with scopes | A full authorization server, consent screens and refresh tokens, for a need served by one grant record. The bearer grant can later be issued through OAuth without changing what it authorizes. |
| A default grant lifetime | A token lifetime is a consequential policy; the source declares the cap, and without it no grant exists. |
| Grants that name permissions instead of actions | Permissions are coarser than what the person means ("let it move cards"), and an assistant would gain new actions whenever a permission gains them. |
| Making `assistants:` mandatory on every action | Would make every action author decide about assistants and invite a reflexive `allow`. The absent line is the safe, explicit people-only posture. |

## Slices

- [ ] B11-1. Grammar: `delegation: up_to <duration>` in the identity block (bounds checked) and `assistants: allow "..."`; contract carries both. Shared grammar: built after the review window.
- [x] B11-2a. The grant store (`SessionAuthRuntime::create_delegation`, `resolve_delegation`, `list_delegations`, `revoke_delegation`), grammar-independent, so built during the review window. Built 2026-11-26.
- [ ] B11-2b. `POST/GET/DELETE /__delegations`, expiry bounded by the declared cap, traced.
- [ ] B11-3. Bearer authority on `/mcp` and `/actions/<name>`: the three-way intersection read fresh, `allow` without confirmation through a grant or session, grant id and label in the trace.
- [ ] B11-4. Revocation: pending confirmations under a revoked or expired grant are dropped and answer `410`; tests revoke mid-flow.
- [ ] B11-5. `corvid act --server --token-file`, and workers carrying the grant (B10-4b).

## Non-scope

- Stopping a call that is already running between its writes: durable runs (B13).
- Assistants acting for many people, or for a tenant rather than a person.
- Outside assistants discovering a deployed app's actions without a person's grant (B26 builds on this).
