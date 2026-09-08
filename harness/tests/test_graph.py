"""Dry-run tests: graph wiring, fan-out, budget, gate, resume, blackboard."""

from __future__ import annotations

import json
import sqlite3
import sys
import tempfile
import unittest
import threading
from dataclasses import replace
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from langgraph.checkpoint.memory import InMemorySaver
try:
    from langgraph.checkpoint.sqlite import SqliteSaver
except ImportError:
    SqliteSaver = None

from harness.blackboard import Blackboard
from harness.config import HarnessConfig
from harness.graphs.fleet_review import build_graph, dedupe, parse_findings, resume_command
from harness.workers import DryRunWorker, WorkerResult

FINDINGS = {
    "code-reviewer": json.dumps({"findings": [
        {"file": "src/Repo.php", "line": 10, "severity": "medium",
         "claim": "Unbound PDO parameter", "evidence": "raw interpolation"},
    ]}),
    "security-reviewer": json.dumps({"findings": [
        {"file": "src/Repo.php", "line": 10, "severity": "high",
         "claim": "Unbound PDO parameter", "evidence": "injection risk"},
        {"file": "src/Auth.php", "line": 4, "severity": "low",
         "claim": "Verbose error leaks stack", "evidence": "catch block"},
    ]}),
}


class RecordingBlackboard(Blackboard):
    """In-memory double that records every call and validates nothing."""

    def __init__(self) -> None:
        self.events: list[tuple[str, str]] = []
        self.capsules: list[str] = []

    @property
    def available(self) -> bool:  # type: ignore[override]
        return True

    def ensure_task(self, task_id: str, goal: str) -> None:
        self.events.append(("start", task_id))

    def validate_capsule(self, capsule: str) -> list[str]:
        self.capsules.append(capsule)
        return []

    def dispatch_spawn(self, task_id: str, agent: str, capsule: str) -> None:
        self.events.append(("spawn", agent))

    def dispatch_complete(self, task_id: str, agent: str, note: str) -> None:
        self.events.append(("complete", agent))

    def record_progress(self, task_id: str, progress: str) -> None:
        self.events.append(("progress", progress))


class FleetReviewTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="harness-test-")
        base = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.config = HarnessConfig(
            project=base,
            worker="dry-run",
            lenses=("code-reviewer", "security-reviewer"),
            budget_usd=5.0,
            state_dir=base / "state",
            reports_dir=base / "reports",
        )
        self.worker = DryRunWorker(script=dict(FINDINGS))
        self.blackboard = RecordingBlackboard()
        self.config.state_dir.mkdir(parents=True)
        self.saver = InMemorySaver()
        self.graph = build_graph(
            self.config, self.worker, self.blackboard, self.saver
        )

    def invoke(self, thread: str = "t-1"):
        return self.graph.invoke(
            {
                "scope": "HEAD~1..HEAD",
                "task_id": "harness/t-1",
                "thread_id": thread,
                "lenses": list(self.config.lenses),
            },
            config={"configurable": {"thread_id": thread}},
        )

    def test_fans_out_pauses_and_resumes_to_approved_report(self) -> None:
        state = self.invoke()
        self.assertIn("__interrupt__", state)
        gate = state["__interrupt__"][0].value
        # Three raw findings; the duplicate claim collapses before the gate.
        self.assertEqual(2, gate["findings"])
        self.assertEqual(1, gate["high"])
        self.assertEqual(2, len(self.worker.calls))
        spawned = [agent for event, agent in self.blackboard.events if event == "spawn"]
        self.assertEqual(
            ["code-reviewer", "security-reviewer"], sorted(spawned)
        )
        final = self.graph.invoke(
            resume_command(True),
            config={"configurable": {"thread_id": "t-1"}},
        )
        report = final["report"]
        self.assertIn("Unbound PDO parameter", report)
        # The duplicate claim keeps only its highest-severity copy.
        self.assertEqual(1, report.count("Unbound PDO parameter"))
        self.assertTrue(
            (self.config.reports_dir / "t-1.md").is_file()
        )
        progress = [p for e, p in self.blackboard.events if e == "progress"]
        self.assertTrue(progress and "2 findings" in progress[-1], progress)

    def test_rejection_discards_the_report(self) -> None:
        self.invoke(thread="t-2")
        final = self.graph.invoke(
            resume_command(False),
            config={"configurable": {"thread_id": "t-2"}},
        )
        self.assertEqual("", final["report"])
        self.assertFalse((self.config.reports_dir / "t-2.md").exists())
        progress = [p for e, p in self.blackboard.events if e == "progress"]
        self.assertIn("rejected", progress[-1])

    def test_budget_exhaustion_skips_lenses(self) -> None:
        config = HarnessConfig(
            project=self.config.project,
            worker="dry-run",
            lenses=self.config.lenses,
            budget_usd=0.001,
            state_dir=self.config.state_dir,
            reports_dir=self.config.reports_dir,
        )
        worker = DryRunWorker(script=dict(FINDINGS), cost_per_call_usd=1.0)
        graph = build_graph(config, worker, RecordingBlackboard(), None)
        state = graph.invoke({
            "scope": "s", "task_id": "harness/t-3", "thread_id": "t-3",
            "lenses": ["code-reviewer"], "cost_usd": 1.0,
        })
        self.assertEqual(["code-reviewer"], state["skipped"])
        self.assertEqual([], worker.calls)

    def test_observer_identifies_budget_stop_without_disclosing_worker_errors(self):
        events = []
        def worker(prompt, config):
            return WorkerResult(text='', ok=False, cost_usd=1.62,
                                error='private provider diagnostic', limit_reached='USD')
        graph = build_graph(self.config, worker, RecordingBlackboard(), observer=events.append)
        with self.assertRaisesRegex(RuntimeError, 'USD budget'):
            graph.invoke({'scope':'s', 'task_id':'harness/budget-stop', 'thread_id':'budget-stop',
                          'lenses':['code-reviewer']})
        failure = next(e for e in events if e.get('kind') == 'fleet_reviewer' and e.get('status') == 'failed')
        self.assertEqual(failure['limit_reached'], 'USD')
        self.assertNotIn('private', json.dumps(events))

    def test_parallel_reviewers_receive_disjoint_remaining_budget_allowances(self) -> None:
        config = replace(self.config, budget_usd=0.5)
        allowances = []
        barrier = threading.Barrier(2)

        def worker(prompt, allocated):
            allowances.append(allocated.budget_usd)
            self.assertEqual(len(allocated.lenses), 1)
            self.assertIn(allocated.lenses[0], prompt)
            barrier.wait(timeout=3)
            return WorkerResult(text='{"findings": []}', ok=True, cost_usd=allocated.budget_usd)

        graph = build_graph(config, worker, RecordingBlackboard(), None)
        state = graph.invoke({
            "scope": "s", "task_id": "harness/budget", "thread_id": "budget",
            "lenses": list(config.lenses), "cost_usd": 0.2,
        }, config={"max_concurrency": 2})
        self.assertEqual(sorted(allowances), [0.15, 0.15])
        self.assertAlmostEqual(state["cost_usd"], 0.5)
        self.assertEqual(state["skipped"], [])

    def test_dry_run_does_not_invent_spending_above_its_allowance(self) -> None:
        config = replace(self.config, budget_usd=0.5)
        worker = DryRunWorker(cost_per_call_usd=1.0)
        events = []
        with self.assertRaisesRegex(RuntimeError, "Reviewer .* failed"):
            build_graph(config, worker, RecordingBlackboard(), observer=events.append).invoke({
                "scope": "s", "task_id": "harness/budget", "thread_id": "budget",
                "lenses": list(config.lenses),
            })
        self.assertTrue(worker.calls)
        failed = [item for item in events if item.get("status") == "failed"]
        self.assertTrue(failed)
        self.assertTrue(all(item["cost_usd"] == 0.0 for item in failed))
        self.assertFalse(any(item.get("stage") == "gate" for item in events))

    def test_unpriced_reviewers_keep_total_unknown_and_direct_mode_does_not_spawn(self) -> None:
        config = replace(self.config, budget_usd=None, delegate_to_roster=False, thinking_effort="high")
        calls = []

        def worker(prompt, allocated):
            calls.append(prompt)
            self.assertIsNone(allocated.budget_usd)
            self.assertEqual(len(allocated.lenses), 1)
            self.assertIn(allocated.lenses[0], prompt)
            self.assertEqual(allocated.thinking_effort, "high")
            return WorkerResult(text='{"findings": []}', ok=True,
                                cost_usd=None if "security-reviewer" in prompt else 0.1)

        graph = build_graph(config, worker, self.blackboard, self.saver)
        state = graph.invoke({"scope": "s", "task_id": "harness/unpriced", "thread_id": "unpriced",
                              "lenses": list(config.lenses)},
                             config={"configurable": {"thread_id": "unpriced"}})
        self.assertIsNone(state["cost_usd"])
        self.assertIsNone(state["__interrupt__"][0].value["cost_usd"])
        self.assertTrue(all("Do not spawn additional agents" in prompt for prompt in calls))
        self.assertFalse(any("Use the Task tool" in prompt for prompt in calls))
        final = graph.invoke(resume_command(True), config={"configurable": {"thread_id": "unpriced"}})
        self.assertIn("cost: unknown", final["report"])
        self.assertEqual(len(calls), 2)

    def test_observer_reports_failures_and_resume_does_not_repeat_reviewers_or_waiting(self) -> None:
        events = []
        calls = []
        recovered = False

        def worker(prompt, allocated):
            calls.append(prompt)
            if "security-reviewer" in prompt and not recovered:
                return WorkerResult(text="", ok=False, cost_usd=None, duration_seconds=0.5,
                                    error="private native diagnostic")
            return WorkerResult(text=FINDINGS["code-reviewer"], ok=True, cost_usd=0.1,
                                duration_seconds=0.25)

        graph = build_graph(replace(self.config, budget_usd=None), worker, self.blackboard,
                            self.saver, observer=events.append)
        options = {"configurable": {"thread_id": "events"}}
        with self.assertRaisesRegex(RuntimeError, "Reviewer security-reviewer failed") as error:
            graph.invoke({"scope": "s", "task_id": "harness/events", "thread_id": "events",
                          "lenses": list(self.config.lenses)}, config=options)
        self.assertNotIn("private native diagnostic", str(error.exception))
        self.assertEqual(events[0], {"kind": "fleet_stage", "stage": "scope", "status": "running"})
        reviewers = [item for item in events if item["kind"] == "fleet_reviewer"]
        self.assertEqual(len(reviewers), 4)
        failed = next(item for item in reviewers if item["status"] == "failed")
        self.assertEqual((failed["lens"], failed["error"]), ("security-reviewer", "Reviewer failed."))
        self.assertIsNone(failed["cost_usd"])
        self.assertNotIn("private native diagnostic", json.dumps(events))
        self.assertNotIn("capsule", json.dumps(events))
        self.assertEqual(len(calls), 2)
        self.assertFalse(any(item.get("stage") == "gate" for item in events))
        recovered = True
        retried = graph.invoke(None, config=options)
        self.assertIn("__interrupt__", retried)
        self.assertEqual(len(calls), 3)
        self.assertEqual(sum("code-reviewer" in prompt for prompt in calls), 1)
        self.assertFalse(any("failed" in item["claim"] for item in retried["findings"]))
        graph.invoke(resume_command(False), config=options)
        self.assertEqual(len(calls), 3)
        self.assertEqual(sum(item.get("stage") == "gate" and item["status"] == "waiting"
                             for item in events), 1)
        self.assertEqual(events[-1], {"kind": "fleet_stage", "stage": "record", "status": "completed"})

    @unittest.skipUnless(SqliteSaver is not None, "Install langgraph-checkpoint-sqlite for persistence coverage")
    def test_sqlite_gate_reopens_without_repeating_completed_reviewers(self) -> None:
        options = {"configurable": {"thread_id": "persisted"}}
        connection = sqlite3.connect(str(self.config.checkpoint_path), check_same_thread=False)
        try:
            graph = build_graph(self.config, self.worker, self.blackboard, SqliteSaver(connection))
            graph.invoke({"scope": "s", "task_id": "harness/persisted", "thread_id": "persisted",
                          "lenses": list(self.config.lenses)}, config=options)
        finally:
            connection.close()
        self.assertEqual(len(self.worker.calls), 2)
        reopened = sqlite3.connect(str(self.config.checkpoint_path), check_same_thread=False)
        try:
            graph = build_graph(self.config, self.worker, self.blackboard, SqliteSaver(reopened))
            final = graph.invoke(resume_command(True), config=options)
        finally:
            reopened.close()
        self.assertTrue(final["approved"])
        self.assertEqual(len(self.worker.calls), 2)

    @unittest.skipUnless(SqliteSaver is not None, "Install langgraph-checkpoint-sqlite for persistence coverage")
    def test_sqlite_failed_node_retries_after_reopen_without_repeating_successful_peer(self) -> None:
        attempts = {"code-reviewer": 0, "security-reviewer": 0}
        allow_failure = True
        config = replace(self.config, budget_usd=None)

        def worker(prompt, allocated):
            lens = "security-reviewer" if "security-reviewer" in prompt else "code-reviewer"
            attempts[lens] += 1
            if lens == "security-reviewer" and allow_failure:
                return WorkerResult(text="", ok=False, error="private provider failure")
            return WorkerResult(text=FINDINGS[lens], ok=True, cost_usd=0.1)

        options = {"configurable": {"thread_id": "partial"}}
        connection = sqlite3.connect(str(self.config.checkpoint_path), check_same_thread=False)
        try:
            graph = build_graph(config, worker, self.blackboard, SqliteSaver(connection))
            with self.assertRaisesRegex(RuntimeError, "Reviewer security-reviewer failed"):
                graph.invoke({"scope": "s", "task_id": "harness/partial", "thread_id": "partial",
                              "lenses": list(self.config.lenses)}, config=options)
        finally:
            connection.close()
        self.assertEqual(attempts, {"code-reviewer": 1, "security-reviewer": 1})
        allow_failure = False
        reopened = sqlite3.connect(str(self.config.checkpoint_path), check_same_thread=False)
        try:
            graph = build_graph(config, worker, self.blackboard, SqliteSaver(reopened))
            state = graph.invoke(None, config=options)
            self.assertIn("__interrupt__", state)
            final = graph.invoke(resume_command(True), config=options)
        finally:
            reopened.close()
        self.assertEqual(attempts, {"code-reviewer": 1, "security-reviewer": 2})
        self.assertTrue(final["approved"])
        self.assertNotIn("private provider failure", final["report"])

    def test_underspecified_capsule_refuses_dispatch(self) -> None:
        class StrictBlackboard(RecordingBlackboard):
            def validate_capsule(self, capsule: str) -> list[str]:
                return ["missing mandatory section: objective"]

        graph = build_graph(self.config, self.worker, StrictBlackboard(), None)
        with self.assertRaises(Exception) as caught:
            graph.invoke({
                "scope": "s", "task_id": "harness/t-4", "thread_id": "t-4",
                "lenses": ["code-reviewer"],
            })
        self.assertIn("under-specified", str(caught.exception))

    def test_capsule_carries_all_mandatory_sections(self) -> None:
        self.invoke(thread="t-5")
        capsule = self.blackboard.capsules[0].lower()
        for section in ("objective", "output format", "tool and source guidance",
                        "boundaries", "decisions and assumptions"):
            self.assertIn(section, capsule)


class ParsingTest(unittest.TestCase):
    def test_parses_fenced_and_bare_json(self) -> None:
        bare = json.dumps({"findings": [{"claim": "x", "severity": "low"}]})
        fenced = f"Report:\n```json\n{bare}\n```"
        for text in (bare, fenced):
            findings = parse_findings(text, "lens")
            self.assertEqual(1, len(findings))
            self.assertEqual("lens", findings[0]["lens"])

    def test_unparsable_output_becomes_one_unparsed_finding(self) -> None:
        findings = parse_findings("all good, nothing to report", "lens")
        self.assertEqual("unparsed", findings[0]["severity"])
        self.assertEqual([], parse_findings("   ", "lens"))

    def test_dedupe_keeps_highest_severity(self) -> None:
        unique = dedupe([
            {"file": "a", "claim": "Same Thing", "severity": "low"},
            {"file": "a", "claim": "same thing", "severity": "high"},
        ])
        self.assertEqual(1, len(unique))
        self.assertEqual("high", unique[0]["severity"])


if __name__ == "__main__":
    unittest.main()
