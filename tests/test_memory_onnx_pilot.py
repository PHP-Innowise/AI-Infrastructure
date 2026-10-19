"""Policy boundary, excerpt budget and exact fallback tests for the opt-in pilot."""

from __future__ import annotations

import copy
import importlib.util
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import memory_eval
from memory_onnx import ScoreResult
from memory_onnx_pilot import ExcerptPilot, write_stats
from tests import test_memory_eval as fixtures

sys.path.insert(0, str(ROOT / "PHP Core/memory-bank/scripts"))
import context_retrieval as runtime
sys.path.pop(0)


class Excerpt(str):
    heading = ""


class ExcerptPilotTests(unittest.TestCase):
    def setUp(self) -> None:
        self.baseline = {"docs/public.md": Excerpt("Original lexical passage.")}
        self.reads: list[str] = []
        self.requests: list[list[str]] = []
        def marked(_connection, path, _query):
            self.reads.append(path)
            if path != "docs/public.md":
                raise AssertionError("read of a nonselected path")
            return "# First\nOriginal lexical passage.\n\n# Other\nBetter answer about payments."
        self.context = SimpleNamespace(
            capsule_excerpts=lambda _connection, _capsule: self.baseline,
            excerpt_weights=lambda _connection, _query: {},
            sanitize_automatic_query=lambda text: text,
            marked_document=marked, Excerpt=Excerpt,
            EXCERPT_CHARACTERS=(800, 600, 400), EPISODIC_EXCERPT_CHARACTERS=600,
            AUTOMATIC_LINK_SELECTIONS=("prompt-link", "touch-link"),
        )
        self.capsule = {"query": "payment problem", "semantic": [{"path": "docs/public.md"}],
                        "episodic": [], "procedural": [{"path": "private/secret.md"}]}

    def pilot(self, failure=None, tied=False):
        def score(query, passages):
            self.assertEqual("payment problem", query)
            self.requests.append(passages)
            return (ScoreResult(None, failure) if failure else
                    ScoreResult([0.0 if tied else float("Better answer" in text) for text in passages]))
        return ExcerptPilot(self.context, runtime, SimpleNamespace(score=score))

    def test_legacy_callable_keeps_exact_object_without_scoring(self) -> None:
        before = copy.deepcopy(self.capsule)
        pilot = self.pilot()
        self.assertIs(self.baseline, pilot(None, self.capsule))
        self.assertEqual(before, self.capsule)
        self.assertEqual([], self.requests)
        self.assertEqual([], self.reads)
        self.assertEqual(1, pilot.stats["fallback:legacy-interface"])

    def test_legacy_repeat_never_starts_an_optional_worker(self) -> None:
        pilot = self.pilot()
        self.assertIs(self.baseline, pilot(None, self.capsule))
        self.assertIs(self.baseline, pilot(None, self.capsule))
        self.assertEqual([], self.requests)
        self.assertEqual(2, pilot.stats["fallback:legacy-interface"])

    def test_legacy_does_not_hide_baseline_errors(self) -> None:
        self.context.capsule_excerpts = lambda *_: (_ for _ in ()).throw(ValueError("baseline failure"))
        with self.assertRaisesRegex(ValueError, "baseline failure"):
            self.pilot()(None, self.capsule)

    def test_optional_stats_write_failure_is_nonfatal(self) -> None:
        from collections import Counter
        with tempfile.TemporaryDirectory() as temporary:
            self.assertFalse(write_stats(Path(temporary) / "absent" / "stats.json", Counter(calls=1)))

    def test_cli_missing_worker_matches_real_lexical_delivery(self) -> None:
        with tempfile.TemporaryDirectory(prefix="memory-pilot-integration-") as temporary:
            base = Path(temporary)
            project = fixtures.build_ledger(base / "projects")
            for path in (fixtures.PATH_A, fixtures.PATH_B, fixtures.CHUNK):
                with (project / path).open("a", encoding="utf-8") as handle:
                    handle.write("\n\n## Alternative export guidance\n\nInvoice totals are checked before the CSV export.\n")
            fixtures.git(project, "add", fixtures.PATH_A, fixtures.PATH_B, fixtures.CHUNK)
            fixtures.git(project, "commit", "-q", "-m", "alternative excerpts", when=fixtures.T2)
            for name, value in (("set", fixtures.SET), ("judgments", fixtures.JUDGMENTS), ("passages", fixtures.PASSAGES)):
                (base / (name + ".json")).write_text(json.dumps(value), encoding="utf-8")
            common = ["run", "--set", str(base / "set.json"), "--judgments", str(base / "judgments.json"),
                      "--passages", str(base / "passages.json"), "--projects-root", str(base / "projects"),
                      "--edition", "PHP Core", "--ids", "p2", "--cache", str(base / "cache")]
            results = []
            for name, extra in (("lexical", []), ("optional", ["--onnx-excerpts", str(base / "missing-assets"),
                                      "--onnx-python", str(base / "missing-python")])):
                out = base / (name + ".json")
                process = fixtures.run_cli(*common, "--out", str(out), *extra)
                self.assertEqual(0, process.returncode, process.stderr)
                results.append(json.loads(out.read_text()))
            first, second = results
            self.assertNotIn("onnx_excerpts", first["meta"])
            self.assertNotIn("diagnose_misses", first["meta"])
            for key in ("class", "delivered", "useful_delivered", "noise_delivered", "capsule_chars",
                        "answer_passages_existed", "answer_passages_delivered"):
                self.assertEqual(first["items"]["p2"][key], second["items"]["p2"][key], key)
            self.assertGreater(second["items"]["p2"]["refresh"]["onnx_excerpts"].get("fallback:worker-unavailable", 0), 0)



