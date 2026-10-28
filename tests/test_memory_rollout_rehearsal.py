"""scripts/memory_rollout_rehearsal.py: a sync and one turn of hooks, on a copy only."""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "memory_rollout_rehearsal.py"
GIT = ("git", "-c", "user.name=Rehearsal test", "-c", "user.email=rehearsal-test@example.invalid",
       "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null")


def clean_env():
    return {key: value for key, value in os.environ.items()
            if not key.startswith(("GIT_", "CONTEXT_", "ACCELERATOR_", "CLAUDE_PROJECT_DIR"))}


def snapshot(root: Path) -> dict:
    """Every file under root, .git included, by its bytes or link target."""
    state = {}
    for path in sorted(root.rglob("*")):
        key = path.relative_to(root).as_posix()
        if path.is_symlink():
            state[key] = "link:" + os.readlink(path)
        elif path.is_file():
            state[key] = hashlib.sha256(path.read_bytes()).hexdigest()
    return state


class RolloutRehearsalTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls._tmp = tempfile.TemporaryDirectory(prefix="rollout rehearsal ")
        base = Path(cls._tmp.name)
        cls.project = base / "client project"
        (cls.project / "src").mkdir(parents=True)
        (cls.project / "node_modules/left-pad").mkdir(parents=True)
        (cls.project / "composer.json").write_text('{"name": "acme/shop"}\n', encoding="utf-8")
        (cls.project / "src/App.php").write_text("<?php\n", encoding="utf-8")
        (cls.project / "node_modules/left-pad/index.js").write_text("module.exports = 1;\n", encoding="utf-8")
        # A link out of the project, and one inside it.
        cls.outside = base / "outside"
        cls.outside.mkdir()
        (cls.outside / "shared.json").write_text("{}\n", encoding="utf-8")
        (cls.project / "shared.json").symlink_to(cls.outside / "shared.json")
        (cls.project / "app").symlink_to("src", target_is_directory=True)
        env = clean_env()
        subprocess.run([*GIT, "init", "-q", "-b", "feature/SHOP-12-report"], cwd=cls.project, check=True, env=env)
        subprocess.run([*GIT, "add", "composer.json", "src"], cwd=cls.project, check=True, env=env)
        subprocess.run([*GIT, "commit", "-qm", "Shop baseline"], cwd=cls.project, check=True, env=env)
        installed = subprocess.run(
            [sys.executable, str(ROOT / "scripts/install_accelerator.py"), "--edition", "PHP Core",
             "--target", str(cls.project), "--merge-existing"],
            cwd=ROOT, env={**env, "PYTHONDONTWRITEBYTECODE": "1"}, capture_output=True, text=True,
        )
        assert installed.returncode == 0, installed.stdout + installed.stderr

    @classmethod
    def tearDownClass(cls) -> None:
        cls._tmp.cleanup()

    def work(self, name: str) -> Path:
        directory = Path(self._tmp.name) / name
        directory.mkdir()
        return directory

    def rehearse(self, *arguments: str) -> subprocess.CompletedProcess:
        # Your own Claude Code configuration, as the rehearsal reads it: one
        # that never accepted this project's trust dialog.
        config = Path(self._tmp.name) / "claude-config"
        config.mkdir(exist_ok=True)
        (config / ".claude.json").write_text(json.dumps({"projects": {}}), encoding="utf-8")
        return subprocess.run([sys.executable, str(SCRIPT), *arguments], cwd=ROOT,
                              env={**clean_env(), "PYTHONDONTWRITEBYTECODE": "1", "CLAUDE_CONFIG_DIR": str(config)},
                              capture_output=True, text=True, timeout=600)

    def test_each_host_and_profile_saves_and_recalls_and_the_project_is_untouched(self) -> None:
        # Cursor too: its capsule is the rule file the prompt hook renders, and
        # its Stop request comes back as `followup_message`.
        before = snapshot(self.project)
        work = self.work("work")
        result = self.rehearse("--target", str(self.project), "--work", str(work), "--hosts", "claude,codex,cursor",
                               "--json")
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        [report] = json.loads(result.stdout)
        self.assertIsNone(report["sync"]["error"])
        self.assertEqual("recall", report["profile_in_config"])
        # The copy trusts itself; the person's Claude Code never did.
        self.assertIs(False, report["claude_trust"])
        runs = report["runs"]
        self.assertEqual({(host, profile) for host in ("claude", "codex", "cursor") for profile in ("config", "recall")},
                         {(run["host"], run["profile"]) for run in runs})
        for run in runs:
            with self.subTest(host=run["host"], profile=run["profile"]):
                self.assertTrue(run["requested"])
                self.assertTrue(run["saved"], run.get("refusal"))
                self.assertTrue(run["recalled"])
                self.assertEqual([], run["hook_failures"])
                self.assertEqual("composer.json", run["source"])
                for line in ("prompt:armed", "stop:requested", "save:saved"):
                    self.assertIn(line, run["journal"])
        self.assertEqual(before, snapshot(self.project))
        self.assertEqual([], list(work.iterdir()))

    def test_the_copy_leaves_out_dependency_trees_and_keeps_the_project(self) -> None:
        work = self.work("kept")
        result = self.rehearse("--target", str(self.project), "--work", str(work), "--hosts", "claude",
                               "--profiles", "config", "--keep")
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        [copy] = list(work.glob("rollout-rehearsal-*/client-project/synced"))
        self.assertTrue((copy / "src/App.php").is_file())
        self.assertTrue((copy / ".git").is_dir())
        self.assertFalse((copy / "node_modules").exists())
        # A link out of the project is left out; one inside it stays a link.
        self.assertFalse((copy / "shared.json").exists() or (copy / "shared.json").is_symlink())
        self.assertTrue((copy / "app").is_symlink())
        self.assertIn("saved", result.stdout)

    def test_it_refuses_this_clone_and_a_folder_without_an_install(self) -> None:
        # A worktree's Git directory belongs to another checkout.
        worktree = Path(self._tmp.name) / "worktree"
        (worktree / "memory-bank/scripts").mkdir(parents=True)
        (worktree / "memory-bank/scripts/context.py").write_text("", encoding="utf-8")
        (worktree / ".git").write_text("gitdir: /nonexistent/.git/worktrees/w\n", encoding="utf-8")
        result = self.rehearse("--target", str(worktree), "--work", str(self.work("refused-worktree")))
        self.assertEqual(2, result.returncode, result.stdout + result.stderr)
        self.assertIn("worktree or submodule", result.stderr)
        for target in (ROOT, self.project / "src"):
            with self.subTest(target=target.name):
                result = self.rehearse("--target", str(target), "--work", str(self.work(f"refused-{target.name}")))
                self.assertEqual(2, result.returncode, result.stdout + result.stderr)
                self.assertEqual("", result.stdout)


if __name__ == "__main__":
    unittest.main()
