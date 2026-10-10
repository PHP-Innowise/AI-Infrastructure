#!/usr/bin/env python3
"""Regression tests for scripts/memory_eval.py.

The stand exists because a measurement on each project's current state
credited the capsule with knowledge written after the prompt. These tests pin
what keeps that from happening again: a synthetic Git project with commits at
controlled dates, a Brain record committed before a prompt, a chunk committed
after it, records that were never committed (one older than the prompt, one
newer), and a chunk written on the prompt's own day. One end-to-end pass runs
the real refresh of this clone's PHP Core edition; scoring, classification,
the overlay, the report and the transcript parsers are also tested on
hand-made data.

Run: python3 -m unittest tests.test_memory_eval
"""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import re
import sqlite3
import stat
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional
from unittest import mock

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import memory_eval  # noqa: E402

SCRIPT = ROOT / "scripts" / "memory_eval.py"

T1 = "2026-08-01T10:00:00+00:00"  # first commit
T2 = "2026-08-10T10:00:00+00:00"  # the chunk is committed
P1_TS = "2026-08-05T12:00:00Z"
P2_TS = "2026-08-12T12:00:00Z"
P3_TS = "2026-07-01T00:00:00Z"  # before any commit
RECORD_A = "11111111-1111-4111-8111-111111111111"  # committed, created before both prompts
RECORD_B = "22222222-2222-4222-8222-222222222222"  # never committed, created before p1
RECORD_C = "33333333-3333-4333-8333-333333333333"  # never committed, created after p2
RECORD_D = "44444444-4444-4444-8444-444444444444"  # committed at T1, created after p1 (backdated commit)
FINDINGS = "project-brain/dynamic/findings"
PATH_A = f"{FINDINGS}/{RECORD_A}.md"
PATH_B = f"{FINDINGS}/{RECORD_B}.md"
PATH_C = f"{FINDINGS}/{RECORD_C}.md"
PATH_D = f"{FINDINGS}/{RECORD_D}.md"
CHUNK = "memory-bank/chunks/MEM-20260810-0a1b2c3d-invoice-rounding.md"  # committed at T2
SAME_DAY_CHUNK = "memory-bank/chunks/MEM-20260805-5a5a5a5a-export-format.md"  # never committed, p1's day
ANSWER = "Invoice totals use banker's rounding to two decimals"
NO_ARCHIVE = {key: 0 for key in (
    "archive_from_git", "archive_from_worktree", "archive_dropped_future", "archive_undetermined",
    "archive_unreconstructable_updated", "archive_updated_after", "archive_still_dynamic",
)}


def git(cwd: Path, *arguments: str, when: Optional[str] = None) -> None:
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(
        GIT_AUTHOR_NAME="Fixture",
        GIT_AUTHOR_EMAIL="fixture@example.invalid",
        GIT_COMMITTER_NAME="Fixture",
        GIT_COMMITTER_EMAIL="fixture@example.invalid",
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
    )
    if when:
        env.update(GIT_AUTHOR_DATE=when, GIT_COMMITTER_DATE=when)
    subprocess.run(
        ["git", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main", *arguments],
        cwd=str(cwd),
        env=env,
        check=True,
        capture_output=True,
    )


def write(root: Path, relative: str, text: str) -> Path:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def finding(identifier: str, title: str, goal: str, created: str, observation: str, updated: str = "") -> str:
    """An open Project Brain finding the runtime accepts."""
    metadata = {
        "schema_version": 1,
        "id": identifier,
        "type": "finding",
        "external_id": f"FINDING-{identifier[:4]}",
        "title": title,
        "status": "open",
        "revision": 1,
        "owner": "local",
        "authorized_owners": ["local"],
        "privacy": "team",
        "authority": "observed",
        "confidence": 0.8,
        "created_at": created,
        "updated_at": updated or created,
        "transitions": [{"from": None, "to": "open", "at": created, "actor": "local", "reason": "Finding recorded"}],
        "source_fingerprints": [],
        "conflicts": [],
        "supersedes": [],
        "superseded_by": None,
        "goal": goal,
        "progress": "",
        "next_steps": [],
        "files": [],
        "sources": [],
    }
    body = f"# {title}\n\n## Observation\n\n{observation}\n"
    return "---\n" + json.dumps(metadata, indent=2, sort_keys=True) + "\n---\n\n" + body


def chunk(memory_id: str, title: str, created: str, review_after: str, context: str) -> str:
    """An active Memory Bank chunk the runtime accepts; it cites docs/billing.md."""
    metadata = {
        "id": memory_id,
        "title": title,
        "type": "convention",
        "status": "active",
        "scope": ["application"],
        "tags": ["billing"],
        "created": created,
        "last_verified": created,
        "review_after": review_after,
        "sources": ["docs/billing.md"],
        "supersedes": [],
        "superseded_by": None,
        "valid_from": created,
        "valid_to": None,
        "source_digests": [],
    }
    body = f"# {title}\n\n## Durable Context\n\n{context}\n"
    return "---\n" + json.dumps(metadata, indent=2) + "\n---\n\n" + body


def build_ledger(base: Path) -> Path:
    """The project of the end-to-end run; see the module docstring."""
    project = base / "ledger"
    project.mkdir(parents=True)
    git(project, "init", "-q")
    write(project, "README.md", "# Ledger\n\nInvoicing service of the shop.\n")
    write(project, "composer.json", '{"name": "acme/ledger", "require": {"php": "^8.2"}}\n')
    write(project, "src/InvoiceCalculator.php", "<?php\n\nfinal class InvoiceCalculator\n{\n}\n")
    write(
        project,
        "docs/billing.md",
        "# Billing\n\nInvoice totals are rounded half-up to two decimals by the InvoiceCalculator.\n",
    )
    write(
        project,
        PATH_A,
        finding(
            RECORD_A,
            "Invoice export drops cents",
            "Find why exported invoice totals lose cents",
            "2026-07-31T09:00:00Z",
            "The CSV export truncates invoice totals.",
        ),
    )
    write(
        project,
        PATH_D,
        finding(
            RECORD_D,
            "Ledger reconciliation timing",
            "Check when the nightly reconciliation runs",
            "2026-08-06T09:00:00Z",
            "The job overlaps the backup window.",
        ),
    )
    # An older install committed in the project: a stale copy of a skill the
    # edition ships, and a skill of the project's own that no edition ships.
    write(project, ".agents/skills/coder/SKILL.md", "stale copy from an older install\n")
    write(project, ".claude/skills/team-notes/SKILL.md", "---\nname: team-notes\n---\n\n# Team notes\n")
    git(project, "add", "-A")
    git(project, "commit", "-q", "-m", "ledger", when=T1)
    write(
        project,
        CHUNK,
        chunk(
            "MEM-20260810-0a1b2c3d",
            "Invoice rounding rule",
            "2026-08-10",
            "2026-08-20",
            f"{ANSWER}, applied by the InvoiceCalculator.",
        ),
    )
    git(project, "add", "-A")
    git(project, "commit", "-q", "-m", "rounding chunk", when=T2)
    # Never committed: what client projects do with their memory.
    write(
        project,
        PATH_B,
        finding(
            RECORD_B,
            "Export rounding goes through the invoice calculator",
            "Confirm the export rounds invoice totals like the calculator",
            "2026-08-03T08:00:00Z",
            "The export calls InvoiceCalculator::round.",
        ),
    )
    write(
        project,
        PATH_C,
        finding(
            RECORD_C,
            "Invoice rounding rewrite",
            "Rewrite invoice rounding to banker's rounding",
            "2026-08-20T08:00:00Z",
            "Invoice totals move to banker's rounding.",
        ),
    )
    write(
        project,
        SAME_DAY_CHUNK,
        chunk(
            "MEM-20260805-5a5a5a5a",
            "Invoice export format",
            "2026-08-05",
            "2026-12-31",
            "The invoice export is a semicolon-separated CSV.",
        ),
    )
    return project


def tree_state(root: Path) -> Dict[str, tuple]:
    """Every entry under root, .git included: type, size, mtime, content hash."""
    state: Dict[str, tuple] = {}
    for directory, folders, files in os.walk(root):
        for name in folders + files:
            path = Path(directory) / name
            info = path.lstat()
            entry: tuple = (stat.S_IFMT(info.st_mode), info.st_size, info.st_mtime_ns)
            if stat.S_ISREG(info.st_mode):
                entry += (hashlib.sha256(path.read_bytes()).hexdigest(),)
            state[path.relative_to(root).as_posix()] = entry
    return state


def run_cli(*arguments: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *arguments], capture_output=True, text=True, timeout=300
    )


def main_output(*arguments: str) -> str:
    stream = io.StringIO()
    with contextlib.redirect_stdout(stream):
        status = memory_eval.main(list(arguments))
    if status != 0:
        raise AssertionError(f"memory_eval {arguments[0]} exited {status}")
    return stream.getvalue()


