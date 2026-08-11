"""Dry-run tests: graph wiring, fan-out, budget, gate, resume, blackboard."""

from __future__ import annotations

import json
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from langgraph.checkpoint.sqlite import SqliteSaver

from harness.blackboard import Blackboard
from harness.config import HarnessConfig
from harness.graphs.fleet_review import build_graph, dedupe, parse_findings, resume_command
from harness.workers import DryRunWorker

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
        self.saver = SqliteSaver(
            sqlite3.connect(
                str(self.config.checkpoint_path), check_same_thread=False
            )
        )
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
