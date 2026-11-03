#!/usr/bin/env python3
"""Known-answer fact consolidation: explicit versions, deterministic ties."""
from __future__ import annotations

import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "memory_fact_consolidation.py"
SPEC = importlib.util.spec_from_file_location("memory_fact_consolidation", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
FACTS = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(SCRIPT.parent))
try:
    SPEC.loader.exec_module(FACTS)
finally:
    sys.path.pop(0)


class FixtureTest(unittest.TestCase):
    def test_the_known_answer_fixture(self) -> None:
        fixture = json.loads(
            (ROOT / "tests/fixtures/fact-consolidation.json").read_text(encoding="utf-8")
        )
        for case in fixture["cases"]:
            with self.subTest(case=case["name"]):
                result = FACTS.resolve(case["facts"])
                group = result["groups"][0] if len(result["groups"]) == 1 else None
                if group is None:
                    # An unrelated fact forms its own group.
                    for entry in result["groups"]:
                        self.assertEqual(
                            [entry["winner"]], [entry["candidates"][0]]
                        )
                    continue
                if case["expected"]["superseded"]:
                    by_path = {fact["path"]: fact["id"] for fact in case["facts"]}
                    self.assertEqual(
                        [by_path[path] for path in case["expected"]["kept"]],
                        [group["winner"]],
                    )
                    self.assertEqual(
                        sorted(by_path[path] for path in case["expected"]["superseded"]),
                        group["superseded"],
                    )
                else:
                    self.assertTrue(group["ambiguous"])
                    self.assertIsNone(group["winner"])


class ResolveTest(unittest.TestCase):
    def test_timezone_offsets_compare_instants_and_equal_instants_remain_tied(self):
        first={'id':'a','fact':'f','serial':1,'updated_at':'2026-10-31T10:00:00+02:00'}
        second={'id':'b','fact':'f','serial':1,'updated_at':'2026-10-31T09:00:00Z'}
        self.assertEqual('b',FACTS.resolve([first,second])['groups'][0]['winner'])
        first['updated_at']='2026-10-31T10:00:00+01:00'
        group=FACTS.resolve([first,second])['groups'][0]
        self.assertTrue(group['ambiguous']);self.assertIsNone(group['winner'])

    def test_invalid_or_ambiguous_local_timestamps_are_not_ranked(self):
        for stamp in ('not-a-time','2026-10-31T10:00:00',3,
                      '0001-01-01T00:00:00+01:00','9999-12-31T23:59:59-01:00'):
            with self.subTest(stamp=stamp),self.assertRaises(ValueError):
                FACTS.resolve([{'id':'a','fact':'f','updated_at':stamp}])

    def test_version_beats_timestamp_and_timestamp_breaks_equal_versions(self) -> None:
        result = FACTS.resolve([
            {"id": "old", "fact": "f", "serial": 1, "updated_at": "2026-01-02"},
            {"id": "new", "fact": "f", "serial": 2, "updated_at": "2026-01-01"},
            {"id": "same-version", "fact": "f", "serial": 2, "updated_at": "2026-01-03"},
        ])
        group = result["groups"][0]
        self.assertEqual("same-version", group["winner"])
        self.assertEqual(["new", "old"], group["superseded"])

    def test_equal_version_and_timestamp_is_ambiguous(self) -> None:
        result = FACTS.resolve([
            {"id": "a", "fact": "f", "serial": 1, "updated_at": "2026-01-01"},
            {"id": "b", "fact": "f", "serial": 1, "updated_at": "2026-01-01"},
        ])
        group = result["groups"][0]
        self.assertTrue(group["ambiguous"])
        self.assertIsNone(group["winner"])
        self.assertEqual(["a", "b"], group["candidates"])

    def test_duplicate_ids_and_missing_keys_are_refused(self) -> None:
        with self.assertRaises(ValueError):
            FACTS.resolve([{"id": "a", "fact": "f"}, {"id": "a", "fact": "g"}])
        with self.assertRaises(ValueError):
            FACTS.resolve([{"id": "a"}])


class CliTest(unittest.TestCase):
    def test_json_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "facts.json"
            path.write_text(
                json.dumps([
                    {"id": "a", "fact": "f", "serial": 1},
                    {"id": "b", "fact": "f", "serial": 2},
                ]),
                encoding="utf-8",
            )
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                code = FACTS.main(["--input", str(path), "--json"])
            self.assertEqual(0, code)
            group = json.loads(stdout.getvalue())["groups"][0]
            self.assertEqual("b", group["winner"])
            self.assertEqual(["a"], group["superseded"])


if __name__ == "__main__":
    unittest.main()