SET = [
    {
        "id": "p1",
        "project": "ledger",
        "ts": P1_TS,
        "prompt": "How are invoice totals rounded in the billing export?",
        "session": "s-1",
        "cwd": "/somewhere/else",
    },
    {"id": "p2", "project": "ledger", "ts": P2_TS, "prompt": "Which rounding rule do invoice totals follow?"},
    {"id": "p3", "project": "ledger", "ts": P3_TS, "prompt": "Set up the ledger project"},
]
JUDGMENTS = {
    "p1": {
        "docs/billing.md": 2,
        PATH_B: 1,
        CHUNK: 2,
        ".claude/skills/coder/SKILL.md": 0,
    },
    "p2": {CHUNK: 2, "docs/billing.md": 1},
}
PASSAGES = {
    "p1": {"docs/billing.md": {"useful": True, "passages": ["rounded  HALF-UP to two\ndecimals"]}},
    "p2": {CHUNK: {"useful": True, "passages": [ANSWER]}},
}


class EndToEndTest(unittest.TestCase):
    """One synthetic project, the real PHP Core refresh, three runs."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.temporary = tempfile.TemporaryDirectory(prefix="memory-eval-test-")
        base = Path(cls.temporary.name)
        cls.projects = base / "projects"
        cls.project = build_ledger(cls.projects)
        cls.before = tree_state(cls.project)
        data = base / "data"
        data.mkdir()
        cls.set_path = data / "set.json"
        cls.set_path.write_text(json.dumps(SET), encoding="utf-8")
        cls.judgments_path = data / "judgments.json"
        cls.judgments_path.write_text(json.dumps(JUDGMENTS), encoding="utf-8")
        cls.passages_path = data / "passages.json"
        cls.passages_path.write_text(json.dumps(PASSAGES), encoding="utf-8")
        cls.cache = base / "cache"
        common = [
            "run", "--set", str(cls.set_path), "--judgments", str(cls.judgments_path),
            "--passages", str(cls.passages_path), "--projects-root", str(cls.projects),
            "--edition", "PHP Core", "--cache", str(cls.cache),
        ]
        cls.outputs: Dict[str, Path] = {}
        cls.processes: Dict[str, subprocess.CompletedProcess] = {}
        for name, extra in (
            ("prompt", []),
            ("now", ["--as-of", "now"]),
            ("real-clock", ["--ids", "p2", "--clock", "real"]),
            ("no-match", ["--ids", "nothing-like-this"]),
        ):
            out = base / "results" / f"{name}.json"
            cls.outputs[name] = out
            cls.processes[name] = run_cli(*common, "--out", str(out), *extra)
        cls.after = tree_state(cls.project)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.temporary.cleanup()

    def result(self, name: str) -> Dict[str, Any]:
        process = self.processes[name]
        self.assertEqual(process.returncode, 0, process.stderr)
        return json.loads(self.outputs[name].read_text(encoding="utf-8"))

    def test_a_run_that_evaluated_nothing_fails(self) -> None:
        # Ids that match no prompt, like an empty set, measure nothing.
        self.assertEqual(1, self.processes["no-match"].returncode)
        empty = self.cache.parent / "data" / "empty.json"
        empty.write_text("[]", encoding="utf-8")
        process = run_cli("run", "--set", str(empty), "--judgments", str(self.judgments_path),
                          "--projects-root", str(self.projects), "--edition", "PHP Core",
                          "--cache", str(self.cache), "--out", str(self.cache.parent / "results" / "empty.json"))
        self.assertEqual(1, process.returncode, process.stderr)

    def test_the_project_is_untouched(self) -> None:
        # .git included: borrowing the project's objects would refresh their mtimes.
        self.assertEqual(self.before, self.after)

    def test_a_prompt_before_any_commit_is_skipped(self) -> None:
        item = self.result("prompt")["items"]["p3"]
        self.assertEqual(item["status"], "skipped")
        self.assertEqual(item["reason"], "no-history")

    def test_as_of_reconstruction_leaves_out_the_future(self) -> None:
        p1 = self.result("prompt")["items"]["p1"]
        self.assertEqual(p1["status"], "ok", p1)
        self.assertEqual(
            p1["provenance"],
            # A from the commit; D (backdated commit), the chunk and C were
            # written after the prompt; B existed uncommitted; the same-day
            # chunk cannot be placed before or after it.
            {"from_git": 1, "from_worktree": 1, "dropped_future": 3, "undetermined": 1, "updated_after": 0,
             "unreconstructable_updated": 0, **NO_ARCHIVE},
        )
        self.assertNotIn(CHUNK, p1["existed_useful"])
        self.assertNotIn(CHUNK, p1["delivered"])
        self.assertTrue(p1["could_help"])
        self.assertTrue(p1["history_linked"])

    def test_an_uncommitted_record_older_than_the_prompt_is_included(self) -> None:
        p1 = self.result("prompt")["items"]["p1"]
        self.assertEqual(p1["existed_useful"], ["docs/billing.md", PATH_B])
        p2 = self.result("prompt")["items"]["p2"]
        self.assertEqual(
            p2["provenance"],
            {"from_git": 3, "from_worktree": 2, "dropped_future": 1, "undetermined": 0, "updated_after": 0,
             "unreconstructable_updated": 0, **NO_ARCHIVE},
        )

    def test_as_of_now_includes_the_future_document(self) -> None:
        p1 = self.result("now")["items"]["p1"]
        self.assertEqual(p1["status"], "ok", p1)
        self.assertIn(CHUNK, p1["existed_useful"])
        self.assertEqual(p1["provenance"]["future_present"], 3)
        # Without history the current tree is still there to measure.
        self.assertEqual(self.result("now")["items"]["p3"]["status"], "ok")

    def test_classification_and_answer_in_text(self) -> None:
        items = self.result("prompt")["items"]
        p1 = items["p1"]
        self.assertIn("docs/billing.md", p1["delivered"])
        self.assertEqual(p1["class"], "useful")
        self.assertIn("docs/billing.md", p1["useful_delivered"])
        # Whitespace and case differ from the document; still the answer.
        self.assertTrue(p1["answer_in_text"])
        skills = [path for path in p1["delivered"] if "/skills/" in path]
        self.assertTrue(all(path.startswith(".agents/skills/") for path in skills))
        p2 = items["p2"]
        self.assertIn(CHUNK, p2["delivered"])
        self.assertEqual(p2["class"], "useful")
        self.assertTrue(p2["answer_in_text"])
        self.assertEqual(p2["refresh"]["exit"], 0)
        # The real runtime's layer report, as the stand records it.
        self.assertEqual({layer: "updated" for layer in memory_eval.LAYERS}, p2["refresh"]["layers"])

    def test_the_runtime_clock_reads_the_prompt(self) -> None:
        # The chunk is due for review on 2026-08-20: live at p2 (08-12), overdue today.
        pinned = self.result("prompt")["items"]["p2"]
        real = self.result("real-clock")["items"]["p2"]
        self.assertEqual(self.result("prompt")["meta"]["clock"], "pinned")
        self.assertEqual(self.result("real-clock")["meta"]["clock"], "real")
        self.assertIn(CHUNK, pinned["delivered"])
        self.assertNotIn(CHUNK, real["delivered"])
        self.assertIn(CHUNK, real["existed_useful"])

    def test_the_overlay_replaces_old_accelerator_copies_and_keeps_the_projects_own(self) -> None:
        overlay = self.result("prompt")["items"]["p1"]["overlay"]
        self.assertGreater(overlay["copied"], 100)
        self.assertGreaterEqual(overlay["removed"], 1)  # the stale coder skill
        self.assertGreaterEqual(overlay["kept"], 1)  # the project's README

    def test_the_output_carries_no_prompt_or_document_text(self) -> None:
        for name in self.outputs:
            text = self.outputs[name].read_text(encoding="utf-8")
            for prompt in SET:
                self.assertNotIn(prompt["prompt"], text)
            self.assertNotIn("rounded half-up", text)
            self.assertNotIn(ANSWER, text)

    def test_meta_names_what_was_measured(self) -> None:
        meta = self.result("prompt")["meta"]
        self.assertEqual(meta["edition"], "PHP Core")
        self.assertEqual(meta["as_of"], "prompt")
        self.assertEqual(meta["editions"]["PHP Core"]["path"], "PHP Core")
        self.assertRegex(meta["editions"]["PHP Core"]["digest"], r"^[0-9a-f]{64}$")
        self.assertEqual(meta["set_sha256"], hashlib.sha256(self.set_path.read_bytes()).hexdigest())
        self.assertIsNotNone(meta["finished"])
        summary = self.result("prompt")["summary"]
        self.assertEqual(summary["prompts"], 3)
        self.assertEqual(summary["evaluated"], 2)
        self.assertEqual(summary["skipped"], {"no-history": 1})

    def test_report_and_compare(self) -> None:
        prompt, now = str(self.outputs["prompt"]), str(self.outputs["now"])
        single = main_output("report", prompt)
        self.assertIn("useful turns", single)
        self.assertIn("skipped in A: no-history 1", single)
        self.assertIn("latency p95 (s)", single)
        compared = main_output("report", prompt, "--compare", now, "--show-unjudged")
        self.assertIn("delta", compared)
        self.assertIn("paired over 2 prompt(s) evaluated in both", compared)
        self.assertIn("pair(s) without a judgment", compared)
        for prompt_entry in SET:
            self.assertNotIn(prompt_entry["prompt"], compared)

    def test_the_cache_holds_no_leftover_corpus(self) -> None:
        self.assertEqual(list((self.cache / "runs").iterdir()), [])
        self.assertTrue(any((self.cache / "public-trees-v1").iterdir()))
        self.assertTrue(any((self.cache / "history").iterdir()))


class AutoEditionTest(unittest.TestCase):
    def test_auto_reads_the_install_manifest(self) -> None:
        with tempfile.TemporaryDirectory(prefix="memory-eval-auto-") as name:
            base = Path(name)
            project = base / "projects" / "shop"
            project.mkdir(parents=True)
            git(project, "init", "-q")
            write(project, "README.md", "# Shop\n")
            write(project, "docs/orders.md", "# Orders\n\nOrders are confirmed by e-mail.\n")
            git(project, "add", "-A")
            git(project, "commit", "-q", "-m", "shop", when=T1)
            # Ignored local state, never committed: the installer's sync manifest.
            write(project, memory_eval.INSTALL_MANIFEST, json.dumps({"schema": 1, "edition": "Symfony"}))
            prompts = [{"id": "s1", "project": str(project), "ts": P1_TS, "prompt": "How are orders confirmed?"}]
            (base / "set.json").write_text(json.dumps(prompts), encoding="utf-8")
            (base / "judgments.json").write_text("{}", encoding="utf-8")
            out = base / "result.json"
            process = run_cli(
                "run", "--set", str(base / "set.json"), "--judgments", str(base / "judgments.json"),
                "--out", str(out), "--cache", str(base / "cache"), "--keep",
            )
            self.assertEqual(process.returncode, 0, process.stderr)
            document = json.loads(out.read_text(encoding="utf-8"))
            item = document["items"]["s1"]
            self.assertEqual(item["status"], "ok", item)
            self.assertEqual((item["edition"], item["edition_source"]), ("Symfony", "install-manifest"))
            self.assertEqual(document["meta"]["edition"], "auto")
            self.assertIn("Symfony", document["meta"]["editions"])
            corpus = Path(item["corpus"])

            def first_line(path: Path) -> str:
                return path.read_text(encoding="utf-8").splitlines()[0]

            self.assertEqual(first_line(corpus / "AGENTS.md"), first_line(ROOT / "Symfony" / "AGENTS.md"))
            self.assertEqual(first_line(corpus / "docs" / "orders.md"), "# Orders")
            # The corpus commit sits on the project's history, as a checkout's would.
            self.assertTrue(item["history_linked"])

            def corpus_git(*arguments: str) -> str:
                return subprocess.run(
                    ["git", "-C", str(corpus), *arguments], capture_output=True, text=True, check=True
                ).stdout.strip()

            self.assertEqual(corpus_git("log", "-1", "--format=%P"), item["commit"])
            self.assertEqual(corpus_git("branch", "--show-current"), "eval/s1")

    def test_detection_order(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            project = Path(name)
            self.assertEqual(memory_eval.detect_edition(project), ("PHP Core", "default"))
            write(project, "artisan", "#!/usr/bin/env php\n")
            self.assertEqual(memory_eval.detect_edition(project), ("Laravel", "markers"))
            write(project, memory_eval.POLICY_LOCK, json.dumps({"edition": "Cms/wordpress"}))
            self.assertEqual(memory_eval.detect_edition(project), ("WordPress", "policy-lock"))
            write(project, memory_eval.INSTALL_MANIFEST, json.dumps({"edition": "PHP Core"}))
            self.assertEqual(memory_eval.detect_edition(project), ("PHP Core", "install-manifest"))

    def test_edition_names(self) -> None:
        self.assertEqual(memory_eval.edition_name("php-core"), "PHP Core")
        self.assertEqual(memory_eval.edition_name("Cms/wordpress"), "WordPress")
        self.assertEqual(memory_eval.edition_name(str(ROOT / "Laravel")), "Laravel")
        self.assertIsNone(memory_eval.edition_name("Infrastructure-Creator"))


def refresh_result(paths: Dict[str, List[str]], text: str = "", withheld: bool = False) -> Dict[str, Any]:
    """A refresh JSON as `context.py refresh --json` prints it, trimmed to what is scored."""
    if withheld:
        return {"capsule": None, "query_withheld": True}
    capsule = {
        layer: [{"path": path, "snippet": "x"} for path in paths.get(layer, [])] for layer in memory_eval.LAYERS
    }
    return {"capsule": capsule, "capsule_text": text}


def passage(*texts: str, useful: bool = True) -> Dict[str, Dict[str, Any]]:
    return {"docs/a.md": {"useful": useful, "passages": list(texts)}}


class ScoringTest(unittest.TestCase):
    GRADES = {"docs/a.md": 2, "docs/b.md": 0, ".agents/skills/coder/SKILL.md": 1}

    def test_counts_distinct_existing_passages_in_delivered_text(self) -> None:
        passages = {"docs/a.md": {"useful": True, "passages": ["First answer.", "  first  answer. ", "Second answer."]}}
        result = refresh_result({"semantic": ["docs/a.md"]})
        result["capsule_text"] = "First answer."
        one = memory_eval.score(result, self.GRADES, passages, ["docs/a.md"], ["docs/a.md"])
        self.assertEqual(2, one["answer_passages_existed"])
        self.assertEqual(1, one["answer_passages_delivered"])
        result["capsule_text"] += " Second answer."
        two = memory_eval.score(result, self.GRADES, passages, ["docs/a.md"], ["docs/a.md"])
        self.assertEqual(2, two["answer_passages_delivered"])
        result["capsule_text"] = "docs/a.md"
        pointer = memory_eval.score(result, self.GRADES, passages, ["docs/a.md"], ["docs/a.md"])
        self.assertEqual(0, pointer["answer_passages_delivered"])
        one.update(status="ok")
        two.update(status="ok")
        summary = memory_eval.summarize({"one": one, "two": two})
        self.assertEqual(4, summary["answer_passages_existed"])
        self.assertEqual(3, summary["answer_passages_delivered"])
        self.assertEqual(0.75, summary["answer_passage_recall"])
        self.assertEqual("75.0%", memory_eval.row_cell(summary, "answer_passage_recall"))
        self.assertIsNone(memory_eval.summarize({"legacy": {"status": "ok"}})["answer_passage_recall"])

    def test_passage_ceiling_uses_actual_prompt_time_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            corpus = Path(temporary)
            write(corpus, "docs/a.md", "First answer.")
            passages = {"docs/a.md": {"passages": ["First answer.", "Future answer."]}}
            expected = memory_eval.existing_answer_passages(corpus, ["docs/a.md"], passages)
            result = refresh_result({"semantic": ["docs/a.md"]})
            result["capsule_text"] = "First answer. Future answer."
            scored = memory_eval.score(result, self.GRADES, passages, ["docs/a.md"],
                                       ["docs/a.md"], expected_passages=expected)
            self.assertEqual(1, scored["answer_passages_existed"])
            self.assertEqual(1, scored["answer_passages_delivered"])

    def test_keeps_only_bounded_safe_graph_manifest_counters(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            corpus = Path(temporary)
            reference = "memory-bank/local/retrieval-manifests/11111111-1111-4111-8111-111111111111.json"
            stats = {"anchors": 1, "candidates": 2, "delivered": 0}
            write(corpus, reference, json.dumps({"source_links": {**stats, "withheld_path": "private/source.md"}}))
            result = {"capsule": {"manifest": reference}}
            self.assertEqual(stats, memory_eval.source_link_stats(corpus, result))
            write(corpus, reference, json.dumps({"source_links": {**stats, "candidates": 100}}))
            self.assertIsNone(memory_eval.source_link_stats(corpus, result))
            self.assertIsNone(memory_eval.source_link_stats(corpus, {"capsule": {"manifest": "../private/source.md"}}))

    def test_classes(self) -> None:
        cases = [
            ({"semantic": ["docs/a.md", "docs/b.md"]}, "useful"),
            ({"semantic": ["docs/b.md", "docs/c.md"]}, "noise-only"),
            ({"semantic": ["docs/c.md"]}, "unjudged-only"),
            ({}, "silent"),
            # A tool's skill tree is the canonical one.
            ({"procedural": [".claude/skills/coder/SKILL.md"]}, "useful"),
        ]
        for paths, expected in cases:
            with self.subTest(paths=paths):
                scored = memory_eval.score(refresh_result(paths), self.GRADES, {}, [])
                self.assertEqual(scored["class"], expected)
        withheld = memory_eval.score(refresh_result({}, withheld=True), self.GRADES, {}, ["docs/a.md"])
        self.assertEqual(
            (withheld["class"], withheld["query_withheld"], withheld["could_help"]), ("silent", True, True)
        )
        self.assertEqual(memory_eval.score(None, self.GRADES, {}, [])["class"], "silent")

    def test_delivered_paths_are_ordered_unique_and_normalised(self) -> None:
        capsule = {
            "semantic": [{"path": "docs/a.md"}, {"path": "./docs/a.md"}, {"id": "episode-without-path"}],
            "episodic": [{"path": "CHANGELOG.md"}],
            "procedural": [{"path": ".cursor/skills/coder/SKILL.md"}],
        }
        self.assertEqual(
            memory_eval.delivered_paths(capsule),
            ["docs/a.md", "CHANGELOG.md", ".agents/skills/coder/SKILL.md"],
        )

    def test_answer_in_text(self) -> None:
        text = "memory:\n- memory docs/a.md — A\n  Totals are   Rounded\nhalf-up."
        useful = ["docs/a.md"]
        found = memory_eval.answer_in_text
        self.assertTrue(found(useful, passage("totals are rounded half-up"), text))
        self.assertFalse(found(useful, passage("rounded half-even"), text))
        # An empty passage is a substring of everything; it proves nothing.
        self.assertFalse(found(useful, passage("", "  "), text))
        self.assertFalse(found(useful, passage("rounded", useful=False), text))
        # Only a useful delivered document's passage counts.
        self.assertFalse(found([], passage("rounded"), text))

    def test_an_answer_written_after_the_prompt_is_not_one_that_existed(self) -> None:
        # The file existed at the prompt; the entry that answered it did not.
        with tempfile.TemporaryDirectory() as name:
            corpus = Path(name)
            (corpus / "CHANGELOG.md").write_text("# Changelog\n\n- Older entry.\n", encoding="utf-8")
            (corpus / "docs").mkdir()
            (corpus / "docs/a.md").write_text("Totals are rounded\nhalf-up.\n", encoding="utf-8")
            grades = {"CHANGELOG.md": 2, "docs/a.md": 1, ".agents/skills/x/SKILL.md": 2, "docs/gone.md": 2}
            passages = {
                "CHANGELOG.md": {"useful": True, "passages": ["the entry written for this very work"]},
                "docs/a.md": {"useful": True, "passages": ["totals are rounded half-up"]},
                "docs/gone.md": {"useful": True, "passages": ["anything"]},
            }
            self.assertEqual(["docs/a.md"], memory_eval.answer_existing(corpus, grades, passages))
            self.assertEqual(["CHANGELOG.md", "docs/a.md"],
                             [path for path in memory_eval.existing_useful(corpus, grades) if "/skills/" not in path])
            scored = memory_eval.score(refresh_result({}), grades, passages, ["CHANGELOG.md"], [])
            self.assertEqual((True, False), (scored["could_help"], scored["answer_could_help"]))

    def test_a_skill_the_edition_does_not_install_did_not_exist(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            corpus = Path(name)
            grades = {".agents/skills/removed/SKILL.md": 2, ".agents/skills/kept/SKILL.md": 2}
            skill = corpus / ".cursor/skills/kept/SKILL.md"
            skill.parent.mkdir(parents=True)
            skill.write_text("# Kept\n", encoding="utf-8")
            self.assertEqual([".agents/skills/kept/SKILL.md"], memory_eval.existing_useful(corpus, grades))

    def test_a_skills_labelled_answer_counts_where_the_overlay_put_it(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            corpus = Path(name)
            skill = corpus / ".claude/skills/rounding/SKILL.md"
            skill.parent.mkdir(parents=True)
            skill.write_text("# Rounding\n\nUse half-up   rounding for totals.\n", encoding="utf-8")
            grades = {".agents/skills/rounding/SKILL.md": 2}
            passages = {".agents/skills/rounding/SKILL.md": {"useful": True, "passages": ["Use half-up rounding"]}}
            self.assertEqual([".agents/skills/rounding/SKILL.md"],
                             memory_eval.answer_existing(corpus, grades, passages))
            text = "- skill .agents/skills/rounding/SKILL.md — Rounding\n  Use half-up rounding for totals."
            scored = memory_eval.score(refresh_result({"procedural": [".agents/skills/rounding/SKILL.md"]}, text),
                                       grades, passages, list(grades), list(grades))
            self.assertEqual((True, True), (scored["answer_could_help"], scored["answer_in_text"]))

    def test_judgments_take_grades_0_1_2_and_refuse_anything_else(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "judgments.json"
            grades = {"docs/a.md": 2, "docs/b.md": 0, ".codex/skills/x/SKILL.md": 1, ".agents/skills/x/SKILL.md": 0}
            path.write_text(json.dumps({"p": grades, "q": {}}), encoding="utf-8")
            self.assertEqual(
                memory_eval.load_judgments(path),
                {"p": {"docs/a.md": 2, "docs/b.md": 0, ".agents/skills/x/SKILL.md": 1}, "q": {}},
            )
            # A grade of 3 or -1 counted as useful or noise; one that is not a
            # number was dropped and its document counted unjudged. Either way
            # the measures moved without a word.
            for grade in (3, -1, 10 ** 30, True, False, 1.5, 2.0, "2", None, [2]):
                with self.subTest(grade=grade):
                    path.write_text(json.dumps({"p17": {"docs/a.md": 1, "docs/odd.md": grade}}), encoding="utf-8")
                    with self.assertRaises(memory_eval.EvalError) as caught:
                        memory_eval.load_judgments(path)
                    message = str(caught.exception)
                    self.assertIn("'p17'", message)
                    self.assertIn("docs/odd.md", message)
                    self.assertIn("0, 1 or 2", message)
            path.write_text(json.dumps({"p17": [["docs/a.md", 2]]}), encoding="utf-8")
            with self.assertRaisesRegex(memory_eval.EvalError, "'p17'"):
                memory_eval.load_judgments(path)
            # A reviewer's note beside the grades is read past; only as text.
            path.write_text(json.dumps({"p": {"docs/a.md": 2, "_note": "overview prompt; spec and README"}}),
                            encoding="utf-8")
            self.assertEqual({"p": {"docs/a.md": 2}}, memory_eval.load_judgments(path))
            path.write_text(json.dumps({"p": {"docs/a.md": 2, "_note": ["overview"]}}), encoding="utf-8")
            with self.assertRaisesRegex(memory_eval.EvalError, "_note"):
                memory_eval.load_judgments(path)

    def test_run_refuses_judgments_out_of_range_before_any_work(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            base = Path(name)
            (base / "set.json").write_text(json.dumps(SET[:1]), encoding="utf-8")
            (base / "judgments.json").write_text(json.dumps({"p1": {"docs/billing.md": 3}}), encoding="utf-8")
            process = run_cli(
                "run", "--set", str(base / "set.json"), "--judgments", str(base / "judgments.json"),
                "--projects-root", str(base), "--edition", "PHP Core",
                "--cache", str(base / "cache"), "--out", str(base / "result.json"),
            )
            self.assertEqual(2, process.returncode, process.stderr)
            self.assertIn("docs/billing.md", process.stderr)
            self.assertFalse((base / "cache").exists())
            self.assertFalse((base / "result.json").exists())

    def test_summary(self) -> None:
        def evaluated(kind: str, delivered: List[str], noise: List[str], answer: bool, seconds: float) -> Dict[str, Any]:
            return {
                "status": "ok", "class": kind, "could_help": True, "answer_in_text": answer,
                "delivered": delivered, "noise_delivered": noise, "unjudged": [], "refresh": {"seconds": seconds},
            }

        items = {
            "a": evaluated("useful", ["x", "y"], ["y"], True, 0.2),
            "b": evaluated("silent", [], [], False, 0.4),
            "c": {"status": "skipped", "reason": "no-history"},
        }
        summary = memory_eval.summarize(items)
        self.assertEqual(summary["evaluated"], 2)
        self.assertEqual(summary["skipped"], {"no-history": 1})
        self.assertEqual((summary["useful"], summary["could_help"], summary["useful_among_could_help"]), (1, 2, 1))
        self.assertEqual((summary["silent"], summary["answer_in_text"]), (1, 1))
        self.assertEqual((summary["mean_delivered"], summary["mean_noise"]), (1.0, 0.5))
        self.assertEqual((summary["latency_p50"], summary["latency_p95"]), (0.2, 0.4))

    def test_paired_diff(self) -> None:
        first = {
            "a": {"status": "ok", "class": "useful", "answer_in_text": True},
            "b": {"status": "ok", "class": "noise-only"},
            "c": {"status": "ok", "class": "silent"},
            "d": {"status": "skipped"},
        }
        second = {
            "a": {"status": "ok", "class": "silent"},
            "b": {"status": "ok", "class": "useful", "answer_in_text": True},
            "c": {"status": "ok", "class": "noise-only"},
            "d": {"status": "ok", "class": "useful"},
        }
        count, changes = memory_eval.paired_diff(first, second)
        self.assertEqual(count, 3)
        self.assertEqual(changes["useful"], (["b"], ["a"]))
        self.assertEqual(changes["noise-only"], (["c"], ["b"]))
        self.assertEqual(changes["answer"], (["b"], ["a"]))

    def test_compare_deltas_are_over_the_prompts_evaluated_in_both(self) -> None:
        # p1 is useful in both; p2 is useful in A and failed in B. Nothing was
        # lost on a prompt B measured, so no measure may show a loss.
        def ok(kind: str, seconds: float, delivered: int) -> Dict[str, Any]:
            return {
                "status": "ok", "class": kind, "could_help": True, "answer_in_text": kind == "useful",
                "delivered": ["x"] * delivered, "noise_delivered": [], "unjudged": [],
                "refresh": {"seconds": seconds},
            }

        first = {"meta": {}, "items": {"p1": ok("useful", 0.1, 1), "p2": ok("useful", 3.0, 5)}}
        second = {"meta": {}, "items": {"p1": ok("useful", 0.1, 1), "p2": {"status": "skipped", "reason": "refresh-error"}}}
        report = memory_eval.format_report([("A", Path("a.json"), first), ("B", Path("b.json"), second)])
        rows = {line[:42].strip(): line.split()[-1] for line in report.splitlines() if line[:1].strip()}
        for title in ("useful turns", "could help (a useful document existed)", "answer in the capsule text",
                      "useful among could-help"):
            self.assertEqual("+0", rows[title], (title, report))
        for title in ("mean delivered per turn", "latency p50 (s)", "latency p95 (s)"):
            self.assertEqual("+0.000", rows[title], (title, report))
        # The coverage rows compare the whole runs, and the report says so.
        self.assertEqual("-1", rows["evaluated"], report)
        self.assertEqual("+1", rows["skipped"], report)
        self.assertIn("1 evaluated in both", report)
        self.assertIn("paired delta", report)
        # The same prompts in both: the delta is the difference of the columns.
        second["items"]["p2"] = ok("noise-only", 1.0, 2)
        report = memory_eval.format_report([("A", Path("a.json"), first), ("B", Path("b.json"), second)])
        rows = {line[:42].strip(): line.split()[-1] for line in report.splitlines() if line[:1].strip()}
        self.assertEqual("-1", rows["useful turns"])
        self.assertNotIn("evaluated in both", report)
        self.assertNotIn("paired delta", report)
        # Nothing evaluated in both: there is no measure to compare.
        second["items"] = {key: {"status": "skipped", "reason": "refresh-error"} for key in ("p1", "p2")}
        report = memory_eval.format_report([("A", Path("a.json"), first), ("B", Path("b.json"), second)])
        rows = {line[:42].strip(): line.split()[-1] for line in report.splitlines() if line[:1].strip()}
        self.assertEqual(("-", "-", "-2"), (rows["useful turns"], rows["latency p50 (s)"], rows["evaluated"]))
        self.assertIn("0 evaluated in both, 2 only in A, 0 only in B", report)

    def test_stderr_tail_withholds_lines_quoting_the_prompt(self) -> None:
        prompt = "Please explain how the invoice totals are rounded for ACME exports"
        stderr = (
            b"Traceback (most recent call last):\n"
            b"ValueError: bad query 'how the invoice totals are rounded'\n"
            b"ok\n"
        )
        tail = memory_eval.stderr_tail(stderr, prompt)
        self.assertNotIn("invoice totals", tail)
        self.assertIn("[line withheld: it quotes the prompt]", tail)
        self.assertIn("Traceback", tail)

    def test_timestamps(self) -> None:
        parse = memory_eval.parse_instant
        self.assertEqual(parse("2026-08-05T12:00:00Z"), datetime(2026, 8, 5, 12, tzinfo=timezone.utc))
        self.assertEqual(
            parse("2026-08-05T14:00:00.1234567+0200"), datetime(2026, 8, 5, 12, 0, 0, 123456, tzinfo=timezone.utc)
        )
        self.assertEqual(parse("2026-08-05 12:00:00"), datetime(2026, 8, 5, 12, tzinfo=timezone.utc))
        self.assertIsNone(parse("2026-08-05"))
        self.assertIsNone(parse("yesterday"))


class MechanismCounterTest(unittest.TestCase):
    """Counters that show which mechanism delivered what, never a path."""

    def test_delivery_breakdown_counts_kinds_selections_marks_and_skill_subfiles(self) -> None:
        capsule = {
            "procedural": [{"path": ".claude/skills/x/references/a.md", "kind": "skill"}],
            "semantic": [
                {"path": "README.md", "kind": "doc", "selection": "prompt-link"},
                {"path": "project-brain/dynamic/findings/f.md", "kind": "brain-finding",
                 "source_changed": ["src/A.php"]},
                {"path": "docs/x.md", "kind": "Weird Kind!"},
            ],
            "episodic": [{"path": "CHANGELOG.md", "kind": "changelog"},
                         {"path": "project-brain/dynamic/events/e.md", "kind": "brain-event",
                          "selection": "source-link"}],
        }
        secret = "DB_" + "PASSWORD=" + "hunter22x"
        item = memory_eval.score({"capsule": capsule, "capsule_text": secret}, {}, {}, [],
                                 secret_patterns=memory_eval.runtime_policy().SECRET_PATTERNS)
        self.assertEqual(1, item["skill_subfiles_delivered"])
        self.assertEqual({"path-link": 0, "source-link": 1, "prompt-link": 1, "touch-link": 0},
                         item["selections"])
        self.assertEqual(1, item["source_changed_delivered"])
        self.assertEqual({"doc": 1, "brain-finding": 1, "other": 1}, item["layer_kinds"]["semantic"])
        self.assertEqual({"changelog": 1, "brain-event": 1}, item["layer_kinds"]["episodic"])
        self.assertEqual(1, item["capsule_secret_matches"])
        self.assertNotIn("src/A.php", json.dumps(item))
        empty = memory_eval.score({"capsule": None}, {}, {}, [])
        self.assertEqual(0, empty["skill_subfiles_delivered"])
        self.assertIsNone(empty["capsule_secret_matches"])

    def test_index_stats_reads_the_index_without_writing_it(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            corpus = Path(temporary)
            self.assertIsNone(memory_eval.index_stats(corpus))
            database = corpus / "memory-bank/local/context.db"
            database.parent.mkdir(parents=True)
            connection = sqlite3.connect(database)
            connection.execute("CREATE VIRTUAL TABLE documents USING fts5(path UNINDEXED, title, summary, content)")
            connection.execute("CREATE TABLE document_links(path, ref_path, ref_kind)")
            leaked = "MAIL_" + "PASSWORD=" + "mailpit7x"
            for row in (("a", "A", "", "clean"), ("b", "B", "", leaked),
                        ("c", "C", "", "DB_PASSWORD=[redacted: assigned credential]")):
                connection.execute("INSERT INTO documents VALUES (?, ?, ?, ?)", row)
            for row in (("a", "x", "source"), ("b", "y", "file"), ("c", "y", "file")):
                connection.execute("INSERT INTO document_links VALUES (?, ?, ?)", row)
            connection.commit()
            connection.close()
            before = (hashlib.sha256(database.read_bytes()).hexdigest(), database.stat().st_mtime_ns)
            self.assertEqual({"documents": 3, "links": {"file": 2, "source": 1}, "secret_documents": 1},
                             memory_eval.index_stats(corpus))
            self.assertEqual(before, (hashlib.sha256(database.read_bytes()).hexdigest(),
                                      database.stat().st_mtime_ns))
            database.write_bytes(b"not sqlite")
            self.assertIsNone(memory_eval.index_stats(corpus))

    def test_summary_counters_are_none_when_a_result_predates_them(self) -> None:
        new = memory_eval.score({"capsule": {"procedural": [], "semantic": [], "episodic": []}}, {}, {}, [],
                                secret_patterns={})
        new.update(status="ok", refresh={"index": {"documents": 3, "links": {"file": 2}, "secret_documents": 0},
                                         "index_seconds": 0.5})
        legacy = {"status": "ok", "class": "silent", "delivered": [], "useful_delivered": [],
                  "noise_delivered": [], "unjudged": [], "existed_useful": []}
        both = memory_eval.summarize({"new": new})
        self.assertEqual(0, both["skill_subfiles_delivered"])
        self.assertEqual({"file": 2}, both["index_link_rows"])
        self.assertEqual(1, both["index_snapshots_with_links"])
        self.assertEqual(0.5, both["index_latency_p95"])
        mixed = memory_eval.summarize({"new": new, "legacy": legacy})
        for key in ("skill_subfiles_delivered", "index_link_rows", "secret_documents", "episodic_brain_events",
                    "capsule_secret_matches"):
            self.assertIsNone(mixed[key], key)

    def test_knowledge_and_skill_routing_are_measured_apart(self) -> None:
        skill, other_skill, doc, noise_doc = (".agents/skills/coder/SKILL.md", ".agents/skills/sdd/SKILL.md",
                                              "docs/a.md", "docs/b.md")

        def item(delivered, useful, noise, existed):
            return {"status": "ok", "class": memory_eval.classify(delivered, useful, noise, False),
                    "delivered": delivered, "useful_delivered": useful, "noise_delivered": noise,
                    "unjudged": [path for path in delivered if path not in useful and path not in noise],
                    "existed_useful": existed}
        items = {
            # useful only because of the skill: knowledge noise-only, skill hit
            "a": item([skill, noise_doc], [skill], [noise_doc], [skill]),
            # the wrong skill, a useful document
            "b": item([other_skill, doc], [doc], [other_skill], [skill, doc]),
            # a useful skill existed, none delivered; nothing else
            "c": item([], [], [], [skill]),
            # no useful skill; a noise skill is the only delivery
            "d": item([other_skill], [], [other_skill], []),
            # no useful skill, none delivered, a useful document
            "e": item([doc], [doc], [], [doc]),
        }
        summary = memory_eval.summarize(items)
        self.assertEqual(3, summary["useful"])
        self.assertEqual((2, 1, 2), (summary["knowledge_useful"], summary["knowledge_noise_only"],
                                     summary["knowledge_silent"]))
        self.assertEqual(2, summary["knowledge_could_help"])
        self.assertEqual((2, 2, 1), (summary["knowledge_useful_paths_delivered"],
                                     summary["knowledge_useful_paths_existed"], summary["knowledge_noise_paths"]))
        self.assertEqual((3, 1, 1, 1), (summary["skill_with_useful"], summary["skill_hit"],
                                        summary["skill_wrong"], summary["skill_empty"]))
        self.assertEqual((2, 1, 1, 0), (summary["skill_without_useful"], summary["skill_noise"],
                                        summary["skill_silent"], summary["skill_unjudged"]))
        report = memory_eval.format_report([("A", Path("a.json"), {"meta": {}, "items": items,
                                                                  "summary": summary})])
        self.assertIn("knowledge: useful turns", report)
        self.assertIn("1/3", report)

    @unittest.skipIf(os.name == "nt", "POSIX modes")
    def test_results_are_written_owner_only(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "r.json"
            old = os.umask(0o022)
            try:
                memory_eval.write_json(target, {})
            finally:
                os.umask(old)
            self.assertEqual(0o600, stat.S_IMODE(target.stat().st_mode))
            self.assertEqual([target], list(Path(temporary).iterdir()))


class RefreshOutcomeTest(unittest.TestCase):
    """A prompt is scored only on a refresh that succeeded; the runtime itself is replaced."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.temporary = tempfile.TemporaryDirectory(prefix="memory-eval-refresh-")
        base = Path(cls.temporary.name)
        project = base / "projects" / "shop"
        project.mkdir(parents=True)
        git(project, "init", "-q")
        write(project, "README.md", "# Shop\n")
        write(project, "docs/orders.md", "# Orders\n\nOrders are confirmed by e-mail.\n")
        git(project, "add", "-A")
        git(project, "commit", "-q", "-m", "shop", when=T1)
        prompts = [{"id": "s1", "project": "shop", "ts": P1_TS, "prompt": "How are orders confirmed?"}]
        (base / "set.json").write_text(json.dumps(prompts), encoding="utf-8")
        (base / "judgments.json").write_text(json.dumps({"s1": {"docs/orders.md": 2}}), encoding="utf-8")
        arguments = memory_eval.build_parser().parse_args([
            "run", "--set", str(base / "set.json"), "--judgments", str(base / "judgments.json"),
            "--projects-root", str(base / "projects"), "--edition", "PHP Core",
            "--cache", str(base / "cache"), "--out", str(base / "result.json"),
        ])
        cls.stand = memory_eval.Run(arguments)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.temporary.cleanup()

    def evaluate(self, refresh: Dict[str, Any]) -> Dict[str, Any]:
        def runtime(corpus: Path, arguments: List[str], env: Dict[str, str], timeout: float) -> Dict[str, Any]:
            if arguments[0] == "index":
                return {"exit": 0, "seconds": 0.5, "stdout": b"", "stderr": b""}
            return {"seconds": 0.25, "stderr": b"", **refresh}

        with mock.patch.object(memory_eval, "run_runtime", runtime):
            return self.stand.evaluate(self.stand.prompts[0])

    @staticmethod
    def printed(capsule: Optional[Dict[str, Any]], layer_state: str = "updated") -> bytes:
        result = {layer: layer_state for layer in memory_eval.LAYERS}
        result.update(phases={"index": 0.1}, warnings=[], capsule=capsule)
        if capsule is not None:
            result["capsule_text"] = "- memory docs/orders.md - Orders"
        return json.dumps(result).encode("utf-8")

    def test_a_refresh_that_exits_non_zero_is_an_error_not_a_silent_turn(self) -> None:
        # The runtime prints its JSON and exits 1 when a memory layer failed to update.
        item = self.evaluate({"exit": 1, "stdout": self.printed(None, "failed"), "stderr": b"index refresh failed\n"})
        self.assertEqual(("skipped", "refresh-error"), (item["status"], item.get("reason")), item)
        self.assertNotIn("class", item)
        self.assertEqual(1, item["refresh"]["exit"])
        self.assertEqual({"procedural": "failed", "semantic": "failed", "episodic": "failed"}, item["refresh"]["layers"])
        self.assertIn("index refresh failed", item["refresh"]["stderr_tail"])
        # Even one that delivered: a failed layer means a stale index.
        capsule = {"semantic": [{"path": "docs/orders.md"}]}
        item = self.evaluate({"exit": 1, "stdout": self.printed(capsule, "failed")})
        self.assertEqual(("skipped", "refresh-error"), (item["status"], item.get("reason")))
        summary = memory_eval.summarize({"s1": item})
        self.assertEqual((0, {"refresh-error": 1}, None), (summary["evaluated"], summary["skipped"], summary["latency_p50"]))

    def test_a_timeout_is_one_whatever_it_printed(self) -> None:
        item = self.evaluate({"exit": None, "stdout": self.printed({"semantic": [{"path": "docs/orders.md"}]})})
        self.assertEqual(("skipped", "refresh-timeout"), (item["status"], item.get("reason")))

    def test_a_refresh_that_succeeded_is_scored(self) -> None:
        item = self.evaluate({"exit": 0, "stdout": self.printed({"semantic": [{"path": "docs/orders.md"}]})})
        self.assertEqual(("ok", "useful"), (item["status"], item["class"]), item)
        self.assertEqual({"procedural": "updated", "semantic": "updated", "episodic": "updated"}, item["refresh"]["layers"])
        item = self.evaluate({"exit": 0, "stdout": b"Traceback (most recent call last):\n"})
        self.assertEqual(("skipped", "refresh-error"), (item["status"], item.get("reason")))

    def test_layer_states_are_fixed_words_never_client_text(self) -> None:
        stdout = json.loads(self.printed(None))
        stdout.update(procedural="How are orders confirmed?", semantic=["x"], episodic="failed")
        item = self.evaluate({"exit": 1, "stdout": json.dumps(stdout).encode("utf-8")})
        self.assertEqual({"procedural": "other", "semantic": "other", "episodic": "failed"}, item["refresh"]["layers"])