from stateful_pilot import StatefulExcerptPilot
sys.path.insert(0, str(ROOT / "PHP Core/memory-bank/scripts"))
import context as context_cli
sys.path.pop(0)
SPEC = importlib.util.spec_from_file_location("optional_stateful_runtime_fixtures", ROOT / "PHP Core/project-brain/tests/test_runtime.py")
runtime_fixtures = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime_fixtures)

class FakeScorer:
    def __init__(self, target="Recovery codes", failure=None):
        self.target, self.failure = target, failure
        self.calls = []

    def score(self, query, passages, *, deadline=None):
        self.calls.append((query, passages, deadline))
        if self.failure:
            return ScoreResult(None, self.failure)
        return ScoreResult([float(self.target in text) for text in passages])


class ExcerptAdmissionTests(unittest.TestCase):
    def build(self, options_by_document, scorer):
        items = [{"path": doc_ref, "title": "Guide", "snippet": "The original lexical guidance. " * 2,
                  "estimated_tokens": runtime._estimate_tokens("Guide" + "The original lexical guidance. " * 2),
                  "source_hash": "a" * 64, "layer": "semantic", "kind": "spec", "match": "covered"}
                 for doc_ref in options_by_document]
        local = SimpleNamespace(retrieve=lambda *_args, **_kwargs: None,
                                excerpt_section=lambda *_args, **_kwargs: None,
                                excerpt_weights=lambda *_args: {}, marked_document=lambda *_args: "bounded document",
                                AUTOMATIC_LINK_SELECTIONS=runtime.AUTOMATIC_LINK_SELECTIONS,
                                SNIPPET_WINDOW_CHARS=runtime.SNIPPET_WINDOW_CHARS,
                                _estimate_tokens=runtime._estimate_tokens)
        context = SimpleNamespace(retrieve=local.retrieve, excerpt_section=local.excerpt_section,
                                  sanitize_automatic_query=lambda text: text)
        pilot = StatefulExcerptPilot(context, local, scorer)
        pilot.options = lambda _connection, item, *_args: copy.deepcopy(options_by_document[item["path"]])
        # The admission seam must keep the canonical full-section hash, even
        # though the chosen window and its structured snippet are shorter.
        local.delivery_identity = lambda _connection, item, _query: (
            item["path"] + "#" + pilot.prepared[item["path"]]["heading"],
            pilot.prepared[item["path"]]["hash"])
        return pilot, items

    @staticmethod
    def section(text, heading="lexical", digest="b" * 64):
        return {"heading": heading, "hash": digest, "units": [text]}

    def test_oversized_winner_does_not_hide_a_feasible_quote(self):
        scorer = FakeScorer("useful answer")
        options = [self.section("Lexical guidance."), self.section("oversized winner " * 12, "oversized"),
                   self.section("A useful answer.", "answer", "c" * 64)]
        pilot, items = self.build({"specs/guide.md": options}, scorer)
        before = copy.deepcopy(items)
        pilot.prepare(None, items, "find the useful answer")
        self.assertEqual(["Lexical guidance.", "A useful answer."], scorer.calls[0][1])
        self.assertTrue(pilot.active)
        self.assertEqual("A useful answer.", items[0]["snippet"])
        self.assertLessEqual(items[0]["estimated_tokens"], before[0]["estimated_tokens"])
        self.assertEqual(before[0]["source_hash"], items[0]["source_hash"])
        self.assertEqual(("specs/guide.md#answer", "c" * 64), items[0]["_excerpt_identity"])
        self.assertEqual(1, pilot.stats["filtered-options"])
        self.assertEqual(1, len(scorer.calls))

    def test_no_admissible_alternative_preserves_exact_lexical_items(self):
        scorer = FakeScorer()
        options = [self.section("Lexical guidance."), self.section("oversized " * 30), self.section("")]
        pilot, items = self.build({"specs/guide.md": options}, scorer)
        before = copy.deepcopy(items)
        pilot.prepare(None, items, "find guidance")
        self.assertEqual([], scorer.calls)
        self.assertEqual(before, items)
        self.assertEqual({}, pilot.prepared)
        self.assertFalse(pilot.active)
        self.assertEqual(2, pilot.stats["filtered-options"])
        self.assertEqual(1, pilot.stats["no-alternatives"])

    def test_inadmissible_early_groups_do_not_exhaust_worker_slots(self):
        options = {"specs/early-" + str(index) + ".md":
                   [self.section("Lexical guidance.")] + [self.section("oversized " * 30)] * 7
                   for index in range(4)}
        options["specs/later.md"] = [self.section("Later lexical."), self.section("A useful answer.", "answer")]
        scorer = FakeScorer("useful answer")
        pilot, items = self.build(options, scorer)
        before = copy.deepcopy(items)
        pilot.prepare(None, items, "find the useful answer")
        self.assertEqual(["Later lexical.", "A useful answer."], scorer.calls[0][1])
        self.assertEqual(before[:4], items[:4])
        self.assertEqual("A useful answer.", items[4]["snippet"])
        self.assertEqual(28, pilot.stats["filtered-options"])
        self.assertLessEqual(len(scorer.calls[0][1]), 32)

    def test_shared_passage_cap_and_lexical_ties_are_preserved(self):
        options = {"specs/guide-" + str(index) + ".md":
                   [self.section("Lexical " + str(index))] +
                   [self.section("Alternative " + str(index) + " " + str(number)) for number in range(7)]
                   for index in range(5)}
        scorer = FakeScorer("absent score target")
        pilot, items = self.build(options, scorer)
        before = copy.deepcopy(items)
        pilot.prepare(None, items, "find guidance")
        passages = scorer.calls[0][1]
        self.assertEqual(32, len(passages))
        self.assertEqual(["Lexical " + str(index) for index in range(4)], passages[::8])
        self.assertEqual(before, items)
        self.assertEqual({}, pilot.prepared)
        self.assertFalse(pilot.active)
        self.assertEqual(1, len(scorer.calls))


