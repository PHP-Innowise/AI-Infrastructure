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
import copy
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
import memory_diagnosis  # noqa: E402

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
        self.assertIn('comparison: incompatible recorded metadata', compared)
        self.assertIn('meta.as_of', compared)
        self.assertIn('ID overlap: 2 prompt(s) evaluated in both', compared)
        self.assertNotIn('paired over', compared)
        useful = next(line for line in compared.splitlines() if line.startswith('useful turns'))
        self.assertEqual('-', useful.split()[-1])
        self.assertIn("pair(s) without a judgment", compared)
        for prompt_entry in SET:
            self.assertNotIn(prompt_entry["prompt"], compared)
        same = main_output('report', prompt, '--compare', prompt)
        self.assertIn('comparison: compatible recorded metadata', same)
        self.assertIn('paired over 2 prompt(s) evaluated in both', same)
        useful = next(line for line in same.splitlines() if line.startswith('useful turns'))
        self.assertEqual('+0', useful.split()[-1])

    def test_the_cache_holds_no_leftover_corpus(self) -> None:
        self.assertEqual(list((self.cache / "runs").iterdir()), [])
        self.assertTrue(any((self.cache / "public-trees-v1").iterdir()))
        self.assertTrue(any((self.cache / "history").iterdir()))


class ComparisonCompatibilityTest(unittest.TestCase):
    @staticmethod
    def document(kind='silent'):
        return {'meta': {'set_sha256':'a'*64,'judgments_sha256':'b'*64,'passages_sha256':None,
            'host':'codex','clock':'pinned','as_of':'prompt','updated_after':'drop','timeout':30},
            'items': {'p1': {'status':'ok','project':'public-project','ts':P1_TS,'commit':'d'*40,
                'edition':'PHP Core','edition_source':'forced','history_linked':True,'contaminated':False,
                'provenance': {'from_git':1,'from_worktree':0,'dropped_future':0,'undetermined':0,
                    'updated_after':0,'unreconstructable_updated':0,**NO_ARCHIVE},
                'class':kind,'could_help':True,'answer_in_text':kind=='useful',
                'delivered':['specs/public.md'] if kind=='useful' else [],
                'noise_delivered':[],'unjudged':[],'refresh':{'seconds':0.1}}}}

    def report(self, first, second):
        return memory_eval.format_report([('A',Path('a.json'),first),('B',Path('b.json'),second)])

    def useful_delta(self, report):
        return next(line.split()[-1] for line in report.splitlines() if line.startswith('useful turns'))

    def command(self, first, second):
        with tempfile.TemporaryDirectory(prefix='comparison-report-') as directory:
            a=Path(directory)/'a.json';b=Path(directory)/'b.json'
            a.write_text(json.dumps(first));b.write_text(json.dumps(second))
            return main_output('report',str(a),'--compare',str(b),'--show-unjudged')

    def test_ten_known_mismatches_withhold_numeric_quality_and_paired_gains(self):
        changes=[('meta','set_sha256','e'*64),('meta','judgments_sha256','e'*64),
            ('meta','passages_sha256','e'*64),('meta','host','cursor'),('meta','clock','real'),
            ('meta','as_of','now'),('meta','timeout',31),('item','commit','e'*40),
            ('item','ts',P2_TS),('item','edition','Symfony')]
        for scope,field,value in changes:
            with self.subTest(field=field):
                first=self.document();second=self.document('useful')
                target=second['meta'] if scope=='meta' else second['items']['p1']
                target[field]=value
                report=self.command(first,second)
                self.assertIn('comparison: incompatible',report)
                self.assertIn(field,report)
                self.assertLess(report.index('comparison: incompatible'),report.index('useful turns'))
                self.assertEqual('-',self.useful_delta(report))
                self.assertNotIn('paired over',report)
                self.assertNotRegex(report,r'useful\s+\+1\s+-0')
                self.assertIn('pair(s) without a judgment',report)

    def test_compatible_different_runtime_treatment_retains_gain(self):
        first=self.document();second=self.document('useful')
        first['meta']['clone_commit']='1'*40;second['meta']['clone_commit']='2'*40
        first['meta']['editions']={'PHP Core':{'digest':'1'*64,'dirty':False}}
        second['meta']['editions']={'PHP Core':{'digest':'2'*64,'dirty':True}}
        report=self.command(first,second)
        self.assertIn('comparison: compatible recorded metadata',report)
        self.assertEqual('+1',self.useful_delta(report))
        self.assertIn('paired over 1 prompt(s)',report)

    def test_legacy_unknown_remains_readable_diagnostic(self):
        first=self.document();second=self.document('useful')
        first['meta']={};second['meta']={}
        report=self.command(first,second)
        self.assertIn('comparison: unknown',report)
        self.assertIn('diagnostic',report)
        self.assertNotIn('paired over',report)
        self.assertEqual('+1',self.useful_delta(report))

    def test_missing_evidence_does_not_hide_a_known_mismatch(self):
        first=self.document();second=self.document('useful')
        first['meta'].pop('judgments_sha256');second['meta']['host']='cursor'
        report=self.command(first,second)
        self.assertIn('comparison: incompatible',report)
        self.assertEqual('-',self.useful_delta(report))

    def test_explicit_absence_is_distinct_from_missing_passage_metadata(self):
        first=self.document();second=self.document('useful')
        self.assertIn('comparison: compatible',self.report(first,second))
        second['meta'].pop('passages_sha256')
        self.assertIn('comparison: unknown',self.report(first,second))

    def test_typed_and_missing_source_evidence_is_unknown_without_traceback(self):
        mutations=[('meta',[]),('meta',True),('history_linked',1),('contaminated',0),
                   ('contaminated',True),('provenance',{'from_git':True}),('commit',None)]
        for field,value in mutations:
            with self.subTest(field=field,value=value):
                first=self.document();second=self.document('useful')
                if field=='meta':second['meta']=value
                else:second['items']['p1'][field]=value
                if field=='contaminated' and value is True:
                    first['items']['p1'][field]=True
                self.assertIn('comparison: unknown',self.command(first,second))

    def test_shared_success_diagnostics_keep_failure_coverage_visible(self):
        first=self.document('useful');second=copy.deepcopy(first)
        first['items']['p2']=copy.deepcopy(first['items']['p1'])
        second['items']['p2']={'status':'skipped','reason':'refresh-timeout'}
        report=self.report(first,second)
        self.assertIn('comparison: compatible',report)
        self.assertEqual('+0',self.useful_delta(report))
        rows={line[:42].strip():line.split()[-1] for line in report.splitlines() if line[:1].strip()}
        self.assertEqual('-1',rows['evaluated']);self.assertEqual('+1',rows['skipped'])

    def test_optional_scorer_and_worker_budgets_are_checked_when_declared(self):
        first=self.document();second=self.document('useful')
        for document in (first,second):
            document['meta']['runtime_under_test']={'scorer_sha256':'c'*64}
            document['meta']['onnx_excerpts']={'worker_timeout':5}
        self.assertIn('comparison: compatible',self.report(first,second))
        second['meta']['runtime_under_test']['scorer_sha256']='e'*64
        self.assertIn('comparison: incompatible',self.report(first,second))
        second['meta']['runtime_under_test']['scorer_sha256']='c'*64
        second['meta']['onnx_excerpts']['worker_timeout']=4
        self.assertIn('comparison: incompatible',self.report(first,second))
        second['meta'].pop('onnx_excerpts')
        self.assertIn('comparison: unknown',self.report(first,second))

    def test_utc_overflow_in_recorded_timestamp_is_unknown(self):
        for stamp in ('0001-01-01T00:00:00+01:00','9999-12-31T23:59:59-01:00'):
            with self.subTest(timestamp=stamp):
                first=self.document();second=self.document('useful')
                second['items']['p1']['ts']=stamp
                report=self.command(first,second)
                self.assertIn('comparison: unknown',report)
                self.assertNotIn('paired over',report)


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


