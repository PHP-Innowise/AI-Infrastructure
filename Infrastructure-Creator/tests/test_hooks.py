#!/usr/bin/env python3
"""Regression tests for the Infrastructure-Creator agent hooks.

The three mirror directories (.claude/.cursor/.codex) ship byte-identical
hooks (a documented invariant of this edition), so behaviour is exercised
through the .claude copies and the identity itself is asserted separately.

Hooks are run exactly as the agent harness runs them: `bash <hook>` with a
JSON payload on stdin, asserting on exit code, stdout and stderr. All loop
state lives under a per-test TMPDIR, so the suite never touches real /tmp
counters and cleans up after itself.

Run from this directory: python3 -m unittest discover
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

EDITION_ROOT = Path(__file__).resolve().parents[1]
MIRROR_DIRS = (".claude/hooks", ".cursor/hooks", ".codex/hooks")
HOOK_NAMES = (
    "bash-validator.sh",
    "file-naming-validator.sh",
    "loop-detection.sh",
    "local-context.sh",
)
HOOK_TIMEOUT = 30


def hook_path(name: str) -> Path:
    return EDITION_ROOT / MIRROR_DIRS[0] / name


def run_hook(name: str, payload, cwd: Path | None = None, tmpdir: Path | None = None):
    """Run a hook the way the harness does: bash, JSON payload on stdin."""
    stdin = payload if isinstance(payload, str) else json.dumps(payload)
    env = dict(os.environ)
    if tmpdir is not None:
        env["TMPDIR"] = str(tmpdir)
    return subprocess.run(
        ["bash", str(hook_path(name))],
        input=stdin,
        capture_output=True,
        text=True,
        cwd=str(cwd) if cwd is not None else str(EDITION_ROOT),
        env=env,
        timeout=HOOK_TIMEOUT,
    )


def restricted_bin(base: Path, names: tuple[str, ...]) -> str:
    """A PATH directory carrying only the named binaries (symlinked)."""
    bindir = base / "bin"
    bindir.mkdir()
    for name in names:
        source = shutil.which(name)
        if source is None:
            raise unittest.SkipTest("required binary missing: {}".format(name))
        (bindir / name).symlink_to(source)
    return str(bindir)


def run_hook_restricted(name: str, payload, path_value: str):
    """Run a hook under a minimal PATH (regression: no external `cat`)."""
    bash = shutil.which("bash")
    if bash is None:
        raise unittest.SkipTest("bash not found")
    stdin = payload if isinstance(payload, str) else json.dumps(payload)
    env = dict(os.environ)
    env["PATH"] = path_value
    return subprocess.run(
        [bash, str(hook_path(name))],
        input=stdin,
        capture_output=True,
        text=True,
        cwd=str(EDITION_ROOT),
        env=env,
        timeout=HOOK_TIMEOUT,
    )


def repo_key(repo: Path) -> str:
    """repo_key exactly as the hooks compute it (git root or pwd -> cksum)."""
    result = subprocess.run(
        [
            "bash",
            "-c",
            'cd "$1" && printf %s "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"'
            " | cksum | cut -d' ' -f1",
            "bash",
            str(repo),
        ],
        capture_output=True,
        text=True,
        check=True,
        timeout=HOOK_TIMEOUT,
    )
    return result.stdout.strip()


def counter_files(directory: Path) -> list[Path]:
    if not directory.is_dir():
        return []
    return [path for path in directory.iterdir() if path.is_file()]


def edit_payload(path: str) -> dict:
    return {
        "tool_name": "Edit",
        "tool_input": {"file_path": path, "old_string": "a", "new_string": "b"},
    }


class FakeRepoMixin:
    """Two throwaway git repositories plus an isolated TMPDIR for state."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="infra-hook-tests-")
        base = Path(self._tmp.name)
        self.state_root = base / "state"
        self.state_root.mkdir()
        self.repo_a = base / "repo-a"
        self.repo_b = base / "repo-b"
        for repo in (self.repo_a, self.repo_b):
            repo.mkdir()
            subprocess.run(
                ["git", "-c", "init.defaultBranch=main", "init", "-q", str(repo)],
                check=True,
                capture_output=True,
                timeout=HOOK_TIMEOUT,
            )
        self.addCleanup(self._tmp.cleanup)

    def state_dir(self, repo: Path) -> Path:
        return self.state_root / "infra-creator-loopdetect-{}".format(repo_key(repo))


