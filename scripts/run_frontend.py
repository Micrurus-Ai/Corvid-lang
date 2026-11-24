"""Develop the browser frontend queue in an isolated Git worktree."""

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

from run_roadmap import FIELDS


ROADMAP = Path("docs/frontend-roadmap.md")
MARKER = "Corvid-Frontend-Complete"
# Stable Git names, independent of queue ordering and edits to display titles.
BRANCHES = {
    "F01": "frontend/semantic-html",
    "F02": "frontend/responsive-styling",
    "F03": "frontend/svg-and-motion",
    "F04": "frontend/component-lifecycle",
    "F05": "frontend/reactive-rendering",
    "F06": "frontend/accessible-interactions",
    "F07": "frontend/navigation",
    "F08": "frontend/async-resources",
    "F09": "frontend/javascript-interop",
    "F10": "frontend/browser-api-bindings",
    "F11": "frontend/offline-sync",
    "F12": "frontend/graphics-and-media",
    "F13": "frontend/internationalization",
    "F14": "frontend/server-rendering-and-hydration",
    "F15": "frontend/assistant-accessible-actions",
    "F16": "frontend/component-tooling",
    "F17": "frontend/performance-and-compatibility",
    "F18": "frontend/application-acceptance",
}


def git(cwd, *args):
    return subprocess.check_output(["git", *args], cwd=cwd, text=True, encoding="utf-8").strip()


def queue(source):
    tasks = {}
    for line in source.splitlines():
        if not re.match(r"^\| F\d", line):
            continue
        task, title, depends, check = (s.strip() for s in line.strip("|").split("|"))
        deps = set(re.findall(r"F\d+", depends))
        if task != f"F{len(tasks) + 1:02d}" or deps - tasks.keys() or not title or not check:
            raise ValueError(f"Invalid task or dependency order: {line}")
        tasks[task] = (title, deps, check)
    if not tasks:
        raise ValueError("Frontend queue is empty.")
    if tasks.keys() != BRANCHES.keys() or len(set(BRANCHES.values())) != len(tasks):
        raise ValueError("Each frontend roadmap task needs a unique, descriptive branch name.")
    return tasks


@contextmanager
def exclusive(state):
    with (state / "lock").open("a+b") as lock:
        if os.name == "nt":
            import msvcrt
            lock.seek(0)
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def load(path):
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}


def write(path, value):
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(value, indent=2), encoding="utf-8")
    tmp.replace(path)


def worker(tree, state, task, title, check, previous):
    revision = git(tree, "rev-parse", "HEAD")
    receipt = state / f"verification-{revision}.txt"
    host_result = receipt.read_text(encoding="utf-8") if receipt.is_file() else "None."
    prompt = f"""Implement one tested increment of {task}, {title}, from {ROADMAP}.
Acceptance for the whole task: {check}
Inspect the repository and relevant examples before editing. Work only on frontend
capabilities. Coordinate shared compiler changes by keeping them frontend-specific.
Claude Code owns backend and AI syntax. If a new backend interface is required,
report that dependency. Use ordinary readable Corvid and document every public
function or grammar change in the learning guide and reference. Run the relevant
checks through the public toolchain. No release, deployment, messages, or Git
mutations. The supervisor handles commits and GitHub. Report the exact changed
files and commands/results. Return complete only if all acceptance criteria
pass on this revision. Return continue for a tested increment with work left.
Return blocked or awaiting_feedback for a concrete dependency. Never invent
browser, device, assistant, accessibility, performance, or human feedback results.
First identify the remaining acceptance criteria. If the implementation already
covers them and only an external check remains, do not add optional features.
Use a matching host result for the checks it actually ran. If verification is
unavailable, return blocked with the exact command and dependency. If your pass
changes the inputs to that check, the earlier result no longer covers those edits.
Host verification for starting revision {revision}: {host_result}
Prior report (may predate integration with current main): {previous}
"""
    with tempfile.TemporaryDirectory(dir=state) as tmp:
        result = Path(tmp) / "result.json"
        command = ["codex", "exec", "--sandbox", "workspace-write", "--output-schema",
                   str(state / "schema.json"), "-o", str(result), "-"]
        with (state / f"{task}.log").open("ab") as log:
            for attempt in range(3):
                result.unlink(missing_ok=True)
                response = subprocess.run(command, cwd=tree, input=prompt.encode("utf-8"), stdout=log,
                                          stderr=subprocess.STDOUT)
                if response.returncode == 0 and result.is_file():
                    break
                if attempt == 2:
                    raise RuntimeError(f"Worker failed after three attempts. See {state / (task + '.log')}.")
                time.sleep(2 ** attempt)
        return load(result)