class ReconstructionTest(unittest.TestCase):
    """reconstruct_memory without Git or a runtime."""

    def test_snapshots_never_read_private_blobs_or_copy_private_worktree_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            project = build_ledger(base)
            private = (".env", "config/.env.local", "credentials/token.json", "config/example.key")
            for relative in private:
                write(project, relative, "synthetic private sentinel")
            git(project, "add", ".")
            git(project, "commit", "-m", "Private sentinel paths", when=T2)
            ids = {subprocess.check_output(["git", "-C", str(project), "rev-parse", "HEAD:" + path]).strip()
                   for path in private}
            reads = []
            original = memory_eval.BlobReader.read

            def read(reader, blob):
                reads.append(blob)
                return original(reader, blob)

            tree = memory_eval.Repository(project).tree(memory_eval.Repository(project).head())
            with mock.patch.object(memory_eval.BlobReader, "read", read):
                snapshot = memory_eval.materialize(project, tree, base / "cache")
            self.assertTrue(ids.isdisjoint(reads), "private Git blobs must not be opened")
            self.assertTrue((snapshot / "docs/billing.md").is_file())
            self.assertTrue(all(not (snapshot / path).exists() for path in private))
            copied = []
            original_copy = memory_eval.shutil.copy2

            def copy(source, destination, *args, **kwargs):
                copied.append(Path(source).relative_to(project).as_posix())
                return original_copy(source, destination, *args, **kwargs)

            with mock.patch.object(memory_eval.shutil, "copy2", copy):
                current = memory_eval.snapshot_worktree(project, base / "current")
            self.assertTrue(set(private).isdisjoint(copied))
            self.assertTrue(all(not (current / path).exists() for path in private))
            self.assertTrue((current / "docs/billing.md").is_file())

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        base = Path(self.temporary.name)
        self.project, self.corpus = base / "project", base / "corpus"
        self.corpus.mkdir()
        self.moment = memory_eval.parse_instant("2026-08-05T12:00:00Z")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def same_day_chunk(self) -> Path:
        return write(self.project, SAME_DAY_CHUNK, chunk("MEM-20260805-5a5a5a5a", "X", "2026-08-05", "2026-12-31", "x"))

    def test_an_unmodified_file_existed_as_it_is(self) -> None:
        # Same day as the prompt, no promotion: undecidable - unless the file
        # has not been modified since before the prompt.
        path = self.same_day_chunk()
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual((counts["from_worktree"], counts["undetermined"]), (0, 1))
        earlier = datetime(2026, 8, 5, 9, tzinfo=timezone.utc).timestamp()
        os.utime(path, (earlier, earlier))
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual((counts["from_worktree"], counts["undetermined"]), (1, 0))
        self.assertTrue((self.corpus / SAME_DAY_CHUNK).is_file())

    def test_a_promotion_places_a_same_day_chunk(self) -> None:
        self.same_day_chunk()
        promotion = {
            "id": "promo",
            "destination_memory_id": "MEM-20260805-5a5a5a5a",
            "created_at": "2026-08-05T13:00:00Z",
            "updated_at": "2026-08-05T13:00:02Z",
        }
        write(self.project, f"{memory_eval.PROMOTIONS}/promo.json", json.dumps(promotion))
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual((counts["dropped_future"], counts["undetermined"]), (1, 0))
        promotion.update(created_at="2026-08-05T08:00:00Z", updated_at="2026-08-05T08:00:02Z")
        write(self.project, f"{memory_eval.PROMOTIONS}/promo.json", json.dumps(promotion))
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual(counts["from_worktree"], 1)

    def test_a_backdated_mtime_does_not_hide_an_edit_after_the_prompt(self) -> None:
        text = finding(RECORD_B, "B", "goal", "2026-08-03T08:00:00Z", "b", updated="2026-08-09T08:00:00Z")
        path = write(self.project, PATH_B, text)
        earlier = datetime(2026, 8, 1, tzinfo=timezone.utc).timestamp()
        os.utime(path, (earlier, earlier))
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual((counts["from_worktree"], counts["unreconstructable_updated"]), (0, 1))
        self.assertFalse((self.corpus / PATH_B).exists())

    def test_a_record_edited_after_the_prompt_is_left_out_unless_kept(self) -> None:
        # Its body today may hold the very answer the prompt's work produced.
        text = finding(RECORD_B, "B", "goal", "2026-08-03T08:00:00Z", "b", updated="2026-08-09T08:00:00Z")
        write(self.project, PATH_B, text)
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual((counts["from_worktree"], counts["unreconstructable_updated"]), (0, 1))
        self.assertFalse((self.corpus / PATH_B).exists())
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment, "keep")
        self.assertEqual((counts["from_worktree"], counts["updated_after"]), (1, 1))
        self.assertTrue((self.corpus / PATH_B).is_file())


    ARCHIVED = "project-brain/archive/finding/55555555-5555-4555-8555-555555555555.md"

    def archived(self, root: Path, created: str, updated: str = "") -> Path:
        text = finding("55555555-5555-4555-8555-555555555555", "E", "goal", created, "e", updated=updated)
        return write(root, self.ARCHIVED, text.replace('"status": "open"', '"status": "resolved"'))

    def test_an_archived_record_that_existed_then_is_restored_so_its_chunk_validates(self) -> None:
        self.archived(self.project, "2026-08-03T08:00:00Z", "2026-08-04T08:00:00Z")
        name = "memory-bank/chunks/MEM-20260804-55555555-export-rounding.md"
        text = chunk("MEM-20260804-55555555", "Export rounding", "2026-08-04", "2026-12-31", "Exports round.")
        write(self.project, name, text.replace('"sources": ["docs/billing.md"]',
                                               f'"sources": ["{self.ARCHIVED}", "docs/billing.md"]'))
        for root in (self.project, self.corpus):
            write(root, "docs/billing.md", "# Billing\n")
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual(1, counts["archive_from_worktree"])
        self.assertTrue((self.corpus / self.ARCHIVED).is_file())
        self.assertTrue((self.corpus / name).is_file())
        scripts = str(ROOT / "PHP Core/memory-bank/scripts")
        sys.path.insert(0, scripts)
        try:
            import validate as runtime_validate
            metadata = runtime_validate.parse_frontmatter(self.corpus / name)
            runtime_validate.validate_metadata(self.corpus / name, metadata, self.corpus)
        finally:
            sys.path.remove(scripts)

    def test_an_archived_record_resolved_after_the_prompt_is_left_out_unless_kept(self) -> None:
        self.archived(self.project, "2026-08-03T08:00:00Z", "2026-08-09T08:00:00Z")
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual(1, counts["archive_unreconstructable_updated"])
        self.assertFalse((self.corpus / self.ARCHIVED).exists())
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment, "keep")
        self.assertEqual((1, 1), (counts["archive_from_worktree"], counts["archive_updated_after"]))
        self.assertTrue((self.corpus / self.ARCHIVED).is_file())
        self.assertTrue(memory_eval.contaminated(counts))

    def test_an_archived_record_created_after_the_prompt_is_never_restored(self) -> None:
        self.archived(self.project, "2026-08-20T08:00:00Z")
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual(1, counts["archive_dropped_future"])
        self.assertFalse((self.corpus / self.ARCHIVED).exists())
        # A materialized commit's archive obeys the same clock.
        self.archived(self.corpus, "2026-08-20T08:00:00Z")
        earlier = "project-brain/archive/finding/66666666-6666-4666-8666-666666666666.md"
        write(self.corpus, earlier, finding("66666666-6666-4666-8666-666666666666", "F", "goal",
                                            "2026-08-01T08:00:00Z", "f"))
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual((1, 1), (counts["archive_dropped_future"], counts["archive_from_git"]))
        self.assertFalse((self.corpus / self.ARCHIVED).exists())
        self.assertTrue((self.corpus / earlier).is_file())

    def test_archived_handoffs_private_paths_and_records_still_active_are_not_restored(self) -> None:
        old = "2026-08-01T08:00:00Z"
        handoff = "project-brain/archive/handoffs/77777777-7777-4777-8777-777777777777.md"
        private = "project-brain/archive/secrets/88888888-8888-4888-8888-888888888888.md"
        write(self.project, handoff, finding("77777777-7777-4777-8777-777777777777", "H", "goal", old, "h"))
        write(self.project, private, finding("88888888-8888-4888-8888-888888888888", "S", "goal", old, "s"))
        self.archived(self.project, old)
        write(self.corpus, f"{FINDINGS}/55555555-5555-4555-8555-555555555555.md",
              finding("55555555-5555-4555-8555-555555555555", "E", "goal", old, "e"))
        counts = memory_eval.reconstruct_memory(self.project, self.corpus, self.moment)
        self.assertEqual(1, counts["archive_still_dynamic"])
        self.assertEqual(0, counts["archive_from_worktree"])
        for path in (handoff, private, self.ARCHIVED):
            self.assertFalse((self.corpus / path).exists(), path)


