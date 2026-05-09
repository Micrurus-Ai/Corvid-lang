# Connectors

## What ships

Typed connectors for:

- **Gmail** — read, send, search, label.
- **MS365** — Outlook mail, Teams, OneDrive.
- **Calendar** — Google Calendar, Microsoft Calendar.
- **Slack** — read channels, post messages, react.
- **Tasks** — Linear, GitHub issues, Notion.
- **Files** — Google Drive, OneDrive, Dropbox.

## Three modes

Every connector ships in three modes that share the same typed surface:

- **Mock** — in-memory simulator for tests. No network.
- **Replay** — serves recorded responses from a trace. No network.
- **Real** — actual API calls. Requires OAuth or API key.

A test or replay run cannot accidentally hit `real` mode — the runtime
distinguishes them at the connector boundary, and a quarantine fires
if a replay tries to escape into real-mode.

## Using a connector

```corvid
import "@connector/gmail" as gmail

@budget($0.05)
agent triage_inbox(user_id: String) -> List<Triage> uses gmail_effect:
    let recent = gmail.recent(user_id, since: yesterday())
    return recent.map(fn (m) -> classify(m))
```

The first call requires OAuth setup:

```sh
corvid connectors auth gmail --user=alice@example.com
```

This opens the browser, completes the OAuth flow, encrypts the
refresh token, and stores it under `db.tokens`.

## Self-test

```sh
corvid connectors test gmail
```

Runs:
1. Mock-mode test (no network).
2. Replay-mode test against a recorded fixture.
3. Real-mode test (if credentials are configured).

A diff between mock/replay/real shows up as a connector contract
drift. Phase 41L's connector-contract test pins this.

## Writing your own connector

A connector is a Corvid module that:
- Declares effect rows for each operation.
- Implements three modes (mock, replay, real).
- Ships a typed surface (one struct per response shape).
- Lands in `examples/connectors/<name>/` with self-tests.

The connector template:

```sh
corvid connector new my-connector
```

Generates a starter project with the three-mode skeleton + self-test
harness.

See per-connector deep docs in
[`docs/connectors-*.md`](https://github.com/Corvid-lang/Corvid-lang/tree/main/docs).

## Adversarial coverage

Replay quarantine fires for every connector type. The threat corpus
exercises:

- Replay → real mode escape.
- Approval bypass through a connector wrapper.
- Token leakage through trace records.
- DSSE-bundle bilateral signature acceptance.

Each is caught with the matching `guarantee_id`.
