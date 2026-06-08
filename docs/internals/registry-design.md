# Corvid package registry — agreed shape (33R4a)

**Status:** design locked 2026-06-08; implementation tracked under
33R4b/c/d in [`ROADMAP.md`](../../ROADMAP.md).

This document captures the decisions made in the 33R4a pre-phase
chat so the implementation slices (33R4b client pointer, 33R4c
hosted index, 33R4d seed packages) have a single spec to build to.
The companion doc [`package-manager-scope.md`](./package-manager-scope.md)
describes what the package manager does *today*; this doc
describes what the v1.0 registry adds on top of that base.

## Decisions locked

| Concern | Decision | Rationale |
|---|---|---|
| Hosting | GitHub Releases for artifacts, static `index.json` from the existing `web/` Cloudflare Worker at `corvid-lang.org` | Locked in the 33R kickoff. Smallest hosting story; reuses the Worker that already serves `install.sh` / `install.ps1`. |
| Index shape | Single global `index.json` for v1.0 (gated by a `version` field for forward-compatible reshape) | At 5–10 first-party packages × ~5 versions each, the index stays under 10 KB. One client fetch resolves everything. Per-package indexes split when the file grows past ~100 KB. |
| Signing | Separate **registry signing key** (ed25519), public key checked into the index root | Distinct threat model from `corvid build --sign` (binary attestation). Compromise of one key doesn't revoke the other. Independent rotation. |
| Publish flow | Committed per-version manifests under `web/registry/<pkg>/<version>.json`; CI/regenerator script emits `index.json`; PR-driven | Auditable in git history. No live mutation of a database. Worker stays a static-file server. |
| Artifacts | One GitHub Release per `pkg-<name>-v<semver>` tag, carrying `<name>-<version>.corvid` and `<name>-<version>.corvid.sig` | GitHub Releases CDN gives free download bandwidth; the worker doesn't proxy artifact bytes. |

## URL + file layout

### What the worker serves

```
https://corvid-lang.org/install.sh                                (existing — 33R2)
https://corvid-lang.org/install.ps1                               (existing — 33R2)
https://corvid-lang.org/registry/index.json                       (new — 33R4c)
```

The worker does NOT serve artifact bytes. The `index.json` carries
GitHub Releases URLs the client downloads directly from
`github.com/Micrurus-Ai/Corvid-lang/releases/download/...`.

### What lives in this repo

```
web/
├─ worker.js                               (extended with /registry/* route in 33R4c)
├─ wrangler.toml
└─ registry/
   ├─ index.json                           (generated; committed for review-in-PR)
   ├─ regenerate.sh                        (walks <pkg>/<version>.json → index.json)
   └─ <pkg>/
      └─ <version>.json                    (per-version manifest; committed by publish PR)
```

### Per-version manifest shape

`web/registry/<pkg>/<version>.json`:

```json
{
  "name": "json",
  "version": "0.1.0",
  "url": "https://github.com/Micrurus-Ai/Corvid-lang/releases/download/pkg-json-v0.1.0/json-0.1.0.corvid",
  "sha256": "<hex of the .corvid artifact bytes>",
  "signature": "<hex ed25519 detached signature of the .corvid bytes (128 chars)>",
  "deps": {},
  "description": "JSON encode/decode helpers",
  "published_at": "2026-06-08T12:00:00Z"
}
```

### Global `index.json` shape

```json
{
  "version": "1",
  "generated_at": "2026-06-08T12:00:00Z",
  "signing_key": "ed25519:<hex-pub-of-registry-signing-key>",
  "packages": {
    "json": {
      "latest": "0.1.0",
      "versions": {
        "0.1.0": { /* the per-version manifest body, inlined */ }
      }
    }
  }
}
```

The `version` field is the index-schema version. A future move to
per-package indexes bumps it to `"2"` so clients can dispatch.

## Signature model

Two distinct ed25519 keypairs in the project:

