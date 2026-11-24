"""Regression checks for parallel frontend task scheduling."""

from pathlib import Path
from tempfile import TemporaryDirectory
from threading import Barrier
import unittest
from unittest.mock import patch

import run_frontend as runner


class FrontendQueueTests(unittest.TestCase):
    def test_independent_tasks_overlap_before_their_dependent_starts(self):
        first_pair = Barrier(2, timeout=5)
        merged = set()
        assigned = {}
        tasks = {"F03": ("Motion", set(), ""), "F04": ("Components", set(), ""),
                 "F05": ("Rendering", {"F03", "F04"}, "")}

        def work(tree, state, task, title, check):
            assigned[task] = tree
            if task != "F05":
                first_pair.wait()
            else:
                self.assertEqual(merged, {"F03", "F04"})
            merged.add(task)

        with TemporaryDirectory() as tmp:
            state = Path(tmp)
            with patch.object(runner, "git", return_value=""), \
                 patch.object(runner, "completed", side_effect=lambda _: merged.copy()), \
                 patch.object(runner, "run_task", side_effect=work):
                runner.run_queue(state, [state / "one", state / "two"], state, tasks, False)
            self.assertNotEqual(assigned["F03"], assigned["F04"])
            self.assertEqual(runner.load(state / "current.json")["remaining"], [])


if __name__ == "__main__":
    unittest.main()
