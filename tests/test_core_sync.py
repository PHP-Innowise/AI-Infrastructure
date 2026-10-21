"""scripts/core_sync.py writes PHP Core's memory core to its copies, and only
the copies are marked generated for review."""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import core_sync  # noqa: E402

RUNTIME = core_sync.manifest(ROOT)


class SyntheticMonorepoTest(unittest.TestCase):
    """A four-edition tree small enough to see every action."""

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        for relative in RUNTIME.CROSS_EDITION_PATHS.values():
            (self.root / relative / "memory-bank" / "scripts").mkdir(parents=True)
            (self.root / relative / ".claude" / "hooks").mkdir(parents=True)
        self.canonical = self.root / "PHP Core"
        self.write(self.canonical, "memory-bank/scripts/engine.py", "ENGINE = 2\n")
        self.write(self.canonical, ".claude/hooks/working-memory-read.sh", "#!/bin/sh\necho read\n", mode=0o755)
        self.write(self.canonical, ".claude/hooks/bash-validator.sh", "#!/bin/sh\necho core rules\n", mode=0o755)
        for edition in ("Laravel", "Symfony", "Cms/wordpress"):
            base = self.root / edition
            self.write(base, "memory-bank/scripts/engine.py", "ENGINE = 1\n")
            self.write(base, "memory-bank/scripts/retired.py", "OLD = True\n")
            self.write(base, ".claude/hooks/working-memory-read.sh", "#!/bin/sh\necho read\n", mode=0o644)
            self.write(base, ".claude/hooks/bash-validator.sh", f"#!/bin/sh\necho {edition} rules\n", mode=0o755)

    def tearDown(self) -> None:
        self.directory.cleanup()

    @staticmethod
    def write(base: Path, relative: str, text: str, mode: int = 0o644) -> None:
        path = base / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        os.chmod(path, mode)

    def test_the_plan_names_every_difference_once_per_copy(self) -> None:
        actions = {(a["edition"], a["path"], a["action"]) for a in core_sync.plan(self.root, RUNTIME)}
        for edition in ("Laravel", "Symfony", "WordPress"):
            self.assertIn((edition, "memory-bank/scripts/engine.py", "update"), actions)
            self.assertIn((edition, "memory-bank/scripts/retired.py", "remove"), actions)
            self.assertIn((edition, ".claude/hooks/working-memory-read.sh", "mode"), actions)
        self.assertFalse(any(path.endswith("bash-validator.sh") for _, path, _ in actions))

    def test_write_makes_every_copy_canonical_and_keeps_framework_hooks(self) -> None:
        self.assertEqual(0, core_sync.main(["--write", "--no-chain", "--root", str(self.root)]))
        for edition in ("Laravel", "Symfony", "Cms/wordpress"):
            base = self.root / edition
            self.assertEqual("ENGINE = 2\n", (base / "memory-bank/scripts/engine.py").read_text(encoding="utf-8"))
            self.assertFalse((base / "memory-bank/scripts/retired.py").exists())
            self.assertTrue(os.stat(base / ".claude/hooks/working-memory-read.sh").st_mode & 0o111)
            self.assertIn(edition, (base / ".claude/hooks/bash-validator.sh").read_text(encoding="utf-8"))
        self.assertEqual([], core_sync.plan(self.root, RUNTIME))
        self.assertEqual(0, core_sync.main(["--check", "--root", str(self.root)]))

    def test_check_fails_while_a_copy_differs(self) -> None:
        self.assertEqual(1, core_sync.main(["--check", "--root", str(self.root)]))


class RepositoryTest(unittest.TestCase):
    def test_the_repository_copies_match_php_core(self) -> None:
        self.assertEqual([], core_sync.plan(ROOT, RUNTIME))

    @unittest.skipUnless(shutil.which("git") and (ROOT / ".git").exists(), "needs a Git checkout")
    def test_only_the_copies_are_marked_generated(self) -> None:
        canonical = core_sync.core_files(ROOT / "PHP Core", RUNTIME)
        paths = [f"PHP Core/{relative}" for relative in canonical]
        for name, relative_root in RUNTIME.CROSS_EDITION_PATHS.items():
            if name != "PHP Core" and (ROOT / relative_root).is_dir():
                paths += [f"{relative_root.as_posix()}/{relative}" for relative in canonical]
        framework_hooks = [
            f"{relative_root.as_posix()}/.claude/hooks/{hook}"
            for name, relative_root in RUNTIME.CROSS_EDITION_PATHS.items() if name != "PHP Core"
            for hook in ("bash-validator.sh", "local-context.sh")
        ]
        listing = subprocess.run(
            ["git", "-C", str(ROOT), "check-attr", "--stdin", "linguist-generated"],
            input="\n".join(paths + framework_hooks) + "\n", capture_output=True, text=True, check=True,
        ).stdout
        values = dict(line.rsplit(": linguist-generated: ", 1) for line in listing.splitlines())
        for path in paths:
            expected = "unspecified" if path.startswith("PHP Core/") else "true"
            self.assertEqual(expected, values[path], path)
        for path in framework_hooks:
            self.assertEqual("unset", values[path], path)


if __name__ == "__main__":
    unittest.main()
