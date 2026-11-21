# corvid.sh — short-URL installer

A Cloudflare Worker that turns one short URL into the right install experience for everyone.

| Caller | What `corvid.sh` returns |
| --- | --- |
| `curl` / `wget` | `install/install.sh` (POSIX shell) |
| PowerShell | `install/install.ps1` |
| Web browser | A landing page with copy-paste commands auto-highlighted for the visitor's OS |

The Worker reads `User-Agent` at the edge, fetches the relevant script straight from `raw.githubusercontent.com`, and caches the response for 5 minutes. Free Cloudflare tier covers ~100k installs/day.

## End-user commands (after deploy)

```powershell
irm https://corvid.sh | iex          # Windows
```

```sh
curl -fsSL https://corvid.sh | sh    # macOS / Linux
```

That's the whole thing.

## One-time deploy

Prerequisites:

1. **Register the domain.** The canonical project domain is `corvid-lang.org` (decided in the 33R market-readiness track 2026-06-08). If you're forking and want a different domain, the `wrangler.toml` and `worker.js` references below need updating.
2. **Free Cloudflare account** at [dash.cloudflare.com](https://dash.cloudflare.com).
3. **wrangler CLI** locally: `npm install -g wrangler`.

Steps:

```sh
# from this directory
wrangler login
wrangler deploy
```

Then in the Cloudflare dashboard:

1. **Websites → Add a site** → enter `corvid.sh` → Free plan → follow nameserver steps at your registrar.
2. **Workers & Pages → corvid-installer → Triggers → Custom Domains → Add Custom Domain → `corvid.sh`**.

DNS propagation takes a few minutes; SSL is automatic. Once it resolves:

```sh
curl -I https://corvid.sh
# expect: 200 OK with content-type text/html (browser path)
curl -I -A 'curl/8.0' https://corvid.sh
# expect: 200 OK with content-type text/x-shellscript
curl -I -A 'PowerShell/7.4' https://corvid.sh
# expect: 200 OK with content-type text/plain (install.ps1)
```

## Updating the scripts

You don't redeploy the Worker — it always fetches the latest `install/install.{sh,ps1}` from `main`. Just edit the scripts in `install/` and merge to `main`. Cache TTL is 5 minutes, so changes propagate fast.

To change the source branch (e.g. for staging tests), edit `BRANCH` at the top of `worker.js` and `wrangler deploy` again.

## Local dev

```sh
wrangler dev
# then in another terminal:
curl -fsSL http://localhost:8787              # browser path -> HTML
curl -fsSL -A 'curl/8.0' http://localhost:8787 # shell path  -> install.sh
```

## Why a Worker instead of GitHub Pages + redirects?

Pages can do path-based redirects but not User-Agent-based content negotiation. We need to serve different bodies (not 302s) at the same URL because `curl | sh` won't follow 302s by default without `-L`, and we'd rather not depend on every dev knowing that. The Worker is ~50 lines and free.

## Cost

Cloudflare Workers free tier: 100,000 requests/day. A install pipe is one request. You'd need >100k installs/day before paying anything, and even then it's $5/month for 10M.

## Package discovery

Open `/packages` on the local Worker to search by package name or task. Each detail page has a source link, tested CLI version, example, and import. The initial entry exposes a versioned snapshot of Corvid's UI module. Its native disclosure preview renders the example's HTML. Search metadata is curated in `catalog.mjs`. The Worker and development CLI share the release index at `crates/corvid-driver/src/package_registry/catalog.json`.

The editor reads the same entries at `/packages/catalog.json`. The existing CLI reads the registry at `/packages/index.json`. Development runs lock exact-version imports automatically. Set `CORVID_PACKAGE_REGISTRY` to your local registry URL when testing. During local development set the VS Code user setting `corvid.catalog.url` to `http://localhost:8787/packages/catalog.json` and run **Corvid: Add Package Import** in a Corvid file. Deployment of the Worker is a separate step.
