#!/usr/bin/env python3
"""Regression tests for the memory-chunk content gate.

The bank's runtime validator proves shape; this gate proves the body says
something. A measured publication seeded fifteen chunks whose bodies were one
sentence with the title and paths substituted - the gate exists so that can
never validate again.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VALIDATOR_PATH = (
    ROOT / ".agents/skills/bootstrap-verifier/scripts/validate_memory_content.py"
)
SPEC = importlib.util.spec_from_file_location(
    "validate_memory_content", VALIDATOR_PATH
)
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)


def chunk_text(
    *,
    title: str,
    sources: list[str],
    durable: str,
    consequences: str,
    verification: str,
    status: str = "active",
    tags: list[str] | None = None,
) -> str:
    meta = {
        "id": "MEM-0001",
        "title": title,
        "type": "domain",
        "status": status,
        "scope": ["application"],
        "tags": tags or ["domain"],
        "created": "2026-08-20",
        "last_verified": "2026-08-20",
        "review_after": "2026-11-20",
        "sources": sources,
        "supersedes": [],
        "superseded_by": None,
        "valid_from": "2026-08-20",
        "valid_to": None,
    }
    return (
        "---\n"
        + json.dumps(meta, indent=2)
        + "\n---\n\n"
        + f"# {title}\n\n"
        + f"## Durable Context\n\n{durable}\n\n"
        + f"## Consequences\n\n{consequences}\n\n"
        + f"## Verification\n\n{verification}\n"
    )


class MemoryContentFixture(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.bank = self.base / "memory-bank"
        (self.bank / "chunks").mkdir(parents=True)
        self.target = self.base / "target"
        handler = self.target / "app/Services/ExportHandler.php"
        handler.parent.mkdir(parents=True)
        handler.write_text("<?php\n" + "// line\n" * 60, encoding="utf-8")
        observer = self.target / "app/Observers/JobObserver.php"
        observer.parent.mkdir(parents=True)
        observer.write_text("<?php\n" + "// line\n" * 40, encoding="utf-8")

    def write_chunk(self, name: str, text: str) -> None:
        (self.bank / "chunks" / name).write_text(text, encoding="utf-8")

    def good_chunk(self) -> str:
        return chunk_text(
            title="Export failure state invariant",
            sources=[
                "app/Services/ExportHandler.php",
                "app/Observers/JobObserver.php",
            ],
            durable=(
                "When the export in app/Services/ExportHandler.php:L24-L57 "
                "throws, the persisted status must remain FAILED and no "
                "FINISHED completion side effects may run; the observer "
                "fires them only on FINISHED."
            ),
            consequences=(
                "Any change to the handler's finally block must keep the "
                "FAILED state terminal; emitting completion webhooks after a "
                "failed export is a forbidden outcome a regression test must "
                "assert."
            ),
            verification=(
                "app/Services/ExportHandler.php:L24-L57 proves the catch "
                "sets FAILED inside the try/finally; re-review when the "
                "finally block or status writes change.\n"
                "app/Observers/JobObserver.php:L19-L39 proves the observer "
                "emits completion effects only on FINISHED transitions."
            ),
        )

    def codes(self) -> list[str]:
        return [
            item.code
            for item in validator.validate(self.bank, self.target)
            if item.severity == "error"
        ]


class MemoryContentTest(MemoryContentFixture):
    def test_a_concrete_chunk_passes(self) -> None:
        self.write_chunk("MEM-0001-export-failure.md", self.good_chunk())
        self.assertEqual(self.codes(), [])

    def test_the_shipped_boilerplate_is_refused(self) -> None:
        self.write_chunk(
            "MEM-0001-export-failure.md",
            chunk_text(
                title="Export failure state invariant",
                sources=["app/Services/ExportHandler.php"],
                durable=(
                    "Export failure state invariant is a confirmed "
                    "application authority boundary recorded by TASK-001. "
                    "Use only the cited source ranges; preserve explicit "
                    "contradictions and do not infer missing behavior."
                ),
                consequences=(
                    "Before changing intersecting work, re-open the cited "
                    "sources, apply the matching generated specialist, and "
                    "report drift or missing authority instead of promoting "
                    "an assumption."
                ),
                verification=(
                    "Recompute authority from: "
                    "app/Services/ExportHandler.php:L24-L57."
                ),
            ),
        )
        codes = self.codes()
        self.assertIn("MEMORY_GENERIC_PHRASE", codes)
        self.assertIn("MEMORY_SOURCE_UNEXPLAINED", codes)

    def test_two_bodies_sharing_a_skeleton_are_refused(self) -> None:
        template = (
            "{} is the settled operating rule for this module. Future "
            "changes must keep the recorded behavior exactly as the cited "
            "range 42 establishes it across every intersecting workflow."
        )
        for index, title in enumerate(
            ("Queue topology rule", "Session boundary rule"), start=1
        ):
            self.write_chunk(
                f"MEM-000{index}-case.md",
                chunk_text(
                    title=title,
                    sources=["app/Services/ExportHandler.php"],
                    durable=template.format(title),
                    consequences=(
                        f"Keep {title} intact; a change is a regression 42."
                    ),
                    verification=(
                        "app/Services/ExportHandler.php:L2-L5 proves the "
                        "rule holds and names when to re-review it."
                    ),
                ),
            )
        self.assertIn("MEMORY_BODY_TEMPLATED", self.codes())

    def test_a_body_with_no_rule_is_refused(self) -> None:
        chunk = self.good_chunk().replace(
            "When the export in app/Services/ExportHandler.php:L24-L57 "
            "throws, the persisted status must remain FAILED and no "
            "FINISHED completion side effects may run; the observer "
            "fires them only on FINISHED.",
            "This area of the codebase concerns exports and their "
            "lifecycle behavior generally.",
        )
        self.write_chunk("MEM-0001-export-failure.md", chunk)
        self.assertIn("MEMORY_RULE_MISSING", self.codes())

    def test_a_duplicate_source_is_refused(self) -> None:
        chunk = self.good_chunk().replace(
            '"app/Observers/JobObserver.php"',
            '"app/Services/ExportHandler.php"',
        )
        self.write_chunk("MEM-0001-export-failure.md", chunk)
        self.assertIn("MEMORY_SOURCE_DUPLICATE", self.codes())

    def test_a_missing_source_is_refused(self) -> None:
        chunk = self.good_chunk().replace(
            "app/Observers/JobObserver.php", "app/Observers/Absent.php"
        )
        self.write_chunk("MEM-0001-export-failure.md", chunk)
        self.assertIn("MEMORY_SOURCE_MISSING", self.codes())

    def test_a_range_outside_the_file_is_refused(self) -> None:
        chunk = self.good_chunk().replace(
            "app/Observers/JobObserver.php:L19-L39",
            "app/Observers/JobObserver.php:L19-L390",
        )
        self.write_chunk("MEM-0001-export-failure.md", chunk)
        self.assertIn("MEMORY_VERIFICATION_RANGE", self.codes())

    def test_a_settled_contradiction_is_refused(self) -> None:
        self.write_chunk(
            "MEM-0001-contradiction.md",
            chunk_text(
                title="Export status contradiction",
                sources=["app/Services/ExportHandler.php"],
                status="active",
                durable=(
                    "app/Services/ExportHandler.php:L24-L30 sets FAILED in "
                    "catch, while app/Services/ExportHandler.php:L50-L57 "
                    "contradicts it by promoting to FINISHED in finally; the "
                    "current safe operating rule is to treat FAILED as "
                    "terminal until resolved by the ContentJobs owner."
                ),
                consequences=(
                    "No completion side effect may be trusted for failed "
                    "exports 42 until the finally block is fixed."
                ),
                verification=(
                    "app/Services/ExportHandler.php:L24-L57 proves both "
                    "writes exist; re-review when either status write moves."
                ),
            ),
        )
        codes = self.codes()
        self.assertIn("MEMORY_CONTRADICTION_SETTLED", codes)
        self.assertNotIn("MEMORY_CONTRADICTION_IMPLICIT", codes)

    def test_a_contradiction_stating_no_claims_is_refused(self) -> None:
        self.write_chunk(
            "MEM-0001-contradiction.md",
            chunk_text(
                title="Export status contradiction",
                sources=["app/Services/ExportHandler.php"],
                status="needs-review",
                durable=(
                    "The export status handling must be treated carefully; "
                    "FAILED handling is inconsistent 42."
                ),
                consequences=(
                    "Exercise caution 42; failed exports must not emit "
                    "completion effects."
                ),
                verification=(
                    "app/Services/ExportHandler.php:L24-L57 proves the "
                    "writes exist and names when to re-review this."
                ),
            ),
        )
        self.assertIn("MEMORY_CONTRADICTION_IMPLICIT", self.codes())

    def test_an_empty_bank_fails_closed(self) -> None:
        self.assertEqual(self.codes(), ["MEMORY_BANK_EMPTY"])

    def test_a_missing_section_is_unreadable(self) -> None:
        chunk = self.good_chunk().replace("## Consequences", "## Notes")
        self.write_chunk("MEM-0001-export-failure.md", chunk)
        self.assertIn("MEMORY_CHUNK_UNREADABLE", self.codes())

    def test_the_json_report_is_byte_stable_across_runs(self) -> None:
        import contextlib
        import io

        self.write_chunk("MEM-0001-export-failure.md", self.good_chunk())
        outputs = []
        for _ in range(2):
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                validator.main(
                    [
                        "--bank",
                        str(self.bank),
                        "--target",
                        str(self.target),
                        "--json",
                    ]
                )
            outputs.append(buffer.getvalue())
        self.assertEqual(outputs[0], outputs[1])


if __name__ == "__main__":
    unittest.main()
