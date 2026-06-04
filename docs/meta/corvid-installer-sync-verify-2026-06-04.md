# corvid-installer sync — verify-on-HEAD ask (2026-06-04)

> Follow-up to the [maintainer reply doc](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/meta/corvid-installer-sync-reply-2026-06-04.md)
> we sent after you confirmed **Option A** and pointed us at the four
> gap-tracking files in [Micrurus-Ai/corvid-installer](https://github.com/Micrurus-Ai/corvid-installer).
> This is the round-trip closure on `LIVE-TEST-GAPS.md` Gap #1.

---

## TL;DR

Pull Corvid-lang HEAD and re-run your original Gap #1 reproducer.
Confirm it now produces **zero diagnostics**. That's the one
load-bearing next step — everything else listed below is secondary
and only matters if Gap #1's external repro is closed.

Commit under verify:
[`7b92e90`](https://github.com/Micrurus-Ai/Corvid-lang/commit/7b92e90b6ef40006d9139e3f4e1ce3c7c44e35b7) (`fix(corvid-driver): vendor std/ into src/std/`)

---

## Why this is the load-bearing step

Option A says **Corvid-lang/install/ is the single source of truth**.
The only thing that *proves* that model end-to-end is: a gap you
caught from outside our test suite gets shipped a fix in our
repo, and **your** external reproducer then comes back clean
without any code changes on your side. If that round-trip works,
Option A is validated for every future gap; if it doesn't, the
canonical-repo agreement is just paperwork.

Our internal integration test
([`vendor_std_from_corvid_new_scaffold_lets_src_main_import_std_effects`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-driver/src/tests.rs))
passes at HEAD, but our test wrote a stubbed
`public type EffectEnvelope:` to a fake stdlib in a tempdir — it
does NOT exercise the real `std/effects.cor` from the source tree
or the real install-bootstrap path. Your reproducer does both,
which is why your re-run is the actual close-out.

---

## Exact verify steps

From a clean working directory:

```bash
# 1. Get Corvid-lang at the Gap #1 fix commit.
git clone https://github.com/Micrurus-Ai/Corvid-lang.git
cd Corvid-lang
git rev-parse --short HEAD   # expect 806a32b or newer; 7b92e90 is the fix commit

# 2. Build the corvid CLI locally so we know the binary under test
#    has the Gap #1 fix. (Avoids any nightly/stable channel drift
#    while you verify.)
cargo build -p corvid-cli --release
export PATH="$PWD/target/release:$PATH"   # or use the absolute path below
corvid --version                          # expect "corvid 0.0.1 (<sha>, <date>)"

# 3. Run the exact reproducer from your LIVE-TEST-GAPS Gap #1 entry.
cd /tmp
rm -rf triage_bot
corvid new triage_bot
cd triage_bot

# 4. Sanity-check the vendored layout. The fix means std/ MUST land
#    at src/std/, NOT at the project root. If the wrong path
#    exists, the fix didn't ship correctly.
test -f src/std/effects.cor && echo "OK: src/std/effects.cor exists"
test ! -e std/effects.cor   && echo "OK: std/effects.cor does NOT exist (correct)"

# 5. Add the import that was previously broken, then check.
printf '\nimport "./std/effects" use EffectEnvelope\n' >> src/main.cor
corvid check src/main.cor
echo "exit code: $?"
```

Expected end-state:

- `src/std/effects.cor` exists (vendored from the source tree's
  [`std/effects.cor`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/std/effects.cor)).
- `std/effects.cor` (project root) does **not** exist.
- `corvid check src/main.cor` prints no diagnostics and exits `0`.

PowerShell equivalent of step 3-5, if you're verifying on Windows
before your code-signing decision lands:

```powershell
Set-Location $env:TEMP
if (Test-Path triage_bot) { Remove-Item triage_bot -Recurse -Force }
corvid new triage_bot
Set-Location triage_bot
Test-Path src/std/effects.cor  # expect True
Test-Path std/effects.cor      # expect False
Add-Content src/main.cor "`nimport `"./std/effects`" use EffectEnvelope`n"
corvid check src/main.cor
$LASTEXITCODE                  # expect 0
```

---

## If the verify passes

Reply with the abbreviated output (the three `OK:` lines + the
`corvid check` exit code is enough). After that, in order:

1. **Mark Gap #1 closed** in
   [`LIVE-TEST-GAPS.md`](https://github.com/Micrurus-Ai/corvid-installer/blob/main/LIVE-TEST-GAPS.md)
   with a link to
   [`7b92e90`](https://github.com/Micrurus-Ai/Corvid-lang/commit/7b92e90).
2. **Send the release-matrix target list** — the exact set of
   `target` triples your install scripts try to fetch. We'll
   reconcile against
   [`.github/workflows/release.yml`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/.github/workflows/release.yml)'s
   `matrix.include` and post the diff. (FOLLOWUPS.md item we
   marked as actionable on our side.)
3. **OPEN-GAP-PROMPTS.md close-out** — either you drive it from
   your side (mark L-3, L-4, L-7 closed pointing at our commits in
   the [maintainer reply doc](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/meta/corvid-installer-sync-reply-2026-06-04.md))
   or tell us to send the PR. Default to whichever is easier for
   your audit trail; we don't have a preference.

---

## If the verify fails

Stop and reply with the exact failure mode. The most plausible
failure shapes, ranked:

- **`src/std/effects.cor` exists but `corvid check` still
  reports a diagnostic.** Means the resolver path is right but
  the typecheck path is wrong — different fix from the one in
  `7b92e90`. Send the diagnostic verbatim.
- **`src/std/effects.cor` doesn't exist.** Means `vendor_std`
  silently no-op'd because `find_std_source()` returned `None`.
  In that case, `$CORVID_HOME` isn't set and the `<exe-dir>/../std`
  layout isn't there. This is plausible if you built with
  `cargo build` (the std/ tree won't be next to the
  `target/release/` binary). Set `CORVID_HOME=$PWD` (in the
  Corvid-lang repo root) and re-run `corvid new triage_bot`.
- **Both `src/std/effects.cor` AND `std/effects.cor` exist.**
  Means a refactor mid-flight shipped a defensive both-locations
  vendor. The integration test's adversarial guard should have
  caught this — if it didn't and you're seeing both, we have a
  bigger problem; send the file listings from `find . -name
  effects.cor` and we'll triage.

In all three cases, **don't** patch around it on the corvid-installer
side. Under Option A the fix belongs in Corvid-lang.

---

## Out of scope for this round

Listed so they don't get re-asked:

- **Windows code-signing (Gap #3)** — filed as ROADMAP slice
  [33P7](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md)
  on our side. CA-cert purchase + `signtool.exe` wiring is on us;
  there's nothing for you to do until the signed Windows binary
  starts shipping in `release.yml`. If you want to add a
  detect-and-warn line to `install.ps1` saying "if you see a
  SmartScreen prompt, this is expected for the pre-v1.0 builds,"
  that's fine — but it's not blocking anything.
- **LANGUAGE-GAPS.md** — we owe you a full triage in the next
  reply round, not this one. Flag if there's a specific entry
  you'd like lifted forward.
- **Cloudflare Worker (`corvid-installer` at the
  [`web/`](https://github.com/Micrurus-Ai/Corvid-lang/tree/main/web)
  path)** — under Option A, the Worker stays the public-fetch
  shim. No change needed.

---

## Timeline

Same posture as before: matches your "well under EOD" pacing.
The verify is ~5 minutes of mechanical work; the secondary asks
fit in whatever window you have after.

Thanks again — Gap #1 was the most load-bearing of the four
findings and your one-line diff was exactly right.
