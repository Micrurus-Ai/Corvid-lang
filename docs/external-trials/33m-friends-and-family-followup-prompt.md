# Follow-up to anonymous-2026-06-04 — what we fixed and what to retry

> The maintainer's reply to the first 33M trial report at
> <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/33m-trial-anonymous-2026-06-04.md>.
> Sent through the same DM / email / Signal channel the
> original outreach came through.
> Path-A timing posture preserved — this re-test happens against
> the corrected prompt before the actual launch-readiness window
> opens.

---

## Thank you

Your half-day caught the kind of friction nobody on the
maintainer side could see anymore. The prompt itself had **6
real bugs** (1 in the Dockerfile renderer, 5 across the
suggested-build-path commands) plus **8 follow-up slices**
worth of code/docs/test work. If we'd sent that prompt to
five hand-picked reviewers as-written, every one of them
would have burned their first 30 minutes the same way you
did. We're glad you went first.

Below is the disposition + what's ready for a focused retry.

## What we fixed and pushed (commit `1455b6c`)

| Your finding | Disposition | Where it landed |
|---|---|---|
| `corvid new my_app --template backend` — no `--template` flag | DOCS | Prompt now shows `corvid new my_app` + tells you to copy/paste from a reference app's `main.cor` + `migrations/` |
| `corvid build --sign --key dev.key` — `--key` flag doesn't exist | DOCS | Prompt now shows `corvid build --sign dev.key` — `--sign <KEY_PATH>` takes the key path directly |
| `corvid deploy package . ...` — `.` rejected | DOCS | Prompt now shows `corvid deploy package "$(pwd)" --out deploy/` and exports `CORVID_DEPLOY_SIGNING_KEY` first (which the impl requires) |
| `corvid jobs run --kill-after 2s some_job` — flag doesn't exist | DOCS | Prompt now shows `corvid jobs run --workers 4 --max-runtime-ms 30000` — real flag, no positional |
| `corvid audit my_app` — directory rejected | DOCS | Prompt now shows `corvid audit src/main.cor` — takes a FILE not a dir |
| Dockerfile assumes Corvid monorepo (`cargo build -p corvid-cli`, `COPY examples/backend/<name>`, `COPY std std`) | **CODE** | `render_dockerfile` rewritten in `1455b6c` to pull a published `corvid` binary from `ghcr.io/micrurus-ai/corvid:${CORVID_VERSION}` into a distroless runtime, COPY only your standalone app's `src/` / `corvid.toml` / `migrations/` / `evals/` / `traces/` into `/app/`, CMD into `corvid serve --listen 0.0.0.0:8000`. Regression-guarded by 3 adversarial assertions so the monorepo paths can't quietly come back. |

## What was a stale-binary finding (already shipped at HEAD)

You were on `corvid 0.0.1` which predated several recent
slices. At HEAD these surfaces exist — please re-test them on
the refreshed install (see below):

| Your finding | Status at HEAD |
|---|---|
| `corvid serve` doesn't exist | **Shipped in `9c2faf6` + this session's E0-serve-5/6 HTTP approval queue.** `corvid serve <app>/src/main.cor --listen 127.0.0.1:8000` runs the in-process interpreter dispatcher for `server` block routes; approval-gated POSTs answer `202 + {"approval_id", "poll"}` and you can transition via `POST /__approvals/<id>/{approve,deny}`. |
| `--cdylib` flag missing on `deploy package` | **It exists.** `corvid deploy package <app> --out <dir> --cdylib <path>` binds the deploy attestation to the cdylib SHA-256 so `corvid claim --explain` and `corvid deploy package` can't drift apart. |
| `--explain-failures` flag missing on `claim audit` | **Shipped in `f3a8d0d`** (43T claim-audit-explain-failures). `corvid claim audit --explain-failures` returns typed `ClaimFindingKind` + `suggested_fix` per finding. |
| `sbom.spdx.json` not generated | **Shipped in `a06f1fe`** (43M SPDX SBOM in deploy package). |
| Hardcoded `C:\Users\SBW\...` staticlib path baked into binary | **Retired in `3f77ec1`** — `discover_staticlib` dynamically searches now. (See "Still filed but not shipped" below for the remaining `cargo install` install-side friction.) |

## What's filed but not yet shipped — please skip these on the retry

These are real follow-ups we owe; we want your retry time
spent on the surfaces that ARE fixed, not on these.

