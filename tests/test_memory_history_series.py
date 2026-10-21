#!/usr/bin/env python3
"""Regression tests for the opt-in same-ticket history-series pilot."""

from __future__ import annotations

import importlib.util
import os
import sys
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parent.parent
RUNTIME = ROOT / "PHP Core" / "memory-bank" / "scripts" / "context_retrieval.py"
SPEC = importlib.util.spec_from_file_location("memory_history_series_runtime", RUNTIME)
assert SPEC is not None and SPEC.loader is not None
retrieval = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(RUNTIME.parent))
try:
    SPEC.loader.exec_module(retrieval)
finally:
    sys.path.pop(0)


class HistorySeriesTests(unittest.TestCase):
    def test_pilot_is_off_by_default_and_accepts_an_explicit_opt_in(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertFalse(retrieval._history_series_pilot())
        for value in ("1", "true", "YES", " yes "):
            with self.subTest(value=value), mock.patch.dict(
                os.environ, {"CONTEXT_EVENT_SERIES": value}
            ):
                self.assertTrue(retrieval._history_series_pilot())
        with mock.patch.dict(os.environ, {"CONTEXT_EVENT_SERIES": "0"}):
            self.assertFalse(retrieval._history_series_pilot())

    def test_the_largest_same_ticket_series_is_selected_in_original_order(self) -> None:
        events = [
            {"path": "event-a", "title": "ABC-7 first"},
            {"path": "event-b", "title": "OTHER-1 unrelated"},
            {"path": "event-c", "title": "ABC-7 second"},
            {"path": "event-d", "title": "ABC-7 third"},
        ]
        self.assertEqual(
            ["event-a", "event-c", "event-d"],
            [item["path"] for item in retrieval._same_ticket_series(events)],
        )

    def test_events_without_a_ticket_are_not_grouped(self) -> None:
        self.assertEqual(
            [],
            retrieval._same_ticket_series([
                {"path": "event-a", "title": "first"},
                {"path": "event-b", "title": "second"},
            ]),
        )


if __name__ == "__main__":
    unittest.main()
