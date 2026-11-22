"""Run the numbered roadmap serially in its dedicated Git checkout."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time


BRANCHES = {"corvid/roadmap-tasks", "corvid/remaining-roadmap"}
FIELDS = {
    "status": {"type": "string", "enum": ["complete", "continue", "awaiting_feedback", "blocked"]},
    "summary": {"type": "string"},
    "verification": {"type": "string"},
    "files": {"type": "array", "items": {"type": "string"}},
}


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def run(review_pending=False, retry_blocked=False):
    branch = git("branch", "--show-current")
    if branch not in BRANCHES:
        raise SystemExit("Run in a dedicated roadmap checkout: " + ", ".join(sorted(BRANCHES)))
    if git("diff", "--cached", "--name-only"):
        raise SystemExit("Inspect staged changes before starting the roadmap worker.")
    state = Path(git("rev-parse", "--absolute-git-dir")) / "corvid-roadmap"
    state.mkdir(exist_ok=True)
    lock = (state / "lock").open("a+b")
    lock.seek(0)
    if os.name == "nt":
        import msvcrt
        msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
    else:
        import fcntl
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    schema = state / "response-schema.json"
    schema.write_text(json.dumps({"type": "object", "properties": FIELDS,
                                  "required": list(FIELDS), "additionalProperties": False}))
    steps = re.findall(r"^\| (\d+) \| ([^|]+) \| ([^|]+) \|$",
                       Path("ROADMAP.md").read_text(encoding="utf-8"), re.M)
    if [int(n) for n, _, _ in steps] != list(range(1, 14)):
        raise SystemExit("Expected the ordered roadmap steps 1 through 13.")
    git("fetch", "origin", branch)
    git("merge-base", "--is-ancestor", "FETCH_HEAD", "HEAD")
    # Also finishes a previously interrupted push. Never force-push.
    git("push", "origin", f"HEAD:refs/heads/{branch}")
    trailers = git("log", "--format=%(trailers:key=Corvid-Roadmap-Step,valueonly)").splitlines()
    pending = set(git("log", "--format=%(trailers:key=Corvid-Roadmap-Feedback,valueonly)").split())
    pending.difference_update(trailers)
    blocked = set(git("log", "--format=%(trailers:key=Corvid-Roadmap-Blocked,valueonly)").split())
    blocked.difference_update(trailers)
    for number, task, acceptance in steps:
        if number in blocked and not retry_blocked:
            print(f"Step {number} remains blocked. Use --retry-blocked when ready.", flush=True)
            continue
        if number in trailers or (number in pending and not review_pending):
            continue
        note = state / f"step-{number}.json"
        while True:
            head = git("rev-parse", "HEAD")
            previous = note.read_text(encoding="utf-8") if note.exists() else ""
            print(f"Working on step {number}: {task}", flush=True)
            prompt = f"""Implement step {number} in ROADMAP.md: {task}
Acceptance: {acceptance}
Read the linked detailed roadmap. Work only on this step in this dedicated
checkout. Preserve existing work. Use the existing language and tooling where
possible. Update the learning files and roadmap progress with actual results.
Run relevant tests at normal entry points. Never commit, push, deploy, release,
contact people, use paid providers, or perform external publication. The runner
handles Git. Return complete only if every acceptance condition really passed;
list exactly the project-relative files to stage, including deletions. Return
continue with concrete progress if more development fits a later pass. Return
blocked for external dependencies and state precisely what is missing. Do not
invent user studies, authorizations, benchmarks or independent results.
Unresolved technical checkpoints: {sorted(blocked - {number})}. Read their ROADMAP.md
entries before starting. Only proceed with independent work. If a required result
is missing, record the dependency in ROADMAP.md and return blocked. Never infer
release readiness or adoption results from incomplete steps.
Earlier steps awaiting external feedback: {sorted(pending)}. Proceed with independent
engineering work. Keep design choices provisional where feedback is pending.
If all feasible engineering and tests for this step are verified and only
independent reader or adoption feedback remains, update ROADMAP.md with the
specific pending check and return awaiting_feedback. This is a checkpoint,
not a completed step. Use blocked for a genuine technical dependency or required
approval. Never publish or claim release readiness with unresolved prerequisites.
Each pass should finish one useful, tested increment. For step 9 start with the
offline drawing editor, then work through its remaining acceptance criteria.
Read the latest commits before repeating old checks. For continue, include the
verified files for this increment so the runner can commit and push it now.
If a checkpoint is already accurately documented and unchanged, use an empty
file list and identify the existing roadmap entry in verification.
Previous pass: {previous}"""
            with tempfile.TemporaryDirectory() as temp:
                result = Path(temp) / "response.json"
                command = ["codex", "exec", "--sandbox", "workspace-write",
                           "--output-schema", str(schema), "-o", str(result), "-"]
                for attempt in range(3):
                    code = subprocess.run(command, input=prompt, text=True, encoding="utf-8").returncode
                    if code == 0 and result.exists():
                        break
                    time.sleep(2 ** attempt)
                else:
                    raise SystemExit(f"Worker failed on step {number}; rerun to resume.")
                report = json.loads(result.read_text(encoding="utf-8"))
            note.write_text(json.dumps(report, indent=2), encoding="utf-8")
            print(f"Step {number}: {report['status']} - {report['summary']}", flush=True)
            if git("rev-parse", "HEAD") != head:
                raise SystemExit("Checkout changed during development; inspect before resuming.")
            progressing = report["status"] == "continue"
            if not report["verification"].strip():
                raise SystemExit("Missing verification.")
            feedback = report["status"] == "awaiting_feedback"
            deferred = report["status"] == "blocked"
            if not report["files"]:
                if (feedback or deferred) and not git("status", "--porcelain"):
                    (pending if feedback else blocked).add(number)
                    break
                raise SystemExit("Missing deliverables for this pass.")
            if (feedback or deferred) and "ROADMAP.md" not in report["files"]:
                raise SystemExit("Record outstanding feedback in ROADMAP.md.")
            git("--literal-pathspecs", "add", "--", *report["files"])
            git("diff", "--cached", "--check")
            marker = "Corvid-Roadmap-Feedback" if feedback else "Corvid-Roadmap-Step"
            verb = "Checkpoint" if feedback else "Complete"
            if progressing:
                marker, verb = "Corvid-Roadmap-Progress", "Advance"
            if deferred:
                marker, verb = "Corvid-Roadmap-Blocked", "Checkpoint"
            git("commit", "-m", f"{verb} roadmap step {number}: {report['summary'].splitlines()[0]}",
                "-m", report["verification"], "-m", f"{marker}: {number}")
            git("push", "origin", f"HEAD:refs/heads/{branch}")
            if git("ls-remote", "origin", f"refs/heads/{branch}").split()[0] != git("rev-parse", "HEAD"):
                raise SystemExit("Remote commit verification failed.")
            if progressing:
                continue
            if deferred:
                blocked.add(number)
            else:
                blocked.discard(number)
            if feedback:
                pending.add(number)
            else:
                pending.discard(number)
            break
    if blocked:
        print("Technical checkpoints open:", ", ".join(sorted(blocked, key=int)))
    if pending:
        print("External feedback outstanding for steps:", ", ".join(sorted(pending, key=int)))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--review-pending", action="store_true", help="revisit feedback checkpoints when results are available")
    parser.add_argument("--retry-blocked", action="store_true", help="retry open technical checkpoints")
    args = parser.parse_args()
    run(args.review_pending, args.retry_blocked)