class StatefulTests(unittest.TestCase):
    def setUp(self):
        self.fixture = runtime_fixtures.DeliveryTest("test_the_excerpt_is_the_stretch_that_answers")
        self.fixture.setUp()
        self.addCleanup(self.fixture.tearDown)
        self.fixture.write_auth_guide()
        self.fixture.start("TASK-DELIVER")
        self.query = "how do I revoke sessions after a password reset"

    def refresh(self, scorer, *, session="night-1", gate="shadow", deadline=None):
        pilot = StatefulExcerptPilot(context_cli, runtime, scorer, deadline=deadline)
        pilot.install()
        try:
            arguments = ["refresh", "--task-id", "TASK-DELIVER", "--query", self.query, "--json", "--gate", gate]
            if session:
                arguments.extend(["--session-id", session])
            code, output, error = self.fixture.run_main(*arguments)
            self.assertEqual(0, code, error)
            result = json.loads(output)
            # Final rendering must use the already prepared choice.
            return result, dict(pilot.stats)
        finally:
            pilot.close()

    def receipts(self):
        connection = context_cli.connect(context_cli.default_database(self.fixture.repository))
        try:
            return runtime._load_session_deliveries(connection)
        finally:
            connection.close()

    def test_chosen_section_is_rendered_recorded_and_suppressed(self):
        scorer = FakeScorer()
        first, _ = self.refresh(scorer)
        self.assertIn("specs/auth.md § Recovery codes", first["capsule_text"])
        keys = self.receipts()["night-1"]["items"]
        self.assertIn("specs/auth.md#recovery-codes", keys)
        self.assertNotIn("specs/auth.md#sessions", keys)
        item = next(item for item in first["capsule"]["semantic"] if item["path"] == "specs/auth.md")
        self.assertIn(item["snippet"], first["capsule_text"])
        self.assertEqual(runtime._estimate_tokens(item["title"] + item["snippet"]), item["estimated_tokens"])
        again, _ = self.refresh(FakeScorer())
        self.assertNotIn("specs/auth.md", again["capsule_text"])
        self.assertEqual(1, len(scorer.calls))
        self.assertEqual(keys["specs/auth.md#recovery-codes"], self.receipts()["night-1"]["items"]["specs/auth.md#recovery-codes"])

    def test_another_chosen_section_is_not_suppressed(self):
        self.refresh(FakeScorer())
        other, _ = self.refresh(FakeScorer("SessionStore::purge"))
        self.assertIn("specs/auth.md § Sessions", other["capsule_text"])
        self.assertIn("specs/auth.md#sessions", self.receipts()["night-1"]["items"])

    def test_enforcing_non_session_gate_observes_section_change(self):
        first, _ = self.refresh(FakeScorer(), session=None, gate="enforce")
        other, _ = self.refresh(FakeScorer("SessionStore::purge"), session=None, gate="enforce")
        self.assertIn("Recovery codes", first["capsule_text"])
        self.assertIn("SessionStore::purge", other["capsule_text"])
        again, _ = self.refresh(FakeScorer("SessionStore::purge"), session=None, gate="enforce")
        self.assertEqual("skip", again["capsule"]["gate"]["decision"])

    def test_all_worker_failures_keep_lexical_body_and_receipt(self):
        for index, reason in enumerate(("timeout", "busy", "invalid-output", "asset-missing", "worker-unavailable", "hash-mismatch",
                                        "invalid-assets", "invalid-shape", "dependency-unavailable", "inference-failed")):
            scorer = FakeScorer(failure=reason)
            result, stats = self.refresh(scorer, session="fallback-" + str(index))
            self.assertIn("specs/auth.md § Sessions", result["capsule_text"])
            self.assertNotIn("specs/auth.md § Recovery codes", result["capsule_text"])
            keys = self.receipts()["fallback-" + str(index)]["items"]
            self.assertIn("specs/auth.md#sessions", keys)
            self.assertNotIn("specs/auth.md#recovery-codes", keys)
            self.assertEqual(1, len(scorer.calls))
            self.assertEqual(1, stats["fallback:" + reason])

    def test_low_remaining_deadline_skips_optional_work(self):
        scorer = FakeScorer()
        result, stats = self.refresh(scorer, deadline=time.monotonic() + 0.1)
        self.assertEqual([], scorer.calls)
        self.assertIn("specs/auth.md § Sessions", result["capsule_text"])
        self.assertEqual(1, stats["fallback:deadline-reserve"])

    def test_profile_failures_match_the_default_lexical_capsule_and_receipts(self):
        arguments = ["refresh", "--task-id", "TASK-DELIVER", "--query", self.query,
                     "--json", "--gate", "shadow", "--session-id", "default-lexical"]
        code, output, error = self.fixture.run_main(*arguments)
        self.assertEqual(0, code, error)
        lexical = json.loads(output)
        lexical_items = self.receipts()["default-lexical"]["items"]
        for index, reason in enumerate(("invalid-assets", "invalid-shape", "dependency-unavailable", "inference-failed")):
            with self.subTest(reason=reason):
                session = "profile-failure-" + str(index)
                scorer = FakeScorer(failure=reason)
                result, stats = self.refresh(scorer, session=session)
                self.assertEqual(lexical["capsule_text"], result["capsule_text"])
                for key in ("procedural", "semantic", "episodic", "selected", "categories", "token_estimates"):
                    self.assertEqual(lexical["capsule"].get(key), result["capsule"].get(key), key)
                self.assertEqual(lexical_items, self.receipts()[session]["items"])
                self.assertEqual(1, len(scorer.calls))
                self.assertEqual(1, stats["fallback:" + reason])
                self.assertEqual(0, stats.get("changed-excerpts", 0))

    def test_pack_rejection_recomputes_whole_session_selection(self):
        original = context_cli._render_capsule
        def render(capsule, excerpts, *args, **kwargs):
            lines, shown = original(capsule, excerpts, *args, **kwargs)
            if "specs/auth.md" in excerpts and "Recovery codes" in excerpts["specs/auth.md"]:
                shown = [item for item in shown if item.get("path") != "specs/auth.md"]
            return lines, shown
        scorer = FakeScorer()
        with mock.patch.object(context_cli, "_render_capsule", render):
            result, stats = self.refresh(scorer)
        self.assertEqual(1, stats["fallback:pack-change"])
        self.assertEqual(1, len(scorer.calls))
        self.assertIn("specs/auth.md § Sessions", result["capsule_text"])
        keys = self.receipts()["night-1"]["items"]
        self.assertIn("specs/auth.md#sessions", keys)
        self.assertNotIn("specs/auth.md#recovery-codes", keys)
        manifests = list((self.fixture.repository / "project-brain/control/retrieval-manifests").glob("*.json"))
        self.assertEqual(1, len(manifests))