- **Build signing key** — already exists; signs binary cdylib
  attestations via `corvid build --sign`. Its public counterpart
  is distributed independently (per the existing `claim --explain`
  flow).
- **Registry signing key** — new; signs each `.corvid` artifact's
  bytes. Its public hex is committed to `index.json`'s
  `signing_key` field. Maintainer holds the private key.

Verification at `corvid add` time:

1. Client fetches `index.json`.
2. Reads `signing_key` from the index root.
3. Looks up `packages[name].versions[version]` for the requested
   package.
4. Downloads the artifact from `url`.
5. Verifies SHA-256 of the downloaded bytes matches `sha256`.
6. Verifies the ed25519 `signature` against `signing_key` for the
   artifact bytes.
7. On success, extracts the package and writes a `Corvid.lock`
   row pinning name + version + sha256.

Failure at any step → abort with a structured error naming which
check failed.

### What `index.json` itself relies on

`index.json` is HTTPS-served from the Worker. Its integrity comes
from (a) HTTPS to the Worker, and (b) the Worker's deploy
controls (only maintainers can `wrangler deploy`). The `index.json`
is NOT itself ed25519-signed in v1.0 — that hardening is filed
as a post-v1.0 follow-up. Threat model: a Worker takeover or an
HTTPS MITM could swap the `signing_key` field and serve an
attacker-signed `.corvid`. The git-history audit trail
(`index.json` is committed) and the per-PR review for publish
catches mismatches in the maintainer's normal review flow.

## Publish flow (maintainer side)

```bash
# 1. Build the artifact.
corvid package build packages/json
# emits: packages/json/dist/json-0.1.0.corvid
#        packages/json/dist/json-0.1.0.corvid.sig (signed with registry priv key)

# 2. Open a tagged GitHub Release with both artifacts.
gh release create pkg-json-v0.1.0 \
    --title "json 0.1.0" \
    --notes "..." \
    packages/json/dist/json-0.1.0.corvid \
    packages/json/dist/json-0.1.0.corvid.sig

# 3. Add the per-version manifest in this repo.
mkdir -p web/registry/json
cp packages/json/dist/json-0.1.0.json web/registry/json/0.1.0.json

# 4. Regenerate the index.
web/registry/regenerate.sh

# 5. Open a PR with the new manifest + regenerated index.
# 6. After merge, wrangler deploy ships the worker with the new index.
```

CI in this repo runs `regenerate.sh` and fails the PR if
`web/registry/index.json` doesn't match what the manifests
produce — preventing index/manifest drift.

## Client `--registry` flag

The existing `corvid add` and `package` subcommands accept
`--registry <url>` and the `CORVID_PACKAGE_REGISTRY` env var. Per
33R4b, the **default** changes from "no default; user must specify"
to `https://corvid-lang.org/registry/`. Operators wanting a
private/self-hosted registry still override via the flag or env.

## Out-of-scope for v1.0

- Search (`corvid search <query>`) — index lookup only, no fuzzy
  search server-side.
- Discovery pages on the website — `corvid package metadata`
  renders the same data; no per-package web page in v1.0.
- DSSE-signed `index.json` — relies on Worker deploy controls +
  git-history audit. Post-v1.0 hardening.
- Mirroring / regional CDN — GitHub Releases CDN is what artifacts
  ride on; no separate distribution.
- Yanking — to deprecate, the maintainer commits a `yanked: true`
  field to the manifest in a follow-up PR. No mutation-in-place
  protocol.

## What unblocks what

- **33R4b** (client default-registry pointer) — needs only this
  doc's URL decision. Can start independently of 33R4c.
- **33R4c** (hosted static index) — needs this full doc. Builds
  the worker route, the regenerator, the schema doc page.
- **33R4d** (seed packages) — needs 33R4c shipped AND 33R5b/c
  (the `json` and `strings` stdlib batteries) so there's
  something to publish.