def completed(tree):
    return set(git(tree, "log", "origin/main", f"--format=%(trailers:key={MARKER},valueonly)").split())


def remote_tip(tree, branch):
    line = git(tree, "ls-remote", "--heads", "origin", f"refs/heads/{branch}")
    return line.split()[0] if line else None


def ancestor(tree, older, newer):
    result = subprocess.run(["git", "merge-base", "--is-ancestor", older, newer], cwd=tree)
    if result.returncode not in (0, 1):
        result.check_returncode()
    return result.returncode == 0


def finish(tree, branch):
    tip = git(tree, "rev-parse", "HEAD")
    git(tree, "fetch", "origin", "main")
    if not ancestor(tree, tip, "origin/main"):
        if not ancestor(tree, "origin/main", tip):
            return False
        try:
            git(tree, "push", "origin", "HEAD:refs/heads/main")
        except subprocess.CalledProcessError:
            git(tree, "fetch", "origin", "main")
            if not ancestor(tree, "origin/main", tip) and not ancestor(tree, tip, "origin/main"):
                return False
            raise
        git(tree, "fetch", "origin", "main")
    if not ancestor(tree, tip, "origin/main"):
        raise RuntimeError("Main verification failed.")
    if remote_tip(tree, branch) == tip:
        git(tree, "push", f"--force-with-lease=refs/heads/{branch}:{tip}", "origin", f":refs/heads/{branch}")
    git(tree, "switch", "--detach", "origin/main")
    git(tree, "branch", "-d", branch)
    return True


def verify_blocked_browser(tree, state, task, report):
    suites = [name for name in ("trail_conditions", "trail_guide", "browser_stream")
              if f"verify_{name}.py" in report["verification"]]
    if not suites:
        return False
    revision = git(tree, "rev-parse", "HEAD")
    receipt = state / f"verification-{revision}.txt"
    previous = receipt.read_text(encoding="utf-8") if receipt.exists() else ""
    results = []
    for suite in suites:
        title = f"Browser check for {suite} at {revision}"
        if title in previous:
            continue
        workflow = "frontend-browser-check.yml"
        def gh(*args):
            return subprocess.check_output(["gh", *args], cwd=tree, text=True).strip()

        def find_run():
            runs = json.loads(gh("run", "list", "--workflow", workflow, "--branch", "main",
                                 "--limit", "100", "--json", "displayTitle,databaseId"))
            return next((r["databaseId"] for r in runs if r["displayTitle"] == title), None)

        run = find_run()
        if run is None:
            gh("workflow", "run", workflow, "--ref", "main",
               "-f", f"revision={revision}", "-f", f"suite={suite}")
            for _ in range(12):
                time.sleep(5)
                run = find_run()
                if run is not None:
                    break
        if run is None:
            raise RuntimeError(f"GitHub has not listed the dispatched run: {title}")
        write(state / "current.json", {"task": task, "head": revision,
                                       "status": "waiting for browser check", "run": run})
        while True:
            result = json.loads(gh("run", "view", str(run), "--json", "status,conclusion,url"))
            if result["status"] == "completed":
                break
            time.sleep(30)
        results.append(f"{title}: {result['conclusion']}. {result['url']}")
    if results:
        receipt.write_text("\n".join(filter(None, [previous, *results])) + "\n", encoding="utf-8")
    return bool(results)