class ChangelogHeadPilotTests(unittest.TestCase):
    """A changelog keeps the runtime's head-first quote under the pilot.

    The runtime started passing `prefer_head` to `excerpt_section`, and the
    pilot's stand-in for it did not take the argument: every refresh under
    the pilot failed with a TypeError."""

    def test_the_head_choice_reaches_the_runtime_and_beats_a_scored_section(self):
        received = []

        def excerpt_section(connection, item, marked, weights=None, *, query="", prefer_head=False):
            received.append(prefer_head)
            return {"shown": "head" if prefer_head else "lexical"}

        seam = SimpleNamespace(retrieve=lambda *a, **k: None, excerpt_section=excerpt_section)
        pilot = StatefulExcerptPilot(seam, seam, FakeScorer())
        connection = sqlite3.connect(":memory:")
        self.addCleanup(connection.close)
        pilot.active, pilot.connection, pilot.query = True, connection, "release notes"
        pilot.prepared = {"CHANGELOG.md": {"shown": "scored"}}
        changelog = {"path": "CHANGELOG.md", "kind": "changelog"}
        self.assertEqual({"shown": "head"}, pilot.section(
            connection, changelog, "text", query="release notes", prefer_head=True))
        self.assertEqual({"shown": "scored"}, pilot.section(connection, changelog, "text", query="release notes"))
        self.assertEqual([True], received)

    def test_a_runtime_without_the_head_choice_still_takes_the_call(self):
        def excerpt_section(connection, item, marked, weights=None, *, query=""):
            return {"shown": "lexical"}

        seam = SimpleNamespace(retrieve=lambda *a, **k: None, excerpt_section=excerpt_section)
        pilot = StatefulExcerptPilot(seam, seam, FakeScorer())
        self.assertEqual({"shown": "lexical"}, pilot.section(None, {"path": "docs/a.md"}, "text", query="q"))


