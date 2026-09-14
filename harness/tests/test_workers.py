"""Native worker protocol and budget checks with no external model calls."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from harness.config import HarnessConfig
from harness.workers import claude_cli_worker, codex_cli_worker


class WorkerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="harness-worker-test-")
        self.addCleanup(temporary.cleanup)
        self.project = Path(temporary.name)

    def config(self, **options):
        return HarnessConfig(project=self.project, **options)

    def completed(self, output, exit_code=0):
        return subprocess.CompletedProcess([], exit_code, output, "private stderr diagnostic")

    def test_claude_passes_allowance_model_effort_and_preserves_cost_on_semantic_failure(self):
        config = self.config(budget_usd=0.25, model="fixture-model", thinking_effort="high")
        payload = {"type": "result", "subtype": "error_max_budget_usd", "is_error": True,
                   "result": "Budget exhausted", "total_cost_usd": 0.25, "session_id": "fixture"}
        with patch("harness.workers._run", return_value=(self.completed(json.dumps(payload)), 1.0, None)) as run:
            result = claude_cli_worker("Review fixture", config)
        argv = run.call_args.args[0]
        self.assertEqual(argv[argv.index("--max-budget-usd") + 1], "0.25")
        self.assertEqual(argv[argv.index("--model") + 1], "fixture-model")
        self.assertEqual(argv[argv.index("--effort") + 1], "high")
        self.assertFalse(result.ok)
        self.assertEqual(result.cost_usd, 0.25)
        self.assertEqual(result.limit_reached, 'USD')
        self.assertNotIn("private stderr", result.error)

    def test_claude_requires_successful_terminal_json_and_leaves_missing_cost_unknown(self):
        for payload in ("invalid JSON", "[]", json.dumps({"result": "unfinished"}),
                        json.dumps({"subtype": "success", "is_error": True})):
            with self.subTest(payload=payload), patch("harness.workers._run", return_value=(self.completed(payload), 0, None)):
                self.assertFalse(claude_cli_worker("Review fixture", self.config()).ok)
        payload = {"type": "result", "subtype": "success", "is_error": False, "result": '{"findings": []}'}
        with patch("harness.workers._run", return_value=(self.completed(json.dumps(payload)), 0, None)) as run:
            result = claude_cli_worker("Review fixture", self.config(budget_usd=None))
        self.assertTrue(result.ok)
        self.assertIsNone(result.cost_usd)
        self.assertNotIn("--max-budget-usd", run.call_args.args[0])

    def test_claude_success_above_budget_is_not_accepted_as_a_completed_review(self):
        payload = {'subtype':'success', 'is_error':False, 'result':'{"findings":[]}',
                   'total_cost_usd':1.62}
        with patch('harness.workers._run', return_value=(self.completed(json.dumps(payload)), 1, None)):
            result = claude_cli_worker('Review fixture', self.config(budget_usd=1.5))
        self.assertFalse(result.ok)
        self.assertEqual(result.cost_usd, 1.62)
        self.assertIn('budget', result.error.lower())
        self.assertIn('1.6200', result.error)
        self.assertEqual(result.limit_reached, 'USD')

    def test_codex_rejects_finite_budget_before_launch(self):
        with patch("harness.workers._run") as run, self.assertRaisesRegex(ValueError, "USD limit"):
            codex_cli_worker("Review fixture", self.config(budget_usd=1))
        run.assert_not_called()

    def test_codex_forwards_model_effort_and_has_unknown_cost(self):
        output = "\n".join(json.dumps(event) for event in (
            {"type": "thread.started", "thread_id": "fixture"},
            {"type": "item.completed", "item": {"type": "reasoning", "text": "hidden reasoning"}},
            {"type": "item.completed", "item": {"type": "agent_message", "text": '{"findings": []}'}},
            {"type": "turn.completed", "usage": {"input_tokens": 12, "output_tokens": 6}},
        ))
        with patch("harness.workers._run", return_value=(self.completed(output), 1, None)) as run:
            result = codex_cli_worker("Review fixture", self.config(budget_usd=None, model="fixture", thinking_effort="high"))
        argv = run.call_args.args[0]
        self.assertEqual(argv[argv.index("--sandbox") + 1], "read-only")
        self.assertEqual(argv[argv.index("--model") + 1], "fixture")
        self.assertIn('model_reasoning_effort="high"', argv)
        self.assertTrue(result.ok)
        self.assertIsNone(result.cost_usd)
        self.assertEqual(result.session_id, "fixture")
        self.assertEqual(result.text, '{"findings": []}')

    def test_codex_exit_zero_without_terminal_success_is_failure(self):
        for events in (
            [{"type": "item.completed", "item": {"type": "agent_message", "text": "partial"}}],
            [{"type": "turn.failed", "error": {"message": "private stderr diagnostic"}}],
            [{"type": "error"}, {"type": "turn.completed"}],
        ):
            output = "\n".join(map(json.dumps, events))
            with self.subTest(events=events), patch("harness.workers._run", return_value=(self.completed(output), 0, None)):
                result = codex_cli_worker("Review fixture", self.config(budget_usd=None))
                self.assertFalse(result.ok)
                self.assertNotIn("private stderr", result.error)

    def test_config_accepts_unlimited_budget_and_rejects_nonfinite_or_nonmonetary_values(self):
        self.assertIsNone(self.config(budget_usd=None).budget_usd)
        for value in (0, -1, float("nan"), float("inf"), True, "1"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.config(budget_usd=value)


if __name__ == "__main__":
    unittest.main()