def run_task(tree, state, task, title, check):
    branch = BRANCHES[task]
    note = state / f"{task}.json"
    refs = git(tree, "branch", "--list", branch)
    if refs:
        git(tree, "switch", branch)
    elif remote_tip(tree, branch):
        git(tree, "fetch", "origin", branch)
        git(tree, "switch", "-c", branch, "FETCH_HEAD")
    else:
        git(tree, "switch", "-c", branch, "origin/main")
    while True:
        git(tree, "fetch", "origin", "main")
        if not ancestor(tree, "origin/main", "HEAD"):
            git(tree, "merge", "--no-edit", "origin/main")
        head = git(tree, "rev-parse", "HEAD")
        print(f"{task} started at {head[:9]}: {title}", flush=True)
        previous = load(note)
        write(state / "current.json", {"task": task, "head": head, "status": "working"})
        report = worker(tree, state, task, title, check, previous)
        if git(tree, "rev-parse", "HEAD") != head or git(tree, "diff", "--cached", "--name-only"):
            raise RuntimeError("Checkout changed during the worker pass.")
        status = report["status"]
        files = report["files"]
        if not report["verification"].strip() or status not in FIELDS["status"]["enum"]:
            raise ValueError("Incomplete worker report.")
        if files:
            git(tree, "--literal-pathspecs", "add", "--", *files)
        if git(tree, "diff", "--name-only") or git(tree, "ls-files", "--others", "--exclude-standard"):
            raise RuntimeError("There are unreported changes. Inspect the dedicated checkout.")
        staged = git(tree, "diff", "--cached", "--name-only")
        if status == "continue" and not staged:
            raise RuntimeError("Worker returned continue without an increment.")
        if staged or status == "complete":
            git(tree, "diff", "--cached", "--check")
            marker = MARKER if status == "complete" else "Corvid-Frontend-Increment"
            git(tree, "commit", "--allow-empty", "-m", f"{task} {report['summary'].splitlines()[0]}",
                "-m", report["verification"], "-m", f"{marker}: {task}")
        git(tree, "push", "origin", f"HEAD:refs/heads/{branch}")
        if remote_tip(tree, branch) != git(tree, "rev-parse", "HEAD"):
            raise RuntimeError("Task push verification failed.")
        write(note, report)
        print(f"{task} {status}: {report['summary']}", flush=True)
        if status == "complete":
            if finish(tree, branch):
                return
            continue
        if status == "blocked" and verify_blocked_browser(tree, state, task, report):
            continue
        if status != "continue":
            return


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--status", action="store_true")
    mode.add_argument("--run", action="store_true")
    parser.add_argument("--retry-blocked", action="store_true")
    args = parser.parse_args()
    root = Path(git(".", "rev-parse", "--show-toplevel"))
    common = Path(git(root, "rev-parse", "--path-format=absolute", "--git-common-dir"))
    state = common / "corvid-frontend"
    suffix = hashlib.sha256(str(common).encode()).hexdigest()[:10]
    tree = Path(os.environ.get("LOCALAPPDATA", Path.home() / ".cache")) / "Corvid" / f"frontend-{suffix}"
    tasks = queue((root / ROADMAP).read_text(encoding="utf-8"))
    if args.status:
        done = completed(root)
        for task, (title, deps, _) in tasks.items():
            status = "merged" if task in done else load(state / f"{task}.json").get("status", "ready")
            if task not in done and deps - done:
                status = "waiting for " + ", ".join(sorted(deps - done))
            print(f"{task}  {status:24} {title}")
        print("Last runner event:", load(state / "current.json"), "\nCheckout:", tree, "\nLogs:", state)
        return
    state.mkdir(exist_ok=True)
    with exclusive(state):
        try:
            git(root, "fetch", "origin", "main")
            if not tree.exists():
                tree.parent.mkdir(parents=True, exist_ok=True)
                git(root, "worktree", "add", "--detach", str(tree), "origin/main")
            if Path(git(tree, "rev-parse", "--path-format=absolute", "--git-common-dir")).resolve() != common.resolve():
                raise RuntimeError("Unexpected worktree repository.")
            if git(tree, "status", "--porcelain"):
                event = load(state / "current.json")
                resume = event.get("task", "")
                if (resume not in tasks or git(tree, "branch", "--show-current") != BRANCHES[resume]
                        or event.get("head") != git(tree, "rev-parse", "HEAD")
                        or git(tree, "diff", "--cached", "--name-only")):
                    raise RuntimeError(f"Unattributed or staged changes in {tree}. Inspect before resuming.")
                title, _, check = tasks[resume]
                run_task(tree, state, resume, title, check)
            write(state / "schema.json", {"type": "object", "properties": FIELDS,
                                         "required": list(FIELDS), "additionalProperties": False})
            tasks = queue(git(root, "show", "origin/main:docs/frontend-roadmap.md"))
            for task, (title, deps, check) in tasks.items():
                done = completed(tree)
                if task in done or deps - done:
                    continue
                if load(state / f"{task}.json").get("status") in ("blocked", "awaiting_feedback") and not args.retry_blocked:
                    continue
                run_task(tree, state, task, title, check)
            write(state / "current.json", {"status": "finished pass", "remaining": sorted(tasks.keys() - completed(tree))})
        except Exception as error:
            write(state / "current.json", {**load(state / "current.json"), "status": "stopped", "error": str(error)})
            raise


if __name__ == "__main__":
    main()