class CitationAndPrivacyTests(unittest.TestCase):
    def setUp(self):
        self.fixture = runtime_fixtures.LinkedMemoryExcerptTest("test_linked_rule_survives_filename_matches_in_sources")
        self.fixture.setUp()
        self.addCleanup(self.fixture.tearDown)

    def test_only_safe_chosen_text_reaches_structured_and_rendered_output(self):
        original = "The safe introduction explains continuity. " * 8
        body = "# Safety boundary\n\n" + original + "\n\n## Billing rule\nInvoice decisions for alice@example.test require durable review.\n\n## Sources\n- src/continuity.py\n- src/rejection.py\n"
        self.fixture.document(body)
        capsule = self.fixture.capsule()
        item = capsule["semantic"][0]
        lexical = context_cli.capsule_excerpts(self.fixture.connection, capsule)[item["path"]]
        item["snippet"] = str(lexical)
        item["estimated_tokens"] = runtime._estimate_tokens(item["title"] + item["snippet"])
        scorer = FakeScorer("Invoice decisions")
        pilot = StatefulExcerptPilot(context_cli, runtime, scorer)
        pilot.install()
        try:
            pilot.connection, pilot.query = self.fixture.connection, capsule["query"]
            pilot.prepare(self.fixture.connection, [item], capsule["query"])
            excerpts = context_cli.capsule_excerpts(self.fixture.connection, capsule)
            self.assertTrue(pilot.active)
            self.assertNotIn("alice@example.test", json.dumps(scorer.calls))
            self.assertNotIn("alice@example.test", item["snippet"])
            self.assertEqual(item["snippet"], str(excerpts[item["path"]]))
            self.assertEqual("Billing rule", excerpts[item["path"]].heading)
            self.assertEqual("a" * 64, item["source_hash"])
        finally:
            pilot.close()

    def test_pure_citation_footer_is_not_an_optional_alternative(self):
        self.fixture.document("# Safety boundary\n\n" + self.fixture.RULE + "\n\n## Sources\n- src/continuity.py\n- src/rejection.py\n")
        capsule = self.fixture.capsule()
        item = capsule["semantic"][0]
        marked = runtime.marked_document(self.fixture.connection, item["path"], capsule["query"])
        pilot = StatefulExcerptPilot(context_cli, runtime, FakeScorer())
        options = pilot.options(self.fixture.connection, item, marked, {}, capsule["query"])
        self.assertTrue(options)
        self.assertNotIn("Sources", {option["heading"] for option in options})
        explicit = pilot.options(self.fixture.connection, item, marked, {}, "Sources src/continuity.py")
        self.assertEqual({"Sources"}, {option["heading"] for option in explicit})


if __name__ == "__main__":
    unittest.main()
