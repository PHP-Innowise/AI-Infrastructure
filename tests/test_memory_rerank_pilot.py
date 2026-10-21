#!/usr/bin/env python3
"""Regression tests for the read-only document reranker pilot."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import memory_rerank_pilot as pilot  # noqa: E402


class RerankPilotTests(unittest.TestCase):
    def test_path_and_title_overlap_raise_the_feature_score(self) -> None:
        candidate = {"path": "docs/alpha-guide.md", "title": "Alpha guide"}
        self.assertGreater(
            pilot.feature_score(candidate, "alpha guide"),
            pilot.feature_score(candidate, "zirconium"),
        )
        self.assertEqual(1.0, pilot.feature_score(candidate, ""))
        self.assertEqual(
            1.0,
            pilot.feature_score({"path": "docs/php.md", "title": ""}, "php"),
        )

    def test_rerank_promotes_a_path_match_over_a_slightly_higher_bm25(self) -> None:
        candidates = [
            {"path": "docs/beta.md", "title": "Beta", "adjusted_score": 1.10},
            {"path": "docs/alpha-guide.md", "title": "Guide", "adjusted_score": 1.00},
            {"path": "docs/gamma.md", "title": "Gamma", "adjusted_score": 1.40},
        ]
        selected = pilot.rerank(candidates, "alpha guide", limit=2)
        self.assertEqual(
            ["docs/alpha-guide.md", "docs/gamma.md"],
            [item["path"] for item in selected],
        )

    def test_metrics_count_useful_and_noise_paths(self) -> None:
        result = pilot.metrics(
            [{"path": "docs/a.md"}, {"path": "docs/b.md"}],
            {"docs/a.md": 2, "docs/b.md": 0, "docs/c.md": 1},
        )
        self.assertEqual(1, result["useful"])
        self.assertEqual(1, result["noise"])
        self.assertEqual(0.5, result["recall"])

    def test_evaluate_reports_baseline_and_reranked_without_a_database(self) -> None:
        class FakeRuntime:
            @staticmethod
            def _candidates(connection, query, limit):
                self.assertIsNone(connection)
                self.assertEqual(100, limit)
                return [
                    {"path": "docs/beta.md", "title": "Beta", "adjusted_score": 1.1},
                    {"path": "docs/alpha.md", "title": "Alpha", "adjusted_score": 1.0},
                ], {}

        result = pilot.evaluate(
            FakeRuntime(), None, [{"id": "q", "query": "alpha"}],
            {"q": {"docs/alpha.md": 2}}, 1,
        )
        self.assertEqual(1, result["queries"])
        self.assertEqual(0, result["baseline_useful"])
        self.assertEqual(1, result["reranked_useful"])
        self.assertTrue(result["rows"][0]["changed"])

    def test_load_queries_and_grades_reject_malformed_input(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "queries.json"
            path.write_text(json.dumps({"a": "one", "b": "two"}), encoding="utf-8")
            self.assertEqual(
                [{"id": "a", "query": "one"}, {"id": "b", "query": "two"}],
                pilot.load_queries(path),
            )
            path.write_text(json.dumps([{"query": "one"}]), encoding="utf-8")
            self.assertEqual([{"id": "0", "query": "one"}], pilot.load_queries(path))
            path.write_text(json.dumps([{"id": "p1", "prompt": "one"}]), encoding="utf-8")
            self.assertEqual([{"id": "p1", "query": "one"}], pilot.load_queries(path))
            path.write_text(json.dumps([{"id": "x"}]), encoding="utf-8")
            with self.assertRaises(ValueError):
                pilot.load_queries(path)
            path.write_text(json.dumps({"a": {"docs/a.md": 2, "docs/b.md": 0}}), encoding="utf-8")
            self.assertEqual(
                {"a": {"docs/a.md": 2, "docs/b.md": 0}},
                pilot.load_judgments(path),
            )
            path.write_text(json.dumps({"a": {"docs/a.md": True}}), encoding="utf-8")
            self.assertEqual({"a": {}}, pilot.load_judgments(path))


if __name__ == "__main__":
    unittest.main()
