"""Generator-specific admission contracts, isolated from edition imports."""

from __future__ import annotations

import contextlib
import importlib
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPTS = Path(__file__).resolve().parents[1] / (
    "Infrastructure-Creator/.agents/skills/memory-seed/assets/scripts"
)
SEED_TITLE = "Orbit reference"
SEED_CONTENT = "Reuse stable orbital anchors when assembling coordinates."
NEW_TITLE = "Lattice digest"
NEW_CONTENT = "Store lattice fingerprints before adopting measurements."


def candidate(identifier, title=NEW_TITLE, content=NEW_CONTENT, **extra):
    return {"record": {"id": identifier, "type": "finding", "title": title},
            "content": content, **extra}


def load_generator():
    sys.path.insert(0, str(SCRIPTS))
    brain = importlib.import_module("brain_runtime")
    for module_name in ("brain_runtime", "validate", "workspace_roots"):
        actual = Path(sys.modules[module_name].__file__).resolve()
        if actual.parent != SCRIPTS.resolve():
            raise AssertionError("Fixture imported an edition instead of generator: " + module_name)
    return brain


class Fixture(unittest.TestCase):
    """Real generator algorithm/lock/similarity, controlled promotion callbacks."""

    def __init__(self, brain, candidates, chunks=(), stalled=(), failure=None):
        super().__init__()
        self.brain = brain
        self.temporary = tempfile.TemporaryDirectory(prefix="generator-admission-")
        self.repository = Path(self.temporary.name)
        self.stack = contextlib.ExitStack()
        self.events = []
        self.proposals = {item["id"]: dict(item) for item in stalled}
        self.applied = {}
        self.failure = failure
        self.config = {"automatic_promotion": True, "bank_duplicate_ratio": 0.98}
        self.candidates = candidates
        self.chunks = list(chunks)
        self.producer = self.patch("promotable_records", return_value=(candidates, []))
        self.patch("load_config", return_value=self.config)
        self.patch("promotion_ledger", return_value=(set(), self.applied))
        self.patch("stalled_automatic_promotions", side_effect=self.stalled)
        self.patch("obsolete_promotion", side_effect=self.obsolete)
        self.patch("withdraw_promotion", side_effect=self.withdraw)
        self.patch("active_chunk_texts", side_effect=lambda root: list(self.chunks))
        self.patch("create_promotion", side_effect=self.create_proposal)
        self.patch("auto_review_promotion", side_effect=self.review_proposal)
        self.patch("apply_promotion", side_effect=self.apply_proposal)
        self.duplicate = self.patch("near_duplicate_chunk", wraps=brain.near_duplicate_chunk)

    def patch(self, name, **options):
        return self.stack.enter_context(mock.patch.object(self.brain, name, **options))

    def close(self):
        self.stack.close()
        self.temporary.cleanup()

    def stalled(self, root):
        return [(identifier, dict(item)) for identifier, item in self.proposals.items()
                if item["status"] in ("proposed", "reviewed")]

    def obsolete(self, root, proposal, applied):
        return "fixture obsolete" if proposal["id"] == "stalled-obsolete" else None

    def withdraw(self, root, identifier, reason):
        self.events.append(("withdraw", identifier, reason))
        self.proposals[identifier]["status"] = "closed"

    def maybe_fail(self, stage, identifier):
        if self.failure == (stage, identifier):
            raise self.brain.BrainError("fixture promotion failure")

    def create_proposal(self, root, identifiers, title, content, *, proposer, review_mode):
        identifier = identifiers[0]
        self.events.append(("create", identifier, proposer, review_mode))
        self.maybe_fail("create", identifier)
        proposal = {"id": "proposal-" + identifier, "status": "proposed",
                    "source_records": [{"id": identifier, "type": "finding"}],
                    "title": title, "content": content}
        self.proposals[proposal["id"]] = proposal
        return dict(proposal)

    def review_proposal(self, root, identifier):
        self.events.append(("review", identifier))
        self.maybe_fail("review", self.proposals[identifier]["source_records"][0]["id"])
        self.assertEqual("proposed", self.proposals[identifier]["status"])
        self.proposals[identifier]["status"] = "reviewed"

    def apply_proposal(self, root, identifier):
        self.events.append(("apply", identifier))
        proposal = self.proposals[identifier]
        self.maybe_fail("apply", proposal["source_records"][0]["id"])
        self.assertEqual("reviewed", proposal["status"])
        proposal["status"] = "applied"
        proposal["destination_memory_id"] = "memory-" + proposal["source_records"][0]["id"]
        self.chunks.append((proposal["destination_memory_id"],
                            proposal["title"] + "\n" + proposal["content"]))
        return dict(proposal)

    def run_promotion(self, **options):
        return self.brain.auto_promote(self.repository, owner="fixture-owner", **options)

    def new_attempts(self):
        return [event[1] for event in self.events if event[0] == "create"]


