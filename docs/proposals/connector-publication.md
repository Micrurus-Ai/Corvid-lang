# Proposal: declared publication of outbound integrations

Status: **proposed 2026-11-26, waiting for frontend agreement.** Owner: backend and AI development, macOS session. It changes what `corvid serve` publishes, which the TypeScript client reads, so it goes to frontend development (Codex) through the owner before anything published changes.

## Problem

`corvid serve` publishes the Application Contract at `/.well-known/corvid` and `/openapi.json`. Today the contract lists every connector: its name, base URL (or `allowed_hosts`), allowed modes, redirect policy, and every operation's method, path, effects, timeout and provider protocol. Anyone who can read the contract can therefore see which outside services the app calls and at which endpoints. That is a decision with a security consequence (it maps the app's vendors and integration surface for an attacker), and the program never makes it: it is a default from slice 52g-3d.

The repository's law on consequential policy says such a choice is declared in source, and leaving it out is a compile error. Hiding by default would be wrong too: an operator who wants assistants or partners to see the integrations could not get that without an undeclared rule.

## Who reads the published connectors today

- **The TypeScript client** (`corvid-abi/src/ts_client.rs`) reads one thing: an operation's `protocol` (its statuses, states, terminal states and deadline target). It generates a status union and a state union per protocol operation, so a frontend that shows a durable provider intent (a payment that is pending, settled or expired) gets exhaustive types. Nothing else in a connector reaches the client.
- **Assistants and partners** reading the contract see the whole connector list.
- **OpenAPI** carries no connectors.

## Decision

Each connector declares whether the published contract lists it, in one line, and leaving the line out is a compile error that names the choice:

```corvid
connector payments:
    base_url: "https://api.payments.example"
    timeout: 10s
    redirects: never
    published: no
    modes: [real, replay]
    operation charge(new: NewCharge) -> Result<Charge, ChargeError> uses payment_write:
        POST "/v1/charges" body new
```

- `published: no` keeps the connector out of `/.well-known/corvid` entirely: no name, URL, paths or protocol. The backend still knows everything (the startup check, traces, the private wire description `corvid-abi` keeps for tests).
- `published: yes` lists it as today.
- Omitting `published:` is `E05xx Connector publication required`: "connector `payments` must declare whether the published contract lists it: add `published: no` (backend only) or `published: yes` (listed at /.well-known/corvid)".

**A protocol the frontend needs.** A frontend that tracks a provider intent needs the protocol's state types. If a route or a public type in the contract exposes a protocol operation's intent state while its connector says `published: no`, the checker refuses it and names the connector: the contract would reference a protocol it does not publish. The fix is `published: yes`, which says the integration is visible. A protocol operation whose state never reaches a route needs nothing from the client and may stay unpublished.

## Alternatives considered

- **`publish: everything | protocol_only | nothing`.** Three values would let an app publish a protocol's state types without its URL and paths, which is what a frontend needs. Rejected for now: no program needs the middle value yet, and the law's rule that the common case is shortest favours two words. If one appears, a third value can be added without breaking the two.
- **Hide by default.** Simpler, but a default for a consequential choice is what the law forbids, and an operator who wants integrations visible would have no say.
- **A project-wide setting in `corvid.toml`.** One line for all connectors, but publication is a property of each integration (a status-page provider may be public, a payment provider not), and the source is where the reader looks.
- **`visible_to_clients:` or `listed:`.** `published:` reads as the plain description of what happens and matches the endpoint's name ("the published contract").

## Frontend impact (for Codex)

- The TypeScript client generates protocol types only for connectors with `published: yes`. A frontend that imports a protocol status type from an unpublished connector stops compiling, which is the point: the choice is visible in both tiers.
- The contract's `connectors` array lists only published connectors. Its shape is unchanged.
- Nothing the frontend parses changes: `published:` is connector grammar, which only the backend parses.

## Across the tiers

- **Backend:** the startup check, real and replay modes and traces are unchanged.
- **Frontend:** the client's protocol types follow the declaration, and the checker stops a route from exposing a protocol the contract would not publish.
- **AI:** an assistant reading the contract sees exactly the integrations the source chose to show; nothing is exposed to it by default.

## Slices

1. **P1.** The `published:` line and its compile error; the contract and the TypeScript client follow it; the route check for an exposed protocol. Every connector in the repository's examples, fixtures, tours and book chapters gets the line. Tests: an unpublished connector is absent from `/.well-known/corvid` and the generated client; a published one is listed as today; omission is the compile error; the route check.

## Not in scope

- What `[http] allow` permits, which stays in `corvid.toml`.
- Publishing a connector's private wire description (request and response records), which stays unpublished.