class OverlayTest(unittest.TestCase):
    """The runtime under test replaces the accelerator, never the project's state."""

    def test_rules(self) -> None:
        installer = memory_eval.installer
        with tempfile.TemporaryDirectory() as name:
            corpus = Path(name)
            old_policy = (ROOT / "Laravel" / "AGENTS.md").read_text(encoding="utf-8").splitlines()[0]
            write(corpus, "AGENTS.md", old_policy + "\n\nold rules\n")
            write(corpus, ".claude/CLAUDE.md", "# Team instructions\n\nRun the linters.\n")
            write(corpus, ".gitignore", "vendor/\n.env\n")
            write(corpus, "README.md", "# Ledger\n")
            write(corpus, "project-brain/config/runtime.json", '{"mode": "governed", "retrieval_gate": "enforce"}\n')
            write(corpus, "specs/MANIFEST.md", "# Our specs\n")
            write(corpus, "memory-bank/INDEX.md", "# Our index\n")
            write(corpus, PATH_A, "record\n")
            write(corpus, "memory-bank/local/context.db", "stale index\n")
            write(corpus, ".agents/skills/coder/SKILL.md", "stale\n")
            write(corpus, ".claude/skills/team-notes/SKILL.md", "ours\n")
            # A Laravel-only skill from an older install of another edition.
            laravel_only = sorted(
                set(installer.load_inventory("Laravel", ROOT)["installed"]["codex"])
                - set(installer.load_inventory("PHP Core", ROOT)["installed"]["codex"])
            )[0]
            write(corpus, laravel_only, "laravel\n")
            memory_eval.overlay(corpus, memory_eval.Edition("PHP Core"))

            def text(relative: str) -> str:
                return (corpus / relative).read_text(encoding="utf-8")

            core = ROOT / "PHP Core"
            self.assertEqual(text("AGENTS.md"), (core / "AGENTS.md").read_text(encoding="utf-8"))
            self.assertIn("Run the linters.", text(".claude/CLAUDE.md"))
            self.assertIn(installer.AGENTS_BEGIN, text(".claude/CLAUDE.md"))
            self.assertIn("vendor/", text(".gitignore"))
            self.assertIn("memory-bank/local/", text(".gitignore"))
            self.assertEqual(text("README.md"), "# Ledger\n")
            self.assertIn('"enforce"', text("project-brain/config/runtime.json"))
            self.assertEqual(text("specs/MANIFEST.md"), "# Our specs\n")
            self.assertEqual(text("memory-bank/INDEX.md"), "# Our index\n")
            self.assertEqual(text(PATH_A), "record\n")
            coder = ".agents/skills/coder/SKILL.md"
            self.assertEqual(text(coder), (core / coder).read_text(encoding="utf-8"))
            self.assertEqual(text(".claude/skills/team-notes/SKILL.md"), "ours\n")
            self.assertFalse((corpus / laravel_only).exists())
            self.assertFalse((corpus / "memory-bank/local/context.db").exists())
            self.assertTrue(os.access(corpus / ".claude/hooks/working-memory-read.sh", os.X_OK))
            self.assertTrue((corpus / "memory-bank/scripts/context.py").is_file())


