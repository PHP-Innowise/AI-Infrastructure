"""scripts/accelerator_attach.py: lend an edition to a project from the clone, copying nothing into it."""

import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "accelerator_attach.py"
sys.path.insert(0, str(ROOT / "scripts"))
import accelerator_attach as attach  # noqa: E402


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


class DetectEditionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def project(self, name, files):
        folder = self.root / name
        folder.mkdir()
        for relative, text in files.items():
            write(folder / relative, text)
        return folder

    def test_each_edition_is_found_from_the_projects_own_files(self):
        cases = {
            "Laravel": {"composer.json": json.dumps({"require": {"laravel/framework": "^11.0", "symfony/console": "^7"}})},
            "Symfony": {"composer.json": json.dumps({"require": {"symfony/framework-bundle": "^7.1"}})},
            "WordPress": {"style.css": "/*\nTheme Name: Shop\n*/\n"},
            "PHP Core": {"composer.json": json.dumps({"require": {"php": "^8.3"}})},
        }
        for edition, files in cases.items():
            with self.subTest(edition):
                self.assertEqual(edition, attach.detect_edition(self.project(edition.replace(" ", "-"), files))[0])
        locked = self.project("locked", {"composer.json": "{}", "composer.lock": json.dumps(
            {"packages": [{"name": "laravel/framework", "version": "v11.0.0"}]})})
        self.assertEqual("Laravel", attach.detect_edition(locked)[0])
        plugin = self.project("plugin", {"shop.php": "<?php\n/**\n * Plugin Name: Shop\n */\n"})
        self.assertEqual("WordPress", attach.detect_edition(plugin)[0])

    def test_a_folder_without_php_has_no_edition(self):
        edition, evidence = attach.detect_edition(self.project("notes", {"README.md": "# Notes\n"}))
        self.assertIsNone(edition)
        self.assertIn("no composer.json", evidence)


class LauncherTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "shop"
        write(self.project / "composer.json", json.dumps({"require": {"laravel/framework": "^11.0"}}))
        self.state_base = self.root / "state"

    def run_script(self, *arguments):
        return subprocess.run([sys.executable, str(SCRIPT), *arguments], capture_output=True, text=True,
                              cwd=self.root, timeout=120)

    def test_env_names_the_three_roots(self):
        result = self.run_script("env", "--project", str(self.project), "--state-base", str(self.state_base))
        self.assertEqual(0, result.returncode, result.stderr)
        state = attach.state_directory(self.project.resolve(), self.state_base)
        self.assertIn(f"export ACCELERATOR_HOME={shlex.quote(str(ROOT / 'Laravel'))}", result.stdout)
        self.assertIn(f"export ACCELERATOR_STATE_DIR={shlex.quote(str(state))}", result.stdout)
        self.assertIn(f"export ACCELERATOR_PROJECT_DIR={shlex.quote(str(self.project.resolve()))}", result.stdout)

    @unittest.skipIf(os.name == "nt" or not shutil.which("echo"), "needs a POSIX echo to stand in for the CLI")
    def test_run_passes_the_arguments_after_the_separator_through(self):
        echo = shutil.which("echo")
        result = self.run_script("run", "codex", "--project", str(self.project), "--state-base", str(self.state_base),
                                 "--executable", echo, "--", "exec", "--json", "Review the routes")
        self.assertEqual(0, result.returncode, result.stderr)
        printed = result.stdout
        self.assertIn("developer_instructions=", printed)
        self.assertIn("hooks.SessionStart=", printed)
        self.assertTrue(printed.rstrip().endswith("exec --json Review the routes"))
        self.assertEqual(["composer.json"], sorted(path.name for path in self.project.iterdir()))

    def test_the_clone_itself_is_refused(self):
        result = self.run_script("env", "--project", str(ROOT))
        self.assertEqual(2, result.returncode)
        self.assertIn("outside this clone", result.stderr)

    def test_arguments_after_the_separator_belong_to_run_only(self):
        result = self.run_script("env", "--project", str(self.project), "--", "--version")
        self.assertNotEqual(0, result.returncode)


if __name__ == "__main__":
    unittest.main()
