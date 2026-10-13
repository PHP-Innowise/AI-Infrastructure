#!/usr/bin/env python3
"""Focused regressions for the opt-in, pinned-clock memory miss audit."""

from __future__ import annotations

import hashlib
import json
import shutil
import sqlite3
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import memory_eval  # noqa: E402
import memory_miss_audit as audit  # noqa: E402


class MissAuditTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.corpus = Path(self.temporary.name)
        shutil.copytree(ROOT / "PHP Core" / "memory-bank" / "scripts", self.corpus / "memory-bank" / "scripts")
        (self.corpus / "memory-bank" / "local" / "retrieval-manifests").mkdir(parents=True)
        self.database = self.corpus / "memory-bank" / "local" / "context.db"
        self.connection = sqlite3.connect(self.database)
        self.connection.execute(
            "CREATE VIRTUAL TABLE documents USING fts5(path UNINDEXED, layer UNINDEXED, kind UNINDEXED, title, summary, content, tokenize='porter unicode61')"
        )
        self.connection.execute(
            "CREATE TABLE document_metadata(path TEXT PRIMARY KEY, category TEXT NOT NULL, privacy TEXT NOT NULL, owner TEXT NOT NULL, authority TEXT NOT NULL, lifecycle TEXT NOT NULL, source_hash TEXT NOT NULL, record_id TEXT, conflicts TEXT NOT NULL, source_fingerprints TEXT NOT NULL DEFAULT '[]', updated_at TEXT NOT NULL DEFAULT '', confidence REAL NOT NULL DEFAULT 1.0, attestation TEXT NOT NULL DEFAULT '')"
        )

    def tearDown(self) -> None:
        self.connection.close()
        self.temporary.cleanup()

    def add_document(self, path: str, content: str, *, title: str = "Neutral") -> None:
        target = self.corpus / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")
        self.connection.execute(
            "INSERT INTO documents(path, layer, kind, title, summary, content) VALUES (?, 'semantic', 'memory', ?, '', ?)",
            (path, title, content),
        )
        self.connection.execute(
            "INSERT INTO document_metadata(path, category, privacy, owner, authority, lifecycle, source_hash, record_id, conflicts) VALUES (?, 'durable', 'internal', '*', 'verified', 'active', ?, NULL, '[]')",
            (path, hashlib.sha256(content.encode()).hexdigest()),
        )
        self.connection.commit()

    def manifest(self, query: str, selected: list[dict] | None = None) -> str:
        reference = "memory-bank/local/retrieval-manifests/11111111-1111-4111-8111-111111111111.json"
        (self.corpus / reference).write_text(json.dumps({"query": query, "selected": selected or [], "excluded": []}), encoding="utf-8")
        return reference

    def test_reports_no_fts_candidate_for_known_indexed_path(self) -> None:
        self.add_document("docs/a.md", "unrelated material")
        result = audit.audit(self.corpus, self.manifest("zirconium"), ["docs/a.md"])
        self.assertEqual("no-fts-candidate", result["items"][0]["stage"])

    def test_reports_relevance_drop_after_bounded_fts_window(self) -> None:
        self.add_document("docs/a.md", "alpha only")
        self.add_document("docs/b.md", "beta only")
        result = audit.audit(self.corpus, self.manifest("alpha beta"), ["docs/a.md"])
        self.assertEqual("relevance-drop", result["items"][0]["stage"])

    def test_partial_answer_in_selected_document_is_excerpt_loss(self) -> None:
        content = "First required answer. Second required answer."
        self.add_document("docs/a.md", content)
        result = audit.audit(
            self.corpus,
            self.manifest("required", [{"path": "docs/a.md"}]),
            ["docs/a.md"],
            answer_passages={"docs/a.md": ["First required answer.", "Second required answer."]},
            capsule_text="First required answer.",
        )
        item = result["items"][0]
        self.assertEqual("selected-answer-not-in-text", item["stage"])
        self.assertEqual("partial-delivery", item["answer"])
        self.assertEqual({"eligible": 2, "indexed": 2, "delivered": 1}, item["answer_counts"])

    def test_summary_keeps_per_path_answer_counts_separate(self) -> None:
        summary = memory_eval.summarize({
            "one": {
                "status": "ok", "class": "silent", "answer_passages_existed": 1,
                "answer_passages_delivered": 0, "miss_audit": {"status": "ok", "items": [
                    {"path": "a", "stage": "selected-answer-not-in-text", "answer": "partial-delivery", "answer_counts": {"eligible": 2, "indexed": 2, "delivered": 1}},
                    {"path": "b", "stage": "selected", "answer": "delivered", "answer_counts": {"eligible": 1, "indexed": 1, "delivered": 1}},
                ]},
            }
        })
        self.assertEqual(1, summary["answer_passages_existed"])
        self.assertEqual({"eligible": 3, "indexed": 3, "delivered": 2}, summary["miss_audit"]["per_path_answer_passages"])
        self.assertEqual(1, summary["miss_audit"]["stages"]["selected-answer-not-in-text"])

    def test_grade_one_explicit_passage_participates_in_audit(self) -> None:
        passages = memory_eval.audit_answer_passages(
            {"docs/a.md": 1, "docs/b.md": 2},
            {
                "docs/a.md": {"passages": ["A useful sentence"]},
                "docs/b.md": {"useful": False, "passages": ["Excluded"]},
            },
        )
        self.assertEqual({"docs/a.md": ["A useful sentence"]}, passages)

    def test_history_uses_whole_screened_prompt_and_distinct_pipeline(self) -> None:
        runtime = SimpleNamespace(
            miss_audit_sanitizer=lambda text: text,
            history_query=mock.Mock(return_value={"history": True}),
            _history_candidates=mock.Mock(return_value=([
                {"path": "winner", "adjusted_score": 2}, {"path": "target", "adjusted_score": 1}
            ], [])),
            _best_history_event=mock.Mock(return_value=([{"path": "winner", "adjusted_score": 2}], [])),
            load_config=mock.Mock(return_value={}),
        )
        self.assertEqual("unknown-history", audit._history_stage(self.corpus, self.connection, runtime, "target", None))
        self.assertEqual("history-event-limit", audit._history_stage(self.corpus, self.connection, runtime, "target", "whole subject"))
        runtime.history_query.assert_called_once_with(self.connection, "whole subject")
        self.assertEqual("history-admission-drop", audit._history_stage(self.corpus, self.connection, runtime, "absent", "subject"))
        runtime._best_history_event.return_value = ([], [{"path": "target", "reason": "privacy"}])
        self.assertEqual("runtime-exclusion", audit._history_stage(self.corpus, self.connection, runtime, "target", "subject"))

    def test_manifest_link_cap_and_report_are_visible_only_when_opted_in(self) -> None:
        self.add_document("docs/a.md", "answer")
        ref = self.manifest("answer")
        document = json.loads((self.corpus / ref).read_text())
        document["excluded"] = [{"path": "docs/a.md", "reason": "file-link-limit"}]
        (self.corpus / ref).write_text(json.dumps(document))
        result = audit.audit(self.corpus, ref, ["docs/a.md"])
        self.assertEqual("automatic-link-limit", result["items"][0]["stage"])
        first = {"items": {"one": {"status": "ok", "class": "silent"}}}
        second = {"items": {"one": {"status": "ok", "class": "silent", "miss_audit": result}}}
        ordinary = memory_eval.format_report([("A", Path("first.json"), first)])
        compared = memory_eval.format_report([("A", Path("first.json"), first), ("B", Path("second.json"), second)])
        self.assertNotIn("audit:", ordinary)
        self.assertIn("audit knowledge: automatic-link-limit", compared)


if __name__ == "__main__":
    unittest.main()
