"""Run the numbered roadmap serially in its dedicated Git checkout."""

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time


BRANCH = "corvid/roadmap-tasks"
FIELDS = {
    "status": {"type": "string", "enum": ["complete", "continue", "blocked"]},
    "summary": {"type": "string"},
    "verification": {"type": "string"},
    "files": {"type": "array", "items": {"type": "string"}},
}


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def run():
    if git("branch", "--show-current") != BRANCH:
        raise SystemExit(f"Run in the dedicated {BRANCH} checkout.")
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
    git("fetch", "origin", BRANCH)
    git("merge-base", "--is-ancestor", "FETCH_HEAD", "HEAD")
    # Also finishes a previously interrupted push. Never force-push.
    git("push", "origin", f"HEAD:refs/heads/{BRANCH}")
    trailers = git("log", "--format=%(trailers:key=Corvid-Roadmap-Step,valueonly)").splitlines()
    for number, task, acceptance in steps:
        if number in trailers:
            continue
        note = state / f"step-{number}.json"
        while True:
            head = git("rev-parse", "HEAD")
            previous = note.read_text(encoding="utf-8") if note.exists() else ""
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
            if report["status"] == "blocked":
                raise SystemExit(2)
            if report["status"] == "continue":
                continue
            if not report["verification"].strip() or not report["files"]:
                raise SystemExit("Missing verification or deliverables.")
            git("--literal-pathspecs", "add", "--", *report["files"])
            git("diff", "--cached", "--check")
            git("commit", "-m", f"Complete roadmap step {number}: {task.rstrip('.')}",
                "-m", report["verification"], "-m", f"Corvid-Roadmap-Step: {number}")
            git("push", "origin", f"HEAD:refs/heads/{BRANCH}")
            if git("ls-remote", "origin", f"refs/heads/{BRANCH}").split()[0] != git("rev-parse", "HEAD"):
                raise SystemExit("Remote commit verification failed.")
            break


if __name__ == "__main__":
    run()