class PassageInputTest(unittest.TestCase):
    def load(self, value: object) -> Dict[str, Dict[str, Dict[str, Any]]]:
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "passages.json"
            path.write_text(json.dumps(value), encoding="utf-8")
            return memory_eval.load_passages(path)

    def test_valid_labels_and_identical_skill_aliases_are_preserved(self) -> None:
        entry = {"useful": True, "passages": ["Exact source text."]}
        self.assertEqual({"p": {".agents/skills/x/SKILL.md": entry}}, self.load({"p": {
            ".claude/skills/x/SKILL.md": entry, ".agents/skills/x/SKILL.md": entry,
        }}))
        for label in ({"passages": []}, {"passages": ["Exact source text."]}, {"useful": False}):
            with self.subTest(label=label):
                self.assertEqual({"p": {"docs/a.md": label}}, self.load({"p": {"docs/a.md": label}}))
        self.assertEqual({}, memory_eval.load_passages(None))

    def test_malformed_labels_are_rejected_without_printing_their_text(self) -> None:
        private_text = "private-label-content"
        values = [None, [], True,
            {"p": []}, {"p": {"docs/a.md": []}},
            *({"p": {"docs/a.md": {"passages": value}}}
              for value in (private_text, None, 7, {private_text: True}, [private_text, 7])),
            *({"p": {"docs/a.md": {"useful": value, "passages": [private_text]}}}
              for value in ("false", 0, 1, None, [])),
        ]
        for value in values:
            with self.subTest(value=value):
                with self.assertRaises(memory_eval.EvalError) as caught:
                    self.load(value)
                self.assertNotIn(private_text, str(caught.exception))

    def test_conflicting_aliases_are_rejected_in_either_order(self) -> None:
        entries = [(".claude/skills/x/SKILL.md", {"useful": True, "passages": ["First."]}),
                   (".agents/skills/x/SKILL.md", {"useful": False, "passages": ["Second."]})]
        for ordered in (entries, list(reversed(entries))):
            with self.subTest(order=[key for key, _ in ordered]):
                with self.assertRaisesRegex(memory_eval.EvalError, "conflicting.*aliases"):
                    self.load({"p": dict(ordered)})

    def test_run_refuses_passage_string_before_creating_a_corpus(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            base = Path(name)
            (base / "set.json").write_text(json.dumps(SET[:1]), encoding="utf-8")
            (base / "judgments.json").write_text(json.dumps({"p1": {"docs/billing.md": 2}}), encoding="utf-8")
            (base / "passages.json").write_text(json.dumps({"p1": {"docs/billing.md": {
                "useful": True, "passages": "private-label-content",
            }}}), encoding="utf-8")
            process = run_cli(
                "run", "--set", str(base / "set.json"), "--judgments", str(base / "judgments.json"),
                "--passages", str(base / "passages.json"), "--projects-root", str(base),
                "--edition", "PHP Core", "--cache", str(base / "cache"), "--out", str(base / "result.json"),
            )
            self.assertEqual(2, process.returncode)
            self.assertNotIn("private-label-content", process.stderr)
            self.assertFalse((base / "cache").exists())
            self.assertFalse((base / "result.json").exists())


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

    def test_deliverable_ceiling_is_what_the_index_holds(self) -> None:
        # The runtime quotes the text its index holds, not the file: an answer
        # the index entry lacks is not a retrieval miss. An answer in a
        # document longer than the runtime marks whole stays reachable, but is
        # counted apart.
        with tempfile.TemporaryDirectory() as temporary:
            corpus = Path(temporary)
            write(corpus, "docs/a.md", "First answer. Hidden answer.")
            write(corpus, "docs/long.md", "Long answer.")
            database = corpus / "memory-bank/local/context.db"
            database.parent.mkdir(parents=True)
            with contextlib.closing(sqlite3.connect(database)) as connection:
                connection.execute("CREATE VIRTUAL TABLE documents USING fts5("
                                   "path UNINDEXED, layer UNINDEXED, kind UNINDEXED, title, summary, content)")
                connection.execute("INSERT INTO documents VALUES ('docs/a.md', 'semantic', 'doc', 'A', '', 'First answer.')")
                connection.execute("INSERT INTO documents VALUES ('docs/long.md', 'semantic', 'doc', 'L', '', ?)",
                                   ("Long answer. " + "x" * memory_eval.RUNTIME_MARKED_CHARACTERS,))
                connection.commit()
            passages = {"docs/a.md": {"passages": ["First answer.", "Hidden answer."]},
                        "docs/long.md": {"passages": ["Long answer."]}}
            paths = ["docs/a.md", "docs/long.md"]
            expected = memory_eval.existing_answer_passages(corpus, paths, passages)
            reachable, long_documents = memory_eval.indexed_answer_passages(corpus, paths, expected)
            self.assertEqual(3, len(expected))
            self.assertEqual({memory_eval.flatten("First answer."), memory_eval.flatten("Long answer.")}, reachable)
            self.assertEqual({memory_eval.flatten("Long answer.")}, long_documents)
            result = refresh_result({"semantic": paths})
            result["capsule_text"] = "First answer. Hidden answer."
            scored = memory_eval.score(result, {"docs/a.md": 2, "docs/long.md": 2}, passages, paths, paths,
                                       expected_passages=expected, deliverable_passages=reachable)
            self.assertEqual(3, scored["answer_passages_existed"])
            self.assertEqual(2, scored["answer_passages_delivered"])
            self.assertEqual(2, scored["answer_passages_deliverable"])
            self.assertEqual(1, scored["answer_passages_delivered_deliverable"])
            scored.update(status="ok", answer_passages_long_document=len(long_documents))
            summary = memory_eval.summarize({"one": scored})
            self.assertEqual(0.5, summary["deliverable_passage_recall"])
            self.assertEqual(1, summary["answer_passages_long_document"])
            self.assertEqual("50.0%", memory_eval.row_cell(summary, "deliverable_passage_recall"))

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

    def test_heading_only_labels_are_reported_but_not_answers(self) -> None:
        labels = {
            "docs/a.md": {"useful": True, "passages": ["# Changelog ## 1.0"]},
            "docs/b.md": {"useful": True, "passages": ["# Changelog\nThe body answer."]},
        }
        self.assertEqual("", memory_eval.passage_body("# Changelog ## 1.0"))
        self.assertEqual(
            {"the body answer."},
            memory_eval.labelled_passages(["docs/a.md", "docs/b.md"], labels),
        )
        self.assertEqual(
            {"docs/a.md": [], "docs/b.md": ["# Changelog\nThe body answer."]},
            memory_eval.audit_answer_passages({"docs/a.md": 1, "docs/b.md": 1}, labels),
        )

    def test_candidate_and_selected_recall_summary(self) -> None:
        audit = {
            "items": [
                {"path": "docs/a.md", "stage": "selected", "answer": "delivered"},
                {"path": "docs/b.md", "stage": "relevance-drop", "answer": "not-in-delivery"},
                {"path": "docs/c.md", "stage": "no-fts-candidate", "answer": "not-in-delivery"},
                {"path": "docs/d.md", "stage": "absent-source", "answer": "not-in-source"},
                {"path": "docs/e.md", "stage": "unknown", "answer": "unknown"},
                {"path": "docs/event.md", "stage": "unknown-history", "answer": "delivered"},
            ]
        }
        summary = memory_eval.summarize({
            "a": {
                "status": "ok", "class": "useful", "could_help": True,
                "answer_in_text": True, "delivered": ["docs/a.md"],
                "noise_delivered": [], "unjudged": [], "miss_audit": audit,
                "refresh": {"seconds": 0.1},
            },
            "b": {"status": "skipped", "reason": "no-history"},
        })
        result = summary["miss_audit"]
        self.assertEqual(6, result["known_paths"])
        self.assertEqual(2, result["unknown_paths"])
        self.assertEqual(3, result["eligible_paths"])
        self.assertEqual(0.6667, result["candidate_recall"])
        self.assertEqual(0.3333, result["selected_recall"])
        self.assertEqual(1, result["history_paths"])
        self.assertEqual(1, result["history_delivered"])
        self.assertEqual(1.0, result["history_recall"])
        self.assertEqual(1, result["stages"]["selected"])
        report = memory_eval.format_report([(
            "A", Path("a.json"),
            {"meta": {}, "items": {"a": {
                "status": "ok", "class": "useful", "could_help": True,
                "answer_in_text": True, "delivered": ["docs/a.md"],
                "noise_delivered": [], "unjudged": [], "miss_audit": audit,
                "refresh": {"seconds": 0.1},
            }}},
        )])
        self.assertIn("audit: candidate recall", report)
        self.assertIn("audit: selected recall", report)
        self.assertIn("audit: history recall", report)
        self.assertIn("retrieval loss", report)

    def test_history_event_limit_is_part_of_the_selected_denominator(self) -> None:
        audit = {
            "items": [
                {"path": "docs/a.md", "stage": "selected", "answer": "delivered"},
                {"path": "docs/event.md", "stage": "history-event-limit", "answer": "not-in-delivery"},
            ]
        }
        summary = memory_eval.summarize({
            "a": {
                "status": "ok", "class": "useful", "could_help": True,
                "answer_in_text": True, "delivered": ["docs/a.md"],
                "noise_delivered": [], "unjudged": [], "miss_audit": audit,
                "refresh": {"seconds": 0.1},
            }
        })
        result = summary["miss_audit"]
        self.assertEqual(2, result["eligible_paths"])
        self.assertEqual(1.0, result["candidate_recall"])
        self.assertEqual(0.5, result["selected_recall"])
        self.assertEqual(1, result["history_paths"])
        self.assertEqual(0, result["history_delivered"])
        self.assertEqual(0.0, result["history_recall"])

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


class NumericIntegrityTest(unittest.TestCase):
    def arguments(self, *extra):
        return memory_eval.build_parser().parse_args([
            'run', '--set', 'unused-set.json', '--judgments', 'unused-judgments.json',
            '--out', 'unused-result.json', *extra,
        ])

    @staticmethod
    def item(**refresh):
        return {'status': 'ok', 'class': 'silent', 'refresh': refresh}

    def test_invalid_cli_timeouts_reject_before_run_or_cache(self):
        for value in ('nan', 'inf', '-inf', '1e309', '0', '-1'):
            with self.subTest(value=value), mock.patch.object(memory_eval, 'Run') as run:
                errors = io.StringIO()
                with contextlib.redirect_stderr(errors):
                    result = memory_eval.main([
                        'run', '--set', 'unused-set.json', '--judgments', 'unused-judgments.json',
                        '--out', 'unused-result.json', '--timeout=' + value,
                    ])
                self.assertEqual(2, result)
                self.assertIn('--timeout must be finite and positive', errors.getvalue())
                run.assert_not_called()

    def test_finite_positive_timeouts_and_defaults_dispatch(self):
        for extra in ((), ('--timeout', '0.001'), ('--timeout', '1e2')):
            with self.subTest(extra=extra), mock.patch.object(memory_eval, 'Run') as run, \
                    mock.patch.object(memory_eval, 'warn_if_inside_clone'):
                run.return_value.execute.return_value = 7
                self.assertEqual(7, memory_eval.command_run(self.arguments(*extra)))
                run.assert_called_once()

    def test_direct_bool_overflow_and_onnx_timeouts_reject(self):
        for field in ('timeout', 'onnx_timeout'):
            for value in (True, False, 10 ** 400, float('nan'), float('inf'), -1, 0):
                arguments = self.arguments()
                setattr(arguments, field, value)
                with self.subTest(field=field, value_type=type(value)), \
                        mock.patch.object(memory_eval, 'Run') as run:
                    with self.assertRaises(memory_eval.EvalError):
                        memory_eval.command_run(arguments)
                    run.assert_not_called()

    def test_invalid_durations_reject_at_both_direct_aggregators(self):
        bad = (True, False, -0.1, float('nan'), float('inf'), -float('inf'), 10 ** 400, '0.2')
        for value in bad:
            with self.subTest(value_type=type(value)):
                with self.assertRaisesRegex(memory_eval.EvalError, 'refresh.seconds'):
                    memory_eval.summarize({'x': self.item(seconds=value)})
                with self.assertRaisesRegex(memory_eval.EvalError, 'refresh.index_seconds'):
                    memory_eval.mechanism_totals([self.item(index_seconds=value)])

    def test_missing_null_and_finite_zero_durations_keep_legacy_meaning(self):
        for refresh in ({}, {'seconds': None, 'index_seconds': None}):
            summary = memory_eval.summarize({'x': self.item(**refresh)})
            self.assertIsNone(summary['latency_p50'])
            self.assertIsNone(summary['index_latency_p95'])
        summary = memory_eval.summarize({
            'zero': self.item(seconds=0, index_seconds=-0.0),
            'positive': self.item(seconds=0.25, index_seconds=1),
            'legacy': {'status': 'ok', 'class': 'silent'},
        })
        self.assertEqual((0.0, 0.25), (summary['latency_p50'], summary['latency_p95']))
        self.assertEqual((0.0, 1.0), (summary['index_latency_p50'], summary['index_latency_p95']))

    def test_malformed_refresh_containers_raise_clean_errors_at_all_entry_points(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'invented.json'
            for value in ([1], 'invented metadata', True, 1, []):
                item = {'status': 'ok', 'class': 'silent', 'refresh': value}
                with self.subTest(value_type=type(value)):
                    with self.assertRaises(memory_eval.EvalError):
                        memory_eval.summarize({'x': item})
                    with self.assertRaises(memory_eval.EvalError):
                        memory_eval.mechanism_totals([item])
                    target.write_text(json.dumps({'items': {'x': item}}))
                    with self.assertRaises(memory_eval.EvalError):
                        memory_eval.load_result(target)

    def test_loaded_result_rejects_bad_json_timing_even_for_skipped_items(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'invented.json'
            for token in ('NaN', 'Infinity', '-Infinity', '1e309', 'true', '-0.1', str(10 ** 400)):
                for field in ('seconds', 'index_seconds'):
                    target.write_text('{"items":{"x":{"status":"skipped","refresh":{"' + field + '":' + token + '}}}}')
                    with self.subTest(token=token[:12], field=field), self.assertRaises(memory_eval.EvalError):
                        memory_eval.load_result(target)
                target.write_text('{"items":{"x":{"refresh":{"phases":{"index":' + token + '}}}}}')
                with self.subTest(token=token[:12], field='phases'), self.assertRaises(memory_eval.EvalError):
                    memory_eval.load_result(target)

    def test_known_persisted_budgets_reject_but_opaque_metadata_stays_compatible(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'invented.json'
            for meta in ({'timeout': float('inf')}, {'timeout': True}, {'timeout': 0},
                         {'onnx_excerpts': {'worker_timeout': float('nan')}}):
                target.write_text(json.dumps({'meta': meta, 'items': {}}))
                with self.subTest(meta_keys=list(meta)), self.assertRaises(memory_eval.EvalError):
                    memory_eval.load_result(target)
            document = {'meta': {'timeout': None, 'extra': float('nan'), 'integer': 10 ** 400,
                                 'timestamp': P1_TS, 'string': 'NaN'},
                        'items': {'x': self.item(seconds=0, index_seconds=None)}}
            target.write_text(json.dumps(document))
            loaded = memory_eval.load_result(target)
            self.assertEqual(10 ** 400, loaded['meta']['integer'])
            self.assertEqual(P1_TS, loaded['meta']['timestamp'])
            self.assertEqual('NaN', loaded['meta']['string'])
            self.assertEqual(0, memory_eval.summarize(loaded['items'])['latency_p50'])

    def test_optional_phases_keep_only_finite_nonnegative_numbers(self):
        phases = {'zero': 0, 'positive': 0.2, 'negative-zero': -0.0, 'bool': True,
                  'negative': -1, 'nan': float('nan'), 'inf': float('inf'),
                  'overflow': 10 ** 400, 'text': '0.3', 'missing': None}
        self.assertEqual({'zero': 0, 'positive': 0.2, 'negative-zero': -0.0},
                         memory_eval._phase_durations(phases))
        for value in (None, [], 'client sentinel', True, 1):
            self.assertEqual({}, memory_eval._phase_durations(value))

    def test_row_rendering_preserves_exact_integer_counts(self):
        number = 10 ** 400 + 7
        self.assertEqual(number, memory_eval.row_value({'prompts': number}, 'prompts'))
        self.assertEqual(str(number), memory_eval.row_cell({'prompts': number}, 'prompts'))
        self.assertEqual(number, memory_eval.row_value({'skipped': {'reason': number}}, 'skipped_total'))
        for value in (True, float('nan'), float('inf')):
            self.assertIsNone(memory_eval.row_value({'latency_p50': value}, 'latency_p50'))

    def test_invalid_export_is_rejected_before_creating_directories_or_files(self):
        cyclic = []
        cyclic.append(cyclic)
        invalid = (float('nan'), {'nested': float('inf')}, -float('inf'), cyclic,
                   {'unsupported': object()}, {'text': '\ud800'})
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary) / 'new-directory'
            target = parent / 'result.json'
            for document in invalid:
                with self.subTest(document_type=type(document)), self.assertRaisesRegex(
                        memory_eval.EvalError, 'finite UTF-8 JSON'):
                    memory_eval.write_json(target, document)
                self.assertFalse(parent.exists())

    def test_serialization_recursion_failure_is_a_clean_error_before_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary) / 'new-directory'
            with mock.patch.object(memory_eval.json, 'dumps', side_effect=RecursionError):
                with self.assertRaisesRegex(memory_eval.EvalError, 'finite UTF-8 JSON'):
                    memory_eval.write_json(parent / 'result.json', {})
            self.assertFalse(parent.exists())

    def test_invalid_export_preserves_existing_target_and_stale_temp(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            target = parent / 'result.json'
            target.write_bytes(b'original')
            stale = parent / ('.result.json.tmp-' + str(os.getpid()))
            stale.write_bytes(b'unrelated stale sentinel')
            with self.assertRaises(memory_eval.EvalError):
                memory_eval.write_json(target, {'duration': float('nan')})
            self.assertEqual(b'original', target.read_bytes())
            self.assertEqual(b'unrelated stale sentinel', stale.read_bytes())
            self.assertEqual({target, stale}, set(parent.iterdir()))

    def test_replace_failure_cleans_owned_temp_and_preserves_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            target = parent / 'result.json'
            target.write_bytes(b'original')
            with mock.patch.object(memory_eval.os, 'replace', side_effect=OSError('synthetic replace fault')):
                with self.assertRaisesRegex(memory_eval.EvalError, 'could not write result JSON'):
                    memory_eval.write_json(target, {'seconds': 0.5})
            self.assertEqual(b'original', target.read_bytes())
            self.assertEqual([target], list(parent.iterdir()))

    def test_successful_replace_releases_former_temp_path_ownership(self):
        replace = os.replace
        recreated = []

        def replace_and_recreate(source, target):
            replace(source, target)
            former = Path(source)
            former.write_bytes(b'unrelated pathname reuse sentinel')
            recreated.append(former)

        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'result.json'
            with mock.patch.object(memory_eval.os, 'replace', replace_and_recreate):
                memory_eval.write_json(target, {'seconds': 0.5})
            self.assertEqual({'seconds': 0.5}, json.loads(target.read_text()))
            self.assertEqual(1, len(recreated))
            self.assertEqual(b'unrelated pathname reuse sentinel', recreated[0].read_bytes())
            self.assertEqual({target, recreated[0]}, set(Path(temporary).iterdir()))

    def test_write_failure_closes_descriptor_and_cleans_temp(self):
        original = os.fdopen
        descriptors = []

        def failed_writer(descriptor, mode):
            descriptors.append(descriptor)
            real = original(descriptor, mode)

            class FailedWrite:
                def __enter__(self):
                    return self

                def write(self, _data):
                    raise OSError('synthetic write fault')

                def __exit__(self, *_args):
                    real.close()

            return FailedWrite()

        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'result.json'
            target.write_bytes(b'original')
            with mock.patch.object(memory_eval.os, 'fdopen', failed_writer):
                with self.assertRaisesRegex(memory_eval.EvalError, 'could not write result JSON'):
                    memory_eval.write_json(target, {'seconds': 0.5})
            self.assertEqual(b'original', target.read_bytes())
            self.assertEqual([target], list(Path(temporary).iterdir()))
            for descriptor in descriptors:
                with self.assertRaises(OSError):
                    os.fstat(descriptor)

    @unittest.skipIf(os.name == 'nt', 'POSIX modes')
    def test_finite_export_replaces_broad_mode_target_owner_only(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'result.json'
            target.write_text('old')
            target.chmod(0o666)
            old = os.umask(0)
            try:
                memory_eval.write_json(target, {'seconds': 0, 'text': 'Пример'})
            finally:
                os.umask(old)
            self.assertEqual(0o600, stat.S_IMODE(target.stat().st_mode))
            self.assertEqual({'seconds': 0, 'text': 'Пример'},
                             json.loads(target.read_text(), parse_constant=lambda value: self.fail(value)))
            self.assertTrue(target.read_bytes().endswith(b'\n'))
            self.assertEqual([target], list(Path(temporary).iterdir()))


class FunnelDiagnosisMetadataTest(unittest.TestCase):
    @staticmethod
    def observed(count=2):
        value = memory_diagnosis.unavailable_funnel_diagnosis()
        value['observation_state'] = 'observed'
        value['loss_observations']['pack'] = {'count': count, 'state': 'exact'}
        value['next_action'] = 'inspect-observed-loss' if count else 'unknown'
        return value

    def test_fixed_schema_agrees_with_checkout_canonical_producer(self):
        import ast
        selected_names = {'FUNNEL_COUNT_LIMIT', 'FUNNEL_BOUNDARIES', 'FUNNEL_RENDER_REASONS',
                          'FUNNEL_DIAGNOSIS_LOSSES', '_funnel_counter',
                          'unknown_funnel_diagnosis', 'valid_funnel_diagnosis'}
        tree = ast.parse((ROOT / 'PHP Core/memory-bank/scripts/context_retrieval.py').read_text())
        selected = []
        for node in tree.body:
            names = ({node.name} if isinstance(node, ast.FunctionDef) else
                     {item.id for item in node.targets if isinstance(item, ast.Name)}
                     if isinstance(node, ast.Assign) else set())
            if names & selected_names:
                selected.append(node)
        self.assertEqual(7, len(selected))
        namespace = {'Any': Any}
        exec(compile(ast.Module(body=selected, type_ignores=[]), '<canonical-fixed-schema>', 'exec'), namespace)
        self.assertEqual(namespace['unknown_funnel_diagnosis'](), memory_diagnosis.unavailable_funnel_diagnosis())
        for child in (self.observed(), self.observed(0), memory_diagnosis.unavailable_funnel_diagnosis()):
            self.assertTrue(namespace['valid_funnel_diagnosis'](child))
            self.assertEqual(child, memory_diagnosis.normalize_funnel_diagnosis(child))

    def test_rejected_shapes_do_not_retain_arbitrary_metadata(self):
        for count in (True, False, -1, 65536, 2.0, float('nan'), float('inf'), 'PRIVATE-SENTINEL'):
            child = self.observed()
            child['loss_observations']['pack']['count'] = count
            with self.subTest(kind=type(count).__name__):
                self.assertIsNone(memory_diagnosis.normalize_funnel_diagnosis(child))
        changes = ({'schema_version': True}, {'schema_version': 2}, {'query': 'PRIVATE-SENTINEL'},
                   {'next_action': 'unknown'}, {'coverage': {}}, {'emission': 'observed'})
        for change in changes:
            self.assertIsNone(memory_diagnosis.normalize_funnel_diagnosis({**self.observed(), **change}))
        for value in (None, [], {}, 'PRIVATE-SENTINEL'):
            self.assertIsNone(memory_diagnosis.normalize_funnel_diagnosis(value))

    def test_known_limits_unknowns_and_fresh_copy_are_preserved(self):
        value = self.observed(65535)
        clean = memory_diagnosis.normalize_funnel_diagnosis(value)
        self.assertEqual('exact', clean['loss_observations']['pack']['state'])
        self.assertEqual({'count': None, 'state': 'unknown'}, clean['boundaries']['pack_input'])
        clean['loss_observations']['pack']['count'] = 1
        self.assertEqual(65535, value['loss_observations']['pack']['count'])
        value['loss_observations']['pack']['state'] = 'lower-bound'
        self.assertEqual(value, memory_diagnosis.normalize_funnel_diagnosis(value))
        value['loss_observations']['pack']['count'] = 65534
        self.assertIsNone(memory_diagnosis.normalize_funnel_diagnosis(value))
        unavailable = memory_diagnosis.unavailable_funnel_diagnosis()
        unavailable['boundaries']['pack_input'] = {'count': 0, 'state': 'exact'}
        self.assertIsNone(memory_diagnosis.normalize_funnel_diagnosis(unavailable))


class PlannedDeliveryTest(unittest.TestCase):
    """Actual producer persistence, including failures outside successful pairs."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='memory-eval-planned-')
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        project = self.base / 'projects/shop'
        project.mkdir(parents=True)
        git(project, 'init', '-q')
        self.answers = ['Alpha orders use email.', 'Beta orders use post.',
                        'Gamma orders use courier.', 'Delta orders use pickup.']
        write(project, 'docs/orders.md', '# Orders\n\n'+'\n\n'.join(self.answers)+'\n')
        git(project, 'add', '-A')
        git(project, 'commit', '-q', '-m', 'public source', when=T1)
        prompts = [{'id': key, 'project': 'shop', 'ts': P1_TS, 'prompt': 'PRIVATE-PROMPT-'+key}
                   for key in ('p1', 'p2')]
        passages = {key: {'docs/orders.md': {'useful': True, 'passages': answers}}
                    for key, answers in (('p1', self.answers[:3]), ('p2', self.answers[3:]))}
        for name, value in (('set', prompts), ('judgments', {key: {'docs/orders.md': 2} for key in ('p1', 'p2')}),
                            ('passages', passages)):
            (self.base / (name+'.json')).write_text(json.dumps(value))
        self.out = self.base / 'result.json'
        args = memory_eval.build_parser().parse_args([
            'run', '--set', str(self.base/'set.json'), '--judgments', str(self.base/'judgments.json'),
            '--passages', str(self.base/'passages.json'), '--projects-root', str(self.base/'projects'),
            '--edition', 'PHP Core', '--cache', str(self.base/'cache'), '--out', str(self.out)])
        self.stand = memory_eval.Run(args)

    def saved(self):
        return json.loads(self.out.read_text())

    def runtime(self, corpus, arguments, env, timeout):
        if arguments[0] == 'index':
            saved = self.saved()
            key = 'p1' if corpus.name.startswith('p1-') else 'p2'
            unit = saved['planned']['units'][key]
            self.assertEqual('runtime-admitted', unit['state'])
            self.assertEqual(3 if key == 'p1' else 1, unit['source_opportunity']['answer_passages'])
            return {'exit': 0, 'seconds': 0.1, 'stdout': b'', 'stderr': b''}
        output = {'capsule': {'semantic': [{'path': 'docs/orders.md'}]},
                  'capsule_text': '\n'.join(self.answers)}
        return {'exit': 0, 'seconds': 0.2, 'stdout': json.dumps(output).encode(), 'stderr': b''}

    def execute(self, runtime=None):
        with mock.patch.object(memory_eval, 'run_runtime', runtime or self.runtime), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return self.stand.execute()

    def test_full_manifest_is_durable_before_first_evaluate(self):
        def first(prompt):
            saved = self.saved()
            self.assertEqual(['p1', 'p2'], saved['planned']['ids'])
            self.assertEqual({}, saved['items'])
            self.assertEqual('unattempted', saved['planned']['units']['p2']['state'])
            raise KeyboardInterrupt()
        with mock.patch.object(self.stand, 'evaluate', first), self.assertRaises(KeyboardInterrupt):
            self.execute()
        result = memory_eval.planned_delivery(self.saved())
        self.assertEqual(('incomplete', 2, 1, 2),
                         (result['status'], result['planned'], result['unattempted'], result['without_terminal']))
        self.assertIsNone(result['unit_delivery_rate'])

    def test_timeout_keeps_denominator_and_asymmetric_passage_ceiling(self):
        for failed, recall in (('p2', .75), ('p1', .25)):
            if failed == 'p1':
                # Each execute has its own Run/cache, like the public CLI.
                self.temporary.cleanup()
                self.setUp()
            def runtime(corpus, arguments, env, timeout):
                normal = self.runtime(corpus, arguments, env, timeout)
                if arguments[0] == 'refresh' and corpus.name.startswith(failed+'-'):
                    return {**normal, 'exit': None}
                return normal
            self.assertEqual(0, self.execute(runtime))
            saved = self.saved()
            result = memory_eval.planned_delivery(saved)
            self.assertEqual(('complete', 2, 2, 2, .5, recall),
                             (result['status'], result['planned'], result['terminal'], result['source_known'],
                              result['unit_delivery_rate'], result['passage_recall']))
            self.assertEqual(1, saved['summary']['evaluated'])
            self.assertEqual({'refresh-timeout': 1}, saved['summary']['skipped'])
            rendered = memory_eval.format_report([('A', self.out, saved)])
            self.assertIn('whole-planned A: complete', rendered)
            self.assertIn('unit delivery=50.0%', rendered)
            self.assertNotIn('PRIVATE-PROMPT', json.dumps(saved))
            self.assertNotIn('Alpha orders', json.dumps(saved))

    def test_interrupt_during_first_or_second_warm_is_not_unattempted(self):
        for failed, terminal, unattempted in (('p1', 0, 1), ('p2', 1, 0)):
            if failed == 'p2':
                self.temporary.cleanup()
                self.setUp()
            def runtime(corpus, arguments, env, timeout):
                normal = self.runtime(corpus, arguments, env, timeout)
                if arguments[0] == 'index' and corpus.name.startswith(failed+'-'):
                    raise KeyboardInterrupt()
                return normal
            with self.assertRaises(KeyboardInterrupt):
                self.execute(runtime)
            saved = self.saved()
            result = memory_eval.planned_delivery(saved)
            self.assertEqual(('incomplete', 2, terminal, unattempted, 2-terminal),
                             (result['status'], result['planned'], result['terminal'], result['unattempted'],
                              result['without_terminal']))
            self.assertEqual('interrupted', saved['planned']['units'][failed]['state'])
            self.assertTrue(saved['planned']['units'][failed]['runtime_admitted'])
            self.assertIsNone(result['passage_recall'])

    def test_source_opportunity_is_captured_once_before_warm(self):
        self.stand.passages['p1']['docs/orders.md']['passages'].append('Future answer only after warm.')
        original = memory_eval.existing_answer_passages
        observed = []
        def capture(corpus, answered, passages):
            observed.append(corpus.name)
            return original(corpus, answered, passages)
        def runtime(corpus, arguments, env, timeout):
            normal = self.runtime(corpus, arguments, env, timeout)
            if arguments[0] == 'index':
                self.assertEqual(len(observed), 1 if corpus.name.startswith('p1-') else 2)
                with (corpus/'docs/orders.md').open('a') as stream:
                    stream.write('\nFuture answer only after warm.\n')
            return normal
        with mock.patch.object(memory_eval, 'existing_answer_passages', capture):
            self.execute(runtime)
        self.assertEqual(2, len(observed))
        self.assertEqual(3, self.saved()['items']['p1']['answer_passages_existed'])
        self.assertEqual(1.0, memory_eval.planned_delivery(self.saved())['passage_recall'])

    def test_missing_source_and_legacy_are_unknown_not_zero(self):
        self.stand.prompts[1]['project'] = 'missing'
        self.execute()
        result = memory_eval.planned_delivery(self.saved())
        self.assertEqual(('incomplete', 1, 1), (result['status'], result['source_known'], result['source_excluded']))
        self.assertIsNone(result['passage_recall'])
        self.assertEqual('unknown', memory_eval.planned_delivery({'items': {}})['status'])

    def test_failed_opportunity_checkpoint_stops_before_runtime(self):
        original = memory_eval.write_json
        writes = []
        def checkpoint(target, document):
            writes.append(document['planned']['units']['p1']['state'])
            if len(writes) == 3:
                raise memory_eval.EvalError('injected checkpoint write failure')
            original(target, document)
        with mock.patch.object(memory_eval, 'write_json', checkpoint), mock.patch.object(memory_eval, 'run_runtime') as runtime:
            with self.assertRaises(memory_eval.EvalError):
                self.execute(runtime)
        runtime.assert_not_called()
        self.assertEqual(['unattempted', 'preparing', 'runtime-admitted'], writes)
        saved = self.saved()
        self.assertEqual('preparing', saved['planned']['units']['p1']['state'])
        self.assertEqual({}, saved['items'])
        self.assertEqual('incomplete', memory_eval.planned_delivery(saved)['status'])
        self.assertFalse(self.stand.work.exists())

    def test_invalid_unicode_plan_input_preserves_output_and_cleans_work(self):
        previous = {'meta': {}, 'items': {}, 'sentinel': 'previous result'}
        memory_eval.write_json(self.out, previous)
        prompts = json.loads((self.base/'set.json').read_text())
        prompts[0]['prompt'] = 'invalid-unicode-'+chr(0xD800)
        (self.base/'set.json').write_text(json.dumps(prompts))
        self.stand.prompts = memory_eval.load_set(self.base/'set.json')
        with mock.patch.object(memory_eval, 'run_runtime') as runtime, self.assertRaises(memory_eval.EvalError):
            self.execute(runtime)
        runtime.assert_not_called()
        self.assertEqual(previous, self.saved())
        self.assertFalse(self.stand.work.exists())

    def test_known_zero_opportunity_is_complete_without_fabricated_rate(self):
        self.stand.judgments = {}
        self.stand.passages = {}
        def runtime(corpus, arguments, env, timeout):
            return {'exit': 0, 'seconds': .1, 'stderr': b'',
                    'stdout': b'' if arguments[0] == 'index' else b'{"capsule": null}'}
        self.execute(runtime)
        result = memory_eval.planned_delivery(self.saved())
        self.assertEqual(('complete', 2, 0, 0),
                         (result['status'], result['source_known'], result['useful_eligible'], result['answer_passages_existed']))
        self.assertIsNone(result['unit_delivery_rate'])
        self.assertIsNone(result['passage_recall'])

    def test_bad_bindings_states_and_counts_cannot_publish_rates(self):
        self.execute()
        baseline = self.saved()
        mutations = [lambda doc: doc['planned']['ids'].append('missing'),
                     lambda doc: doc['planned']['ids'].append('p1'),
                     lambda doc: doc['items']['p1'].update(input_sha256='0'*64),
                     lambda doc: doc['planned']['units']['p1'].update(state='unattempted'),
                     lambda doc: doc['items'].update(extra=copy.deepcopy(doc['items']['p1'])),
                     lambda doc: doc['items']['p1'].update(answer_passages_delivered=4)]
        for value in (True, -1, 10**400, float('nan'), 'PRIVATE-SENTINEL'):
            mutations.append(lambda doc, value=value: doc['items']['p1'].update(answer_passages_delivered=value))
        def copied(doc):
            doc['planned']['units']['p2'] = copy.deepcopy(doc['planned']['units']['p1'])
            doc['items']['p2'] = copy.deepcopy(doc['items']['p1'])
        def swapped(doc):
            for mapping in (doc['planned']['units'], doc['items']):
                mapping['p1'], mapping['p2'] = mapping['p2'], mapping['p1']
        def blank_id(doc):
            doc['planned']['ids'][0] = ' '
            doc['planned']['units'][' '] = doc['planned']['units'].pop('p1')
            doc['items'][' '] = doc['items'].pop('p1')
        mutations.extend((copied, swapped, blank_id))
        for path in ('', '../outside.md', '/absolute.md', './docs/orders.md'):
            def bad_path(doc, path=path):
                for field in ('useful_delivered', 'existed_useful', 'answer_existed'):
                    doc['items']['p1'][field] = [path]
            mutations.append(bad_path)
        for value in (True, -1, 10**400, float('nan'), 'PRIVATE-SENTINEL'):
            mutations.append(lambda doc, value=value: doc['planned']['units']['p1']['source_opportunity'].update(answer_passages=value))
        for mutate in mutations:
            doc = copy.deepcopy(baseline)
            mutate(doc)
            result = memory_eval.planned_delivery(doc)
            self.assertEqual('unknown', result['status'])
            self.assertIsNone(result['unit_delivery_rate'])
            rendered = memory_eval.format_report([('A', self.out, doc)])
            self.assertNotIn('PRIVATE-SENTINEL', rendered)


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

    def test_successful_runtime_bad_optional_phases_do_not_change_quality(self):
        output = json.loads(self.printed({'semantic': [{'path': 'docs/orders.md'}]}))
        output['phases'] = {'zero': 0, 'index': 0.1, 'bool': True, 'negative': -1,
                            'nan': float('nan'), 'inf': float('inf'), 'huge': 10 ** 400}
        item = self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
        self.assertEqual(('ok', 'useful'), (item['status'], item['class']))
        self.assertEqual({'zero': 0, 'index': 0.1}, item['refresh']['phases'])
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'invented.json'
            memory_eval.write_json(target, {'items': {'x': item}})
            json.loads(target.read_text(), parse_constant=lambda value: self.fail(value))

    def test_malformed_optional_phase_container_is_ignored(self):
        output = json.loads(self.printed({'semantic': [{'path': 'docs/orders.md'}]}))
        output['phases'] = ['invented metadata']
        item = self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
        self.assertEqual(('ok', 'useful', {}), (item['status'], item['class'], item['refresh']['phases']))

    def test_valid_diagnosis_is_json_metadata_without_scoring_or_report_change(self):
        output = json.loads(self.printed({'semantic': [{'path': 'docs/orders.md'}]}))
        with mock.patch.object(memory_eval, 'miss_audit', side_effect=AssertionError('unexpected audit')):
            baseline = self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
            output['funnel_diagnosis'] = FunnelDiagnosisMetadataTest.observed()
            observed = self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
        self.assertEqual(output['funnel_diagnosis'], observed['refresh']['funnel_diagnosis'])
        self.assertEqual(memory_eval.summarize({'s1': baseline}), memory_eval.summarize({'s1': observed}))
        def report(item):
            return memory_eval.format_report([('fixture', Path('result.json'), {'items': {'s1': item}, 'meta': {}})])
        self.assertEqual(report(baseline), report(observed))
        observed['refresh'].pop('funnel_diagnosis')
        self.assertEqual(baseline, observed)

    def test_missing_bad_and_unavailable_diagnosis_preserve_usable_refresh(self):
        output = json.loads(self.printed({'semantic': [{'path': 'docs/orders.md'}]}))
        for value in (None, {'query': 'PRIVATE-SENTINEL'}, memory_diagnosis.unavailable_funnel_diagnosis()):
            with self.subTest(kind=type(value).__name__):
                output['funnel_diagnosis'] = value
                item = self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
                self.assertEqual(('ok', 'useful'), (item['status'], item['class']))
                self.assertEqual(value is not None and 'schema_version' in value,
                                 'funnel_diagnosis' in item['refresh'])
                self.assertNotIn('PRIVATE-SENTINEL', json.dumps(item))

    def test_diagnosis_failure_preserves_evaluation_and_cancellation_propagates(self):
        output = json.loads(self.printed({'semantic': [{'path': 'docs/orders.md'}]}))
        baseline = self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
        output['funnel_diagnosis'] = FunnelDiagnosisMetadataTest.observed()
        with mock.patch.object(memory_eval, 'normalize_funnel_diagnosis', side_effect=RuntimeError('metadata-only failure')):
            item = self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
        self.assertEqual(baseline, item)
        for error in (KeyboardInterrupt('cancelled'), SystemExit(7)):
            with mock.patch.object(memory_eval, 'normalize_funnel_diagnosis', side_effect=error):
                with self.assertRaises(type(error)) as caught:
                    self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
            self.assertIs(error, caught.exception)


    def test_receipt_normalizer_matches_core_schema_and_rebuilds_without_aliases(self):
        import ast
        names = {'normalize_ordinary_receipt', 'unknown_ordinary_receipt'}
        tree = ast.parse((ROOT / 'PHP Core/memory-bank/scripts/context_retrieval.py').read_text())
        selected = [node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name in names]
        namespace = {'Any': Any, 'Optional': Optional}
        exec(compile(ast.Module(body=selected, type_ignores=[]), '<canonical-receipt-schema>', 'exec'), namespace)
        unavailable = memory_diagnosis.unavailable_ordinary_receipt()
        self.assertEqual(namespace['unknown_ordinary_receipt'](), unavailable)
        valid = [unavailable] + [{'schema_version': 1, 'observation_state': 'observed',
                                  'plans_present': plans, 'returned_status': status}
                                 for plans, status in ((False, 'skipped'), (True, 'skipped'), (True, 'deferred'), (True, 'committed'))]
        class String(str): pass
        class Integer(int): pass
        class Mapping(dict): pass
        class Hostile:
            def __eq__(self, other): raise AssertionError('equality hook')
            def __hash__(self): raise AssertionError('hash hook')
            def __bool__(self): raise AssertionError('truth hook')
        bad = [None, [], Mapping(valid[-1]), {**valid[-1], 'body': 'PRIVATE-SENTINEL'},
               {**valid[-1], 'schema_version': True}, {**valid[-1], 'schema_version': Integer(1)},
               {**valid[-1], 'returned_status': String('committed')},
               {**valid[-1], 'observation_state': Hostile()}, {**valid[-1], 'plans_present': Hostile()},
               {**valid[-1], 'plans_present': False}, {**unavailable, 'plans_present': False},
               {String('schema_version'): 1, **{key: value for key, value in valid[-1].items() if key != 'schema_version'}}]
        for value in valid + bad:
            # Do not compare hostile input to a reference table.
            expected = value if any(value is item for item in valid) else None
            self.assertEqual(expected, namespace['normalize_ordinary_receipt'](value))
            self.assertEqual(expected, memory_diagnosis.normalize_ordinary_receipt(value))
            if expected is not None:
                self.assertIsNot(value, memory_diagnosis.normalize_ordinary_receipt(value))
        self.assertLessEqual(len(json.dumps(valid[-1], separators=(',', ':')).encode()), 128)

    def test_receipt_refresh_metadata_does_not_change_evaluation(self):
        output = json.loads(self.printed({'semantic': [{'path': 'docs/orders.md'}]}))
        baseline = self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
        receipt = {'schema_version': 1, 'observation_state': 'observed', 'plans_present': True, 'returned_status': 'committed'}
        with mock.patch.object(memory_eval, 'miss_audit', side_effect=AssertionError('unexpected audit')):
            for funnel in (None, {'body': 'PRIVATE'}, FunnelDiagnosisMetadataTest.observed()):
                output.update(ordinary_receipt=receipt, funnel_diagnosis=funnel)
                item = self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
                self.assertEqual(receipt, item['refresh']['ordinary_receipt'])
                self.assertEqual(memory_eval.summarize({'s1': baseline}), memory_eval.summarize({'s1': item}))
                report = lambda value: memory_eval.format_report([('fixture', Path('result.json'), {'items': {'s1': value}, 'meta': {}})])
                self.assertEqual(report(baseline), report(item))
                item['refresh'].pop('ordinary_receipt')
                item['refresh'].pop('funnel_diagnosis', None)
                self.assertEqual(baseline, item)
        failed = self.evaluate({'exit': 1, 'stdout': json.dumps(output).encode()})
        self.assertEqual(('skipped', 'refresh-error'), (failed['status'], failed['reason']))
        self.assertNotIn('class', failed)
        self.assertEqual(receipt, failed['refresh']['ordinary_receipt'])

    def test_receipt_normalizer_failure_and_cancellation_are_metadata_only(self):
        output = json.loads(self.printed({'semantic': [{'path': 'docs/orders.md'}]}))
        output.update(ordinary_receipt=memory_diagnosis.unavailable_ordinary_receipt(),
                      funnel_diagnosis=FunnelDiagnosisMetadataTest.observed())
        for broken, retained in (('normalize_ordinary_receipt', 'funnel_diagnosis'),
                                 ('normalize_funnel_diagnosis', 'ordinary_receipt')):
            with mock.patch.object(memory_eval, broken, side_effect=RuntimeError('metadata-only failure')):
                item = self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
            self.assertIn(retained, item['refresh'])
            self.assertEqual(('ok', 'useful'), (item['status'], item['class']))
        for error in (KeyboardInterrupt('cancelled'), SystemExit(7)):
            with mock.patch.object(memory_eval, 'normalize_ordinary_receipt', side_effect=error), self.assertRaises(type(error)) as caught:
                self.evaluate({'exit': 0, 'stdout': json.dumps(output).encode()})
            self.assertIs(error, caught.exception)


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
        local = self.project / "memory-bank" / "local"

        def step(at: str, host: Optional[str], stage: str, event: str, **fields: Any) -> Dict[str, Any]:
            return {"at": "2026-09-02T" + at + "Z", "host": host, "stage": stage, "event": event, **fields}

        jsonl(
            local / "native-results.ndjson",
            [
                # Before --since.
                {"at": "2026-08-01T09:00:00Z", "host": "claude", "stage": "prompt", "event": "armed"},
                step("10:00:01", "claude", "prompt", "armed"),
                step("10:01:00", "claude", "stop", "requested"),
                step("10:01:00", "claude", "stop-hook", "exited", status=0),
                step("10:02:00", "claude", "save", "refused", code="memory-result-v1:unverified"),
                step("10:02:30", "claude", "save", "saved", learnings=2, promoted=1),
                step("10:03:00", "claude", "stop", "skipped", code="recovery-saved"),
                step("10:03:00", "claude", "stop-hook", "exited", status=0),
                step("10:05:01", "claude", "prompt", "armed", code="previous-failed"),
                # A code is a fixed word; anything else is counted, never printed.
                step("10:06:00", "claude", "prompt", "skipped", code=self.PROMPT),
                step("11:00:00", "codex", "prompt", "armed"),
                step("11:01:00", "codex", "stop", "requested"),
                step("11:01:00", "codex", "stop-hook", "exited", status=124),
                step("11:01:30", "codex", "prompt", "continuation"),
                step("11:02:00", "codex", "stop", "failed", code="unsaved"),
                step("11:03:00", "codex", "save", "saved", code="late", learnings=1, promoted=0),
                step("11:04:00", "codex", "save", "saved", code="repeat", learnings=0, promoted=0),
                step("11:05:00", "cursor", "stop-hook", "skipped", code="capsule-delivered"),
                step("11:06:00", None, "save", "refused", code="native-result:invalid-token"),
            ],
        )
        jsonl(
            local / "refresh-health.ndjson",
            [
                {"at": "2026-08-01T09:00:00Z", "hook_status": 0, "source": "hook"},
                {"at": "2026-09-02T10:00:00Z", "hook_status": 0, "source": "hook"},
                {"at": "2026-09-02T11:00:00Z", "hook_status": 124, "source": "hook"},
                {"at": "2026-09-02T11:30:00Z", "source": "cli"},
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

    NATIVE = ("prompt_hook_runs", "armed", "armed_after_incomplete", "continuations",
              "stop_hook_runs", "stop_hook_timeouts", "stop_hook_failures", "requested",
              "saved", "held", "omitted", "failed", "refused", "errors",
              "learnings_written", "chunks_promoted")

    def test_native_result_funnel_per_host(self) -> None:
        native = self.realized("--since", "2026-09-01")["native"]
        self.assertEqual((native["journal"], native["lines"], native["prompt_hook_health_lines"]), (True, 18, 2))
        hosts = native["hosts"]
        expected = {
            "claude": [3, 2, 1, 0, 2, 0, 0, 1, 1, 0, 0, 0, 1, 0, 2, 1],
            "codex": [2, 1, 0, 1, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0],
            "cursor": [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            "unknown": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0],
        }
        for host, counts in expected.items():
            with self.subTest(host=host):
                self.assertEqual(counts, [hosts[host][key] for key in self.NATIVE])
        self.assertEqual([5, 3, 1, 1, 4, 1, 0, 2, 2, 0, 0, 1, 2, 0, 3, 1],
                         [native["total"][key] for key in self.NATIVE])
        self.assertEqual(hosts["claude"]["reasons"], {
            "prompt armed": {"previous-failed": 1}, "prompt skipped": {"other": 1},
            "save refused": {"memory-result-v1:unverified": 1}, "stop skipped": {"recovery-saved": 1},
            "stop-hook exited": {"0": 2}})
        self.assertEqual(hosts["codex"]["reasons"], {
            "save saved": {"late": 1, "repeat": 1}, "stop failed": {"unsaved": 1},
            "stop-hook exited": {"124": 1}})
        self.assertEqual(native["total"]["reasons"]["save refused"],
                         {"memory-result-v1:unverified": 1, "native-result:invalid-token": 1})
        everything = self.realized()["native"]
        self.assertEqual((everything["lines"], everything["prompt_hook_health_lines"]), (19, 3))

    def test_native_result_funnel_prints_counts_and_codes_only(self) -> None:
        plain = main_output(*self.arguments(), "--since", "2026-09-01")
        for row in ("prompt hook runs", "save requested", "saved", "refused saves",
                    "learnings written", "chunks promoted", "memory-result-v1:unverified", "capsule-delivered"):
            self.assertIn(row, plain)
        self.assertNotIn(self.PROMPT, plain)
        (self.project / "memory-bank" / "local" / "native-results.ndjson").unlink()
        missing = main_output(*self.arguments())
        self.assertIn("no native-results.ndjson", missing)
        self.assertFalse(self.realized()["native"]["journal"])



class OptionalRuntimeDeadlineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="optional-runtime-deadline-")
        self.addCleanup(self.temporary.cleanup)
        self.folder = Path(self.temporary.name)
        self.corpus = self.folder / "corpus"
        self.corpus.mkdir()
        self.driver = self.folder / "driver"
        (self.driver / "scripts").mkdir(parents=True)
        self.pilot_ref = self.driver / "scripts/memory_onnx_pilot.py"
        self.env = {name: os.environ[name] for name in memory_eval.ENV_PASSTHROUGH if name in os.environ}

    def invoke(self, source, budget=.5):
        self.pilot_ref.write_text(source, encoding="utf-8")
        with mock.patch.object(memory_eval, "ROOT", self.driver):
            return memory_eval.run_runtime(self.corpus, ["refresh", "--json"], self.env, budget, ["--stats", "unused.json"])

    def test_parent_deadline_and_configured_timeout_reach_optional_main(self):
        import time
        started = time.monotonic()
        result = self.invoke("import json,sys\nprint(json.dumps({'args':sys.argv[1:]}))\n", budget=3)
        self.assertEqual(0, result["exit"])
        arguments = json.loads(result["stdout"])["args"]
        deadline = float(arguments[arguments.index("--invocation-deadline") + 1])
        self.assertGreaterEqual(deadline, started + 2.8)
        self.assertLess(deadline, time.monotonic() + 2.8)
        self.assertEqual("3", arguments[arguments.index("--invocation-timeout") + 1])
        self.assertEqual(["refresh", "--json"], arguments[arguments.index("--") + 1:])

    def test_cooperative_timeout_is_not_a_successful_partial_json(self):
        source = "import json,signal,time\nsignal.signal(signal.SIGTERM, lambda *_: exit(143))\nprint(json.dumps({'partial':True}),flush=True)\ntime.sleep(30)\n"
        result = self.invoke(source)
        self.assertIsNone(result["exit"])
        self.assertLess(result["seconds"], .8)
        self.assertTrue(json.loads(result["stdout"])["partial"])

    @unittest.skipUnless(os.name == "posix", "POSIX TERM escalation")
    def test_ignored_term_escalates_within_remaining_parent_budget(self):
        source = "import json,signal,time\nsignal.signal(signal.SIGTERM,signal.SIG_IGN)\nprint(json.dumps({'partial':True}),flush=True)\ntime.sleep(30)\n"
        result = self.invoke(source)
        self.assertIsNone(result["exit"])
        self.assertLess(result["seconds"], .8)
        self.assertTrue(json.loads(result["stdout"])["partial"])

    def test_nonpilot_command_remains_direct(self):
        folder = self.corpus / "memory-bank/scripts"
        folder.mkdir(parents=True)
        (folder / "context.py").write_text("import json,sys\nprint(json.dumps(sys.argv[1:]))\n")
        result = memory_eval.run_runtime(self.corpus, ["refresh", "--json"], self.env, 1)
        self.assertEqual(0, result["exit"])
        self.assertEqual(["refresh", "--json"], json.loads(result["stdout"]))


class SaveContinuationCostTest(unittest.TestCase):
    """What the Stop hook's save request after every answer costs, read from
    the transcripts: the journal counts requests, only a transcript has the
    time and the tokens."""

    TOKEN = "INTERNAL_NATIVE_MEMORY_RESULT 0123456789abcdef0123456789abcdef"

    def write_lines(self, records: List[Dict[str, Any]]) -> Path:
        directory = tempfile.TemporaryDirectory(prefix="continuation-cost-")
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "session.jsonl"
        path.write_text("".join(json.dumps(record) + "\n" for record in records), encoding="utf-8")
        return path

    @staticmethod
    def at(seconds: int) -> str:
        return f"2026-10-23T10:{seconds // 60:02d}:{seconds % 60:02d}.000Z"

    def test_a_codex_request_costs_the_time_and_tokens_to_the_next_prompt(self) -> None:
        def message(seconds: int, role: str, text: str) -> Dict[str, Any]:
            return {"timestamp": self.at(seconds), "type": "response_item",
                    "payload": {"type": "message", "role": role, "content": [{"type": "input_text", "text": text}]}}

        def tokens(seconds: int, output: int) -> Dict[str, Any]:
            return {"timestamp": self.at(seconds), "type": "event_msg",
                    "payload": {"type": "token_count", "info": {"total_token_usage": {"output_tokens": output}}}}

        path = self.write_lines([
            tokens(0, 100), message(1, "user", "Where is the report cache cleared?"),
            message(15, "assistant", "In ReportService."), tokens(20, 700),
            message(21, "user", self.TOKEN + "\nThis is an internal continuation"),
            message(25, "assistant", "Saved; the cache is cleared in ReportService."), tokens(31, 1100),
            message(40, "user", "Next question"), message(45, "assistant", "Mentions " + self.TOKEN),
            tokens(46, 1300),
        ])
        [cost] = memory_eval.continuation_costs(path, "codex")
        self.assertEqual({"answer_seconds": 20.0, "answer_output_tokens": 600, "ending": "earlier",
                          "save_seconds": 10.0, "save_output_tokens": 400}, cost)

    def test_a_claude_request_counts_each_response_once(self) -> None:
        def assistant(seconds: int, identifier: str, output: int) -> Dict[str, Any]:
            return {"timestamp": self.at(seconds), "type": "assistant",
                    "message": {"id": identifier, "role": "assistant", "content": [{"type": "text", "text": "x"}],
                                "usage": {"output_tokens": output}}}

        path = self.write_lines([
            {"timestamp": self.at(0), "type": "user", "message": {"role": "user", "content": "Explain the import"}},
            assistant(10, "msg-1", 500), assistant(11, "msg-1", 500),
            {"timestamp": self.at(12), "type": "attachment",
             "attachment": {"type": "hook_blocking_error", "hookEvent": "Stop", "blockingError": self.TOKEN}},
            assistant(18, "msg-2", 300), assistant(19, "msg-2", 300),
            {"timestamp": self.at(30), "type": "user", "message": {"role": "user", "content": "Thanks"}},
        ])
        [cost] = memory_eval.continuation_costs(path, "claude")
        self.assertEqual({"answer_seconds": 12.0, "answer_output_tokens": 500, "ending": "earlier",
                          "save_seconds": 7.0, "save_output_tokens": 300}, cost)
        summary = memory_eval.continuation_summary([cost])
        self.assertEqual((1, 0.58), (summary["requests"], summary["time_share"]))
        self.assertEqual({"requests": 0}, memory_eval.continuation_summary([]))
        self.assertIn("1 request(s): +7.0 s and +300 output tokens",
                      "\n".join(memory_eval.format_continuations({"claude": summary,
                                                                   "codex": memory_eval.continuation_summary([])})))
        self.assertNotIn("endings", summary)

    def test_a_claude_request_sent_as_hook_feedback_counts_once(self) -> None:
        # Claude Code records a Stop hook's additionalContext three times at
        # one instant: the hook's stdout, the context it hands the model, and
        # the summary a person sees. It is one request.
        def assistant(seconds: int, identifier: str, output: int) -> Dict[str, Any]:
            return {"timestamp": self.at(seconds), "type": "assistant",
                    "message": {"id": identifier, "role": "assistant", "content": [{"type": "text", "text": "x"}],
                                "usage": {"output_tokens": output}}}

        step = ("This is an internal continuation. The person has already read your answer above this "
                "continuation: after a successful save or skip, do not repeat it.\n" + self.TOKEN)
        stdout = json.dumps({"hookSpecificOutput": {"hookEventName": "Stop", "additionalContext": step}})
        path = self.write_lines([
            {"timestamp": self.at(0), "type": "user", "message": {"role": "user", "content": "Explain the import"}},
            assistant(10, "msg-1", 500),
            {"timestamp": self.at(11), "type": "attachment",
             "attachment": {"type": "hook_success", "hookEvent": "Stop", "stdout": stdout, "exitCode": 0}},
            {"timestamp": self.at(11), "type": "attachment",
             "attachment": {"type": "hook_additional_context", "hookEvent": "Stop", "content": [step]}},
            {"timestamp": self.at(11), "type": "system", "subtype": "stop_hook_summary", "hookErrors": [],
             "hookAdditionalContext": [step]},
            assistant(14, "msg-2", 60),
            {"timestamp": self.at(30), "type": "user", "message": {"role": "user", "content": "Thanks"}},
        ])
        [cost] = memory_eval.continuation_costs(path, "claude")
        self.assertEqual({"answer_seconds": 11.0, "answer_output_tokens": 500, "ending": "brief",
                          "save_seconds": 3.0, "save_output_tokens": 60}, cost)

    def test_requests_are_told_apart_by_how_they_ask_to_end(self) -> None:
        def message(seconds: int, role: str, text: str) -> Dict[str, Any]:
            return {"timestamp": self.at(seconds), "type": "response_item",
                    "payload": {"type": "message", "role": role, "content": [{"type": "input_text", "text": text}]}}

        def tokens(seconds: int, output: int) -> Dict[str, Any]:
            return {"timestamp": self.at(seconds), "type": "event_msg",
                    "payload": {"type": "token_count", "info": {"total_token_usage": {"output_tokens": output}}}}

        repeat = " After a successful save or skip, return the substantive answer to the original user request"
        brief = " The person has already read your answer above this continuation: after a successful save"
        path = self.write_lines([
            message(0, "user", "Where is the report cache cleared?"), message(10, "assistant", "In ReportService."),
            tokens(10, 300), message(11, "user", self.TOKEN + repeat),
            message(20, "assistant", "In ReportService, as said."), tokens(21, 700),
            message(30, "user", "And the import cache?"), message(40, "assistant", "In ImportService."),
            tokens(40, 1000), message(41, "user", self.TOKEN + brief), tokens(44, 1100),
            message(50, "user", "Thanks"),
        ])
        costs = memory_eval.continuation_costs(path, "codex")
        self.assertEqual([("repeat", 400), ("brief", 100)], [(cost["ending"], cost["save_output_tokens"]) for cost in costs])
        summary = memory_eval.continuation_summary(costs)
        self.assertEqual({"brief": 1, "repeat": 1}, {name: part["requests"] for name, part in summary["endings"].items()})
        text = "\n".join(memory_eval.format_continuations({"codex": summary}))
        self.assertIn("brief:    1 request(s): +3.0 s and +100 output tokens", text)
        self.assertIn("repeat:   1 request(s): +10.0 s and +400 output tokens", text)
        counts = memory_eval.Counter()
        for answer in ("brief", "repeat", None):
            memory_eval.native_step(counts, {}, {"stage": "stop", "event": "requested",
                                                 **({"answer": answer} if answer else {})})
        self.assertEqual((3, 1), (counts["requested"], counts["requested_brief"]))

if __name__ == "__main__":
    unittest.main()
