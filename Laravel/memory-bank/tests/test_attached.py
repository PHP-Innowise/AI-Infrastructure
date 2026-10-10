#!/usr/bin/env python3
"""Attached layout: the accelerator stays in its clone, the project gets nothing.

The runtime under test is this edition's own: in attached mode it indexes the
edition's policy and skills in place, reads the project's documents and Git,
and keeps every piece of accelerator state in a separate directory. These
tests prove the three roots stay apart and that the installed layout - every
other test in this suite - is untouched by the variables.
"""

from __future__ import annotations

import json
import os
import sqlite3
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock


EDITION = Path(__file__).resolve().parents[2]
SCRIPTS = EDITION / "memory-bank" / "scripts"
CONTEXT_SCRIPT = SCRIPTS / "context.py"
sys.path.insert(0, str(SCRIPTS))
try:
    import brain_runtime as brain
    import workspace_roots
finally:
    sys.path.pop(0)


def git(directory: Path, *arguments: str) -> str:
    return subprocess.run(
        ["git", "-C", str(directory), *arguments],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def mode(path: Path) -> int:
    return stat.S_IMODE(path.lstat().st_mode)


# A first run that stops half-way through writing one seed's bytes - into
# whichever file the runtime writes them - until it reads a line.
FIRST_RUN_STOPPED_IN_A_SEED = r'''
import io, json, os, sys
from pathlib import Path

sys.path.insert(0, sys.argv[1])
import brain_runtime

seed = Path(sys.argv[2]).read_bytes()
real_open = io.open
stopped = []


class StopsHalfway:
    def __init__(self, handle):
        self._handle = handle

    def write(self, data):
        if stopped or bytes(data) != seed:
            return self._handle.write(data)
        stopped.append(True)
        half = len(seed) // 2
        self._handle.write(seed[:half])
        self._handle.flush()
        print("stopped", flush=True)
        sys.stdin.readline()
        return half + self._handle.write(seed[half:])

    def __enter__(self):
        self._handle.__enter__()
        return self

    def __exit__(self, *details):
        return self._handle.__exit__(*details)

    def __getattr__(self, name):
        return getattr(self._handle, name)


def opening(file, mode="r", *arguments, **options):
    handle = real_open(file, mode, *arguments, **options)
    return StopsHalfway(handle) if "w" in mode and "b" in mode else handle


io.open = opening
created = brain_runtime.ensure_attached_state(Path(os.environ["ACCELERATOR_STATE_DIR"]))
print(json.dumps(created), flush=True)
'''


class AttachedProject(unittest.TestCase):
    """A Git project with no accelerator files, and an empty state directory."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="attached-layout-test-")
        root = Path(self.temporary.name)
        self.project = root / "shop"
        self.state = root / "state" / "shop"
        self.project.mkdir()
        git(self.project, "init", "--quiet")
        git(self.project, "checkout", "--quiet", "-b", "feature/saffron-checkout")
        files = {
            "README.md": "# Shop\n\nThe saffron checkout flow settles every order.\n",
            "AGENTS.md": "# Shop rules\n\nUse the vermilion naming rule for services.\n",
            "docs/payments.md": "# Payments\n\nRefunds follow the cobalt ledger rule.\n",
            "app/Order.php": "<?php\n\nfinal class Order\n{\n}\n",
        }
        for name, content in files.items():
            path = self.project / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        git(self.project, "add", "-A")
        git(
            self.project,
            "-c", "user.email=test@example.invalid",
            "-c", "user.name=Test",
            "commit", "--quiet", "-m", "Initial shop",
        )
        self.environment = {
            **os.environ,
            "ACCELERATOR_HOME": str(EDITION),
            "ACCELERATOR_STATE_DIR": str(self.state),
            "ACCELERATOR_PROJECT_DIR": str(self.project),
        }
        for variable in ("CONTEXT_TASK_ID", "CONTEXT_CAPSULE_DELIVERED"):
            self.environment.pop(variable, None)
        self.tooling_database = EDITION / "memory-bank" / "local" / "context.db"
        self.tooling_database_before = self._stat(self.tooling_database)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    @staticmethod
    def _stat(path: Path):
        try:
            status = path.stat()
        except OSError:
            return None
        return status.st_mtime_ns, status.st_size

    def run_cli(self, *arguments: str, environment=None) -> subprocess.CompletedProcess[str]:
        # No --root: an attached runtime takes its state root from the
        # environment, as the hooks run it.
        return subprocess.run(
            [sys.executable, str(CONTEXT_SCRIPT), *arguments],
            cwd=self.project,
            env=environment or self.environment,
            capture_output=True,
            text=True,
        )

    def indexed_paths(self) -> set[str]:
        database = self.state / "memory-bank" / "local" / "context.db"
        with sqlite3.connect(database) as connection:
            return {row[0] for row in connection.execute("SELECT path FROM documents")}

    def assert_project_untouched(self) -> None:
        self.assertEqual("", git(self.project, "status", "--porcelain", "--ignored"))



class AttachedLayoutTest(AttachedProject):
    def test_index_reads_each_document_from_its_own_root(self) -> None:
        result = self.run_cli("index", "--json")
        self.assertEqual(0, result.returncode, result.stderr)

        paths = self.indexed_paths()
        # Tooling is keyed by absolute path, so it cannot collide with the
        # project's own policy file of the same name.
        self.assertIn((EDITION / "AGENTS.md").as_posix(), paths)
        self.assertIn("AGENTS.md", paths)
        self.assertTrue(
            any(path.startswith(EDITION.as_posix() + "/") and "/skills/" in path for path in paths),
            "the edition's skills are indexed in place",
        )
        self.assertIn("README.md", paths)
        self.assertIn("docs/payments.md", paths)
        # The accelerator's own README and changelog describe the
        # accelerator, not the work, and stay out of an attached index.
        self.assertNotIn((EDITION / "README.md").as_posix(), paths)
        self.assertNotIn((EDITION / "CHANGELOG.md").as_posix(), paths)

    def test_state_lives_outside_the_project_and_the_clone(self) -> None:
        result = self.run_cli("status", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        status = json.loads(result.stdout)

        self.assertEqual("attached", status["workspace"]["layout"])
        self.assertEqual(str(self.project), status["workspace"]["project"])
        self.assertEqual(
            (self.state / "memory-bank" / "local" / "context.db").resolve(),
            Path(status["database"]).resolve(),
        )
        for directory in brain.ATTACHED_STATE_DIRECTORIES:
            self.assertTrue((self.state / directory).is_dir(), directory)
        self.assertTrue((self.state / "project-brain/config/runtime.json").is_file())
        self.assertEqual(
            (EDITION / "memory-bank/.install/INDEX.md").read_bytes(),
            (self.state / "memory-bank/INDEX.md").read_bytes(),
        )
        # Tooling is read in place, never copied into the state.
        self.assertFalse((self.state / "project-brain/schemas").exists())
        self.assertFalse((self.state / "memory-bank/scripts").exists())
        self.assert_project_untouched()
        self.assertEqual(self.tooling_database_before, self._stat(self.tooling_database))

    def test_search_returns_project_text_and_tooling_paths(self) -> None:
        self.assertEqual(0, self.run_cli("index").returncode)

        result = self.run_cli("search", "saffron checkout git", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        found = [item["path"] for item in json.loads(result.stdout)["documents"]]
        self.assertIn("README.md", found)
        self.assertTrue(
            any(path.startswith(EDITION.as_posix() + "/") for path in found),
            "skills from the clone carry their absolute path",
        )

        result = self.run_cli("search", "vermilion naming", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        found = [item["path"] for item in json.loads(result.stdout)["documents"]]
        self.assertIn("AGENTS.md", found)
        self.assert_project_untouched()

    def test_turn_records_project_changes_in_the_state(self) -> None:
        (self.project / "app" / "Order.php").write_text(
            "<?php\n\nfinal class Order\n{\n    public int $total = 0;\n}\n",
            encoding="utf-8",
        )
        result = self.run_cli(
            "turn", "--task-id", "feature/saffron-checkout", "--flush", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        report = json.loads(
            (self.state / "memory-bank/local/last-turn-report.json").read_text(
                encoding="utf-8"
            )
        )
        self.assertEqual(1, report["files"])
        self.assertTrue(report["flushed"])
        tasks = list((self.state / "project-brain/dynamic/tasks").glob("*.md"))
        self.assertEqual(1, len(tasks))
        # The only change in the project is the user's own edit.
        self.assertEqual(
            " M app/Order.php\n", git(self.project, "status", "--porcelain", "--ignored")
        )

    def test_citations_are_fingerprinted_in_the_project(self) -> None:
        with mock.patch.dict(os.environ, self.environment, clear=True):
            self.state.mkdir(parents=True)
            self.assertTrue(workspace_roots.is_attached(self.state))
            fingerprint = brain.fingerprint(self.state, "app/Order.php")
            record = {"sources": ["app/Order.php"], "source_fingerprints": [fingerprint]}
            self.assertTrue(brain.sources_are_fresh(self.state, record))

            (self.project / "app" / "Order.php").write_text("<?php\n", encoding="utf-8")
            self.assertFalse(brain.sources_are_fresh(self.state, record))
            with self.assertRaises(brain.BrainError):
                brain.fingerprint(self.state, "../../outside.txt")

    def test_a_result_cites_project_files_and_lands_in_the_state(self) -> None:
        started = self.run_cli("start", "--task-id", "TASK-ATTACHED", "--goal", "Check the order", "--json")
        self.assertEqual(0, started.returncode, started.stderr)
        request = self.state.parent / "result.json"
        request.write_text(json.dumps({
            "progress": "Order checked.", "next_steps": [], "verified": True,
            "learnings": [{"type": "finding", "title": "Orders are final classes",
                           "consequence": "Extend an order through composition.", "sources": ["app/Order.php"]}],
        }), encoding="utf-8")
        recorded = self.run_cli("record-result", "--task-id", "TASK-ATTACHED", "--result-id", "attached-1",
                                "--revision", "auto", "--input", str(request), "--json")
        self.assertEqual(0, recorded.returncode, recorded.stderr)
        self.assertEqual(["created"], [item["state"] for item in json.loads(recorded.stdout)["records"]])
        self.assertEqual(1, len(list((self.state / "project-brain/dynamic/findings").glob("*.md"))))
        # A citation still may not leave the project.
        request.write_text(json.dumps({
            "verified": True,
            "learnings": [{"type": "finding", "title": "Outside", "consequence": "No.",
                           "sources": ["../state/shop/project-brain/config/runtime.json"]}],
        }), encoding="utf-8")
        refused = self.run_cli("record-result", "--task-id", "TASK-ATTACHED", "--result-id", "attached-2",
                               "--revision", "auto", "--input", str(request), "--json")
        self.assertNotEqual(0, refused.returncode)
        self.assert_project_untouched()

    def test_variables_naming_another_copy_leave_the_installed_layout(self) -> None:
        environment = {**self.environment, "ACCELERATOR_HOME": str(self.project)}
        with mock.patch.dict(os.environ, environment, clear=True):
            self.assertIsNone(workspace_roots.configuration_error())
            self.assertEqual(workspace_roots.TOOLING_ROOT, workspace_roots.default_state_root())
            layout = workspace_roots.roots(EDITION)
            self.assertFalse(layout.attached)
            self.assertEqual(EDITION, layout.project)

    def test_incomplete_or_unsafe_variables_are_refused(self) -> None:
        cases = {
            "missing project": {"ACCELERATOR_PROJECT_DIR": ""},
            "relative state": {"ACCELERATOR_STATE_DIR": "state"},
            "state inside project": {
                "ACCELERATOR_STATE_DIR": str(self.project / ".accelerator")
            },
        }
        for label, overrides in cases.items():
            with self.subTest(label):
                result = self.run_cli(
                    "status", "--json", environment={**self.environment, **overrides}
                )
                self.assertNotEqual(0, result.returncode)
                self.assertIn("ACCELERATOR_", result.stderr)
                self.assert_project_untouched()

    @unittest.skipIf(os.name == "nt", "creating a symbolic link needs a privilege on Windows")
    def test_a_state_reached_through_a_link_is_refused(self) -> None:
        # A state directory standing as a link took the project's memory to
        # wherever it pointed, and a link at one of its entries took that part.
        elsewhere = self.state.parent / "elsewhere"
        elsewhere.mkdir(parents=True)
        for target in (self.project, elsewhere):
            with self.subTest(target=target.name):
                self.state.symlink_to(target, target_is_directory=True)
                try:
                    result = self.run_cli("status", "--json")
                finally:
                    self.state.unlink()
                self.assertNotEqual(0, result.returncode)
                self.assertIn(f"{self.state} is a symbolic link", result.stderr)
                self.assertIn("ACCELERATOR_STATE_DIR", result.stderr)
                self.assertEqual([], list(elsewhere.iterdir()))
                self.assert_project_untouched()
        self.state.mkdir()
        (self.state / "memory-bank").symlink_to(elsewhere, target_is_directory=True)
        result = self.run_cli("status", "--json")
        self.assertNotEqual(0, result.returncode)
        self.assertIn(f"{self.state / 'memory-bank'} is a symbolic link", result.stderr)
        self.assertEqual([], list(elsewhere.iterdir()))
        self.assertEqual(["memory-bank"], [path.name for path in self.state.iterdir()])


class AttachedBootstrapTest(AttachedProject):
    """Two first runs of one state: the hooks of one event start together."""

    def test_a_second_first_run_never_takes_a_half_written_seed(self) -> None:
        cases = {
            # What the review saw: a half-written configuration is invalid.
            "configuration": ("project-brain/config/runtime.json", "project-brain/config/runtime.json"),
            "brain index": ("project-brain/.install/active.json", "project-brain/indexes/active.json"),
            # The last seed: the rest of the layout is there, so only a seed
            # that is not visible until whole sends the second run to the lock.
            "memory index": ("memory-bank/.install/INDEX.md", "memory-bank/INDEX.md"),
        }
        for label, (source, target) in cases.items():
            with self.subTest(label):
                state = self.state.parent / label
                environment = {**self.environment, "ACCELERATOR_STATE_DIR": str(state)}
                # The second run is this process: it says when it reaches the
                # bootstrap lock, which the first run holds while it writes.
                waiting = threading.Event()
                real_lock = brain._file_lock

                def lock(handle, unlock=False):
                    if not unlock:
                        waiting.set()
                    return real_lock(handle, unlock)

                outcome: dict = {}

                def second_run() -> None:
                    try:
                        outcome["created"] = brain.ensure_attached_state(state)
                        outcome["mode"] = brain.load_config(state)["mode"]
                        outcome["seed"] = (state / target).read_bytes()
                    except Exception as error:  # reported by the assertions below
                        outcome["error"] = error

                with mock.patch.dict(os.environ, environment, clear=True), mock.patch.object(
                    brain, "_file_lock", lock
                ):
                    first = subprocess.Popen(
                        [sys.executable, "-c", FIRST_RUN_STOPPED_IN_A_SEED, str(SCRIPTS), str(EDITION / source)],
                        cwd=self.project,
                        env=environment,
                        stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE,
                        text=True,
                    )
                    try:
                        self.assertEqual("stopped", first.stdout.readline().strip())
                        second = threading.Thread(target=second_run, daemon=True)
                        second.start()
                        deadline = time.monotonic() + 60
                        while second.is_alive() and not waiting.is_set() and time.monotonic() < deadline:
                            time.sleep(0.01)
                        # Only now does the first run finish its write.
                        first.stdin.write("go\n")
                        first.stdin.flush()
                        second.join(60)
                        created_by_first = json.loads(first.communicate(timeout=60)[0])
                    finally:
                        if first.poll() is None:
                            first.kill()
                            first.communicate()
                self.assertNotIn("error", outcome)
                self.assertTrue(waiting.is_set(), "the second run waits for the first run's layout")
                self.assertIn(target, created_by_first)
                self.assertEqual([], outcome["created"])
                self.assertEqual((EDITION / source).read_bytes(), outcome["seed"])
                self.assertEqual(
                    json.loads((EDITION / "project-brain/config/runtime.json").read_text(encoding="utf-8"))["mode"],
                    outcome["mode"],
                )
                self.assertEqual([], list(state.rglob(".*.*")), "no temporary file is left behind")


@unittest.skipIf(os.name == "nt", "POSIX mode bits; Windows keeps its profile's permissions")
class AttachedPrivacyTest(AttachedProject):
    """The state is the project's memory kept outside it: its owner's alone."""

    def run_with_umask(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        previous = os.umask(0o022)
        try:
            return self.run_cli(*arguments)
        finally:
            os.umask(previous)

    def test_state_is_owner_only_whatever_the_umask(self) -> None:
        for arguments in (
            ("status", "--json"),
            ("start", "--task-id", "TASK-PRIVATE", "--goal", "Keep the memory private", "--json"),
        ):
            result = self.run_with_umask(*arguments)
            self.assertEqual(0, result.returncode, result.stderr)

        self.assertTrue((self.state / "memory-bank/local/context.db").is_file())
        self.assertTrue(list((self.state / "project-brain/dynamic/tasks").glob("*.md")))
        # The state's parent did not exist either: the runtime made it too.
        entries = [self.state.parent, self.state, *self.state.rglob("*")]
        self.assertEqual(
            {str(path): 0o700 if path.is_dir() else 0o600 for path in entries},
            {str(path): mode(path) for path in entries},
        )
        self.assert_project_untouched()

    def test_an_existing_state_is_tightened_and_nothing_outside_it(self) -> None:
        self.assertEqual(0, self.run_with_umask("status", "--json").returncode)
        outside = self.state.parent / "notes.md"
        outside.write_text("Beside the state, not in it.\n", encoding="utf-8")
        (self.state / "memory-bank/local/notes.md").symlink_to(outside)
        # A state from before the rule: everything open to others.
        for path in [self.state.parent, self.state, *self.state.rglob("*"), outside]:
            if not path.is_symlink():
                path.chmod(0o755 if path.is_dir() else 0o644)

        result = self.run_with_umask("status", "--json")
        self.assertEqual(0, result.returncode, result.stderr)

        open_to_others = [
            str(path)
            for path in [self.state, *self.state.rglob("*")]
            if not path.is_symlink() and mode(path) & 0o077
        ]
        self.assertEqual([], open_to_others)
        # Neither the directory above the state nor a link's target changes.
        self.assertEqual(0o755, mode(self.state.parent))
        self.assertEqual(0o644, mode(outside))

    def test_a_state_directory_holding_other_files_keeps_its_own_mode(self) -> None:
        # A variable can name any directory; a person's own files in it are
        # not the accelerator's, and neither is closing the directory to them.
        self.state.mkdir(parents=True)
        self.state.chmod(0o755)
        own = self.state / "notes.md"
        own.write_text("A person's own file.\n", encoding="utf-8")
        own.chmod(0o644)

        result = self.run_with_umask("status", "--json")
        self.assertEqual(0, result.returncode, result.stderr)

        self.assertEqual(0o755, mode(self.state))
        self.assertEqual(0o644, mode(own))
        for name in ("project-brain", "memory-bank"):
            self.assertEqual(0o700, mode(self.state / name))
        self.assertEqual(0o600, mode(self.state / "memory-bank/local/context.db"))


class AttachedHooksTest(AttachedProject):
    """The edition's own hooks, run from the clone against the project."""

    def run_hook(self, name: str, payload: dict) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(EDITION / ".claude" / "hooks" / name)],
            input=json.dumps(payload),
            cwd=self.project,
            env={**self.environment, "CONTEXT_HOOK_BUDGET": "120"},
            capture_output=True,
            text=True,
            timeout=300,
        )

    def test_session_start_names_the_clone_and_the_state(self) -> None:
        result = self.run_hook("local-context.sh", {"hook_event_name": "SessionStart"})
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn(f"Accelerator: attached from {EDITION}", result.stdout)
        self.assertIn(str(self.state), result.stdout)
        self.assertIn("Branch: feature/saffron-checkout", result.stdout)
        self.assert_project_untouched()

    def test_prompt_and_stop_hooks_write_only_the_state(self) -> None:
        result = self.run_hook(
            "working-memory-read.sh", {"prompt": "Explain the saffron checkout flow"}
        )
        self.assertEqual(0, result.returncode, result.stderr)
        health = (self.state / "memory-bank/local/refresh-health.ndjson").read_text(
            encoding="utf-8"
        )
        self.assertIn('"hook_status":0', health)
        self.assert_project_untouched()

        (self.project / "app" / "Order.php").write_text(
            "<?php\n\nfinal class Order\n{\n    public int $total = 0;\n}\n",
            encoding="utf-8",
        )
        result = self.run_hook("working-memory-write.sh", {"hook_event_name": "Stop"})
        self.assertEqual(0, result.returncode, result.stderr)
        report = json.loads(
            (self.state / "memory-bank/local/last-turn-report.json").read_text(
                encoding="utf-8"
            )
        )
        # The task comes from the project's branch, not the clone's.
        self.assertEqual("feature/saffron-checkout", report["task_id"])
        self.assertEqual(1, report["files"])
        self.assertEqual(
            " M app/Order.php\n", git(self.project, "status", "--porcelain", "--ignored")
        )
        self.assertEqual(self.tooling_database_before, self._stat(self.tooling_database))

    @unittest.skipIf(os.name == "nt", "creating a symbolic link needs a privilege on Windows")
    def test_the_prompt_hook_writes_nothing_through_a_linked_state(self) -> None:
        # The runtime refuses such a state; the hook's own health line, which
        # it appends whatever the runtime answered, went through the link.
        for target in ("elsewhere", "project"):
            with self.subTest(target=target):
                pointed = self.state.parent / target if target == "elsewhere" else self.project
                pointed.mkdir(parents=True, exist_ok=True)
                self.state.parent.mkdir(parents=True, exist_ok=True)
                self.state.symlink_to(pointed, target_is_directory=True)
                try:
                    result = self.run_hook(
                        "working-memory-read.sh", {"prompt": "Explain the saffron checkout flow"}
                    )
                finally:
                    self.state.unlink()
                self.assertEqual(0, result.returncode, result.stderr)
                if target == "elsewhere":
                    self.assertEqual([], list(pointed.iterdir()))
                self.assert_project_untouched()


if __name__ == "__main__":
    unittest.main()
