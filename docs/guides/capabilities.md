# Extending your agent: skills, MCP, connectors

One verb — `corvid add` — three capability kinds. Everything lands as
visible source and declarative config, inside the moat: effects,
approval, tracing, replay, and budgets apply to added capabilities
exactly as they do to code you wrote yourself.

## Skills — effect-audited packages

A skill is a directory of Corvid source plus a `skill.toml` declaring
its **capability label**: which stdlib capability groups it uses
(`io`, `http`, `db`, `llm`, ...), its maximum trust tier and
per-effect cost, which data classes it touches, its declared reach
(hosts, paths), and the config it needs (env secrets).

```sh
corvid add skill ./summarize-repo
corvid add skill github:acme/skills/summarize-repo@v1.2
corvid add skill git:https://example.com/skills.git#main//summarize
```

The label is **computed from the source, never self-reported** —
capability groups by scanning for stdlib tool usage, trust/cost/data
from the skill's own `effect` declarations. What you consent to at
add time is what the code actually does:

```text
skill: summarize-repo v1.2.0
  Summarizes repository activity.

capability label (verified against the source):
  uses:       http, llm
  max trust:  supervisor_required
  max cost:   $0.2500 per call
  data:       external
  reach:      hosts api.github.com (enforced at runtime by [http] allow)
  requires:   env GITHUB_TOKEN
```

Enforcement is double: a **dishonest label refuses to install**, and
every `corvid check` / `corvid run` re-verifies each vendored skill —
so a skill edited past what you consented to fails the very next
check, naming the exceeded dimension.

The skill vendors into `src/skills/<name>/` as visible, git-diffable
source with a `skill.lock` pin (source + consented content hash +
signer). Write skill imports for the installed layout: from
`src/skills/<name>/`, the vendored stdlib is `../../std/<module>`.

### Signing (registry-free)

```sh
corvid skill sign ./summarize-repo --key signing.hex   # publisher
corvid add skill ... --publisher-key their-key.hex     # consumer
```

The signature covers the skill's content manifest (per-file sha256),
so one verification proves publisher identity AND content integrity —
tampered content refuses at add time. Unsigned skills install behind
a loud UNSIGNED banner; there is no registry to trust, only keys you
chose to accept.

### Updating

```sh
corvid skill update summarize-repo
```

Re-fetches the pinned source, hash-diffs, and — only when upstream
actually changed — shows the NEW label for fresh consent before
replacing the vendored copy and re-pinning. Name-swapped updates
refuse.

## MCP — typed modules in one command

```sh
corvid add mcp github --cmd npx --cmd my-github-mcp
corvid add mcp search --url https://mcp.example.com --trusted
```

Discovery runs first (`tools/list`); the config entry only lands for
a server that was actually reached. The generated
`src/mcp/<name>.cor` has one **typed public agent per tool**, with
parameters from the server's own JSON schemas and arguments built via
the `std/json` builder (escaping is runtime-handled, never string
concatenation):

```corvid-fragment
# GENERATED — regenerate with `corvid mcp regen github`.
public agent search_issues(limit: Int, query: String) -> Result<String, String>:
    args = json_object_new()
    args = json_object_set_int(args, "limit", limit)
    args = json_object_set_string(args, "query", query)
    return mcp_call("github", "search-issues", json_object_finish(args))
```

Servers are **untrusted by default** — every call through the
generated wrappers stays approval-gated until you add
`trust = "autonomous"` after review. Schemas outside the typed v1
mapping (string/integer/number/boolean) fall back to a single
`args_json: String` parameter with the reason stated in the generated
comment. When the server changes, `corvid mcp regen <name>` refreshes
the module.

## Connectors — scaffolds from the shipped manifests

```sh
corvid add connector gmail
```

Generates `src/connectors/gmail.cor` **from the connector's
manifest**: each scope becomes an effect with honest dimensions
(approval-required scopes render `trust: human_required` +
`reversible: false`), each declared operation becomes a tool bound to
the matching effect — quarantined writes are `dangerous`, so call
sites need a compile-checked `approve`. The header carries the setup
checklist: mock mode works immediately; real mode is
`corvid connectors oauth <provider>` plus `CORVID_PROVIDER_LIVE=1`.
Tool bodies live host-side (the host routes the declared names to
`corvid_connector_runtime`).

## Related references

- [`std.secrets`](../reference/stdlib/secrets.md) — skills commonly
  declare their env needs; `secret_read` is the replay-safe way to
  read them.
- [Connectors guide](./connectors.md) — modes, OAuth, webhook
  verification, the adversarial corpus.
- `docs/reference/stdlib/mcp.md` — the governed `mcp_call` the
  generated wrappers delegate to.