# Transcript records, trimmed to the fields the parsers read.


def hook(stdout: str, timestamp: str, cwd: Optional[str] = None) -> Dict[str, Any]:
    attachment = {"type": "hook_success", "hookEvent": "UserPromptSubmit", "stdout": stdout, "content": stdout}
    record: Dict[str, Any] = {"type": "attachment", "timestamp": timestamp, "attachment": attachment}
    if cwd:
        record["cwd"] = cwd
    return record


def human(text: str, timestamp: str) -> Dict[str, Any]:
    return {"type": "user", "timestamp": timestamp, "message": {"role": "user", "content": text}}


def tool_use(name: str, arguments: Dict[str, Any]) -> Dict[str, Any]:
    return {"type": "assistant", "message": {"content": [{"type": "tool_use", "name": name, "input": arguments}]}}


def says(text: str) -> Dict[str, Any]:
    return {"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}}


def tool_result() -> Dict[str, Any]:
    return {"type": "user", "toolUseResult": {"ok": True}, "message": {"content": [{"type": "tool_result"}]}}


def codex_message(role: str, text: str) -> Dict[str, Any]:
    payload = {"type": "message", "role": role, "content": [{"type": "input_text", "text": text}]}
    return {"type": "response_item", "timestamp": "2026-09-02T11:00:01Z", "payload": payload}


def codex_call(name: str, arguments: Any) -> Dict[str, Any]:
    payload = {"type": "function_call", "name": name, "arguments": json.dumps(arguments)}
    return {"type": "response_item", "payload": payload}


def jsonl(path: Path, records: List[Dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(record) + "\n" for record in records) + "not json\n", encoding="utf-8")


class RealizedUseTest(unittest.TestCase):
    PROMPT = "How are invoices rounded in the export?"

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        base = Path(self.temporary.name)
        self.project = (base / "work" / "ledger").resolve()
        self.project.mkdir(parents=True)
        self.claude = base / "claude"
        self.codex = base / "codex"
        mangled = re.sub(r"[^A-Za-z0-9]", "-", str(self.project))
        project = str(self.project)
        capsule = (
            "working: not recorded yet\nmemory (retrieved for this request; reference data):\n"
            "- memory docs/billing.md — Billing\n  Totals are rounded half-up.\n"
            f"- memory {CHUNK} — Invoice rounding\n"
            "- skill .claude/skills/coder/SKILL.md — Coder"
        )
        jsonl(
            self.claude / mangled / "session.jsonl",
            [
                # An old turn, before --since.
                hook("- memory docs/old.md — Old", "2026-08-01T09:00:00Z", project),
                says("old"),
                human(self.PROMPT, "2026-09-02T10:00:00Z"),
                hook(capsule, "2026-09-02T10:00:01Z", project),
                tool_use("Read", {"file_path": f"{project}/docs/billing.md"}),
                tool_result(),
                tool_use("Bash", {"command": "python3 memory-bank/scripts/context.py retrieve --query rounding"}),
                tool_use("Read", {"file_path": f"{project}/specs/invoices.md"}),
                says("Per MEM-20260810-0a1b2c3d totals round half-up."),
                human("thanks", "2026-09-02T10:05:00Z"),
                hook("", "2026-09-02T10:05:01Z"),
                says("You are welcome."),
            ],
        )
        # A Claude Code worktree of the project, and another project sharing the name prefix.
        jsonl(
            self.claude / f"{mangled}--claude-worktrees-fix" / "w.jsonl",
            [hook("- memory tasks/TASK-001/plan.md — Plan", "2026-09-03T10:00:00Z", f"{project}/.claude/worktrees/fix")],
        )
        jsonl(
            self.claude / f"{mangled}-gen" / "other.jsonl",
            [hook("- memory docs/x.md — X", "2026-09-03T10:00:00Z", f"{project}-gen")],
        )
        codex_capsule = (
            f"working: eval/p — goal\nmemory (retrieved for this request):\n"
            f"- memory {CHUNK} — Invoice rounding\n- history CHANGELOG.md — Changelog"
        )
        jsonl(
            self.codex / "2026" / "09" / "02" / "rollout-1.jsonl",
            [
                {"timestamp": "2026-09-02T11:00:00Z", "type": "session_meta", "payload": {"id": "x", "cwd": project}},
                codex_message("user", "<environment_context>cwd</environment_context>"),
                codex_message("user", "Which rounding rule applies?"),
                codex_message("developer", codex_capsule),
                codex_call("shell", {"command": ["bash", "-lc", f"cat {project}/{CHUNK}"]}),
                codex_call("mcp__harness_memory__memory_retrieve", {"query": "rounding"}),
                codex_message("assistant", "The CHANGELOG.md entry agrees."),
            ],
        )
        jsonl(
            self.codex / "2026" / "09" / "02" / "rollout-2.jsonl",
            [
                {"type": "session_meta", "payload": {"id": "y", "cwd": "/somewhere/else"}},
                codex_message("developer", "working: x\n- memory docs/z.md — Z"),
            ],
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def arguments(self) -> List[str]:
        return [
            "realized", "--project", str(self.project),
            "--claude-root", str(self.claude), "--codex-root", str(self.codex),
        ]

    def realized(self, *extra: str) -> Dict[str, Any]:
        return json.loads(main_output(*self.arguments(), "--json", *extra))

    def test_claude_transcript(self) -> None:
        claude = self.realized("--since", "2026-09-01")["claude"]
        self.assertEqual((claude["transcripts"], claude["folders"], claude["folders_skipped"]), (2, 2, 1))
        turns = ("hook_turns", "turns_with_capsule", "turns_with_documents")
        self.assertEqual([claude[key] for key in turns], [3, 2, 2])
        items = ("delivered_items", "delivered_touched", "delivered_mentioned", "touched_not_delivered")
        self.assertEqual([claude[key] for key in items], [4, 1, 1, 1])
        calls = ("agent_retrieve_cli", "agent_context_cli", "agent_retrieve_mcp")
        self.assertEqual([claude[key] for key in calls], [1, 1, 0])
        self.assertEqual(claude["by_kind"]["chunk"], {"delivered": 1, "touched": 0, "mentioned": 1})
        self.assertEqual(claude["by_kind"]["spec-doc"], {"delivered": 1, "touched": 1, "mentioned": 0})
        self.assertEqual(claude["by_kind"]["skill"]["delivered"], 1)
        self.assertEqual(claude["by_kind"]["task"]["delivered"], 1)
        # Without --since the old turn counts too.
        self.assertEqual(self.realized()["claude"]["hook_turns"], 4)

    def test_codex_rollout(self) -> None:
        codex = self.realized()["codex"]
        self.assertEqual(codex["transcripts"], 1)
        keys = ("hook_turns", "turns_with_documents", "delivered_items", "delivered_touched", "delivered_mentioned")
        self.assertEqual([codex[key] for key in keys], [1, 1, 2, 1, 1])
        self.assertEqual(codex["agent_retrieve_mcp"], 1)
        self.assertEqual(codex["by_kind"]["changelog"], {"delivered": 1, "touched": 0, "mentioned": 1})

    def test_aggregates_only_unless_paths_are_asked_for(self) -> None:
        plain = main_output(*self.arguments())
        self.assertIn("turns with documents", plain)
        self.assertNotIn(self.PROMPT, plain)
        self.assertNotIn("docs/billing.md", plain)
        with_paths = self.realized("--paths")
        self.assertIn("docs/billing.md", with_paths["paths"])
        self.assertNotIn(self.PROMPT, json.dumps(with_paths))


if __name__ == "__main__":
    unittest.main()