class BashValidatorTest(unittest.TestCase):
    HOOK = "bash-validator.sh"

    @staticmethod
    def payload(command: str) -> dict:
        return {"tool_name": "Bash", "tool_input": {"command": command}}

    def test_safe_command_passes(self) -> None:
        result = run_hook(self.HOOK, self.payload("ls -la src/"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")

    def test_destructive_commands_blocked_with_stderr(self) -> None:
        commands = (
            "git reset --hard HEAD~1",
            "git push --force origin main",
            "git commit --no-verify -m wip",
            'psql -c "DROP TABLE users;"',
        )
        for command in commands:
            with self.subTest(command=command):
                result = run_hook(self.HOOK, self.payload(command))
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("[bash-validator] BLOCKED", result.stderr)
                self.assertEqual(result.stdout, "")

    def test_escaped_quotes_survive_extraction(self) -> None:
        # The JSON wire format contains \" and \\\" sequences; extraction
        # must decode them instead of scraping up to the first quote.
        command = (
            'git commit -m "fix: handle \\"escaped\\" and'
            ' \\\\\\"double-escaped\\\\\\" quotes"'
        )
        result = run_hook(self.HOOK, self.payload(command))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")

    def test_nested_destructive_command_still_blocked(self) -> None:
        result = run_hook(self.HOOK, self.payload('bash -c "git reset --hard"'))
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("[bash-validator] BLOCKED", result.stderr)

    def test_command_text_in_foreign_tool_payload_is_ignored(self) -> None:
        # "command" rendered inside a *string* field of another tool's
        # payload is data, not a command to validate.
        payload = {
            "tool_name": "Write",
            "tool_input": {
                "file_path": "docs/hooks-notes.md",
                "content": 'JSON example: {"command": "git reset --hard"} must never run.',
            },
        }
        result = run_hook(self.HOOK, payload)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")

    def test_unparsable_stdin_fails_open_quietly(self) -> None:
        # Regression: the old sed fallback matched the whole raw input, so
        # any prose containing "git reset --hard" was falsely blocked.
        result = run_hook(self.HOOK, "prose mentioning git reset --hard, not JSON")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_payload_without_command_key_passes_quietly(self) -> None:
        # Codex and Cursor run this hook for every tool call (no matcher);
        # payloads that cannot carry a shell command must exit cleanly.
        payload = {"tool_name": "Read", "tool_input": {"file_path": "docs/a.md"}}
        result = run_hook(self.HOOK, payload)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "")

    def test_stdin_read_does_not_need_external_cat(self) -> None:
        # TC-030 regression: stdin is read with the bash builtin. With no
        # `cat` in PATH the validator used to lose the payload and silently
        # pass every command - including destructive ones.
        with tempfile.TemporaryDirectory(prefix="no-cat-") as tmp:
            path_value = restricted_bin(Path(tmp), ("grep", "python3"))
            result = run_hook_restricted(
                self.HOOK, self.payload("git reset --hard HEAD~1"), path_value
            )
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn("[bash-validator] BLOCKED", result.stderr)
            self.assertNotIn("command not found", result.stderr)

    def test_no_extractor_warning_survives_missing_cat(self) -> None:
        # TC-030 regression: with neither `cat` nor any JSON extractor in
        # PATH, the fail-open warning must still reach stderr instead of
        # being lost together with the unread payload.
        with tempfile.TemporaryDirectory(prefix="no-extractor-") as tmp:
            path_value = restricted_bin(Path(tmp), ("grep",))
            result = run_hook_restricted(
                self.HOOK, self.payload("git reset --hard HEAD~1"), path_value
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("no JSON extractor available", result.stderr)
            self.assertEqual(result.stdout, "")


class FileNamingValidatorTest(unittest.TestCase):
    HOOK = "file-naming-validator.sh"

    @staticmethod
    def payload(path: str, content: str = "hello", key: str = "file_path") -> dict:
        return {
            "tool_name": "Write",
            "tool_input": {key: path, "content": content},
        }

    def assert_allowed(self, payload) -> None:
        result = run_hook(self.HOOK, payload)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "")

    def assert_warned(self, payload) -> None:
        result = run_hook(self.HOOK, payload)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("[file-naming-validator] WARN", result.stderr)
        self.assertEqual(result.stdout, "")

    def test_valid_paths_pass(self) -> None:
        paths = (
            "tasks/infra-scan-project-profile.md",
            "specs/stack-scanner-findings.md",
            "tasks/README.md",
            "specs/MANIFEST.md",
            "notes/free-form-draft.md",
            "tasks/data.json",
        )
        for path in paths:
            with self.subTest(path=path):
                self.assert_allowed(self.payload(path))

    def test_invalid_names_warned(self) -> None:
        # This edition deliberately stays in WARN mode (exit 1).
        paths = (
            "tasks/Notes.md",
            "tasks/notes.md",
            "specs/UPPER-case-Spec.md",
        )
        for path in paths:
            with self.subTest(path=path):
                self.assert_warned(self.payload(path))

    def test_cursor_file_path_field_supported(self) -> None:
        self.assert_warned(self.payload("tasks/Bad.md", key="filePath"))
        self.assert_allowed(self.payload("tasks/infra-scan-ok.md", key="filePath"))

    def test_escaped_quotes_in_content_do_not_confuse_extraction(self) -> None:
        tricky = 'Example: say \\"hello\\" and mention "tasks/Bad.md" in prose.'
        self.assert_allowed(self.payload("tasks/infra-scan-notes.md", tricky))

    def test_payload_without_path_is_ignored(self) -> None:
        result = run_hook(self.HOOK, {"tool_name": "Write", "tool_input": {}})
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_read_only_tool_payload_is_ignored(self) -> None:
        # Codex registers this hook without a matcher, so a Read of a badly
        # named file must not trigger a naming warning.
        payload = {"tool_name": "Read", "tool_input": {"file_path": "tasks/Bad.md"}}
        self.assert_allowed(payload)


class LoopDetectionTest(FakeRepoMixin, unittest.TestCase):
    HOOK = "loop-detection.sh"
    EDITED = "/work/app/Example.php"

    def edit(self, repo: Path):
        return run_hook(
            self.HOOK, edit_payload(self.EDITED), cwd=repo, tmpdir=self.state_root
        )

    def test_warn_threshold(self) -> None:
        for count in range(1, 5):
            result = self.edit(self.repo_a)
            self.assertEqual(result.returncode, 0, "count={}".format(count))
            self.assertEqual(result.stdout, "")
            self.assertEqual(result.stderr, "")
        for count in (5, 6):
            result = self.edit(self.repo_a)
            self.assertEqual(result.returncode, 1, "count={}".format(count))
            self.assertIn("[loop-detection] WARN", result.stderr)
            self.assertEqual(result.stdout, "")

    def test_counters_namespaced_per_repository(self) -> None:
        dir_a = self.state_dir(self.repo_a)
        dir_b = self.state_dir(self.repo_b)
        self.assertNotEqual(dir_a, dir_b)
        for _ in range(4):
            self.edit(self.repo_a)
        result = self.edit(self.repo_a)
        self.assertEqual(result.returncode, 1)
        # A different repository starts from a clean counter even for the
        # very same edited file path.
        result = self.edit(self.repo_b)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertEqual(len(counter_files(dir_a)), 1)
        self.assertEqual(len(counter_files(dir_b)), 1)

    def test_counter_resets_after_quiet_window(self) -> None:
        for _ in range(5):
            result = self.edit(self.repo_a)
        self.assertEqual(result.returncode, 1)
        state_files = counter_files(self.state_dir(self.repo_a))
        self.assertEqual(len(state_files), 1)
        # Age the last-edit timestamp beyond the 120s window; the next edit
        # must start a fresh streak instead of warning again.
        state_files[0].write_text(
            "{}\n5\n".format(int(time.time()) - 300), encoding="utf-8"
        )
        result = self.edit(self.repo_a)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")

    def test_missing_file_path_is_ignored(self) -> None:
        result = run_hook(
            self.HOOK,
            {"tool_name": "Edit", "tool_input": {}},
            cwd=self.repo_a,
            tmpdir=self.state_root,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.state_dir(self.repo_a).exists())

    def test_read_only_tool_payload_with_file_path_does_not_count(self) -> None:
        # Codex registers this hook without a matcher, so read-only payloads
        # that carry a file_path (for example Read) used to advance the edit
        # counter and could reach the warn threshold without any edit.
        result = run_hook(
            self.HOOK,
            {"tool_name": "Read", "tool_input": {"file_path": self.EDITED}},
            cwd=self.repo_a,
            tmpdir=self.state_root,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertFalse(self.state_dir(self.repo_a).exists())


class LocalContextTest(FakeRepoMixin, unittest.TestCase):
    HOOK = "local-context.sh"

    def test_banner_and_exit_code(self) -> None:
        result = run_hook(self.HOOK, "", cwd=self.repo_a, tmpdir=self.state_root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Infrastructure-Creator", result.stdout)

    def test_session_start_resets_only_own_repo_counters(self) -> None:
        for repo in (self.repo_a, self.repo_b):
            primed = run_hook(
                "loop-detection.sh",
                edit_payload("/work/App.php"),
                cwd=repo,
                tmpdir=self.state_root,
            )
            self.assertEqual(primed.returncode, 0, primed.stderr)
        dir_a = self.state_dir(self.repo_a)
        dir_b = self.state_dir(self.repo_b)
        self.assertEqual(len(counter_files(dir_a)), 1)
        self.assertEqual(len(counter_files(dir_b)), 1)

        result = run_hook(self.HOOK, "", cwd=self.repo_a, tmpdir=self.state_root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(counter_files(dir_a), [])
        self.assertEqual(len(counter_files(dir_b)), 1)


class MirrorConsistencyTest(unittest.TestCase):
    def test_all_hooks_byte_identical_across_mirrors(self) -> None:
        # Documented invariant: the .claude/.cursor/.codex editions of every
        # Infrastructure-Creator hook are identical.
        for name in HOOK_NAMES:
            reference = (EDITION_ROOT / MIRROR_DIRS[0] / name).read_bytes()
            for mirror in MIRROR_DIRS[1:]:
                with self.subTest(hook=name, mirror=mirror):
                    self.assertEqual(
                        reference, (EDITION_ROOT / mirror / name).read_bytes()
                    )


if __name__ == "__main__":
    unittest.main()
