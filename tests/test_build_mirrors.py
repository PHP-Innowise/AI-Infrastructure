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

import os
import shutil
import stat
import subprocess
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


# Shaped like the hooks class every PHP edition declares: canonical scripts in
# .claude/hooks, byte-identical copies in the Cursor and Codex trees.
HOOK_RULES = {
    "version": 1,
    "classes": [
        {
            "name": "hooks",
            "canonical": ".claude/hooks",
            "mirrors": {
                ".cursor/hooks": {"transform": "copy"},
                ".codex/hooks": {"transform": "copy"},
            },
        },
    ],
}
HOOK = "subagent-dispatch.sh"
MIRRORS = (".cursor/hooks", ".codex/hooks")


def executable_on_disk(path: Path) -> bool:
    return bool(path.stat().st_mode & stat.S_IXUSR)


def git(cwd: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=str(cwd), capture_output=True, text=True
    )
    if result.returncode != 0:
        raise AssertionError(f"git {' '.join(args)}: {result.stderr}")
    return result.stdout


def index_modes(repo: Path) -> dict[str, str]:
    return {
        line.split("\t", 1)[1]: line.split(" ", 1)[0]
        for line in git(repo, "ls-files", "--stage").splitlines()
    }


@unittest.skipIf(os.name == "nt", "the filesystem has no executable bit")
class TestMirrorExecutableBit(unittest.TestCase):
    """A mirror carries its canonical file's executable bit.

    Regression: build_mirrors compared and wrote bytes only, and a mirror it
    created was 0644, so the Cursor and Codex copies of subagent-dispatch.sh
    shipped non-executable from all four editions while --check stayed green.
    Cursor runs that hook as a direct command (subagentStop): exit 126, and
    the write-agent lock was never released.
    """

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="build-mirrors-mode-"))
        self.addCleanup(shutil.rmtree, self.tmp)
        self.edition = self.tmp / "Synthetic Edition"
        hooks = self.edition / ".claude" / "hooks"
        hooks.mkdir(parents=True)
        (hooks / HOOK).write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
        (hooks / HOOK).chmod(0o755)
        (hooks / "README.md").write_text("# Hooks\n", encoding="utf-8")
        (hooks / "README.md").chmod(0o644)

    def process(self, write: bool):
        return bm.process_edition(self.edition, HOOK_RULES, None, write=write)

    def test_write_creates_executable_mirrors(self):
        problems, _ = self.process(write=True)
        self.assertEqual(problems, [])
        for mirror in MIRRORS:
            self.assertTrue(executable_on_disk(self.edition / mirror / HOOK), mirror)
            self.assertFalse(executable_on_disk(self.edition / mirror / "README.md"))
        self.assertEqual(self.process(write=False), ([], []))

    def test_check_reports_mode_drift_when_bytes_match(self):
        self.process(write=True)
        lost = self.edition / ".cursor" / "hooks" / HOOK
        lost.chmod(0o644)
        problems, _ = self.process(write=False)
        self.assertEqual(
            problems,
            [
                "Synthetic Edition: [hooks] .cursor/hooks/subagent-dispatch.sh "
                "mode differs from canon (.claude/hooks/subagent-dispatch.sh "
                "is executable)"
            ],
        )

    def test_write_repairs_the_mode_of_a_byte_identical_mirror(self):
        self.process(write=True)
        lost = self.edition / ".codex" / "hooks" / HOOK
        lost.chmod(0o644)
        problems, written = self.process(write=True)
        self.assertEqual(problems, [])
        self.assertEqual(
            written, ["Synthetic Edition/.codex/hooks/subagent-dispatch.sh (mode +x)"]
        )
        self.assertTrue(executable_on_disk(lost))
        self.assertEqual(self.process(write=False), ([], []))

    def test_mirror_is_not_more_executable_than_canon(self):
        self.process(write=True)
        stray = self.edition / ".cursor" / "hooks" / "README.md"
        stray.chmod(0o755)
        problems, _ = self.process(write=False)
        self.assertEqual(len(problems), 1)
        self.assertIn("README.md mode differs from canon", problems[0])
        self.assertIn("is not executable", problems[0])
        self.process(write=True)
        self.assertFalse(executable_on_disk(stray))


