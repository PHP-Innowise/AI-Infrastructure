#!/usr/bin/env python3
"""Regression tests for scripts/build_mirrors.py.

The load-bearing tests cover the "only"-class drift reports (the
governance documents: DOD.md, GOLDEN-PRINCIPLES.md, STABILIZATION.md).
`iter_canonical` silently skips a missing "only" entry, so before the
reverse stray pass existed, deleting a canonical governance document left
its .cursor/.codex mirrors reported by nothing: `--check` flagged only the
stale `.gitattributes`, and once `--write` refreshed that manifest the
orphaned mirrors would have persisted indefinitely as canonical-looking
files no rule accounts for. The same silent skip meant a typo in an "only"
entry mirrored nothing while `--check` stayed green; a listed entry whose
canonical file is gone is now itself a reported problem.

The tests drive `process_edition` against a synthetic edition in a
temporary directory, so they need no scratch copy of a real edition and
touch nothing inside the repository.

Run: python3 -m unittest tests.test_build_mirrors
"""

from __future__ import annotations

import shutil
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

sys.path.insert(0, str(ROOT / "scripts"))
import build_mirrors as bm  # noqa: E402

# Shaped like the governance-docs class every edition declares: an
# "only"-class whose mirror directories also hold files other classes own.
RULES = {
    "version": 1,
    "classes": [
        {
            "name": "governance-docs",
            "canonical": ".claude",
            "only": ["DOD.md", "GOLDEN-PRINCIPLES.md"],
            "mirrors": {
                ".cursor": {"transform": "copy"},
                ".codex": {"transform": "copy"},
            },
        },
    ],
}


# The same class with a rename-typo in one "only" entry: DOD.mdd instead of
# DOD.md. iter_canonical used to skip the listed-but-absent file silently.
TYPO_RULES = {
    "version": 1,
    "classes": [
        {
            "name": "governance-docs",
            "canonical": ".claude",
            "only": ["DOD.mdd", "GOLDEN-PRINCIPLES.md"],
            "mirrors": {
                ".cursor": {"transform": "copy"},
                ".codex": {"transform": "copy"},
            },
        },
    ],
}


class TestOnlyClassReversePass(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="build-mirrors-test-"))
        self.addCleanup(shutil.rmtree, self.tmp)
        self.edition = self.tmp / "Synthetic Edition"
        canonical = self.edition / ".claude"
        canonical.mkdir(parents=True)
        (canonical / "DOD.md").write_text("# DOD\n", encoding="utf-8")
        (canonical / "GOLDEN-PRINCIPLES.md").write_text(
            "# Principles\n", encoding="utf-8"
        )
        # A canonical file another class would own inside the mirror
        # directory; the "only"-class must never treat it as its stray.
        unrelated = self.edition / ".cursor" / "commands"
        unrelated.mkdir(parents=True)
        (unrelated / "unrelated.md").write_text("# Unrelated\n", encoding="utf-8")
        # Generate the mirrors and the .gitattributes manifest.
        problems, written = bm.process_edition(self.edition, RULES, None, write=True)
        self.assertEqual(problems, [])
        self.assertTrue((self.edition / ".cursor" / "DOD.md").is_file())
        self.assertTrue((self.edition / ".codex" / "DOD.md").is_file())

    def check(self):
        problems, written = bm.process_edition(self.edition, RULES, None, write=False)
        self.assertEqual(written, [])
        return problems

    def test_check_passes_on_generated_mirrors(self):
        self.assertEqual(self.check(), [])

    def test_deleted_only_canonical_reports_surviving_mirrors(self):
        """Regression: a deleted canonical "only" file was silently skipped.

        Before the reverse pass for "only"-classes, this scenario produced
        exactly one finding (the stale .gitattributes) and said nothing
        about the two orphaned mirrors.
        """
        (self.edition / ".claude" / "DOD.md").unlink()
        problems = self.check()
        strays = {
            "Synthetic Edition: [governance-docs] .claude/DOD.md "
            "is listed in 'only' but missing from canon",
            "Synthetic Edition: [governance-docs] .cursor/DOD.md "
            "has no source in .claude",
            "Synthetic Edition: [governance-docs] .codex/DOD.md "
            "has no source in .claude",
        }
        self.assertTrue(
            strays.issubset(set(problems)),
            f"orphaned governance mirrors not reported; got: {problems}",
        )
        # The pre-existing .gitattributes finding is still there alongside.
        self.assertTrue(
            any(bm.GITATTRIBUTES_NAME in problem for problem in problems),
            f"stale .gitattributes not reported; got: {problems}",
        )

    def test_strays_survive_a_gitattributes_refresh(self):
        """The orphans stay reported even after --write refreshes the manifest.

        This pins the exact silent-drift scenario: without the reverse pass,
        one --write made the tree look fully clean again while the orphaned
        mirrors persisted.
        """
        (self.edition / ".claude" / "DOD.md").unlink()
        problems, written = bm.process_edition(self.edition, RULES, None, write=True)
        self.assertIn(f"{self.edition.name}/{bm.GITATTRIBUTES_NAME}", written)
        expected = [
            "Synthetic Edition: [governance-docs] .claude/DOD.md "
            "is listed in 'only' but missing from canon",
            "Synthetic Edition: [governance-docs] .cursor/DOD.md "
            "has no source in .claude",
            "Synthetic Edition: [governance-docs] .codex/DOD.md "
            "has no source in .claude",
        ]
        self.assertEqual(problems, expected)
        # A subsequent --check reports exactly the strays, deterministically.
        self.assertEqual(self.check(), expected)
        self.assertEqual(self.check(), expected)

    def test_typoed_only_entry_is_reported(self):
        """Regression: a listed "only" entry with no canonical file was silent.

        iter_canonical skips a listed path with no file behind it, so a
        typo in an "only" entry (DOD.mdd for DOD.md) mirrored nothing while
        --check stayed green. With mirrors already generated under the
        correct name, the typo flagged only the stale .gitattributes, and
        one --write later everything reported clean while the orphaned
        .cursor/DOD.md and .codex/DOD.md persisted on disk.
        """
        missing = (
            "Synthetic Edition: [governance-docs] .claude/DOD.mdd "
            "is listed in 'only' but missing from canon"
        )
        problems, _ = bm.process_edition(self.edition, TYPO_RULES, None, write=False)
        self.assertIn(missing, problems)
        # --write cannot repair a missing canonical file: the finding must
        # survive the manifest refresh instead of going green.
        problems, _ = bm.process_edition(self.edition, TYPO_RULES, None, write=True)
        self.assertEqual(problems, [missing])
        # Subsequent checks keep reporting it, deterministically, and the
        # mirrors generated under the correct name are still on disk.
        for _ in range(2):
            problems, written = bm.process_edition(
                self.edition, TYPO_RULES, None, write=False
            )
            self.assertEqual(written, [])
            self.assertEqual(problems, [missing])
        self.assertTrue((self.edition / ".cursor" / "DOD.md").is_file())
        self.assertTrue((self.edition / ".codex" / "DOD.md").is_file())

    def test_unrelated_mirror_files_stay_untouched(self):
        """The pass examines only the listed names, never the whole mirror.

        The mirror directories of an "only"-class (.cursor, .codex) also hold
        canonical files other classes own; flagging them would break every
        edition.
        """
        (self.edition / ".claude" / "DOD.md").unlink()
        problems = self.check()
        self.assertFalse(
            any("unrelated.md" in problem for problem in problems),
            f"unrelated mirror file flagged: {problems}",
        )


if __name__ == "__main__":
    unittest.main()