def mixed_scenario(brain):
    for stage in ("create", "review", "apply"):
        candidates = [candidate("seed-duplicate", SEED_TITLE, SEED_CONTENT),
                      candidate("novel"), candidate("same-batch-duplicate"),
                      candidate("failure", "Thermal budget", "Measure coolant flow under sustained heat."),
                      candidate("tail-b", "Voltage", "Insulate every conductor."),
                      candidate("tail-c", "Pulse", "Sample pulse intervals.")]
        fixture = Fixture(brain, candidates, [("seed-memory", SEED_TITLE + "\n" + SEED_CONTENT)],
                          failure=(stage, "failure"))
        try:
            result = fixture.run_promotion(limit=2)
            fixture.assertEqual(["novel"], [item["record_id"] for item in result["promoted"]])
            fixture.assertEqual(["seed-duplicate", "same-batch-duplicate"],
                                [item["record_id"] for item in result["blocked"]])
            fixture.assertIn("seed-memory", result["blocked"][0]["reason"])
            fixture.assertIn("memory-novel", result["blocked"][1]["reason"])
            fixture.assertEqual([{"record_id": "failure", "reason": "fixture promotion failure"}], result["failed"])
            fixture.assertEqual(["novel", "failure"], fixture.new_attempts())
            fixture.assertEqual(2, result["skipped"])
            fixture.assertEqual(4, fixture.duplicate.call_count)
            fixture.producer.assert_called_once_with(fixture.repository, fixture.config,
                                                     allow_changed_sources=True, only=None)
            fixture.assertTrue(all(event[2:] == ("fixture-owner", "automatic")
                                   for event in fixture.events if event[0] == "create"))
            if stage != "create":
                fixture.assertEqual("proposed" if stage == "review" else "reviewed",
                                    fixture.proposals["proposal-failure"]["status"])
        finally:
            fixture.close()


def stalled_scenario(brain):
    def proposal(identifier, source, status):
        return {"id": identifier, "status": status,
                "source_records": [{"id": source, "type": "finding"}],
                "title": SEED_TITLE, "content": SEED_CONTENT}
    candidates = [candidate("pending-source"),
                  candidate("ready-duplicate", SEED_TITLE, SEED_CONTENT),
                  candidate("fresh"), candidate("tail", "Voltage", "Insulate every conductor.")]
    stalled = [proposal("stalled-ready", "ready-source", "proposed"),
               proposal("stalled-pending", "pending-source", "reviewed"),
               proposal("stalled-obsolete", "obsolete-source", "reviewed")]
    fixture = Fixture(brain, candidates, stalled=stalled, failure=("apply", "pending-source"))
    try:
        result = fixture.run_promotion(limit=1)
        fixture.assertEqual(["ready-source", "fresh"], [item["record_id"] for item in result["promoted"]])
        fixture.assertEqual(["pending-source"], [item["record_id"] for item in result["failed"]])
        fixture.assertEqual(["obsolete-source"], [item["record_id"] for item in result["closed"]])
        fixture.assertEqual(["ready-duplicate"], [item["record_id"] for item in result["blocked"]])
        fixture.assertEqual(["fresh"], fixture.new_attempts())
        fixture.assertEqual(1, result["skipped"])
        fixture.assertEqual("reviewed", fixture.proposals["stalled-pending"]["status"])
        fixture.assertEqual(("review", "stalled-ready"), fixture.events[0])
        fixture.assertEqual(("apply", "stalled-ready"), fixture.events[1])
        fixture.assertEqual(("apply", "stalled-pending"), fixture.events[2])
        fixture.assertEqual(("withdraw", "stalled-obsolete", "fixture obsolete"), fixture.events[3])
        fixture.assertEqual(2, fixture.duplicate.call_count)
    finally:
        fixture.close()


