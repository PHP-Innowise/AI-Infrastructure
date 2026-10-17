"""The Claude Code and Codex prompt hooks against the runtime they ship with.

The read hook calls the CLI directly under one budget. These tests pin what
that direct call promises the person at the keyboard: a long prompt and a
budget they raised still get memory, a refresh stopped at the budget never
hands over a cut capsule, and a refresh that fails says why.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
EDITION = ROOT / "PHP Core"
TASK = "feature/invoice-export"
GOAL = "Invoice export to CSV"


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "the hooks are bash scripts")
class PromptReadHookTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="prompt-read-")
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name)
        self.root = self.base / "project"
        self.root.mkdir()
        self.env = {k: os.environ[k] for k in ("PATH", "LANG", "LC_ALL", "LC_CTYPE") if k in os.environ}
        self.env.update(HOME=str(self.base / "home"), TMPDIR=str(self.base), PYTHONDONTWRITEBYTECODE="1",
                        GIT_OPTIONAL_LOCKS="0", XDG_CONFIG_HOME=str(self.base / "xdg"))
        (self.base / "home").mkdir()
        self.git("init", "-q", "-b", TASK)
        self.git("config", "user.email", "prompt-read@example.invalid")
        self.git("config", "user.name", "Prompt Read")
        for rel in ("memory-bank/scripts", "memory-bank/templates", "project-brain/config",
                    "project-brain/schemas", "project-brain/templates"):
            shutil.copytree(EDITION / rel, self.root / rel,
                            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        for tool in (".claude", ".codex"):
            hooks = self.root / tool / "hooks"
            hooks.mkdir(parents=True)
            shutil.copy2(EDITION / tool / "hooks/working-memory-read.sh", hooks / "working-memory-read.sh")
        (self.root / "src").mkdir()
        (self.root / "src/Invoice.php").write_text("<?php\nfinal class Invoice {}\n", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "fixture")
        self.cli("start", "--task-id", TASK, "--goal", GOAL, "--json")

    def git(self, *args):
        subprocess.run(["git", "-C", str(self.root), *args], env=self.env, check=True,
                       capture_output=True, timeout=30)

    def cli(self, *args):
        return subprocess.run([sys.executable, str(self.root / "memory-bank/scripts/context.py"), *args],
                              cwd=self.root, env=self.env, check=True, capture_output=True, text=True,
                              timeout=120)

    def hook(self, tool, prompt, **env):
        payload = json.dumps({"hook_event_name": "UserPromptSubmit", "session_id": "s1", "prompt": prompt})
        return subprocess.run(["bash", str(self.root / tool / "hooks/working-memory-read.sh")],
                              cwd=self.root, env={**self.env, **env}, input=payload,
                              capture_output=True, text=True, timeout=120)

    def stub_runtime(self, body):
        # A stand-in CLI: the hook's own handling of what comes back is under
        # test here, not the runtime's.
        (self.root / "memory-bank/scripts/context.py").write_text(body, encoding="utf-8")

    def test_a_long_prompt_still_gets_its_working_state(self):
        # A pasted log or stack trace easily passes 8 KB; it used to cost the
        # whole turn its memory.
        prompt = "Explain the invoice export. " + "Log: export row failed; " * 600
        self.assertGreater(len(prompt.encode("utf-8")), 12000)
        for tool in (".claude", ".codex"):
            with self.subTest(tool=tool):
                result = self.hook(tool, prompt)
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertIn(f"working: {TASK} — {GOAL}", result.stdout)
                self.assertNotIn("Memory refresh unavailable", result.stdout)

    def test_a_raised_budget_is_honoured(self):
        for budget in ("6", "30", "120"):
            with self.subTest(budget=budget):
                result = self.hook(".claude", "How does the invoice export work?", CONTEXT_HOOK_BUDGET=budget)
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertIn(f"working: {TASK} — {GOAL}", result.stdout)

    @unittest.skipUnless(shutil.which("timeout"), "GNU timeout bounds the refresh")
    def test_a_refresh_stopped_at_its_budget_hands_over_nothing_cut(self):
        self.stub_runtime("import sys, time\nprint('working: PARTIAL-CAPSULE', flush=True)\ntime.sleep(30)\n")
        result = self.hook(".claude", "How does the invoice export work?", CONTEXT_HOOK_BUDGET="0.5")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertNotIn("PARTIAL-CAPSULE", result.stdout)
        self.assertIn("Memory refresh unavailable: it exceeded its 0.5s budget.", result.stdout)
        self.assertIn("Working memory was NOT consulted this turn.", result.stdout)

    def test_a_pasted_log_past_the_argument_limit_keeps_the_memory(self):
        # Two megabytes of log: one argument over 128 KiB, and the refresh
        # failed with "Argument list too long".
        log = "\n".join(f"2026-10-16 01:00:{n % 60:02d} ERROR row {n} failed: ledger mismatch"
                         for n in range(40000))
        result = self.hook(".claude", "Why does the invoice export fail? Log:\n" + log
                           + "\nHow does the invoice export work?")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertNotIn("Argument list too long", result.stdout)
        self.assertNotIn("Memory refresh unavailable", result.stdout)
        self.assertIn(f"working: {TASK} — {GOAL}", result.stdout)

    def test_a_detached_head_gets_no_index_report(self):
        # No branch, no task: the hook's refresh only keeps the index current,
        # and its operator report used to reach the model on every prompt.
        self.git("checkout", "-q", "--detach", "HEAD")
        result = self.hook(".claude", "How does the invoice export work?")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual("", result.stdout)
        # The operator's own refresh still reports its layers.
        report = subprocess.run([sys.executable, str(self.root / "memory-bank/scripts/context.py"), "refresh"],
                                cwd=self.root, env=self.env, capture_output=True, text=True, timeout=120)
        self.assertEqual(0, report.returncode, report.stderr)
        self.assertIn("semantic: updated", report.stdout)

    def test_a_failed_refresh_names_its_cause(self):
        self.stub_runtime("import sys\nprint('context: Invalid Project Brain config: example', file=sys.stderr)\nsys.exit(1)\n")
        result = self.hook(".codex", "How does the invoice export work?")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("Memory refresh unavailable: it exited 1 — context: Invalid Project Brain config: example.",
                      result.stdout)

    def test_an_incomplete_refresh_still_delivers_its_layer_report(self):
        # Exit 1 with a report is a refresh that ran and says which layer went
        # stale; the report is the turn's evidence and is delivered whole.
        self.stub_runtime("import sys\nprint('semantic: failed')\nprint('working: x — y')\nsys.exit(1)\n")
        result = self.hook(".claude", "How does the invoice export work?")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("semantic: failed\nworking: x — y", result.stdout)


if __name__ == "__main__":
    unittest.main()
