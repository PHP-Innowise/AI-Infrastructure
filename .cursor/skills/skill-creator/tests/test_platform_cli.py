#!/usr/bin/env python3
"""Deterministic tests for the native skill-evaluation CLI adapter."""

from __future__ import annotations

import os
import tempfile
import textwrap
import unittest
from pathlib import Path

from scripts.platform_cli import call_model, evaluate_trigger, platform_config
from scripts.run_eval import run_eval
from scripts.run_loop import run_loop


FAKE_CLI = r'''#!/usr/bin/env python3
import json
import os
import re
import sys
from pathlib import Path

workspace = Path.cwd()
with Path(os.environ["SKILL_CREATOR_FAKE_LOG"]).open("a", encoding="utf-8") as log:
    log.write(f"{workspace}\n")

skill_files = list(workspace.glob(".agents/skills/*/SKILL.md"))
skill_files += list(workspace.glob(".cursor/skills/*/SKILL.md"))
marker = ""
if skill_files:
    match = re.search(r"SKILL_TRIGGERED_[A-F0-9]+", skill_files[0].read_text(encoding="utf-8"))
    marker = match.group(0) if match else ""

args = sys.argv[1:]
response = marker if marker and "positive" in args[-1].lower() else "NOT_TRIGGERED"

if "--output-last-message" in args:
    output = Path(args[args.index("--output-last-message") + 1])
    output.write_text("<new_description>Improved native description.</new_description>", encoding="utf-8")
elif "--output-format" in args:
    if not skill_files:
        response = "<new_description>Improved native description.</new_description>"
    print(json.dumps({"type": "result", "result": response}))
else:
    print(response)
'''


class PlatformCliTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="skill-cli-test-")
        self.root = Path(self.temporary.name)
        self.executable = self.root / "fake-cli"
        self.executable.write_text(textwrap.dedent(FAKE_CLI), encoding="utf-8")
        self.executable.chmod(0o755)
        self.log = self.root / "workspaces.log"
        config = platform_config()
        self.original_executable = os.environ.get(config.executable_env)
        os.environ[config.executable_env] = str(self.executable)
        os.environ["SKILL_CREATOR_FAKE_LOG"] = str(self.log)

    def tearDown(self) -> None:
        config = platform_config()
        if self.original_executable is None:
            os.environ.pop(config.executable_env, None)
        else:
            os.environ[config.executable_env] = self.original_executable
        os.environ.pop("SKILL_CREATOR_FAKE_LOG", None)
        self.temporary.cleanup()

    def test_trigger_detection_and_workspace_cleanup(self) -> None:
        common = {
            "skill_name": "example-skill",
            "description": "Use for positive trigger requests.",
            "timeout": 5,
            "project_root": self.root,
            "model": None,
        }

        self.assertTrue(evaluate_trigger(query="positive request", **common))
        self.assertFalse(evaluate_trigger(query="unrelated request", **common))

        workspaces = self.log.read_text(encoding="utf-8").splitlines()
        self.assertEqual(2, len(workspaces))
        self.assertTrue(all(not Path(path).exists() for path in workspaces))

    def test_description_improvement_response(self) -> None:
        response = call_model("Improve this description.", model=None, timeout=5)

        self.assertEqual(
            "<new_description>Improved native description.</new_description>",
            response,
        )
        workspace = Path(self.log.read_text(encoding="utf-8").strip())
        self.assertFalse(workspace.exists())

    def test_run_eval_aggregates_positive_and_negative_queries(self) -> None:
        result = run_eval(
            eval_set=[
                {"query": "positive request", "should_trigger": True},
                {"query": "unrelated request", "should_trigger": False},
            ],
            skill_name="example-skill",
            description="Use for positive trigger requests.",
            num_workers=1,
            timeout=5,
            project_root=self.root,
            runs_per_query=1,
        )

        self.assertEqual({"total": 2, "passed": 2, "failed": 0}, result["summary"])
        self.assertEqual(platform_config().key, result["platform"])

    def test_run_loop_supports_no_holdout_mode(self) -> None:
        skill_path = self.root / "example-skill"
        skill_path.mkdir()
        skill_path.joinpath("SKILL.md").write_text(
            "---\n"
            "name: example-skill\n"
            "description: Use for positive trigger requests.\n"
            "---\n\n"
            "# Example Skill\n",
            encoding="utf-8",
        )

        result = run_loop(
            eval_set=[
                {"query": "positive request", "should_trigger": True},
                {"query": "unrelated request", "should_trigger": False},
            ],
            skill_path=skill_path,
            description_override=None,
            num_workers=1,
            timeout=5,
            max_iterations=1,
            runs_per_query=1,
            trigger_threshold=0.5,
            holdout=0,
            model=None,
            verbose=False,
        )

        self.assertEqual("all_passed (iteration 1)", result["exit_reason"])
        self.assertEqual("2/2", result["best_score"])


if __name__ == "__main__":
    unittest.main()