def correction_scenario(brain):
    target = candidate("correction", supersedes="predecessor-memory")
    fixture = Fixture(brain, [candidate("seed-duplicate", SEED_TITLE, SEED_CONTENT), target],
                      [("seed-memory", SEED_TITLE + "\n" + SEED_CONTENT),
                       ("predecessor-memory", NEW_TITLE + "\n" + NEW_CONTENT)])
    try:
        selected = {"correction", "seed-duplicate"}
        result = fixture.run_promotion(limit=1, only=selected)
        fixture.assertEqual(["correction"], fixture.new_attempts())
        fixture.assertEqual(["correction"], [item["record_id"] for item in result["promoted"]])
        fixture.assertEqual(0, result["skipped"])
        fixture.producer.assert_called_once_with(fixture.repository, fixture.config,
                                                 allow_changed_sources=True, only=selected)
        compared = fixture.duplicate.call_args_list[1].args[1]
        fixture.assertEqual(["seed-memory"], [item[0] for item in compared])
    finally:
        fixture.close()


def disabled_zero_scenario(brain):
    fixture = Fixture(brain, [candidate("fresh")])
    try:
        fixture.config["automatic_promotion"] = False
        fixture.assertEqual({"enabled": False, "promoted": [], "failed": [],
                             "blocked": [], "closed": [], "skipped": 0}, fixture.run_promotion())
        fixture.assertEqual([], fixture.events)
        fixture.assertEqual(0, fixture.duplicate.call_count)
        fixture.producer.assert_not_called()
    finally:
        fixture.close()
    ready = {"id": "stalled-ready", "status": "proposed",
             "source_records": [{"id": "ready-source", "type": "finding"}],
             "title": SEED_TITLE, "content": SEED_CONTENT}
    fixture = Fixture(brain, [candidate("first"), candidate("second")], stalled=[ready])
    try:
        result = fixture.run_promotion(limit=0)
        fixture.assertEqual(["ready-source"], [item["record_id"] for item in result["promoted"]])
        fixture.assertEqual([], fixture.new_attempts())
        fixture.assertEqual(0, fixture.duplicate.call_count)
        fixture.assertEqual(2, result["skipped"])
    finally:
        fixture.close()


SCENARIOS = {"mixed": mixed_scenario, "stalled": stalled_scenario,
             "correction": correction_scenario, "disabled-zero": disabled_zero_scenario}


class GeneratorPromotionAdmissionTest(unittest.TestCase):
    def run_scenario(self, name):
        result = subprocess.run([sys.executable, "-B", str(Path(__file__).resolve()), "--scenario", name],
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        self.assertEqual("GENERATOR_SCENARIO_PASS " + name + "\n", result.stdout)

    def test_mixed_duplicate_failure_and_unexamined_tail(self):
        self.run_scenario("mixed")

    def test_stalled_retry_identity_and_fresh_admission_are_separate(self):
        self.run_scenario("stalled")

    def test_correction_excludes_own_predecessor_and_forwards_only(self):
        self.run_scenario("correction")

    def test_disabled_promotion_and_zero_new_admission(self):
        self.run_scenario("disabled-zero")


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--scenario":
        name = sys.argv[2]
        SCENARIOS[name](load_generator())
        print("GENERATOR_SCENARIO_PASS " + name)
    else:
        unittest.main()