@unittest.skipIf(os.name == "nt", "the filesystem has no executable bit")
class TestMirrorExecutableBitFromGitIndex(unittest.TestCase):
    """The bit is read from, and repaired in, the Git index.

    The edition sits one directory below the repository root - "PHP Core"
    and "Cms/wordpress" do too - so index paths carry a prefix.
    """

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="build-mirrors-index-"))
        self.addCleanup(shutil.rmtree, self.tmp)
        self.repo = self.tmp / "repo"
        self.edition = self.repo / "Synthetic Edition"
        canonical = self.edition / ".claude" / "hooks" / HOOK
        canonical.parent.mkdir(parents=True)
        canonical.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
        canonical.chmod(0o755)
        bm.process_edition(self.edition, HOOK_RULES, None, write=True)
        git(self.tmp, "-c", "init.defaultBranch=main", "init", "-q", str(self.repo))
        git(self.repo, "add", "--", ".")
        # The shipped defect: canon 100755, both mirrors recorded 100644.
        for mirror in MIRRORS:
            git(self.repo, "update-index", "--chmod=-x", "--", f"Synthetic Edition/{mirror}/{HOOK}")
        self.canonical = canonical

    def test_index_mode_is_reported_even_when_the_disk_is_right(self):
        problems, _ = bm.process_edition(self.edition, HOOK_RULES, None, write=False)
        self.assertEqual(
            sorted(problems),
            sorted(
                f"Synthetic Edition: [hooks] {mirror}/{HOOK} mode differs from canon "
                f"(.claude/hooks/{HOOK} is executable)"
                for mirror in MIRRORS
            ),
        )

    def test_canonical_bit_comes_from_the_index_on_a_checkout_without_it(self):
        # A Windows-made checkout: canon is 100755 in the index, 0644 on disk.
        self.canonical.chmod(0o644)
        for mirror in MIRRORS:
            (self.edition / mirror / HOOK).chmod(0o644)
        problems, written = bm.process_edition(self.edition, HOOK_RULES, None, write=True)
        self.assertEqual(problems, [])
        self.assertEqual(len(written), 2)
        modes = index_modes(self.repo)
        for mirror in MIRRORS:
            self.assertEqual(modes[f"Synthetic Edition/{mirror}/{HOOK}"], "100755")
            self.assertTrue(executable_on_disk(self.edition / mirror / HOOK))
        self.assertEqual(
            bm.process_edition(self.edition, HOOK_RULES, None, write=False), ([], [])
        )

    def test_write_records_only_the_mode_and_leaves_content_unstaged(self):
        mirror = self.edition / ".cursor" / "hooks" / HOOK
        staged_blob = git(self.repo, "rev-parse", f":Synthetic Edition/.cursor/hooks/{HOOK}")
        # A canonical edit not yet staged: --write regenerates the mirror's
        # bytes and fixes its mode, but must not stage the new content.
        self.canonical.write_text("#!/usr/bin/env bash\nexit 1\n", encoding="utf-8")
        problems, _ = bm.process_edition(self.edition, HOOK_RULES, None, write=True)
        self.assertEqual(problems, [])
        self.assertEqual(mirror.read_text(encoding="utf-8"), "#!/usr/bin/env bash\nexit 1\n")
        self.assertEqual(
            git(self.repo, "rev-parse", f":Synthetic Edition/.cursor/hooks/{HOOK}"),
            staged_blob,
        )
        self.assertEqual(
            index_modes(self.repo)[f"Synthetic Edition/.cursor/hooks/{HOOK}"], "100755"
        )

    def test_untrusted_filesystem_reads_the_index_only(self):
        # core.fileMode=false: the disk bit says nothing, the index decides.
        git(self.repo, "config", "core.fileMode", "false")
        problems, _ = bm.process_edition(self.edition, HOOK_RULES, None, write=False)
        self.assertEqual(len(problems), 2)
        problems, _ = bm.process_edition(self.edition, HOOK_RULES, None, write=True)
        self.assertEqual(problems, [])
        modes = index_modes(self.repo)
        for mirror in MIRRORS:
            self.assertEqual(modes[f"Synthetic Edition/{mirror}/{HOOK}"], "100755")
        self.assertEqual(
            bm.process_edition(self.edition, HOOK_RULES, None, write=False), ([], [])
        )


if __name__ == "__main__":
    unittest.main()
