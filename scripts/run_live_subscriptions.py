"""Finish the live subscription transport, typed clients, and frontend acceptance PRs."""

import argparse
from contextlib import ExitStack
import os
from pathlib import Path
import subprocess
import sys
import time

import run_frontend as frontend

TRANSPORT = "RT-3"
CLIENT = "RT-4"
TASKS = (TRANSPORT, CLIENT, "F08", "F18")
DEPENDENCIES = {TRANSPORT: set(), CLIENT: set(), "F08": {TRANSPORT, CLIENT}, "F18": {"F08"}}
BRANCHES = {**frontend.BRANCHES, TRANSPORT: "runtime/live-subscription-transport",
            CLIENT: "frontend/typed-live-subscriptions"}
REALTIME_MARKER = "Corvid-Realtime-Complete"
TRANSPORT_SCOPE = """Implement the production session validation and authority-change observer needed by
corvid serve, as well as the WebSocket transport. You own this integration. Inspect canonical session,
expiry and permission handling and reuse it. Close the validation/observer race and test revocation on
an idle connection. Integrate the merged RT-1, RT-2 and RT-6 interfaces. Verify a real authenticated
connection, an offline commit in a fresh snapshot, a subsequent live edit, logout, and the documented
deployment modes. Keep RT-5/B10 and unrelated backend grammar outside this slice."""
CLIENT_SCOPE = """Implement the necessary live contract metadata (ordered table keys and public row
schema), generated TypeScript subscriptions and Corvid ui.live binding. You own the contract extension
required by this client. Reuse the canonical table and public JSON schema and existing path-argument
machinery; coordinate against the agreed route syntax. Exercise composite keys, revision identity,
cleanup and typed diagnostics. Verify the real transport as it becomes available. Do not alter the
server session runtime or claim end-to-end success from a mock."""
STATES = {TRANSPORT: "corvid-live-transport", CLIENT: "corvid-live-client",
          "F08": "corvid-frontend", "F18": "corvid-frontend"}


def checkout(root, common, task):
    branch = BRANCHES[task]
    for entry in frontend.git(root, "worktree", "list", "--porcelain").split("\n\n"):
        lines = entry.splitlines()
        if f"branch refs/heads/{branch}" in lines:
            return Path(lines[0].removeprefix("worktree "))
    tree = common / "corvid-live-completion" / "worktrees" / branch.split("/")[-1]
    if not tree.exists():
        tree.parent.mkdir(parents=True, exist_ok=True)
        frontend.git(root, "worktree", "add", "--detach", str(tree), "origin/main")
    return tree


def run_one(root, common, task):
    tree = checkout(root, common, task)
    if frontend.git(tree, "status", "--porcelain"):
        raise RuntimeError(f"Uncommitted work in {tree}; inspect before resuming.")
    state = common / STATES[task]
    frontend.write(state / "schema.json", {"type": "object", "properties": frontend.FIELDS,
                                          "required": list(frontend.FIELDS), "additionalProperties": False})
    if task in (TRANSPORT, CLIENT):
        frontend.ROADMAP = Path("docs/proposals/real-time.md")
        frontend.MARKER = REALTIME_MARKER
        frontend.BRANCHES[task] = BRANCHES[task]
        title, scope = (("WebSocket transport for live subscriptions", TRANSPORT_SCOPE)
                        if task == TRANSPORT else ("Typed live subscriptions for Corvid clients", CLIENT_SCOPE))
        check = scope
    else:
        title, _, check = frontend.queue(frontend.git(root, "show", "origin/main:docs/frontend-roadmap.md"))[task]
        scope = frontend.FRONTEND_SCOPE
    frontend.run_task(tree, state, task, title, check, scope)


def eligible(task, done, running, previous, revision, now):
    return (task not in done and task not in running and DEPENDENCIES[task] <= done
            and (previous.get("revision") != revision or now >= previous.get("retry_after", 0)))


def supervise(root, common, state, force):
    # Reserve the same locks as the older launchers before starting either writer.
    with ExitStack() as locks:
        for name in sorted(set(STATES.values()) | {state.name}):
            directory = common / name
            directory.mkdir(exist_ok=True)
            locks.enter_context(frontend.exclusive(directory))
        history_file = state / "attempts.json"
        history = {} if force else frontend.load(history_file)
        running = {}
        try:
            while True:
                for task, process in list(running.items()):
                    code = process.poll()
                    if code is None:
                        continue
                    del running[task]
                    report = frontend.load(common / STATES[task] / f"{task}.json")
                    entry = history[task]
                    entry["result"] = f"worker exited {code}" if code else report.get("status", "returned")
                    # A new main revision wakes dependencies immediately. Back off if nothing changed.
                    entry["retry_after"] = time.time() + min(3600, 60 * 2 ** min(entry["attempts"], 6))
                    if entry["result"] == "awaiting_feedback":
                        entry["retry_after"] = time.time() + 86400
                    frontend.write(history_file, history)
                try:
                    frontend.git(root, "fetch", "--quiet", "origin", "main")
                except subprocess.CalledProcessError as error:
                    frontend.write(state / "current.json", {"status": "retrying fetch", "error": str(error),
                                                            "running": sorted(running)})
                    time.sleep(60)
                    continue
                revision = frontend.git(root, "rev-parse", "origin/main")
                done = frontend.completed(root) | set(frontend.git(root, "log", "origin/main",
                    f"--format=%(trailers:key={REALTIME_MARKER},valueonly)").split())
                if set(TASKS) <= done and not running:
                    frontend.write(state / "current.json", {"status": "complete", "merged": list(TASKS)})
                    return
                for task in TASKS:
                    previous = history.get(task, {})
                    if not eligible(task, done, running, previous, revision, time.time()):
                        continue
                    count = previous.get("attempts", 0) if previous.get("revision") == revision else 0
                    env = os.environ.copy()
                    if task == CLIENT:
                        env["CARGO_TARGET_DIR"] = str(root / "target")
                    with (state / f"{task}.log").open("ab") as log:
                        running[task] = subprocess.Popen([sys.executable, "-u", __file__, "--task", task],
                            cwd=root, stdout=log, stderr=subprocess.STDOUT, env=env)
                    history[task] = {"revision": revision, "attempts": count + 1, "result": "working"}
                    frontend.write(history_file, history)
                frontend.write(state / "current.json", {"status": "running" if running else "waiting",
                    "workers": {task: proc.pid for task, proc in running.items()},
                    "merged": sorted(set(TASKS) & done), "remaining": sorted(set(TASKS) - done)})
                time.sleep(30)
        finally:
            for process in running.values():
                process.wait()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task", choices=TASKS, help=argparse.SUPPRESS)
    parser.add_argument("--retry", action="store_true", help="retry pending work immediately on startup")
    args = parser.parse_args()
    root = Path(frontend.git(".", "rev-parse", "--show-toplevel"))
    common = Path(frontend.git(root, "rev-parse", "--path-format=absolute", "--git-common-dir"))
    if args.task:
        lock_dir = common / 'corvid-live-completion' / args.task
        lock_dir.mkdir(parents=True, exist_ok=True)
        with frontend.exclusive(lock_dir):
            run_one(root, common, args.task)
    else:
        supervise(root, common, common / "corvid-live-completion", args.retry)
