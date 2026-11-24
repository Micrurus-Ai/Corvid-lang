"""Develop the browser frontend queue in an isolated Git worktree."""

import argparse
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
from threading import Lock

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
    prompt = f"""Implement the remaining acceptance criteria for {task}, {title}, from {ROADMAP}.
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
Group related changes and browser assertions into one pass when feasible.
For Edge checks, add or reuse scripts/verify_<name>.py with --cli and --channel.
If Playwright is denied, finish independent implementation and local checks before
reporting the pending scripts together. The supervisor runs them on hosted Windows.
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


merge_lock = Lock()


def draft_pull_request(tree, branch, title, check):
    args = ["gh", "pr", "list", "--base", "main", "--head", branch, "--state", "open",
            "--json", "number,isDraft"]
    existing = json.loads(subprocess.check_output(args, cwd=tree))
    if existing:
        return existing[0]
    subprocess.run(["gh", "pr", "create", "--draft", "--base", "main", "--head", branch,
                    "--title", title, "--body-file", "-"], input=check + "\n",
                   cwd=tree, text=True, check=True)
    return json.loads(subprocess.check_output(args, cwd=tree))[0]


def finish(tree, branch, pull):
    tip = git(tree, "rev-parse", "HEAD")
    git(tree, "fetch", "origin", "main")
    if not ancestor(tree, tip, "origin/main"):
        if not ancestor(tree, "origin/main", tip):
            return False
        number = str(pull["number"])
        if pull["isDraft"]:
            subprocess.run(["gh", "pr", "ready", number], cwd=tree, check=True)
        while True:
            details = json.loads(subprocess.check_output(
                ["gh", "pr", "view", number, "--json", "mergeStateStatus,headRefOid"], cwd=tree))
            if details["headRefOid"] != tip:
                raise RuntimeError(f"Pull request #{number} changed after verification.")
            if details["mergeStateStatus"] == "BEHIND":
                return False
            if details["mergeStateStatus"] not in ("BLOCKED", "UNSTABLE", "UNKNOWN"):
                break
            checks = json.loads(subprocess.check_output(
                ["gh", "pr", "view", number, "--json", "statusCheckRollup"], cwd=tree))
            if not any(item.get("status") in ("QUEUED", "IN_PROGRESS", "PENDING")
                       or item.get("state") in ("PENDING", "EXPECTED")
                       for item in checks["statusCheckRollup"]):
                break
            time.sleep(30)
        subprocess.run(["gh", "pr", "merge", number, "--merge", "--match-head-commit", tip],
                       cwd=tree, check=True)
        git(tree, "fetch", "origin", "main")
    if not ancestor(tree, tip, "origin/main"):
        raise RuntimeError("Main verification failed.")
    if remote_tip(tree, branch) == tip:
        git(tree, "push", f"--force-with-lease=refs/heads/{branch}:{tip}", "origin", f":refs/heads/{branch}")
    git(tree, "switch", "--detach", "origin/main")
    git(tree, "branch", "-d", branch)
    return True


def verify_blocked_browser(tree, state, task, report):
    revision = git(tree, "rev-parse", "HEAD")
    receipt = state / f"verification-{revision}.txt"
    previous = receipt.read_text(encoding="utf-8") if receipt.exists() else ""
    suites = []
    for name in sorted(set(re.findall(r"\bscripts[/\\]verify_([a-z][a-z0-9_]*)\.py\b", report["verification"]))):
        title = f"Browser check for {name} at {revision}"
        path = tree / "scripts" / f"verify_{name}.py"
        if title in previous or not path.is_file():
            continue
        source = path.read_text(encoding="utf-8")
        if name == "browser_stream" or all(flag in source for flag in ('"--cli"', '"--channel"')):
            suites.append(name)
    if not suites:
        return False
    suite = ",".join(suites)
    title = f"Browser check for {suite} at {revision}"
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
    write(state / f"current-{task}.json", {"task": task, "head": revision,
                                   "status": "waiting for browser check", "run": run})
    while True:
        result = json.loads(gh("run", "view", str(run), "--json", "status,conclusion,url"))
        if result["status"] == "completed":
            break
        time.sleep(30)
    # A failed batch may have stopped before later suites executed.
    summary = f"{title}: {result['conclusion']}. {result['url']}"
    if result["conclusion"] == "success":
        summary = "\n".join(f"Browser check for {name} at {revision}: success. {result['url']}" for name in suites)
    elif summary in previous:
        return False
    receipt.write_text("\n".join(filter(None, [previous, summary])) + "\n", encoding="utf-8")
    return True


def run_task(tree, state, task, title, check):
    branch = BRANCHES[task]
    note = state / f"{task}.json"
    refs = git(tree, "branch", "--list", branch)
    if refs:
        git(tree, "switch", branch)
    elif remote_tip(tree, branch):
        git(tree, "fetch", "origin", branch)
        git(tree, "switch", "-c", branch, f"origin/{branch}")
    else:
        git(tree, "switch", "-c", branch, "origin/main")
    while True:
        git(tree, "fetch", "origin", "main")
        if not ancestor(tree, "origin/main", "HEAD"):
            git(tree, "merge", "--no-edit", "origin/main")
        if git(tree, "rev-parse", "HEAD") == git(tree, "rev-parse", "origin/main"):
            git(tree, "commit", "--allow-empty", "-m", f"Start {title}")
        git(tree, "push", "origin", f"HEAD:refs/heads/{branch}")
        draft_pull_request(tree, branch, title, check)
        head = git(tree, "rev-parse", "HEAD")
        print(f"{task} started at {head[:9]}: {title}", flush=True)
        previous = load(note)
        write(state / f"current-{task}.json", {"task": task, "head": head, "status": "working"})
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
        pull = draft_pull_request(tree, branch, title, check)
        write(note, report)
        print(f"{task} {status}: {report['summary']}", flush=True)
        if status == "complete":
            with merge_lock:
                if finish(tree, branch, pull):
                    write(state / f"current-{task}.json", {"task": task, "status": "merged"})
                    return
            continue
        if status == "blocked" and verify_blocked_browser(tree, state, task, report):
            continue
        if status != "continue":
            write(state / f"current-{task}.json", {"task": task, "status": status})
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
        print("Supervisor:", load(state / "current.json"))
        for event in sorted(state.glob("current-F*.json")):
            print(load(event))
        return
    state.mkdir(exist_ok=True)
    with exclusive(state):
        git(root, "fetch", "origin", "main")
        tasks = queue(git(root, "show", "origin/main:docs/frontend-roadmap.md"))
        write(state / "schema.json", {"type": "object", "properties": FIELDS,
                                     "required": list(FIELDS), "additionalProperties": False})
        trees = [tree, tree.with_name(tree.name + "-parallel")]
        for checkout in trees:
            if not checkout.exists():
                checkout.parent.mkdir(parents=True, exist_ok=True)
                git(root, "worktree", "add", "--detach", str(checkout), "origin/main")
            if Path(git(checkout, "rev-parse", "--path-format=absolute", "--git-common-dir")).resolve() != common.resolve():
                raise RuntimeError(f"Unexpected worktree: {checkout}")
        run_queue(root, trees, state, tasks, args.retry_blocked)


def run_queue(root, trees, state, tasks, retry_blocked):
    attempted = set()
    running = {}
    errors = {}
    with ThreadPoolExecutor(max_workers=len(trees)) as pool:
        while True:
            done = completed(root)
            ready = [task for task, (_, deps, _) in tasks.items()
                     if task not in done | attempted and deps <= done
                     and (retry_blocked or load(state / f"{task}.json").get("status")
                          not in ("blocked", "awaiting_feedback"))]
            branches = {tree: git(tree, "branch", "--show-current") for tree in trees}
            for tree in trees:
                if tree in running.values() or tree in errors:
                    continue
                reserved = {branch for other, branch in branches.items() if other != tree}
                task = next((t for t in ready if BRANCHES[t] == branches[tree]), None)
                task = task or next((t for t in ready if BRANCHES[t] not in reserved), None)
                if task is None:
                    continue
                dirty = git(tree, "status", "--porcelain")
                if dirty:
                    event = load(state / f"current-{task}.json") or load(state / "current.json")
                    if (branches[tree] != BRANCHES[task] or event.get("task") != task
                            or event.get("head") != git(tree, "rev-parse", "HEAD")
                            or git(tree, "diff", "--cached", "--name-only")):
                        errors[tree] = f"Inspect staged or unattributed work in {tree}"
                        continue
                title, _, check = tasks[task]
                attempted.add(task)
                ready.remove(task)
                running[pool.submit(run_task, tree, state, task, title, check)] = tree
            if not running:
                break
            write(state / "current.json", {"status": "running", "workers": len(running),
                                           "capacity": len(trees)})
            finished, _ = wait(running, return_when=FIRST_COMPLETED)
            for future in finished:
                tree = running.pop(future)
                try:
                    future.result()
                except Exception as exc:
                    errors[tree] = str(exc)
                    print(f"{tree}: {exc}", flush=True)
    write(state / "current.json", {"status": "stopped" if errors else "finished pass",
                                   "errors": {str(k): v for k, v in errors.items()},
                                   "remaining": sorted(tasks.keys() - completed(root))})
    if errors:
        raise RuntimeError(str(errors))


if __name__ == "__main__":
    main()
