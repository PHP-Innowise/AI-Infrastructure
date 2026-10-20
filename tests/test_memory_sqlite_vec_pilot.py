#!/usr/bin/env python3
"""The optional sqlite-vec/lembed probe fails closed and RRF is deterministic."""
from __future__ import annotations

import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "memory_sqlite_vec_pilot.py"
SPEC = importlib.util.spec_from_file_location("memory_sqlite_vec_pilot", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
PILOT = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(SCRIPT.parent))
try:
    SPEC.loader.exec_module(PILOT)
finally:
    sys.path.pop(0)


class ProbeTest(unittest.TestCase):
    def test_no_assets_is_a_missing_reason_not_a_crash(self) -> None:
        result = PILOT.probe(None, None, None)
        self.assertFalse(result["available"])
        self.assertEqual("vec-extension-missing", result["reason"])
        with redirect_stdout(io.StringIO()):
            self.assertEqual(1, PILOT.main(["probe"]))

    def test_a_missing_file_is_reported_before_sqlite_starts(self) -> None:
        result = PILOT.probe("/nonexistent/vec0.so", "/nonexistent/lembed0.so", "/nonexistent/model.gguf")
        self.assertEqual("vec-extension-missing", result["reason"])
        self.assertFalse(result["available"])

    def test_a_non_extension_file_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fake = Path(temporary) / "vec0.so"
            fake.write_text("not an extension", encoding="utf-8")
            model = Path(temporary) / "model.gguf"
            model.write_bytes(b"model")
            result = PILOT.probe(str(fake), str(fake), str(model))
        self.assertFalse(result["available"])
        self.assertIn(result["reason"], {"vec-extension-load-failed", "extension-loading-unavailable"})


class FusionTest(unittest.TestCase):
    def test_rrf_rewards_agreement_and_keeps_a_deterministic_order(self) -> None:
        fused = PILOT.reciprocal_rank_fusion(["a", "b"], ["b", "c"], k=60)
        self.assertEqual(["b", "a", "c"], [row["path"] for row in fused])
        self.assertEqual({"lexical": 2, "dense": 1}, fused[0]["ranks"])
        self.assertGreater(fused[0]["score"], fused[1]["score"])

    def test_duplicates_inside_one_list_count_once(self) -> None:
        fused = PILOT.reciprocal_rank_fusion(["a", "a", "b"], [], k=60)
        self.assertEqual(["a", "b"], [row["path"] for row in fused])
        self.assertEqual({"lexical": 1}, fused[0]["ranks"])

    def test_empty_lists_are_empty(self) -> None:
        self.assertEqual([], PILOT.reciprocal_rank_fusion([], [], k=60))

    def test_the_fuse_command_reads_two_lists(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            lexical = Path(temporary) / "lexical.json"
            dense = Path(temporary) / "dense.json"
            lexical.write_text(json.dumps(["a", "b"]), encoding="utf-8")
            dense.write_text(json.dumps(["b", "c"]), encoding="utf-8")
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                code = PILOT.main(["fuse", "--lexical", str(lexical), "--dense", str(dense), "--json"])
            self.assertEqual(0, code)
            self.assertEqual(
                ["b", "a", "c"],
                [row["path"] for row in json.loads(stdout.getvalue())],
            )


if __name__ == "__main__":
    unittest.main()