| Filed slice | What it covers |
|---|---|
| `35V2-P33-install-staticlib-fallback` | Improve the diagnostic when `cargo install --path crates/corvid-cli` produces a binary without the staticlib alongside; fall back to interpreter for `--target=auto` with a clear `↻ running via interpreter: …` notice. The install-script path (`curl -fsSL https://corvid-lang.org/install.sh \| sh`) ships the staticlib alongside the binary and is the recommended install for the retry. |
| `35V2-P33-corvid-run-with-args` | `corvid run` arg passthrough for parameterized agents on the interpreter target. For the retry, please use `corvid serve` (HTTP) or `corvid build --target=cdylib` (signed cdylib) for multi-agent / parameterized apps; both work at HEAD. |
| `35V2-P33-corvid-run-agent-flag` | Either ship the `--agent <name>` flag the diagnostic suggests, or change the diagnostic. Out of scope for the retry. |
| `35V2-P33-prompt-stdlib-framing` | Clarify in the prompt that `std.db` / `std.http` / `std.jobs` are typed envelope surfaces and the executing path is via `corvid migrate up` / `corvid serve` / `corvid jobs run`. Your point was well-taken; clarification doesn't change the runtime behavior, just the prompt's framing. |
| `35V2-P33-readme-status-refresh` | Refresh the README Status section you cited (line 488, 2026-04-29 audit snapshot) to reflect the closed Phase 38 + 39 audit-correction tracks (`t38l_d3_checkpoints_survive_unclean_shutdown` + 38K/38L/38M closed). At HEAD the durable-job crash-recovery + HTTP serving + auth/approvals ARE shipped — the README just hasn't been refreshed to say so. |
| `35V2-P33-corvid-run-with-args-regression` | End-to-end test for parameterized agent run. Code-side gate. |
| `35V2-P33-deploy-dockerfile-builds` | A CI gate that runs `docker build` against the generated Dockerfile from a standalone app dir. Code-side gate. |

## How to refresh your install

```sh
# 1. Discard the corvid 0.0.1 install you had.
rm -f ~/.cargo/bin/corvid             # or wherever your binary was

# 2. Install from the script (this ships the staticlib
#    alongside the binary; cargo-install from source doesn't).
curl -fsSL https://corvid-lang.org/install.sh | sh

# 3. Verify the binary is post-1455b6c.
corvid --version
# expected: something including a SHA at or after `1455b6c`.
# If you see a noticeably earlier SHA, please reply on the
# same channel and we'll send a direct binary link.
```

## What we'd love you to retry (30-60 minutes, not another half-day)

A targeted retest of just the surfaces we fixed:

1. **Re-walk the prompt's "Build path (suggested)" section
   exactly as written.** The 6 fixed commands above (`corvid
   new`, `corvid build --sign`, `corvid deploy package
   "$(pwd)"`, `corvid jobs run`, `corvid audit
   src/main.cor`) all run now. We want you to confirm a
   reviewer following the prompt literally now reaches `ls
   deploy/` without copy-paste failure.
   <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/33m-friends-and-family-prompt.md#build-path-suggested-not-required>

2. **Try `corvid serve`** against your tiny preferences
   agent. The HTTP surface you reported as unreachable
   exists at HEAD. POST to an approval-gated route, watch
   the `202 + approval_id` response, GET `/__approvals`,
   then `POST /__approvals/<id>/approve` and see the
   agent's result come back.
   <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/learnings.md#http-approval-queue-corvid-serve-answers-202-instead-of-403-2026-06-04>

3. **`docker build` the generated Dockerfile.** Carve down
   your preferences agent into a 100-line standalone app
   (no monorepo context), run `corvid deploy package
   "$(pwd)" --out deploy/`, then literally `cd deploy &&
   docker build .` against your local working directory.
   The new Dockerfile pulls
   `ghcr.io/micrurus-ai/corvid:${CORVID_VERSION}` into a
   distroless runtime and COPYs your app sources — should
   build clean from any standalone app dir. If it doesn't,
   that's the slice
   `35V2-P33-deploy-dockerfile-builds` will catch but we'd
   love to know first.

That's it for the retry. If those three steps all work, the
prompt is ready to go out to the next reviewer.

## What you do NOT need to retry

- The compile-time moat surfaces you already validated
  (approve-before-dangerous, compile-time budgets,
  migrations, jobs queue mechanics). Your "what worked"
  section is your write-up of the part that works; we don't
  need a re-validation.
- The `corvid run` parameterized / multi-agent flow. That's
  filed and explicitly out of retest scope.
- The README Status section. That's filed.
- Any honest-moment / over-claim moments — those were
  acknowledged and refreshed in the dispositions above.

## What if you find more

Same channel (DM / email / Signal — whichever the original
outreach came through). One more issue at
<https://github.com/Micrurus-Ai/Corvid-lang/issues/new> with
label `friends-and-family-trial` and title `[Trial-retry]
anonymous-2026-06-04 — <one-line summary>` would be great if
the retry surfaces anything new, but a one-paragraph reply
on the channel is equally valuable.

## One more thing

You said at the end of the report: *"If you want, I can pull
the live docs/reference apps and check whether they paper
over any of these."* The answer is yes please — but it's the
SAME ask as the retry above (just add: clone the repo, skim
the inventions matrix at
<https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/inventions.md#proof-matrix>,
and let us know if anything in the docs reads as having been
written by someone who hadn't tried the commands themselves).

If you'd rather not do the retry — totally fair, the first
report already paid for the bar entry fee twice over —
please reply on the channel so we don't keep the slot open
for you. Otherwise we'll save your handle for the launch
announcement's contributor list unless you ask to be
anonymized.

Thank you again. Your honesty about the friction was the
right shape of feedback at the right time.
