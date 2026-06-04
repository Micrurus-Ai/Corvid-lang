# Handoff — sync `Micrurus-Ai/corvid-installer` with `Corvid-lang/install/`

> **For:** the maintainer of
> <https://github.com/Micrurus-Ai/corvid-installer>.
> **From:** the maintainer of
> <https://github.com/Micrurus-Ai/Corvid-lang>.
> **Sent via:** the same DM / email / Signal channel as the
> language-vs-installer coordination on
> <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/33m-friends-and-family-prompt.md>.

---

## TL;DR

`corvid-installer` was last updated **2026-05-08** and is now **a
month behind** the language repo's install pipeline. Between then
and now `Corvid-lang` has shipped four release-pipeline slices
(SHA in `--version`, staticlib in release archive, nightly
channel, install-script nightly resolution) plus a Dockerfile
generator rewrite — all of which were driven by friction the
first 33M friends-and-family trial reviewer caught running
`corvid 0.0.1` against the install path. **`corvid-installer`
needs to mirror those four slices verbatim** before the next
hand-picked reviewer round opens, or the same friction ships to
them through whichever entry point `corvid-installer` serves.

## Before any of the work — the structural question we need to
## answer together

The Cloudflare Worker config in `Corvid-lang/web/` (deploy name
`corvid-installer`, see
<https://github.com/Micrurus-Ai/Corvid-lang/blob/main/web/wrangler.toml>)
documents that it fetches the install scripts from
`raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.{sh,ps1}`
— so today's `corvid.sh`-shortened install path goes to
`Corvid-lang/install/` directly, NOT to your repo. Meanwhile your
repo `Micrurus-Ai/corvid-installer` has its own `install.sh`,
`install.ps1`, and `release.yml` at the root. **One of two things
needs to be true going forward:**

  - **Option A.** `Corvid-lang/install/` is canonical, the
    Worker keeps fetching from there, and `corvid-installer`
    becomes a documentation / planning repo (the
    `FOLLOWUPS.md` / `LANGUAGE-GAPS.md` / `LIVE-TEST-GAPS.md` /
    `OPEN-GAP-PROMPTS.md` files at your root suggest this might
    already be its real purpose). In that case the install
    scripts at your root are deletable — they're stale dead
    code — and this sync is unnecessary.
  - **Option B.** `corvid-installer` is the canonical install
    repo, the Worker (and the install scripts at
    `Corvid-lang/install/`) all get rewritten to mirror /
    redirect to YOUR repo, and the language repo's
    `install/` directory is what becomes a documentation
    pointer. In that case we sync the four slices to your
    repo now AND change the Worker config.

I cannot tell from outside which one you intend; the answer
ripples through which fix gets applied where. **Please reply
on the channel with A or B before merging anything below.** If
you want a third shape (e.g. periodic mirror sync from
Corvid-lang/install/ to corvid-installer/install/), name it
and we can scope it together.

The four slices below are described as **diffs to apply at
`corvid-installer/install.{sh,ps1}` and
`corvid-installer/release.yml`** assuming Option B. Under
Option A, you'd instead delete those three files from your
repo and merge nothing.

## The four slices to sync

All four landed in `Corvid-lang` between commits `d23d381` and
`dfc8a2b` on 2026-06-04. Each is fully linked below.

### Slice 1 — `35V2-P33-version-output-sha`

**Commit:**
<https://github.com/Micrurus-Ai/Corvid-lang/commit/d23d381>

**What it does.** `corvid --version` now reports the build's
short commit SHA + date alongside the crate version:

```text
corvid 0.0.1 (e8efa23, 2026-06-04)
```

Before this slice the output was bare `corvid 0.0.1` and
reviewers had no way to verify which commit their installed
binary was at — surfaced by the first 33M trial's followup audit
("the followup prompt told the reviewer to check
`corvid --version` for a SHA pin, but the binary didn't print
one"). The fix is a new `crates/corvid-cli/build.rs` that emits
`CORVID_BUILD_SHA` and `CORVID_BUILD_DATE` env vars via direct
`git` invocation, plus a `CORVID_VERSION` const in
`crates/corvid-cli/src/cli/root.rs` that clap reads.

**Files to mirror in `corvid-installer`:** none directly — this
is language-repo-only code work. Your `install.{sh,ps1}` don't
change for this slice. The release artifact your scripts pull
will simply start reporting a SHA in `--version` after slices 3
+ 4 (below) cause `release.yml` to inject the env vars.

**Test for the maintainer to run after upstream propagates:**
```sh
corvid --version
# expected (post-slice-1): `corvid <crate-version> (<short-sha>, <date>)`
# NOT bare `corvid 0.0.1`.
```

### Slice 2 — `35V2-P33-release-archive-staticlib`

**Commit:**
<https://github.com/Micrurus-Ai/Corvid-lang/commit/49d5935>

**What it does.** `release.yml`'s "Stage artifact" step now
copies the corvid-runtime staticlib alongside the binary into
`$stage/lib/libcorvid_runtime.{a,lib}`. Before this slice the
release tarball had `bin/corvid` + `std/` + LICENSE files but
**no staticlib**, so post-install native-tier builds (`corvid
build --target=native`, the cdylib link path) failed with
`corvid-runtime staticlib missing` because cargo only emits the
`staticlib` crate-type output when corvid-runtime is the build
TARGET, not when it's pulled in as an rlib dep. The fix runs
`cargo build -p corvid-runtime` BEFORE the existing CLI build so
the staticlib lands on disk to be copied.

The staticlib's location at `$stage/lib/` is load-bearing — it
matches the `SiblingLibDir` candidate at
<https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-codegen-cl/src/staticlib_discovery.rs>
which checks `exe_parent.parent()/lib/<name>` (i.e.
`$CORVID_HOME/lib/<name>` when the binary is at
`$CORVID_HOME/bin/corvid`). Post-install `discover_staticlib`
finds it without env-var setup.

**Diff to apply in
`corvid-installer/release.yml`** — see commit `49d5935` for the
verbatim. The two new things are:

1. **A new step** ahead of the existing CLI build:
   ```yaml
   - name: Build corvid-runtime (must precede CLI build so staticlib lands)
     run: cargo build --release --locked --target ${{ matrix.target }} -p corvid-runtime
   ```

2. **The "Stage artifact" step** gets a `mkdir -p "$stage/lib"`
   plus an OS-conditional `cp` of either `libcorvid_runtime.a`
   (Linux/macOS) or `corvid_runtime.lib` (Windows) to
   `$stage/lib/`.

### Slice 3 — `35V2-P33-release-nightly-channel`

**Commit:**
<https://github.com/Micrurus-Ai/Corvid-lang/commit/ac1fb62>

**What it does.** `release.yml` now fires on every push to `main`
in addition to `v*.*.*` tags. Pushes to `main` produce a
nightly-channel release tagged `nightly-YYYY-MM-DD-<short-sha>`
(e.g. `nightly-2026-06-04-d23d381`), marked as a pre-release so
GitHub's "latest" pointer stays on the most recent stable —
the install script's fast path `releases/latest/download/...`
continues to mean "latest stable." A new "Compute release tag +
channel" step branches by trigger and outputs the
`release_tag`, `channel`, `prerelease`, `short_sha`, and
`commit_date` for downstream steps.

The build step now also sets `CORVID_BUILD_SHA` +
`CORVID_BUILD_DATE` env vars from the computed outputs, which
slice 1's `build.rs` honors (preserves the values instead of
running its own `git rev-parse`). This is how the released
binary's `--version` ends up reporting the canonical
workflow-computed SHA rather than whatever `git rev-parse`
would have surfaced inside the runner.

**Diff to apply in `corvid-installer/release.yml`** — see commit
`ac1fb62`. The shape changes are: `on.push.branches: [main]`
added, new "Compute release tag + channel" step inserted, build
step gains the two env vars, upload step gains
`tag_name: ${{ steps.tag.outputs.release_tag }}` +
`prerelease: ${{ steps.tag.outputs.prerelease }}` +
`name: ${{ steps.tag.outputs.release_tag }}`. Cache key gains
`-${{ steps.tag.outputs.channel }}` so nightly + stable builds
don't poison each other's caches.

### Slice 4 — `35V2-P33-install-script-nightly`

**Commit:**
<https://github.com/Micrurus-Ai/Corvid-lang/commit/dfc8a2b>

**What it does.** Both install scripts now accept three shapes
for `CORVID_VERSION`:

  - `latest` (default, unchanged) → the most recent STABLE tag.
  - `nightly` (**new**) → queries
    `https://api.github.com/repos/<repo>/releases?per_page=30`,
    finds the first entry whose `tag_name` matches `nightly-*`,
    downloads from `releases/download/<tag>/...`. jq is NOT
    required — install.sh uses grep + sed against the JSON,
    install.ps1 uses `ConvertFrom-Json` (PowerShell 5.1+
    built-in).
  - any other literal value → treated as a specific release tag
    (e.g. `v0.0.1` for a pinned stable, or
    `nightly-2026-06-04-d23d381` for a pinned nightly).

**Files to mirror in
`corvid-installer/install.sh` and `corvid-installer/install.ps1`**
— see commit `dfc8a2b`. The diff replaces the existing
`if [ "$VERSION" = "latest" ]; then ... else ... fi` block (in
install.sh) and the `if ($Version -eq 'latest')` block (in
install.ps1) with a three-case switch covering `latest`,
`nightly`, and `*` (literal tag). Both scripts also gain a
documentation update in the header naming the new `nightly`
value.

**Test for the maintainer after applying:**
```sh
# Should download the most recent stable.
CORVID_VERSION=latest bash install.sh

# Should resolve the most recent nightly-* tag (after the
# language repo's first nightly run completes).
CORVID_VERSION=nightly bash install.sh

# Should download a specific pinned release.
CORVID_VERSION=v0.0.1 bash install.sh
```

## Companion CODE fix (the Dockerfile)

Adjacent to the four slices, the deploy package's Dockerfile
generator (`Corvid-lang/crates/corvid-cli/src/deploy_cmd.rs`
`render_dockerfile`) was rewritten across two commits today
(`1455b6c` → `e8efa23`) to remove the original monorepo-leaking
shape AND remove an intermediate phantom-ghcr.io shape that
referenced a non-existent container image. **Current shape**
downloads the GitHub Release tarball that `release.yml`
actually produces in a `debian:bookworm-slim AS
corvid-installer` stage, then COPYs into a distroless runtime
stage.

This may be relevant to your repo's `release.yml` ONLY in the
sense that the Dockerfile's `corvid-installer` stage name
shares a string with your repo name — that's coincidence (the
stage name describes what the stage does, "install corvid into
a tmp location"), not a structural relationship. The generated
Dockerfile is rendered by the corvid-cli binary at deploy time;
no manifest change is needed in your repo.

If you're curious, the rewrite commits are at:
- <https://github.com/Micrurus-Ai/Corvid-lang/commit/1455b6c> (first attempt — referenced a phantom ghcr image)
- <https://github.com/Micrurus-Ai/Corvid-lang/commit/e8efa23> (self-audit + fix — uses the GitHub Release tarball)

## What success looks like

After this sync lands (under whichever Option you pick above):

1. **The architecture is unambiguous.** Either
   `Corvid-lang/install/` or `corvid-installer/install*` is the
   canonical install scripts source, and there's a single
   answer to "which file should I edit when CLI behavior
   changes."
2. **`CORVID_VERSION=nightly`** works end-to-end from
   `curl install.sh | sh` → most recent main-branch push →
   binary on disk that reports the SHA in `--version`.
3. **`corvid --version`** prints `corvid 0.0.1 (<sha>, <date>)`
   so the next 33M reviewer's "version pin" check works.
4. **Native-tier builds work post-install** because the
   staticlib ships in the tarball at `lib/`.
5. **The `corvid.sh` short URL** routes to whichever repo wins
   the structural question above. The Cloudflare Worker
   config at
   <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/web/wrangler.toml>
   would need updating under Option B.

## What I'd love back from you

Before any merge happens:

1. **A reply on the channel** picking Option A, Option B, or a
   third shape. This is the structural decision the rest of the
   sync depends on.
2. **Confirmation of intent for the four `*.md` gap-tracking
   files** at your root (`FOLLOWUPS.md`, `LANGUAGE-GAPS.md`,
   `LIVE-TEST-GAPS.md`, `OPEN-GAP-PROMPTS.md`). Those look like
   gap-tracking from an earlier round; if they're related to
   the friends-and-family trial track, I'd like to read them
   for context that might inform the next reviewer prompt. If
   they're internal scratchpads, no action needed.
3. **A timeline.** If Option B (mirror the four slices), the
   merge is small (~50 lines across two files + the workflow).
   If Option A (delete the stale install scripts from your
   repo), even smaller. Either way I'd guess under an hour of
   your time. **The first 33M friends-and-family reviewer is on
   hold for a retry against the corrected prompt + install
   pipeline** — please reply with what's blocking before EOD if
   you have visibility.

## Background reading (in priority order if you want context)

1. **The first 33M trial report** —
   <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/33m-trial-anonymous-2026-06-04.md>.
   This is the friction-surfacing report that drove the four
   slices. Reads as one developer's honest 5-hour experience
   trying to follow the install + build prompt.
2. **The followup prompt** —
   <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/33m-friends-and-family-followup-prompt.md>.
   What we plan to send the same reviewer once their retry path
   actually works.
3. **The launch-claim audit's "honest gaps" section** —
   <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/meta/launch-claim-audit.md>
   Section 8 lists every `blocked:` and `non-scope:` item for
   v1.0; the install pipeline gaps are tracked here.
4. **The ROADMAP** —
   <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md>.
   Phase 33A (`-installer-foundation`, closed) named the
   original install scripts; Phase 33P (`-packaging-manager-
   manifests`, post-v1.0) names the Homebrew / Scoop / winget /
   Chocolatey / AUR / APT-RPM manifests that come after launch.
   The four slices above are the bridge — they're tracked under
   `35V2-P33-*` not `33P*` because they're release-pipeline
   plumbing that v1.0 itself depends on.

Thanks for picking this up — the structural decision is the
load-bearing piece. Everything else is a small diff once that's
locked.
