#!/usr/bin/env python3
"""Regression tests for the agent hooks bundled with this edition.

This file is byte-identical across the Laravel, Symfony and PHP Core
editions: it locates the edition root relative to itself and derives every
edition-specific fixture (hook paths, skill prefixes) from the tree, so it
never mentions an edition by name.

Hooks are exercised exactly as the agent harness runs them: `bash <hook>`
with a JSON payload on stdin, asserting on exit code, stdout and stderr.

Run from this directory: python3 -m unittest discover
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from unittest import mock

EDITION_ROOT = Path(__file__).resolve().parents[2]
BASH = shutil.which("bash") or "/bin/bash"

# tool id -> (hooks directory, skills directory, /tmp loop-counter prefix)
MIRRORS = {
    "claude": (".claude/hooks", ".claude/skills", "claude"),
    "cursor": (".cursor/hooks", ".cursor/skills", "cursor"),
    "codex": (".codex/hooks", ".agents/skills", "codex"),
}

HOOK_TIMEOUT = 30


def hook_path(tool: str, name: str) -> Path:
    return EDITION_ROOT / MIRRORS[tool][0] / name


def run_hook(
    tool: str,
    name: str,
    payload,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
):
    """Run a hook the way the harness does: bash, JSON payload on stdin."""
    stdin = payload if isinstance(payload, str) else json.dumps(payload)
    merged_env = None
    if env:
        merged_env = dict(os.environ)
        merged_env.update(env)
    return subprocess.run(
        [BASH, str(hook_path(tool, name))],
        input=stdin,
        capture_output=True,
        text=True,
        cwd=str(cwd) if cwd is not None else str(EDITION_ROOT),
        env=merged_env,
        timeout=HOOK_TIMEOUT,
    )


def shell_line(script: str, *args: str) -> str:
    result = subprocess.run(
        ["bash", "-c", script, "bash", *args],
        capture_output=True,
        text=True,
        check=True,
        timeout=HOOK_TIMEOUT,
    )
    return result.stdout.strip()


def repo_key(repo: Path) -> str:
    """REPO_KEY exactly as the hooks compute it (git root or pwd -> cksum)."""
    return shell_line(
        'cd "$1" && printf %s "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"'
        " | cksum | cut -d' ' -f1",
        str(repo),
    )


def track_dir(tool: str, repo: Path, base: str | None = None) -> Path:
    """The per-user counter directory loop-detection and the guard share.

    Under TMPDIR (else /tmp), named for the host, the user and the repo key;
    `base` stands for a TMPDIR the hook was given explicitly.
    """
    root = (base if base is not None else os.environ.get("TMPDIR") or "/tmp").rstrip("/")
    return Path("{}/{}-loop-detection-{}-{}".format(
        root, MIRRORS[tool][2], os.geteuid(), repo_key(repo)
    ))


def warning_context(result) -> str:
    """The warning a Claude Code or Codex hook hands the model.

    Both hosts add `hookSpecificOutput.additionalContext` to the model's
    context next to the tool result; a non-blocking exit code with text on
    stderr reaches only the user. Cursor has no such channel for these events
    and keeps the user-visible form: text and exit 1.
    """
    output = json.loads(result.stdout)["hookSpecificOutput"]
    return output["additionalContext"]


def patch_payload(*paths: str, session: str | None = None) -> dict:
    """A Codex apply_patch call: the files are named inside the patch text."""
    body = "".join(
        "*** Update File: {}\n@@\n-old\n+new\n".format(path) for path in paths
    )
    payload = {
        "tool_name": "apply_patch",
        "tool_input": {"command": "*** Begin Patch\n" + body + "*** End Patch\n"},
    }
    if session:
        payload["session_id"] = session
    return payload


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
    """Two throwaway git repositories with isolated loop counters."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="hook-tests-")
        base = Path(self._tmp.name)
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
        self.addCleanup(self._cleanup_state)

    def _cleanup_state(self) -> None:
        for tool in MIRRORS:
            for repo in (self.repo_a, self.repo_b):
                directory = track_dir(tool, repo)
                if directory.is_symlink():
                    directory.unlink()
                shutil.rmtree(directory, ignore_errors=True)
        self._tmp.cleanup()


class BashValidatorTest(FakeRepoMixin, unittest.TestCase):
    HOOK = "bash-validator.sh"

    # Commands that reach the repetition guard run against a throwaway repo:
    # its counters are keyed by repo root and removed on cleanup, so one suite
    # run cannot inherit the previous run's counts and start warning.

    @staticmethod
    def payload(command: str) -> dict:
        return {"tool_name": "Bash", "tool_input": {"command": command}}

    def test_safe_command_passes(self) -> None:
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(
                    tool, self.HOOK, self.payload("ls -la src/"), cwd=self.repo_a
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, "")

    def test_destructive_commands_blocked_with_stderr(self) -> None:
        # Only patterns present in every edition's block list are used here.
        commands = (
            "git reset --hard HEAD~1",
            "git commit --no-verify -m wip",
            'mysql -e "DROP TABLE users;"',
        )
        for tool in MIRRORS:
            for command in commands:
                with self.subTest(tool=tool, command=command):
                    result = run_hook(tool, self.HOOK, self.payload(command))
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertIn("BLOCKED", result.stderr)
                    self.assertEqual(result.stdout, "")

    def test_escaped_quotes_survive_extraction(self) -> None:
        # The JSON wire format contains \" and \\\" sequences; the old sed
        # scraper stopped at the first escaped quote. A safe command must not
        # be blocked because of how its quotes are encoded.
        command = (
            'git commit -m "fix: handle \\"escaped\\" and'
            ' \\\\\\"double-escaped\\\\\\" quotes"'
        )
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(
                    tool, self.HOOK, self.payload(command), cwd=self.repo_a
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, "")

    def test_repeated_identical_command_warns_then_blocks(self) -> None:
        # A command loop touches no file, so loop-detection.sh cannot see it.
        # Counting is per exact command string: a changed command starts over.
        command = self.payload("vendor/bin/phpunit --filter Broken")
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                results = [
                    run_hook(tool, self.HOOK, command, cwd=self.repo_a)
                    for _ in range(11)
                ]
                self.assertEqual(
                    [(0, "", "")] * 5,
                    [(r.returncode, r.stdout, r.stderr) for r in results[:5]],
                )
                for count, result in enumerate(results[5:], start=6):
                    expected = "WARNING: this exact command has run {} times".format(count)
                    if tool == "cursor":
                        # No channel to the agent for an allowed shell
                        # command: the warning is the user's.
                        self.assertEqual((1, ""), (result.returncode, result.stdout))
                        self.assertIn(expected, result.stderr)
                    else:
                        # Exit 0 with additionalContext: the model sees it.
                        self.assertEqual((0, ""), (result.returncode, result.stderr))
                        self.assertIn(expected, warning_context(result))
                blocked = run_hook(tool, self.HOOK, command, cwd=self.repo_a)
                self.assertEqual(2, blocked.returncode)
                self.assertIn("BLOCKED", blocked.stderr)
                # Codex has no slash commands: its mirror names the skill.
                self.assertIn(
                    "systematic-debugger" if tool == "codex" else "/debugger",
                    blocked.stderr,
                )
                fresh = run_hook(
                    tool,
                    self.HOOK,
                    self.payload("vendor/bin/phpunit --filter Other"),
                    cwd=self.repo_a,
                )
                self.assertEqual(0, fresh.returncode, fresh.stderr)

    def test_a_file_edit_restarts_the_repetition_count(self) -> None:
        # Edit-and-rerun is progress: the guard counts reruns with nothing
        # changed, so an edit loop-detection.sh records starts the count over.
        edit = edit_payload("/work/app/Fixing.php")
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                self.assert_edit_restarts_the_count(
                    tool, edit, "vendor/bin/phpunit --filter Fixing"
                )

    def assert_edit_restarts_the_count(self, tool: str, edit: dict, command: str) -> None:
        payload = self.payload(command)
        codes = [
            run_hook(tool, self.HOOK, payload, cwd=self.repo_a).returncode
            for _ in range(11)
        ]
        self.assertEqual(1 if tool == "cursor" else 0, codes[-1])
        time.sleep(0.05)  # file timestamps are coarser than a hook run
        recorded = run_hook(tool, "loop-detection.sh", edit, cwd=self.repo_a)
        self.assertEqual(0, recorded.returncode, recorded.stderr)
        rerun = run_hook(tool, self.HOOK, payload, cwd=self.repo_a)
        self.assertEqual((0, "", ""), (rerun.returncode, rerun.stdout, rerun.stderr))

    def test_a_codex_apply_patch_restarts_the_repetition_count(self) -> None:
        # Codex edits through apply_patch: no file_path, the files are named
        # inside the patch text. The hook used to record nothing, so a
        # fix-and-rerun loop on Codex was refused from its twelfth run.
        self.assert_edit_restarts_the_count(
            "codex", patch_payload("app/Fixing.php"), "vendor/bin/phpunit --filter Patched"
        )

    def test_a_claude_write_or_notebook_edit_restarts_the_repetition_count(self) -> None:
        # The Claude hook is wired for every edit tool (see HookWiringTest),
        # and a notebook edit names its file notebook_path.
        for name, edit in (
            ("write", {"tool_name": "Write", "tool_input": {"file_path": "/work/app/W.php", "content": "x"}}),
            ("notebook", {"tool_name": "NotebookEdit", "tool_input": {"notebook_path": "/work/n.ipynb", "new_source": "x"}}),
        ):
            with self.subTest(edit=name):
                self.assert_edit_restarts_the_count(
                    "claude", edit, "vendor/bin/phpunit --filter " + name
                )

    def test_polling_a_status_query_is_not_a_loop(self) -> None:
        # A read-only status query changes its answer without any edit:
        # asking again is waiting, not a rerun of a failing command.
        for tool in MIRRORS:
            for command in (
                "gh pr checks 44",
                "sleep 30 && gh run view 123 --log-failed | tail -n 50",
                "git status",
                "docker compose logs --tail 20 app",
            ):
                with self.subTest(tool=tool, command=command):
                    results = [
                        run_hook(tool, self.HOOK, self.payload(command), cwd=self.repo_a)
                        for _ in range(13)
                    ]
                    self.assertEqual(
                        [(0, "", "")] * 13,
                        [(r.returncode, r.stdout, r.stderr) for r in results],
                    )
            with self.subTest(tool=tool, command="chained"):
                # Chained with anything else, it counts like any command.
                chained = self.payload("gh pr checks 44 && vendor/bin/phpunit")
                codes = [
                    run_hook(tool, self.HOOK, chained, cwd=self.repo_a).returncode
                    for _ in range(12)
                ]
                self.assertEqual(2, codes[-1])

    def test_sessions_keep_their_own_counts(self) -> None:
        # Two sessions in one checkout: neither adds to the other's count, and
        # a third session's start clears only its own counters. TMPDIR keeps
        # the session start's validation cache out of the shared one.
        base = Path(self._tmp.name) / "tmpdir"
        base.mkdir()
        env = {"TMPDIR": str(base)}

        def run(tool: str, session: str):
            payload = dict(self.payload("vendor/bin/phpunit --filter Shared"), session_id=session)
            return run_hook(tool, self.HOOK, payload, cwd=self.repo_a, env=env)

        for tool in MIRRORS:
            with self.subTest(tool=tool):
                for _ in range(5):
                    self.assertEqual(0, run(tool, "session-a").returncode)
                second = [run(tool, "session-b") for _ in range(5)]
                self.assertEqual(
                    [(0, "", "")] * 5,
                    [(r.returncode, r.stdout, r.stderr) for r in second],
                )
                started = run_hook(
                    tool, "local-context.sh", {"session_id": "session-c", "hook_event_name": "SessionStart"},
                    cwd=self.repo_a, env=env,
                )
                self.assertEqual(0, started.returncode, started.stderr)
                sixth = run(tool, "session-a")
                self.assertEqual(1 if tool == "cursor" else 0, sixth.returncode)
                self.assertIn("has run 6 times", sixth.stdout + sixth.stderr)

    def test_counters_stay_private_and_are_never_followed_or_evaluated(self) -> None:
        # The directory name is predictable: the hook makes it private, never
        # follows a link in place of a counter, and reads a count in base 10.
        victim = self.repo_b / "victim.txt"
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                victim.write_text("important\n", encoding="utf-8")
                first = run_hook(tool, self.HOOK, self.payload("composer test"), cwd=self.repo_a)
                self.assertEqual(0, first.returncode, first.stderr)
                directory = track_dir(tool, self.repo_a)
                self.assertEqual(0o700, directory.stat().st_mode & 0o777)
                (counter,) = [p for p in counter_files(directory) if p.name.startswith("cmd-")]
                counter.unlink()
                counter.symlink_to(victim)
                linked = run_hook(tool, self.HOOK, self.payload("composer test"), cwd=self.repo_a)
                self.assertEqual((0, ""), (linked.returncode, linked.stderr))
                self.assertEqual("important\n", victim.read_text(encoding="utf-8"))
                self.assertTrue(counter.is_symlink())
                counter.unlink()
                # A leading zero is not octal, and a count is never shell code.
                counter.write_text("08\n", encoding="utf-8")
                ninth = run_hook(tool, self.HOOK, self.payload("composer test"), cwd=self.repo_a)
                self.assertIn("has run 9 times", ninth.stdout + ninth.stderr)
                marker = self.repo_b / "evaluated"
                counter.write_text("a[$(touch {})]\n".format(marker), encoding="utf-8")
                run_hook(tool, self.HOOK, self.payload("composer test"), cwd=self.repo_a)
                self.assertFalse(marker.exists())
                self.assertEqual("1\n", counter.read_text(encoding="utf-8"))

    def test_a_linked_counter_directory_turns_the_guard_off(self) -> None:
        # Another user's directory (or a link) under the predictable name
        # could pre-seed a count that blocks a first run; the hook does not
        # trust it.
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                planted = self.repo_b / "planted-{}".format(tool)
                planted.mkdir()
                directory = track_dir(tool, self.repo_a)
                shutil.rmtree(directory, ignore_errors=True)
                directory.symlink_to(planted)
                try:
                    for _ in range(12):
                        result = run_hook(
                            tool, self.HOOK, self.payload("composer test"), cwd=self.repo_a
                        )
                        self.assertEqual((0, "", ""), (result.returncode, result.stdout, result.stderr))
                    self.assertEqual([], list(planted.iterdir()))
                finally:
                    directory.unlink()

    def test_nested_destructive_command_still_blocked(self) -> None:
        # The full nested command must survive extraction so the pattern
        # scan still sees the destructive part behind the escaped quotes.
        command = 'bash -c "git reset --hard"'
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(tool, self.HOOK, self.payload(command))
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("BLOCKED", result.stderr)

    def test_command_text_in_foreign_tool_payload_is_ignored(self) -> None:
        # "command" appearing inside a *string* field of another tool's
        # payload is data, not a command; the sed scraper used to match it.
        payload = {
            "tool_name": "Write",
            "tool_input": {
                "file_path": "docs/hooks-notes.md",
                "content": 'JSON example: {"command": "git reset --hard"} must never run.',
            },
        }
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(tool, self.HOOK, payload)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, "")

    def test_unparsable_stdin_fails_open_quietly(self) -> None:
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(
                    tool, self.HOOK, "prose mentioning git reset --hard, not JSON"
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "")

    def test_payload_without_command_key_passes_quietly(self) -> None:
        # Codex and Cursor run this hook for every tool call (no matcher);
        # payloads that cannot carry a shell command must exit cleanly.
        payload = {"tool_name": "Read", "tool_input": {"file_path": "docs/a.md"}}
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(tool, self.HOOK, payload)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertEqual(result.stderr, "")

    def test_empty_input_passes_quietly(self) -> None:
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(tool, self.HOOK, "")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertEqual(result.stderr, "")

    def test_multiline_payload_is_consumed_completely(self) -> None:
        payload = json.dumps(
            {"tool_name": "Bash", "tool_input": {"nested": {"command": "git reset --hard"}}},
            indent=2,
        )
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(tool, self.HOOK, payload)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("rule category: destructive command", result.stderr)

    def test_no_cat_or_json_extractor_warns_once_and_fails_open(self) -> None:
        secret_command = "git reset --hard SECRET-COMMAND-BODY"
        expected = (
            "bash-validator: no JSON extractor available "
            "(jq/php/python3), validation skipped"
        )
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(
                    tool,
                    self.HOOK,
                    json.dumps(self.payload(secret_command), indent=2),
                    env={"PATH": ""},
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertEqual(result.stderr.splitlines(), [expected])
                self.assertNotIn(secret_command, result.stderr)
                self.assertNotIn("SECRET-COMMAND-BODY", result.stderr)

    def test_block_diagnostic_never_leaks_command_body(self) -> None:
        secret = "DO-NOT-PRINT-THIS-BODY"
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(
                    tool, self.HOOK, self.payload("git reset --hard " + secret)
                )
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("rule category: destructive command", result.stderr)
                self.assertNotIn(secret, result.stderr)


class FileNamingValidatorTest(unittest.TestCase):
    HOOK = "file-naming-validator.sh"

    @staticmethod
    def skill_prefix(tool: str) -> str:
        skills = EDITION_ROOT / MIRRORS[tool][1]
        names = sorted(entry.name for entry in skills.iterdir() if entry.is_dir())
        if not names:
            raise AssertionError("no skills discovered under {}".format(skills))
        return names[0]

    @staticmethod
    def payload(path: str, content: str = "hello") -> dict:
        return {
            "tool_name": "Write",
            "tool_input": {"file_path": path, "content": content},
        }

    def assert_allowed(self, tool: str, payload) -> None:
        result = run_hook(tool, self.HOOK, payload)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "")

    def assert_blocked(self, tool: str, payload) -> None:
        result = run_hook(tool, self.HOOK, payload)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("BLOCKED", result.stderr)
        self.assertEqual(result.stdout, "")

    def test_valid_paths_pass(self) -> None:
        for tool in MIRRORS:
            skill = self.skill_prefix(tool)
            paths = (
                "tasks/TASK-001/{}-notes.md".format(skill),
                str(EDITION_ROOT / "tasks" / "TASK-002" / "{}-plan.md".format(skill)),
                "tasks/README.md",
                "specs/MANIFEST.md",
                "memory-bank/chunks/MEM-0001-first-note.md",
                "docs/free-form-notes.md",
                "src/Example.php",
            )
            for path in paths:
                with self.subTest(tool=tool, path=path):
                    self.assert_allowed(tool, self.payload(path))

    def test_invalid_paths_blocked(self) -> None:
        paths = (
            "tasks/bad-file.md",
            str(EDITION_ROOT / "tasks" / "bad-file.md"),
            "tasks/TASK-001/0-unprefixed-note.md",
            "tasks/loose-dir/0-unprefixed-note.md",
            "specs/0-unprefixed-spec.md",
            "memory-bank/chunks/unprefixed-note.md",
        )
        for tool in MIRRORS:
            for path in paths:
                with self.subTest(tool=tool, path=path):
                    self.assert_blocked(tool, self.payload(path))

    def test_escaped_quotes_in_content_do_not_confuse_extraction(self) -> None:
        # Content is a string; a "file_path" key rendered *inside* it (with
        # escaped quotes on the wire) must not be treated as a real path.
        tricky = (
            'Example payload: {"file_path": "tasks/bad-file.md"}'
            ' plus a literal \\" backslash-quote pair.'
        )
        for tool in MIRRORS:
            skill = self.skill_prefix(tool)
            with self.subTest(tool=tool, case="valid-path"):
                self.assert_allowed(
                    tool,
                    self.payload("tasks/TASK-001/{}-notes.md".format(skill), tricky),
                )
            with self.subTest(tool=tool, case="invalid-path"):
                self.assert_blocked(tool, self.payload("tasks/bad-file.md", tricky))

    def test_patch_add_file_paths_are_validated(self) -> None:
        def patch_payload(marker: str, path: str) -> dict:
            patch = "*** Begin Patch\n*** {}: {}\n+hello\n*** End Patch\n".format(
                marker, path
            )
            return {"tool_name": "apply_patch", "tool_input": {"patch": patch}}

        for tool in MIRRORS:
            skill = self.skill_prefix(tool)
            with self.subTest(tool=tool, case="add-file-blocked"):
                self.assert_blocked(tool, patch_payload("Add File", "tasks/bad-file.md"))
            with self.subTest(tool=tool, case="update-file-blocked"):
                self.assert_blocked(
                    tool, patch_payload("Update File", "tasks/bad-file.md")
                )
            with self.subTest(tool=tool, case="add-file-allowed"):
                self.assert_allowed(
                    tool,
                    patch_payload(
                        "Add File", "tasks/TASK-003/{}-report.md".format(skill)
                    ),
                )

    def test_non_edit_tool_payloads_ignored(self) -> None:
        payload = {
            "tool_name": "Read",
            "tool_input": {"file_path": "tasks/bad-file.md"},
        }
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                self.assert_allowed(tool, payload)


class LoopDetectionTest(FakeRepoMixin, unittest.TestCase):
    HOOK = "loop-detection.sh"
    EDITED = "/work/app/Example.php"

    def edit(self, tool: str, repo: Path):
        return run_hook(tool, self.HOOK, edit_payload(self.EDITED), cwd=repo)

    def test_warn_and_block_thresholds(self) -> None:
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                for count in range(1, 7):
                    result = self.edit(tool, self.repo_a)
                    self.assertEqual(result.returncode, 0, "count={}".format(count))
                    self.assertEqual(result.stdout, "")
                    self.assertEqual(result.stderr, "")
                for count in range(7, 10):
                    result = self.edit(tool, self.repo_a)
                    expected = "edited {} times this session".format(count)
                    self.assertEqual(result.stderr, "")
                    if tool == "cursor":
                        # afterFileEdit has no channel back to the agent.
                        self.assertEqual(result.returncode, 1, "count={}".format(count))
                        self.assertIn("WARNING", result.stdout)
                        self.assertIn(expected, result.stdout)
                    else:
                        # Exit 0 with additionalContext: the model sees it.
                        self.assertEqual(result.returncode, 0, "count={}".format(count))
                        self.assertIn(expected, warning_context(result))
                result = self.edit(tool, self.repo_a)
                self.assertEqual(result.returncode, 2)
                self.assertIn("BLOCKED", result.stderr)
                self.assertEqual(result.stdout, "")

    def test_every_file_of_a_codex_patch_is_counted(self) -> None:
        # apply_patch names its files inside the patch text, one marker line
        # each; every file the patch touches is an edit of that file.
        payload = patch_payload("app/A.php", "app/B.php")
        payload["tool_input"]["command"] += "*** Add File: app/C.php\n+new\n"
        for _ in range(6):
            result = run_hook("codex", self.HOOK, payload, cwd=self.repo_a)
            self.assertEqual((0, "", ""), (result.returncode, result.stdout, result.stderr))
        self.assertEqual(3, len(counter_files(track_dir("codex", self.repo_a))))
        seventh = run_hook("codex", self.HOOK, payload, cwd=self.repo_a)
        self.assertEqual(0, seventh.returncode, seventh.stderr)
        context = warning_context(seventh)
        for name in ("app/A.php", "app/B.php", "app/C.php"):
            self.assertIn("File '{}' edited 7 times".format(name), context)

    def test_counts_are_per_session(self) -> None:
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                for _ in range(9):
                    run_hook(tool, self.HOOK, dict(edit_payload(self.EDITED), session_id="one"), cwd=self.repo_a)
                other = run_hook(tool, self.HOOK, dict(edit_payload(self.EDITED), session_id="two"), cwd=self.repo_a)
                self.assertEqual((0, "", ""), (other.returncode, other.stdout, other.stderr))
                tenth = run_hook(tool, self.HOOK, dict(edit_payload(self.EDITED), session_id="one"), cwd=self.repo_a)
                self.assertEqual(2, tenth.returncode)

    def test_a_planted_count_is_never_evaluated(self) -> None:
        marker = self.repo_b / "evaluated"
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                self.assertEqual(0, self.edit(tool, self.repo_a).returncode)
                (counter,) = counter_files(track_dir(tool, self.repo_a))
                counter.write_text("a[$(touch {})]\n".format(marker), encoding="utf-8")
                self.assertEqual(0, self.edit(tool, self.repo_a).returncode)
                self.assertFalse(marker.exists())
                counter.write_text("08\n", encoding="utf-8")
                ninth = self.edit(tool, self.repo_a)
                self.assertIn("edited 9 times", ninth.stdout)

    def test_counters_namespaced_per_repository(self) -> None:
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                dir_a = track_dir(tool, self.repo_a)
                dir_b = track_dir(tool, self.repo_b)
                self.assertNotEqual(dir_a, dir_b)
                for _ in range(9):
                    self.edit(tool, self.repo_a)
                # A different repository starts from a clean counter even for
                # the very same edited file path.
                result = self.edit(tool, self.repo_b)
                self.assertEqual(result.returncode, 0, result.stdout)
                self.assertEqual(result.stdout, "")
                self.assertEqual(len(counter_files(dir_a)), 1)
                self.assertEqual(len(counter_files(dir_b)), 1)
                # And the first repository still blocks on its own tenth edit.
                result = self.edit(tool, self.repo_a)
                self.assertEqual(result.returncode, 2)
                self.assertIn("BLOCKED", result.stderr)

    def test_missing_file_path_is_ignored(self) -> None:
        payload = {"tool_name": "Edit", "tool_input": {}}
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(tool, self.HOOK, payload, cwd=self.repo_a)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertFalse(track_dir(tool, self.repo_a).exists())

    def test_path_field_honoured_only_by_cursor_and_codex(self) -> None:
        payload = {"tool_name": "Edit", "tool_input": {"path": self.EDITED}}
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(tool, self.HOOK, payload, cwd=self.repo_a)
                self.assertEqual(result.returncode, 0, result.stderr)
                expected = 0 if tool == "claude" else 1
                self.assertEqual(
                    len(counter_files(track_dir(tool, self.repo_a))), expected
                )

    def test_read_only_tool_payload_with_file_path_does_not_count(self) -> None:
        # Codex registers this hook without a matcher, so read-only payloads
        # that carry a file_path (for example Read) used to advance the edit
        # counter and could reach the warn/block thresholds without any edit.
        payload = {"tool_name": "Read", "tool_input": {"file_path": self.EDITED}}
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(tool, self.HOOK, payload, cwd=self.repo_a)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertEqual(counter_files(track_dir(tool, self.repo_a)), [])


class LocalContextTest(FakeRepoMixin, unittest.TestCase):
    HOOK = "local-context.sh"

    def setUp(self) -> None:
        super().setUp()
        # The validation cache honours TMPDIR, so each test tool gets an
        # isolated cache root instead of touching the real shared one.
        self._cache_tmp = tempfile.TemporaryDirectory(prefix="hook-cache-")
        self.cache_root = Path(self._cache_tmp.name)
        self.addCleanup(self._cache_tmp.cleanup)

    @staticmethod
    def edition_key() -> str:
        # EDITION_KEY exactly as the hook computes it (edition root -> cksum).
        return shell_line(
            'printf %s "$1" | cksum | cut -d" " -f1', str(EDITION_ROOT)
        )

    def cache_entries(self, tmp_root: Path, kind: str) -> list[Path]:
        cache_dir = tmp_root / "ai-accelerator-session-cache-{}".format(
            self.edition_key()
        )
        if not cache_dir.is_dir():
            return []
        return sorted(
            path
            for path in cache_dir.iterdir()
            if path.name.startswith("{}-".format(kind))
        )

    def test_validation_results_cached_per_repository_state(self) -> None:
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                tmp_root = self.cache_root / tool
                tmp_root.mkdir(exist_ok=True)
                env = {"TMPDIR": str(tmp_root)}

                first = run_hook(tool, self.HOOK, "", cwd=self.repo_a, env=env)
                self.assertEqual(first.returncode, 0, first.stderr)
                self.assertIn("brain-validation=", first.stdout)
                self.assertEqual(len(self.cache_entries(tmp_root, "brain-validation")), 1)

                # Unchanged repository state: the cached result is served and
                # the banner is byte-identical.
                second = run_hook(tool, self.HOOK, "", cwd=self.repo_a, env=env)
                self.assertEqual(second.returncode, 0, second.stderr)
                self.assertEqual(second.stdout, first.stdout)
                self.assertEqual(len(self.cache_entries(tmp_root, "brain-validation")), 1)

                # Any working-tree change (here: a new untracked file) must
                # produce a new cache key, not serve the stale entry.
                marker = self.repo_a / "invalidate-cache.txt"
                marker.write_text("state change", encoding="utf-8")
                try:
                    third = run_hook(tool, self.HOOK, "", cwd=self.repo_a, env=env)
                finally:
                    marker.unlink()
                self.assertEqual(third.returncode, 0, third.stderr)
                self.assertEqual(len(self.cache_entries(tmp_root, "brain-validation")), 2)

    def test_session_start_resets_only_own_repo_counters(self) -> None:
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                tmp_root = self.cache_root / tool
                tmp_root.mkdir(exist_ok=True)
                env = {"TMPDIR": str(tmp_root)}
                for repo in (self.repo_a, self.repo_b):
                    primed = run_hook(
                        tool, "loop-detection.sh", edit_payload("/work/App.php"), cwd=repo, env=env
                    )
                    self.assertEqual(primed.returncode, 0, primed.stderr)
                dir_a = track_dir(tool, self.repo_a, str(tmp_root))
                dir_b = track_dir(tool, self.repo_b, str(tmp_root))
                self.assertEqual(len(counter_files(dir_a)), 1)
                self.assertEqual(len(counter_files(dir_b)), 1)

                result = run_hook(tool, self.HOOK, "", cwd=self.repo_a, env=env)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(counter_files(dir_a), [])
                self.assertEqual(len(counter_files(dir_b)), 1)

    def test_session_start_keeps_a_running_sessions_counters(self) -> None:
        # A new session in the same checkout starts its own counts; it must
        # not wipe those of a session that is still working there. Counters
        # left untouched for a day are pruned whoever owns them.
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                tmp_root = self.cache_root / tool
                tmp_root.mkdir(exist_ok=True)
                env = {"TMPDIR": str(tmp_root)}
                for session in ("running", "stale", "starting"):
                    primed = run_hook(
                        tool, "loop-detection.sh",
                        dict(edit_payload("/work/App.php"), session_id=session),
                        cwd=self.repo_a, env=env,
                    )
                    self.assertEqual(primed.returncode, 0, primed.stderr)
                directory = track_dir(tool, self.repo_a, str(tmp_root))
                (stale,) = [p for p in counter_files(directory) if p.name.startswith("edit-stale-")]
                old = time.time() - 2 * 86400
                os.utime(stale, (old, old))
                result = run_hook(
                    tool, self.HOOK, {"session_id": "starting", "hook_event_name": "SessionStart"},
                    cwd=self.repo_a, env=env,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    ["running"], [p.name.split("-")[1] for p in counter_files(directory)]
                )


class WorkingMemoryRuleTest(unittest.TestCase):
    """Cursor's read path: hooks render the capsule into an alwaysApply rule.

    Cursor's prompt-time event cannot add context to a prompt, so its mirrors
    of the prompt, Stop and sessionStart hooks render the Task Capsule into
    .cursor/rules/working-memory.mdc instead (a MIRROR_RULES transformation,
    not drift). The rendered file is transient local state and must stay out
    of Git history.
    """

    RULE_RELATIVE = ".cursor/rules/working-memory.mdc"
    RENDER_HOOKS = ("working-memory-write.sh", "local-context.sh")

    def test_rendered_rule_is_gitignored(self) -> None:
        gitignore = (EDITION_ROOT / ".gitignore").read_text(encoding="utf-8")
        self.assertIn(self.RULE_RELATIVE, gitignore.splitlines())

    def test_only_cursor_mirrors_render_the_rule(self) -> None:
        for hook in self.RENDER_HOOKS:
            with self.subTest(hook=hook):
                cursor_text = hook_path("cursor", hook).read_text(encoding="utf-8")
                self.assertIn("working-memory.mdc", cursor_text)
                self.assertIn("alwaysApply: true", cursor_text)
                self.assertIn("current-branch session context", cursor_text)
                for tool in ("claude", "codex"):
                    text = hook_path(tool, hook).read_text(encoding="utf-8")
                    self.assertNotIn("working-memory.mdc", text)

    def test_automatic_retrieval_attributes_the_real_host(self) -> None:
        calls = {
            "claude": ("working-memory-read.sh",),
            "codex": ("working-memory-read.sh",),
            "cursor": ("working-memory-read.sh", *self.RENDER_HOOKS),
        }
        for tool, hooks in calls.items():
            for hook in hooks:
                with self.subTest(tool=tool, hook=hook):
                    logical = hook_path(tool, hook).read_text(encoding="utf-8").replace(
                        "\\\n", " "
                    )
                    self.assertIn(f"--host {tool}", logical)

    def test_render_carries_no_per_turn_invalidator(self) -> None:
        """The rule is re-sent on every prompt, so it may not embed a clock.

        A timestamp - or the serialized capsule, whose `manifest` key is a
        fresh UUID path per call - changes the rule every turn even when the
        context is identical, and is paid again on each of them.

        Only the capsule command is inspected: `--json` on the unrelated
        `status` and `validate` calls costs nothing per turn.
        """
        for hook in self.RENDER_HOOKS:
            with self.subTest(hook=hook):
                text = hook_path("cursor", hook).read_text(encoding="utf-8")
                self.assertNotIn("date -u", text)
                logical = text.replace("\\\n", " ")
                capsule_calls = [
                    line
                    for line in logical.splitlines()
                    if "hook-context" in line and not line.lstrip().startswith("#")
                ]
                self.assertTrue(capsule_calls, "no capsule command found")
                for line in capsule_calls:
                    self.assertNotIn("--json", line)


class HostDeliveredCapsuleTest(unittest.TestCase):
    def test_internal_recovery_skips_all_memory_delivery_and_checkpoint_hooks(self):
        for tool in ('claude', 'codex', 'cursor'):
            hooks = ['local-context.sh', 'working-memory-write.sh']
            if tool != 'cursor': hooks.append('working-memory-read.sh')
            for hook in hooks:
                with self.subTest(tool=tool, hook=hook):
                    result = run_hook(tool, hook, {}, env={'CONTEXT_MEMORY_RECOVERY': '1'})
                    self.assertEqual((0, '', ''), (result.returncode, result.stdout, result.stderr))

    """A host that put the turn's capsule into the prompt silences the read hook.

    The Harness retrieves for the message alone and sets
    CONTEXT_CAPSULE_DELIVERED=1. A second capsule, distilled from the whole
    prompt the host assembled, would spend the turn's memory budget twice.
    """

    def test_prompt_adapter_sanitizes_before_retrieval_and_skips_pii_only(self) -> None:
        for tool in ("claude", "codex"):
            with self.subTest(tool=tool), tempfile.TemporaryDirectory(prefix="safe-prompt-hook-") as temp:
                root = Path(temp)
                scripts = root / "memory-bank/scripts"
                scripts.mkdir(parents=True)
                for source in (EDITION_ROOT / "memory-bank/scripts").glob("*.py"):
                    shutil.copy(source, scripts)
                hooks = root / f".{tool}/hooks"
                hooks.mkdir(parents=True)
                shutil.copy(hook_path(tool, "working-memory-read.sh"), hooks)
                (root / "specs").mkdir()
                (root / "specs/cobalt.md").write_text("# Cobalt\n\nCobalt allocation requires one owner.\n")
                (root / "AGENTS.md").write_text("# Policy\n\nCobalt allocation rules.\n")
                subprocess.run(["git", "init", "--quiet", str(root)], check=True, capture_output=True)
                environment = {**os.environ, "CONTEXT_TASK_ID": "TASK-HOOK", "CONTEXT_HOOK_BUDGET": "10"}
                environment.pop("CONTEXT_CAPSULE_DELIVERED", None)
                environment.pop("CONTEXT_MEMORY_RECOVERY", None)
                def call(prompt):
                    return subprocess.run([BASH, str(hooks / "working-memory-read.sh")],
                        input=json.dumps({"prompt": prompt}), text=True, capture_output=True,
                        env=environment, cwd=root, timeout=HOOK_TIMEOUT)
                result = call("Check cobalt allocation for person@example.test; call +370 600 12345")
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertIn("allocation requires one owner", result.stdout)
                self.assertNotIn("person@example.test", result.stdout + result.stderr)
                manifests = list(root.glob("memory-bank/local/retrieval-manifests/*.json"))
                self.assertTrue(manifests)
                for manifest in manifests:
                    self.assertNotIn("person@example.test", manifest.read_text())
                    self.assertNotIn("12345", manifest.read_text())
                    if tool == "codex":
                        self.assertIn({"path": "AGENTS.md", "reason": "host-loaded"},
                                      json.loads(manifest.read_text())["excluded"])
                before = {file.name for file in manifests}
                for prompt in ("person@example.test", "Customer email: person@example.test",
                               "Call +370 600 12345", "Patient name: Alice Smith",
                               "Client address: Main Street 7, Vilnius",
                               "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"):
                    skipped = call(prompt)
                    self.assertEqual((0, "", ""), (skipped.returncode, skipped.stdout, skipped.stderr))
                self.assertEqual(before, {file.name for file in root.glob("memory-bank/local/retrieval-manifests/*.json")})
                # A quoted secret goes whole: no tail of it reaches a manifest.
                quoted = call("Check cobalt allocation password='alpha beta gamma' after the owner change")
                self.assertEqual(0, quoted.returncode, quoted.stderr)
                for manifest in root.glob("memory-bank/local/retrieval-manifests/*.json"):
                    self.assertNotIn("beta gamma", manifest.read_text())
                    self.assertNotIn("alpha", manifest.read_text())
                # A pasted transcript line loses its role prefix and keeps its words.
                pasted = call("User: cobalt allocation owner")
                self.assertEqual(0, pasted.returncode, pasted.stderr)
                self.assertIn("allocation requires one owner", pasted.stdout)
                for manifest in root.glob("memory-bank/local/retrieval-manifests/*.json"):
                    self.assertNotIn("User:", manifest.read_text())

    def test_read_hook_stands_down_only_when_the_host_delivered_the_capsule(self) -> None:
        with tempfile.TemporaryDirectory(prefix="hook-tests-") as directory:
            for tool in ("claude", "codex"):
                with self.subTest(tool=tool):
                    root = Path(directory) / tool
                    hooks = root / MIRRORS[tool][0]
                    hooks.mkdir(parents=True)
                    shutil.copy(hook_path(tool, "working-memory-read.sh"), hooks)
                    marker = root / "cli-ran"
                    cli = root / "memory-bank/scripts/context.py"
                    cli.parent.mkdir(parents=True)
                    shutil.copy(EDITION_ROOT / "memory-bank/scripts/automatic_query.py", cli.parent)
                    cli.write_text(
                        "import pathlib\n"
                        f"pathlib.Path({str(marker)!r}).write_text('ran')\n"
                        "print('working: TASK-HOST')\n",
                        encoding="utf-8",
                    )

                    def run(delivered: str):
                        return subprocess.run(
                            [BASH, str(hooks / "working-memory-read.sh")],
                            input=json.dumps({"prompt": "cobalt allocation"}),
                            capture_output=True,
                            text=True,
                            env={**os.environ, "CONTEXT_TASK_ID": "TASK-HOST",
                                 "CONTEXT_CAPSULE_DELIVERED": delivered},
                            timeout=HOOK_TIMEOUT,
                        )

                    stood_down = run("1")
                    self.assertEqual((0, ""), (stood_down.returncode, stood_down.stdout))
                    self.assertFalse(marker.exists())
                    ordinary = run("0")
                    self.assertEqual(0, ordinary.returncode)
                    self.assertIn("working: TASK-HOST", ordinary.stdout)
                    self.assertTrue(marker.exists())


class ReadHookInputTest(unittest.TestCase):
    """The read hook hands the prompt and the conversation id to the CLI."""

    def run_hook(self, tool: str, payload: object, directory: Path):
        root = directory / tool
        hooks = root / MIRRORS[tool][0]
        hooks.mkdir(parents=True, exist_ok=True)
        shutil.copy(hook_path(tool, "working-memory-read.sh"), hooks)
        argv_file = root / "argv.json"
        cli = root / "memory-bank/scripts/context.py"
        cli.parent.mkdir(parents=True, exist_ok=True)
        cli.write_text(
            "import json, pathlib, sys\n"
            f"pathlib.Path({str(argv_file)!r}).write_text(json.dumps(sys.argv[1:]))\n"
            "print('working: TASK-INPUT')\n",
            encoding="utf-8",
        )
        result = subprocess.run(
            [BASH, str(hooks / "working-memory-read.sh")],
            input=payload if isinstance(payload, str) else json.dumps(payload),
            capture_output=True,
            text=True,
            env={**os.environ, "CONTEXT_TASK_ID": "TASK-INPUT",
                 "CONTEXT_CAPSULE_DELIVERED": ""},
            timeout=HOOK_TIMEOUT,
        )
        argv = json.loads(argv_file.read_text(encoding="utf-8")) if argv_file.exists() else None
        return result, argv

    def test_prompt_and_session_reach_the_cli_unchanged(self) -> None:
        with tempfile.TemporaryDirectory(prefix="hook-input-") as directory:
            for tool in ("claude", "codex"):
                with self.subTest(tool=tool):
                    prompt = "cobalt allocation\nsecond line\n\n"
                    result, argv = self.run_hook(
                        tool, {"prompt": prompt, "session_id": "abc-123"}, Path(directory)
                    )
                    self.assertEqual(0, result.returncode, result.stderr)
                    # No banner: the capsule is the whole output.
                    self.assertEqual("working: TASK-INPUT\n", result.stdout)
                    self.assertEqual(prompt, argv[argv.index("--query") + 1])
                    self.assertEqual("abc-123", argv[argv.index("--session-id") + 1])

    def test_a_missing_or_unsafe_session_id_is_not_passed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="hook-input-") as directory:
            for payload in (
                {"prompt": "cobalt allocation"},
                {"prompt": "cobalt allocation", "session_id": "a b; rm -rf /"},
                {"prompt": "cobalt allocation", "session_id": 42},
            ):
                with self.subTest(payload=payload):
                    _, argv = self.run_hook("claude", payload, Path(directory))
                    self.assertNotIn("--session-id", argv)
                    self.assertEqual("cobalt allocation", argv[argv.index("--query") + 1])

    def test_the_transcript_travels_with_the_conversation(self) -> None:
        # Where a compaction since the last turn shows; only with a session id,
        # and only an absolute path.
        with tempfile.TemporaryDirectory(prefix="hook-input-") as directory:
            transcript = str(Path(directory) / "conversation.jsonl")
            for tool in ("claude", "codex"):
                with self.subTest(tool=tool):
                    _, argv = self.run_hook(tool, {"prompt": "cobalt allocation", "session_id": "abc-123",
                                                   "transcript_path": transcript}, Path(directory))
                    self.assertEqual(transcript, argv[argv.index("--transcript") + 1])
                    self.assertEqual("cobalt allocation", argv[argv.index("--query") + 1])
            for payload in (
                {"prompt": "cobalt allocation", "transcript_path": transcript},
                {"prompt": "cobalt allocation", "session_id": "abc-123", "transcript_path": "relative.jsonl"},
                {"prompt": "cobalt allocation", "session_id": "abc-123", "transcript_path": ["x"]},
            ):
                with self.subTest(payload=payload):
                    _, argv = self.run_hook("claude", payload, Path(directory))
                    self.assertNotIn("--transcript", argv)
                    self.assertEqual("cobalt allocation", argv[argv.index("--query") + 1])

    def test_malformed_input_still_refreshes_without_a_query(self) -> None:
        with tempfile.TemporaryDirectory(prefix="hook-input-") as directory:
            result, argv = self.run_hook("claude", "not json", Path(directory))
            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(["refresh", "--host", "claude"], argv)

    def run_failing(self, tool: str, directory: Path, body: str, budget: str = "5"):
        """The read hook against a stand-in CLI that runs ``body``."""
        root = directory / tool
        hooks = root / MIRRORS[tool][0]
        hooks.mkdir(parents=True, exist_ok=True)
        shutil.copy(hook_path(tool, "working-memory-read.sh"), hooks)
        cli = root / "memory-bank/scripts/context.py"
        cli.parent.mkdir(parents=True, exist_ok=True)
        cli.write_text(body, encoding="utf-8")
        return subprocess.run(
            [BASH, str(hooks / "working-memory-read.sh")],
            input=json.dumps({"prompt": "cobalt allocation"}),
            capture_output=True,
            text=True,
            env={**os.environ, "CONTEXT_TASK_ID": "TASK-INPUT",
                 "CONTEXT_CAPSULE_DELIVERED": "", "CONTEXT_HOOK_BUDGET": budget},
            timeout=HOOK_TIMEOUT,
        )

    def test_a_failed_refresh_says_memory_was_not_consulted(self) -> None:
        # An empty hook and a crashed one looked identical from inside the
        # turn; only the crashed one may not be read as memory consulted.
        crash = (
            "import sys\n"
            "sys.stderr.write('Traceback (most recent call last):\\n')\n"
            "sys.stderr.write('context: database is locked\\n')\n"
            "sys.exit(3)\n"
        )
        with tempfile.TemporaryDirectory(prefix="hook-input-") as directory:
            for tool in ("claude", "codex"):
                with self.subTest(tool=tool):
                    result = self.run_failing(tool, Path(directory), crash)
                    self.assertEqual(0, result.returncode, result.stderr)
                    self.assertIn(
                        "Memory refresh unavailable: it exited 3 — "
                        "context: database is locked.",
                        result.stdout,
                    )
                    self.assertIn("NOT consulted this turn", result.stdout)
                    self.assertNotIn("Traceback", result.stdout)

    def test_a_refresh_past_its_budget_says_so(self) -> None:
        if shutil.which("timeout") is None:
            self.skipTest("no timeout(1): the budget is not enforced here")
        with tempfile.TemporaryDirectory(prefix="hook-input-") as directory:
            result = self.run_failing(
                "claude", Path(directory), "import time\ntime.sleep(5)\n", budget="1"
            )
            self.assertEqual(0, result.returncode, result.stderr)
            self.assertIn("exceeded its 1s budget", result.stdout)
            self.assertIn("NOT consulted this turn", result.stdout)

    def test_a_quiet_successful_refresh_stays_silent(self) -> None:
        # A prompt the sanitizer left nothing of: status 0, nothing to say.
        with tempfile.TemporaryDirectory(prefix="hook-input-") as directory:
            for tool in ("claude", "codex"):
                with self.subTest(tool=tool):
                    result = self.run_failing(tool, Path(directory), "pass\n")
                    self.assertEqual((0, "", ""), (result.returncode, result.stdout, result.stderr))


class CursorPromptHookTest(unittest.TestCase):
    """Cursor's beforeSubmitPrompt hook: the prompt's capsule goes into the rule.

    The event cannot add context to a prompt, only let it through or stop it,
    so the hook renders the capsule for this prompt into the alwaysApply rule
    and always answers "continue".
    """

    PAYLOAD = {
        "prompt": "Order totals are off by a cent after a discount",
        "conversation_id": "c-1",
        "generation_id": "g-1",
        "cursor_version": "2.9.1",
        "hook_event_name": "beforeSubmitPrompt",
    }

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="cursor-prompt-")
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name) / "edition"
        hooks = self.root / ".cursor" / "hooks"
        hooks.mkdir(parents=True)
        shutil.copy2(hook_path("cursor", "working-memory-read.sh"), hooks)
        self.root.joinpath(".cursor/hooks.json").write_text("{}", encoding="utf-8")
        self.cli = self.root / "memory-bank" / "scripts" / "context.py"
        self.cli.parent.mkdir(parents=True)
        self.argv = self.root / "argv.json"
        self.rule = self.root / ".cursor" / "rules" / "working-memory.mdc"

    def stub(self, capsule_text: str = "working: TASK-P - fix rounding\n- memory a.md — A\n  answer text",
             status: int = 0) -> None:
        # Like the real CLI: JSON only when asked for, and a capsule only for
        # a query; without one, refresh prints its layer report.
        self.cli.write_text(
            "import json, pathlib, sys\n"
            f"pathlib.Path({str(self.argv)!r}).write_text(json.dumps(sys.argv[1:]))\n"
            "if '--json' in sys.argv and '--query' in sys.argv:\n"
            f"    print(json.dumps({{'capsule_text': {capsule_text!r}}}))\n"
            "else:\n"
            "    print('semantic: updated')\n"
            f"sys.exit({status})\n",
            encoding="utf-8",
        )

    def run_prompt_hook(self, payload, **env: str):
        return subprocess.run(
            [BASH, str(self.root / ".cursor/hooks/working-memory-read.sh")],
            input=payload if isinstance(payload, str) else json.dumps(payload),
            capture_output=True,
            text=True,
            cwd=str(self.root),
            env={**os.environ, "CONTEXT_TASK_ID": "TASK-P", "CONTEXT_CAPSULE_DELIVERED": "", **env},
            timeout=HOOK_TIMEOUT,
        )

    def test_the_prompts_capsule_goes_into_the_rule(self) -> None:
        self.stub()
        result = self.run_prompt_hook(self.PAYLOAD)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual({"continue": True}, json.loads(result.stdout))
        rule = self.rule.read_text(encoding="utf-8")
        self.assertIn("alwaysApply: true", rule)
        self.assertIn("answer text", rule)
        argv = json.loads(self.argv.read_text(encoding="utf-8"))
        self.assertEqual(self.PAYLOAD["prompt"], argv[argv.index("--query") + 1])
        for flag in ("--json", "--sanitize", "--ephemeral"):
            self.assertIn(flag, argv)
        self.assertEqual("cursor", argv[argv.index("--host") + 1])
        # The rule is re-sent whole with every request: nothing in it may be
        # left out as "handed earlier".
        self.assertNotIn("--session-id", argv)

    def test_the_rule_changes_only_when_a_prompt_brings_a_new_item(self) -> None:
        # Cursor puts rules at the start of the context: a rule rewritten for
        # every prompt would cost the conversation its cached prefix each time.
        first = "working: TASK-P - fix rounding\n- memory a.md \u2014 A\n  first excerpt"
        self.stub(first)
        self.run_prompt_hook(self.PAYLOAD)
        rendered = self.rule.read_text(encoding="utf-8")
        for name, capsule in (
            ("same item, another excerpt", "working: TASK-P - fix rounding\n- memory a.md \u2014 A\n  second excerpt"),
            ("nothing retrieved", "working: TASK-P - fix rounding"),
        ):
            with self.subTest(case=name):
                self.stub(capsule)
                result = self.run_prompt_hook(self.PAYLOAD)
                self.assertEqual({"continue": True}, json.loads(result.stdout))
                self.assertEqual(rendered, self.rule.read_text(encoding="utf-8"))
        self.stub("working: TASK-P - fix rounding\n- memory b.md \u2014 B\n  new excerpt")
        self.run_prompt_hook(self.PAYLOAD)
        self.assertIn("new excerpt", self.rule.read_text(encoding="utf-8"))
        # Another section of a document the rule holds is new, too.
        self.stub("working: TASK-P - fix rounding\n- memory b.md \u00a7 Refunds \u2014 B\n  refund excerpt")
        self.run_prompt_hook(self.PAYLOAD)
        self.assertIn("refund excerpt", self.rule.read_text(encoding="utf-8"))
        # Another task's rule is replaced whatever it holds.
        self.stub(first)
        self.run_prompt_hook(self.PAYLOAD, CONTEXT_TASK_ID="TASK-Q")
        self.assertIn("(task: TASK-Q)", self.rule.read_text(encoding="utf-8"))

    def test_every_exit_lets_the_prompt_through(self) -> None:
        for name, setup, payload, env in (
            ("runtime fails", lambda: self.stub(status=1, capsule_text=""), self.PAYLOAD, {}),
            ("malformed input", self.stub, "not json", {}),
            ("host delivered", self.stub, self.PAYLOAD, {"CONTEXT_CAPSULE_DELIVERED": "1"}),
            ("broken render", lambda: self.stub(capsule_text="not a capsule"), self.PAYLOAD, {}),
        ):
            with self.subTest(case=name):
                self.rule.unlink(missing_ok=True)
                setup()
                result = self.run_prompt_hook(payload, **env)
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertEqual({"continue": True}, json.loads(result.stdout))
                self.assertFalse(self.rule.exists())

    def test_a_failed_refresh_keeps_only_this_tasks_rule(self) -> None:
        # A prompt that renders nothing leaves the rule Cursor will send with
        # it: another task's rule (a switched branch) is removed, this task's
        # stays.
        for name, setup in (
            ("runtime fails", lambda: self.stub(status=1, capsule_text="")),
            ("broken render", lambda: self.stub(capsule_text="not a capsule")),
        ):
            for owner, kept in (("TASK-OTHER", False), ("TASK-P", True)):
                with self.subTest(case=name, rule_of=owner):
                    self.rule.parent.mkdir(parents=True, exist_ok=True)
                    held = (
                        "Session context retrieved for a recent prompt (task: {}).\n"
                        "```\nworking: {} - its goal\n```\n".format(owner, owner)
                    )
                    self.rule.write_text(held, encoding="utf-8")
                    setup()
                    result = self.run_prompt_hook(self.PAYLOAD)
                    self.assertEqual({"continue": True}, json.loads(result.stdout))
                    self.assertEqual(kept, self.rule.exists())
                    if kept:
                        self.assertEqual(held, self.rule.read_text(encoding="utf-8"))

    def test_the_prompts_rule_outlives_the_turn_but_not_the_task_or_the_session(self) -> None:
        # When Cursor reads its rules before this hook has run, the rule the
        # last prompt left is what the next request carries: the stop hook
        # must not rebuild it from the task alone.
        for name in ("working-memory-write.sh", "local-context.sh"):
            shutil.copy2(hook_path("cursor", name), self.root / ".cursor/hooks")
        self.cli.write_text(
            "import json, sys\n"
            "if '--json' in sys.argv and '--query' in sys.argv:\n"
            "    print(json.dumps({'capsule_text': 'working: TASK-P - prompt capsule'}))\n"
            "elif sys.argv[1:2] == ['hook-context']:\n"
            "    print('working: TASK-P - branch capsule')\n",
            encoding="utf-8",
        )

        def hook(name: str, task: str = "TASK-P") -> str:
            result = subprocess.run(
                [BASH, str(self.root / ".cursor/hooks" / name)], input=json.dumps(self.PAYLOAD),
                capture_output=True, text=True, cwd=str(self.root), timeout=HOOK_TIMEOUT,
                env={**os.environ, "CONTEXT_TASK_ID": task, "CONTEXT_CAPSULE_DELIVERED": ""},
            )
            self.assertEqual(0, result.returncode, result.stderr)
            return self.rule.read_text(encoding="utf-8")

        self.assertIn("prompt capsule", hook("working-memory-read.sh"))
        self.assertIn("prompt capsule", hook("working-memory-write.sh"))
        self.assertIn("branch capsule", hook("working-memory-write.sh", task="TASK-Q"))
        self.assertIn("prompt capsule", hook("working-memory-read.sh"))
        self.assertIn("branch capsule", hook("local-context.sh"))

    def test_the_claude_copies_stand_down_under_cursor_where_cursor_hooks_serve(self) -> None:
        claude = self.root / ".claude" / "hooks"
        claude.mkdir(parents=True)
        for name in ("working-memory-read.sh", "working-memory-write.sh"):
            shutil.copy2(hook_path("claude", name), claude)
        marker = self.root / "cli-ran"
        self.cli.write_text(
            f"import pathlib\npathlib.Path({str(marker)!r}).write_text('ran')\nprint('working: X')\n",
            encoding="utf-8",
        )

        def run(name: str, payload: dict) -> None:
            marker.unlink(missing_ok=True)
            subprocess.run(
                [BASH, str(claude / name)], input=json.dumps(payload), capture_output=True,
                text=True, cwd=str(self.root), timeout=HOOK_TIMEOUT,
                env={**os.environ, "CONTEXT_TASK_ID": "TASK-P", "CONTEXT_CAPSULE_DELIVERED": ""},
            )

        claude_payload = {"prompt": "cobalt rule", "session_id": "s-1", "hook_event_name": "UserPromptSubmit"}
        for name in ("working-memory-read.sh", "working-memory-write.sh"):
            with self.subTest(hook=name):
                run(name, self.PAYLOAD)
                self.assertFalse(marker.exists(), "the Claude copy ran under Cursor")
                run(name, claude_payload)
                self.assertTrue(marker.exists(), "the Claude copy did not run under Claude Code")
        # Without Cursor hooks of its own, Cursor is served by the Claude copies.
        self.root.joinpath(".cursor/hooks.json").unlink()
        run("working-memory-read.sh", self.PAYLOAD)
        self.assertTrue(marker.exists())


class CursorCapsuleRenderTest(unittest.TestCase):
    """Functional render tests against a throwaway edition tree.

    The hooks resolve every path from their own location, so copying the
    Cursor mirrors into a fake edition with a stub context.py exercises the
    real render logic without touching this edition's index or rule file.
    """

    TASK_ID = "TASK-STUB"

    # The hooks render the capsule (`hook-context` without --json), so the
    # stubs emit what print_capsule emits: a leading "working:" marker line,
    # plus a "warming:" line while the task is not yet provisioned.
    STUB_CAPSULE = (
        "import sys\n"
        "if len(sys.argv) > 1 and sys.argv[1] == \"hook-context\":\n"
        "    print(\"working: TASK-STUB - stub goal\")\n"
    )
    STUB_FAILURE = "import sys\nsys.exit(1)\n"
    STUB_EMPTY = (
        "import sys\n"
        "if len(sys.argv) > 1 and sys.argv[1] == \"hook-context\":\n"
        "    sys.exit(3)\n"
    )
    # Output that does not open with the working line: a broken render, which
    # must not replace a good rule.
    STUB_MALFORMED = (
        "import sys\n"
        "if len(sys.argv) > 1 and sys.argv[1] == \"hook-context\":\n"
        "    print(\"not-a-capsule\")\n"
    )
    STUB_SEQUENCE = (
        "import pathlib, sys\n"
        "counter = pathlib.Path(__file__).with_name(\"turn-count\")\n"
        "count = int(counter.read_text()) if counter.exists() else 0\n"
        "if len(sys.argv) > 1 and sys.argv[1] == \"turn\":\n"
        "    count += 1\n"
        "    counter.write_text(str(count))\n"
        "elif len(sys.argv) > 1 and sys.argv[1] == \"hook-context\":\n"
        "    print(\"working: TASK-STUB - stub goal\")\n"
        "    if count < 5:\n"
        "        print(f\"warming: {min(count, 4)} turn(s) pending\")\n"
    )
    STUB_TIMEOUT = (
        "import time\n"
        "time.sleep(1)\n"
    )
    # Status 4: the retrieval gate withheld this turn (enforce mode).
    STUB_GATE_SKIP = (
        "import sys\n"
        "if len(sys.argv) > 1 and sys.argv[1] == \"hook-context\":\n"
        "    sys.exit(4)\n"
    )

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="cursor-rule-")
        self.root = Path(self._tmp.name) / "edition"
        self.hooks_dir = self.root / ".cursor" / "hooks"
        self.rules_dir = self.root / ".cursor" / "rules"
        self.cli = self.root / "memory-bank" / "scripts" / "context.py"
        self.hooks_dir.mkdir(parents=True)
        self.rules_dir.mkdir()
        self.cli.parent.mkdir(parents=True)
        for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
            shutil.copy2(hook_path("cursor", hook), self.hooks_dir / hook)
        self.addCleanup(self._tmp.cleanup)

    def run_cursor_hook(
        self, name: str, task_id: str = TASK_ID, budget: str = ""
    ):
        env = dict(os.environ)
        env["CONTEXT_TASK_ID"] = task_id
        if budget:
            env["CONTEXT_HOOK_BUDGET"] = budget
        return subprocess.run(
            ["bash", str(self.hooks_dir / name)],
            input="",
            capture_output=True,
            text=True,
            cwd=str(self.root),
            env=env,
            timeout=HOOK_TIMEOUT,
        )

    def rule_file(self) -> Path:
        return self.rules_dir / "working-memory.mdc"

    def test_stop_hook_renders_alwaysapply_rule(self) -> None:
        self.cli.write_text(self.STUB_CAPSULE, encoding="utf-8")
        result = self.run_cursor_hook("working-memory-write.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        rendered = self.rule_file().read_text(encoding="utf-8")
        self.assertTrue(rendered.startswith("---\n"))
        self.assertIn("alwaysApply: true", rendered)
        self.assertIn("Session context as of end of previous turn", rendered)
        self.assertIn("task: TASK-STUB", rendered)
        self.assertIn("stub goal", rendered)
        self.assertIn("not authoritative", rendered)

    def test_repeat_render_is_byte_identical(self) -> None:
        """Same context in, same rule out - across turns and across hooks."""
        self.cli.write_text(self.STUB_CAPSULE, encoding="utf-8")
        renders = []
        for hook in ("working-memory-write.sh", "local-context.sh",
                     "working-memory-write.sh"):
            result = self.run_cursor_hook(hook)
            self.assertEqual(result.returncode, 0, result.stderr)
            renders.append(self.rule_file().read_text(encoding="utf-8"))
        self.assertEqual(renders[0], renders[1])
        self.assertEqual(renders[1], renders[2])

    def test_no_rule_is_rendered_when_the_host_delivered_the_capsule(self) -> None:
        # The Harness put this turn's capsule into the prompt; a rule built from
        # the branch would deliver a second, staler one.
        self.cli.write_text(self.STUB_CAPSULE, encoding="utf-8")
        for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
            with self.subTest(hook=hook):
                env = dict(os.environ, CONTEXT_TASK_ID=self.TASK_ID, CONTEXT_CAPSULE_DELIVERED="1")
                result = subprocess.run(
                    ["bash", str(self.hooks_dir / hook)], input="", capture_output=True,
                    text=True, cwd=str(self.root), env=env, timeout=HOOK_TIMEOUT,
                )
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertFalse(self.rule_file().exists())

    def test_session_start_renders_rule_without_printing_it(self) -> None:
        self.cli.write_text(self.STUB_CAPSULE, encoding="utf-8")
        result = self.run_cursor_hook("local-context.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("stub goal", result.stdout)
        self.assertIn("stub goal", self.rule_file().read_text(encoding="utf-8"))

    def test_first_four_turns_warm_and_fifth_atomically_governs(self) -> None:
        self.cli.write_text(self.STUB_SEQUENCE, encoding="utf-8")
        for turn in range(1, 5):
            result = self.run_cursor_hook("working-memory-write.sh")
            self.assertEqual(0, result.returncode, result.stderr)
            rendered = self.rule_file().read_text(encoding="utf-8")
            self.assertIn(f"warming: {turn} turn(s) pending", rendered)
        restarted = self.run_cursor_hook("local-context.sh")
        self.assertEqual(0, restarted.returncode, restarted.stderr)
        self.assertIn("warming:", self.rule_file().read_text(encoding="utf-8"))

        fifth = self.run_cursor_hook("working-memory-write.sh")
        self.assertEqual(0, fifth.returncode, fifth.stderr)
        rendered = self.rule_file().read_text(encoding="utf-8")
        self.assertIn("working: TASK-STUB", rendered)
        self.assertNotIn("warming:", rendered)

    @staticmethod
    def labelled_rule(task: str, body: str = "previous capsule") -> str:
        """A rule as the hooks write it: its header names the task."""
        return (
            "---\nalwaysApply: true\n---\n\n# Working Memory (auto-rendered)\n\n"
            "Session context as of end of previous turn (task: {}).\n\n"
            "```\n{}\n```\n".format(task, body)
        )

    def test_failed_render_preserves_previous_rule(self) -> None:
        # The previous rule is this task's: a failed render keeps it.
        previous = self.labelled_rule(self.TASK_ID)
        self.rule_file().write_text(previous, encoding="utf-8")
        self.cli.write_text(self.STUB_FAILURE, encoding="utf-8")
        for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
            with self.subTest(hook=hook):
                result = self.run_cursor_hook(hook)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    self.rule_file().read_text(encoding="utf-8"),
                    previous,
                )
                # No temp litter: an interrupted render must not accumulate.
                self.assertEqual(
                    [path.name for path in self.rules_dir.iterdir()],
                    ["working-memory.mdc"],
                )

    def test_a_render_that_fails_removes_another_tasks_rule(self) -> None:
        # A switched branch whose render fails, breaks or is withheld must not
        # keep the previous branch's capsule as this task's working memory:
        # Cursor sends the rule with every prompt.
        foreign = self.labelled_rule("feature/a", "working: feature/a - rewrite billing")
        for name, stub in (
            ("failure", self.STUB_FAILURE),
            ("malformed", self.STUB_MALFORMED),
            ("gate skip", self.STUB_GATE_SKIP),
        ):
            for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
                with self.subTest(case=name, hook=hook):
                    self.rule_file().write_text(foreign, encoding="utf-8")
                    self.cli.write_text(stub, encoding="utf-8")
                    result = self.run_cursor_hook(hook, "fix/issue-42")
                    self.assertEqual(0, result.returncode, result.stderr)
                    self.assertFalse(self.rule_file().exists())

    def test_a_withheld_render_keeps_this_tasks_rule(self) -> None:
        # An enforce-mode skip must never empty Cursor's only memory channel.
        previous = self.labelled_rule(self.TASK_ID)
        self.cli.write_text(self.STUB_GATE_SKIP, encoding="utf-8")
        for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
            with self.subTest(hook=hook):
                self.rule_file().write_text(previous, encoding="utf-8")
                result = self.run_cursor_hook(hook)
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertEqual(previous, self.rule_file().read_text(encoding="utf-8"))

    @unittest.skipUnless(shutil.which("timeout"), "native timeout unavailable")
    def test_a_timed_out_render_removes_another_tasks_rule(self) -> None:
        foreign = self.labelled_rule("feature/a", "working: feature/a - rewrite billing")
        self.cli.write_text(self.STUB_TIMEOUT, encoding="utf-8")
        for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
            with self.subTest(hook=hook):
                self.rule_file().write_text(foreign, encoding="utf-8")
                result = self.run_cursor_hook(hook, "fix/issue-42", budget="0.05")
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertFalse(self.rule_file().exists())

    def test_valid_empty_branch_context_removes_foreign_rule(self) -> None:
        self.rule_file().write_text("foreign branch capsule\n", encoding="utf-8")
        self.cli.write_text(self.STUB_EMPTY, encoding="utf-8")
        result = self.run_cursor_hook("local-context.sh", "feature/new-branch")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse(self.rule_file().exists())

    def test_no_branch_removes_foreign_rule_on_session_start(self) -> None:
        self.rule_file().write_text("foreign branch capsule\n", encoding="utf-8")
        self.cli.write_text(self.STUB_FAILURE, encoding="utf-8")
        result = self.run_cursor_hook("local-context.sh", "")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse(self.rule_file().exists())

    def test_malformed_render_preserves_previous_rule(self) -> None:
        previous = self.labelled_rule(self.TASK_ID)
        self.rule_file().write_text(previous, encoding="utf-8")
        self.cli.write_text(self.STUB_MALFORMED, encoding="utf-8")
        for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
            with self.subTest(hook=hook):
                result = self.run_cursor_hook(hook)
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertEqual(
                    previous,
                    self.rule_file().read_text(encoding="utf-8"),
                )

    @unittest.skipUnless(shutil.which("timeout"), "native timeout unavailable")
    def test_timed_out_render_preserves_previous_rule(self) -> None:
        previous = self.labelled_rule(self.TASK_ID)
        self.rule_file().write_text(previous, encoding="utf-8")
        self.cli.write_text(self.STUB_TIMEOUT, encoding="utf-8")
        result = self.run_cursor_hook(
            "local-context.sh", budget="0.05"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(
            previous,
            self.rule_file().read_text(encoding="utf-8"),
        )


class SubagentGateTest(unittest.TestCase):
    """Tool-owned subagent gates: only roster agents spawn, built-ins deny.

    The three copies are deliberately NOT byte-identical (each host has a
    different gate contract), so each is exercised against its own contract.
    """

    BUILTIN_CLAUDE = ("Explore", "Plan", "general-purpose", "claude")
    BUILTIN_CURSOR = ("explore", "shell", "bash", "browser", "generalPurpose")

    def setUp(self) -> None:
        self._lock_tmp = tempfile.TemporaryDirectory(prefix="subagent-gate-lock-")
        self.lock_dir = Path(self._lock_tmp.name)
        self.lock_env = {"SUBAGENT_WRITE_LOCK_DIR": str(self.lock_dir)}
        self.addCleanup(self._lock_tmp.cleanup)

    def clear_write_locks(self) -> None:
        key = repo_key(EDITION_ROOT)
        for tool in ("claude", "cursor"):
            (self.lock_dir / f"{tool}-write-agent-lock-{key}").unlink(
                missing_ok=True
            )

    def run_gate(self, tool: str, payload):
        return run_hook(
            tool,
            "subagent-gate.sh",
            payload,
            env=self.lock_env,
        )

    @staticmethod
    def roster(tool: str) -> list[str]:
        agents_dir = EDITION_ROOT / {
            "claude": ".claude/agents",
            "cursor": ".cursor/agents",
        }[tool]
        names = []
        for path in sorted(agents_dir.glob("*.md")):
            if path.name == "README.md":
                continue
            in_frontmatter = False
            for line in path.read_text(encoding="utf-8").splitlines():
                if line == "---":
                    if in_frontmatter:
                        break
                    in_frontmatter = True
                    continue
                if in_frontmatter and line.startswith("name:"):
                    names.append(line.split(":", 1)[1].strip().strip('"'))
                    break
        return names

    @staticmethod
    def spawn_payload(sub_type: str) -> dict:
        return {
            "tool_name": "Agent",
            "tool_input": {"subagent_type": sub_type, "prompt": "x"},
        }

    def test_claude_blocks_builtin_agents(self) -> None:
        for builtin in self.BUILTIN_CLAUDE:
            with self.subTest(agent=builtin):
                result = self.run_gate("claude", self.spawn_payload(builtin))
                self.assertEqual(2, result.returncode)
                self.assertIn("BLOCKED", result.stderr)

    def test_claude_blocks_via_legacy_task_tool_name(self) -> None:
        payload = {
            "tool_name": "Task",
            "tool_input": {"subagent_type": "general-purpose"},
        }
        result = self.run_gate("claude", payload)
        self.assertEqual(2, result.returncode)

    def test_claude_allows_every_roster_agent(self) -> None:
        names = self.roster("claude")
        self.assertTrue(names)
        for name in names:
            with self.subTest(agent=name):
                result = self.run_gate("claude", self.spawn_payload(name))
                self.assertEqual(0, result.returncode, result.stderr)
            # Write-capable agents take the serialization lock on spawn;
            # release it so the roster sweep stays isolation-free.
            self.clear_write_locks()

    def test_claude_ignores_other_tools_and_typeless_payloads(self) -> None:
        for payload in (
            {"tool_name": "Bash", "tool_input": {"command": "ls"}},
            {"tool_name": "Agent", "tool_input": {}},
            "not json",
            "",
        ):
            with self.subTest(payload=payload):
                result = self.run_gate("claude", payload)
                self.assertEqual(0, result.returncode)

    def test_cursor_denies_builtins_with_permission_json(self) -> None:
        for builtin in self.BUILTIN_CURSOR:
            with self.subTest(agent=builtin):
                result = self.run_gate("cursor", {"subagent_type": builtin})
                self.assertEqual(0, result.returncode, result.stderr)
                verdict = json.loads(result.stdout)
                self.assertEqual("deny", verdict["permission"])
                self.assertIn("user_message", verdict)

    def test_cursor_allows_every_roster_agent(self) -> None:
        names = self.roster("cursor")
        self.assertTrue(names)
        for name in names:
            with self.subTest(agent=name):
                result = self.run_gate("cursor", {"subagent_type": name})
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertEqual(
                    "allow", json.loads(result.stdout)["permission"]
                )
            self.clear_write_locks()

    def test_cursor_typeless_payload_allows(self) -> None:
        result = self.run_gate("cursor", {"task": "x"})
        self.assertEqual(0, result.returncode)
        self.assertEqual("allow", json.loads(result.stdout)["permission"])

    def test_codex_blocks_the_multi_agent_tool_family(self) -> None:
        for tool in (
            "spawn_agent",
            "Agent",
            "send_input",
            "resume_agent",
            "wait_agent",
            "close_agent",
        ):
            with self.subTest(tool=tool):
                result = self.run_gate(
                    "codex", {"tool_name": tool, "tool_input": {}}
                )
                self.assertEqual(2, result.returncode)
                self.assertIn("BLOCKED", result.stderr)

    def test_codex_ignores_ordinary_tools(self) -> None:
        for payload in (
            {"tool_name": "Shell", "tool_input": {"command": "ls"}},
            {"tool_name": "Edit", "tool_input": {"file_path": "a.php"}},
            "",
        ):
            with self.subTest(payload=payload):
                result = self.run_gate("codex", payload)
                self.assertEqual(0, result.returncode)


class WriteLockDirectoryOverrideTest(unittest.TestCase):
    @staticmethod
    def snapshot(path: Path) -> tuple[bool, bytes | None]:
        exists = path.exists()
        return exists, path.read_bytes() if exists else None

    def test_lock_directory_override_keeps_default_tmp_untouched(self) -> None:
        with tempfile.TemporaryDirectory(prefix="write-lock-override-") as temp:
            base = Path(temp)
            repo = base / "repo"
            lock_dir = base / "locks"
            lock_dir.mkdir()
            subprocess.run(
                ["git", "-c", "init.defaultBranch=main", "init", "-q", str(repo)],
                check=True,
                capture_output=True,
                timeout=HOOK_TIMEOUT,
            )
            key = repo_key(repo)
            for tool in ("claude", "cursor"):
                hooks = repo / f".{tool}" / "hooks"
                hooks.mkdir(parents=True)
                shutil.copytree(
                    EDITION_ROOT / f".{tool}" / "agents",
                    repo / f".{tool}" / "agents",
                )
                gate = hooks / "subagent-gate.sh"
                shutil.copy(hook_path(tool, "subagent-gate.sh"), gate)
                default_lock = Path(f"/tmp/{tool}-write-agent-lock-{key}")
                default_guard = Path(f"{default_lock}.guard")
                default_before = {
                    path: self.snapshot(path)
                    for path in (default_lock, default_guard)
                }
                payload = (
                    {"tool_name": "Agent", "tool_input": {"subagent_type": "coder"}}
                    if tool == "claude"
                    else {"subagent_type": "coder"}
                )
                env = dict(os.environ)
                env["SUBAGENT_WRITE_LOCK_DIR"] = str(lock_dir)
                result = subprocess.run(
                    [BASH, str(gate)],
                    input=json.dumps(payload),
                    capture_output=True,
                    text=True,
                    cwd=str(repo),
                    env=env,
                    timeout=HOOK_TIMEOUT,
                )
                with self.subTest(tool=tool):
                    self.assertEqual(0, result.returncode, result.stderr)
                    self.assertEqual(
                        default_before,
                        {
                            path: self.snapshot(path)
                            for path in (default_lock, default_guard)
                        },
                        "lock directory override must not touch the default /tmp lock",
                    )
                    self.assertEqual(
                        "coder",
                        (lock_dir / f"{tool}-write-agent-lock-{key}").read_text(),
                    )


class WriteLockMixin:
    """Lock paths as the gate and dispatch hooks compute them.

    Both derive the key from the git toplevel above the hook script, which
    inside this monorepo is the repository root.
    """

    def lock_path(self, tool: str) -> Path:
        return self.lock_dir / f"{tool}-write-agent-lock-{repo_key(EDITION_ROOT)}"


class SubagentWriteLockTest(WriteLockMixin, unittest.TestCase):
    """`writes: true` agents are serialized by a TTL lock in both gates."""

    def setUp(self) -> None:
        self._lock_tmp = tempfile.TemporaryDirectory(prefix="write-lock-test-")
        self.lock_dir = Path(self._lock_tmp.name)
        self.lock_env = {"SUBAGENT_WRITE_LOCK_DIR": str(self.lock_dir)}
        self.addCleanup(self._lock_tmp.cleanup)

    def spawn(self, tool: str, agent: str, lock_dir: str | None = None):
        if tool == "claude":
            payload = {"tool_name": "Agent", "tool_input": {"subagent_type": agent}}
        else:
            payload = {"subagent_type": agent}
        env = dict(self.lock_env)
        if lock_dir is not None:
            env["SUBAGENT_WRITE_LOCK_DIR"] = lock_dir
        return run_hook(
            tool,
            "subagent-gate.sh",
            payload,
            env=env,
        )

    def assert_invalid_lock_dir_is_denied(self, lock_dir: Path | str) -> None:
        key = repo_key(EDITION_ROOT)
        lock_dir = str(lock_dir)
        for tool in ("claude", "cursor"):
            result = self.spawn(tool, "coder", lock_dir)
            with self.subTest(tool=tool, lock_dir=lock_dir):
                if tool == "claude":
                    self.assertEqual(2, result.returncode, result.stderr)
                    self.assertIn("BLOCKED", result.stderr)
                else:
                    self.assertEqual(0, result.returncode, result.stderr)
                    verdict = json.loads(result.stdout)
                    self.assertEqual("deny", verdict["permission"])
                    self.assertNotIn(lock_dir, verdict["user_message"])
                candidate_root = Path(lock_dir)
                if not candidate_root.is_absolute():
                    candidate_root = EDITION_ROOT / candidate_root
                candidate = candidate_root / f"{tool}-write-agent-lock-{key}"
                self.assertFalse(candidate.exists())
                self.assertFalse(Path(f"{candidate}.guard").exists())

    def test_relative_existing_lock_directory_is_denied(self) -> None:
        relative = os.path.relpath(self.lock_dir, EDITION_ROOT)
        self.assert_invalid_lock_dir_is_denied(relative)

    def test_nonexistent_absolute_lock_directory_is_denied(self) -> None:
        self.assert_invalid_lock_dir_is_denied(
            self.lock_dir / 'missing-"\nINJECTED'
        )

    def test_write_agent_takes_the_lock_and_blocks_the_next(self) -> None:
        first = self.spawn("claude", "coder")
        self.assertEqual(0, first.returncode, first.stderr)
        self.assertEqual("coder", self.lock_path("claude").read_text())
        second = self.spawn("claude", "refactorer")
        self.assertEqual(2, second.returncode)
        self.assertIn("already running", second.stderr)

    def test_read_only_agent_passes_while_locked(self) -> None:
        self.assertEqual(0, self.spawn("claude", "coder").returncode)
        result = self.spawn("claude", "code-reviewer")
        self.assertEqual(0, result.returncode, result.stderr)

    def test_a_second_instance_of_the_same_agent_is_blocked(self) -> None:
        # The lock used to exempt its own holder's name, so N concurrent
        # `coder` runs all passed and each refreshed the lock. A fresh lock
        # means the holder is still running — subagent-dispatch.sh clears it
        # on completion — so a same-named spawn is a second writer, not a
        # respawn. Staleness stays the TTL's job (test_stale_lock_is_overwritten).
        self.assertEqual(0, self.spawn("claude", "coder").returncode)
        again = self.spawn("claude", "coder")
        self.assertEqual(2, again.returncode)
        self.assertIn("already running", again.stderr)
        self.assertEqual("coder", self.lock_path("claude").read_text())

    def test_stale_lock_is_overwritten(self) -> None:
        lock = self.lock_path("claude")
        lock.write_text("coder")
        stale = time.time() - 3600
        os.utime(lock, (stale, stale))
        result = self.spawn("claude", "refactorer")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual("refactorer", lock.read_text())

    def test_cursor_gate_denies_with_permission_json(self) -> None:
        self.assertEqual(0, self.spawn("cursor", "coder").returncode)
        result = self.spawn("cursor", "refactorer")
        self.assertEqual(0, result.returncode, result.stderr)
        verdict = json.loads(result.stdout)
        self.assertEqual("deny", verdict["permission"])
        self.assertIn("one at a time", verdict["user_message"])

    def test_cursor_denial_omits_the_custom_lock_path(self) -> None:
        with tempfile.TemporaryDirectory(prefix='cursor-lock-"\n') as temp:
            self.assertEqual(
                0, self.spawn("cursor", "coder", temp).returncode
            )
            result = self.spawn("cursor", "refactorer", temp)
            self.assertEqual(0, result.returncode, result.stderr)
            self.assertNotIn(temp, result.stdout)
            self.assertEqual("deny", json.loads(result.stdout)["permission"])

    def test_cursor_read_only_allows_while_locked(self) -> None:
        self.assertEqual(0, self.spawn("cursor", "coder").returncode)
        result = self.spawn("cursor", "code-reviewer")
        self.assertEqual(
            "allow", json.loads(result.stdout)["permission"], result.stderr
        )

    def test_concurrent_write_spawns_take_the_lock_exactly_once(self) -> None:
        """Parallel spawns in one message must not both pass the gate.

        The check-and-take is serialized with flock; without it both
        invocations read an unlocked state and mutual exclusion is lost.
        """
        if shutil.which("flock") is None:
            self.skipTest("flock unavailable; the gate degrades by design")
        agents = ("coder", "refactorer", "test-generator")
        with ThreadPoolExecutor(max_workers=len(agents)) as pool:
            codes = [
                result.returncode
                for result in pool.map(lambda a: self.spawn("claude", a), agents)
            ]
        self.assertEqual(1, codes.count(0), codes)
        self.assertEqual(len(agents) - 1, codes.count(2), codes)

    def test_body_horizontal_rule_cannot_declare_writes(self) -> None:
        """Only the first frontmatter block is metadata.

        A `---`-delimited section in an agent's body used to re-open the
        sed range, so a body line reading `writes: true` marked a read-only
        agent as write-capable.

        The fixture agent lives in a throwaway repository with its own copy
        of the gate. Written into this edition's `.claude/agents/`, it was for
        a moment an unmirrored agent of the real tree: a mirror, parity or
        route gate reading the tree at that moment failed, and a run stopped
        at that moment left the file behind.
        """
        with tempfile.TemporaryDirectory(prefix="parser-trap-") as temp:
            repo = Path(temp)
            subprocess.run(
                ["git", "-c", "init.defaultBranch=main", "init", "-q", str(repo)],
                check=True,
                capture_output=True,
                timeout=HOOK_TIMEOUT,
            )
            (repo / ".claude" / "agents").mkdir(parents=True)
            (repo / ".claude" / "agents" / "zz-parser-trap-agent.md").write_text(
                "---\nname: zz-parser-trap\ndescription: read-only fixture\n"
                "phase: understanding\n---\n\n# Trap\n\nProse.\n\n---\n"
                "writes: true\n---\n\nMore prose.\n",
                encoding="utf-8",
            )
            (repo / ".claude" / "hooks").mkdir()
            gate = repo / ".claude" / "hooks" / "subagent-gate.sh"
            shutil.copy(hook_path("claude", "subagent-gate.sh"), gate)
            result = subprocess.run(
                [BASH, str(gate)],
                input=json.dumps(
                    {"tool_name": "Agent", "tool_input": {"subagent_type": "zz-parser-trap"}}
                ),
                capture_output=True,
                text=True,
                cwd=str(repo),
                env={**os.environ, **self.lock_env},
                timeout=HOOK_TIMEOUT,
            )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(
            [],
            sorted(self.lock_dir.glob("claude-write-agent-lock-*")),
            "a body horizontal rule must not make an agent write-capable",
        )


class FixtureIsolationTest(unittest.TestCase):
    """Agent fixtures are written outside the edition tree.

    scripts/check.py runs this suite beside the mirror, parity and route
    gates in one checkout, and those read `.claude/agents/` from disk.
    """

    def test_parser_trap_fixture_is_written_outside_the_edition(self) -> None:
        written: list[Path] = []
        write_text = Path.write_text

        def recording(path: Path, *args, **kwargs):
            written.append(Path(os.path.abspath(path)))
            return write_text(path, *args, **kwargs)

        outcome = unittest.TestResult()
        with mock.patch.object(Path, "write_text", recording):
            SubagentWriteLockTest(
                "test_body_horizontal_rule_cannot_declare_writes"
            ).run(outcome)
        self.assertEqual([], outcome.failures + outcome.errors)
        self.assertTrue(written)
        self.assertEqual(
            [], [path for path in written if EDITION_ROOT in path.parents]
        )


class SubagentDispatchTest(WriteLockMixin, unittest.TestCase):
    """The SubagentStop observer records completions and releases the lock.

    The hook resolves its runtime relative to its own location, so it is
    exercised from an isolated copy of the edition layout: hooks plus the
    Python runtime in a throwaway git repository.
    """

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="dispatch-test-")
        self.repo = Path(self._tmp.name)
        (self.repo / ".claude" / "hooks").mkdir(parents=True)
        shutil.copytree(
            EDITION_ROOT / ".claude" / "agents",
            self.repo / ".claude" / "agents",
        )
        scripts = self.repo / "memory-bank" / "scripts"
        scripts.mkdir(parents=True)
        for source in (EDITION_ROOT / "memory-bank" / "scripts").glob("*.py"):
            shutil.copy(source, scripts / source.name)
        self.gate = self.repo / ".claude" / "hooks" / "subagent-gate.sh"
        shutil.copy(hook_path("claude", "subagent-gate.sh"), self.gate)
        self.hook = self.repo / ".claude" / "hooks" / "subagent-dispatch.sh"
        shutil.copy(hook_path("claude", "subagent-dispatch.sh"), self.hook)
        subprocess.run(
            ["git", "-c", "init.defaultBranch=main", "init", "-q", str(self.repo)],
            check=True, capture_output=True, timeout=HOOK_TIMEOUT,
        )
        subprocess.run(
            ["git", "-C", str(self.repo), "checkout", "-qb", "feat/demo"],
            check=True, capture_output=True, timeout=HOOK_TIMEOUT,
        )
        started = self.cli("start", "--task-id", "feat/demo", "--goal", "Demo")
        self.assertEqual(0, started.returncode, started.stderr)
        self.lock_dir = self.repo / ".test-write-locks"
        self.lock_dir.mkdir()
        self.lock_env = {"SUBAGENT_WRITE_LOCK_DIR": str(self.lock_dir)}
        self.addCleanup(self._tmp.cleanup)

    def cli(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(self.repo / "memory-bank" / "scripts" / "context.py"),
             "--root", str(self.repo), *arguments],
            text=True, capture_output=True, timeout=HOOK_TIMEOUT,
        )

    def run_gate(self, agent: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [BASH, str(self.gate)],
            input=json.dumps(
                {"tool_name": "Agent", "tool_input": {"subagent_type": agent}}
            ),
            text=True,
            capture_output=True,
            cwd=str(self.repo),
            env={**os.environ, **self.lock_env},
            timeout=HOOK_TIMEOUT,
        )

    def run_dispatch(
        self, payload, lock_dir: str | None = None
    ) -> subprocess.CompletedProcess[str]:
        stdin = payload if isinstance(payload, str) else json.dumps(payload)
        env = dict(self.lock_env)
        if lock_dir is not None:
            env["SUBAGENT_WRITE_LOCK_DIR"] = lock_dir
        return subprocess.run(
            [BASH, str(self.hook)], input=stdin, text=True,
            capture_output=True, cwd=str(self.repo),
            env={**os.environ, **env}, timeout=HOOK_TIMEOUT,
        )

    def journal(self) -> list[dict[str, object]]:
        result = self.cli("msg-read", "--task-id", "feat/demo", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def test_claude_payload_records_sanitized_completion(self) -> None:
        result = self.run_dispatch({
            "hook_event_name": "SubagentStop",
            "agent_type": "coder",
            "last_assistant_message": "Implemented handler.\nAll tests pass.",
        })
        self.assertEqual(0, result.returncode, result.stderr)
        entries = self.journal()
        self.assertEqual(1, len(entries))
        self.assertEqual("completion", entries[0]["type"])
        self.assertEqual("coder", entries[0]["from_actor"])
        self.assertEqual(
            "Implemented handler. All tests pass.", entries[0]["body"]
        )

    def test_cursor_payload_uses_subagent_type_and_status(self) -> None:
        result = self.run_dispatch(
            {"subagent_type": "code-reviewer", "status": "completed"}
        )
        self.assertEqual(0, result.returncode, result.stderr)
        entries = self.journal()
        self.assertEqual("code-reviewer", entries[0]["from_actor"])
        self.assertEqual("completed", entries[0]["body"])

    def test_releases_only_the_holders_lock(self) -> None:
        key = repo_key(self.repo)
        mine = self.lock_dir / f"claude-write-agent-lock-{key}"
        mine.write_text("coder")
        other = self.lock_dir / f"cursor-write-agent-lock-{key}"
        other.write_text("refactorer")
        self.assertEqual(0, self.run_dispatch({"agent_type": "coder"}).returncode)
        self.assertFalse(mine.exists())
        self.assertTrue(other.exists())

    def test_gate_lock_is_released_by_dispatch_with_the_same_override(self) -> None:
        gate = self.run_gate("coder")
        self.assertEqual(0, gate.returncode, gate.stderr)
        lock = self.lock_dir / f"claude-write-agent-lock-{repo_key(self.repo)}"
        self.assertEqual("coder", lock.read_text())
        dispatched = self.run_dispatch({"agent_type": "coder"})
        self.assertEqual(0, dispatched.returncode, dispatched.stderr)
        self.assertFalse(lock.exists())

    def test_dispatch_with_relative_lock_directory_skips_cleanup_only(self) -> None:
        lock = self.lock_dir / f"claude-write-agent-lock-{repo_key(self.repo)}"
        lock.write_text("coder")
        relative = os.path.relpath(self.lock_dir, self.repo)
        result = self.run_dispatch({"agent_type": "coder"}, relative)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertTrue(lock.exists())
        self.assertEqual("coder", self.journal()[0]["from_actor"])

    def test_dispatch_with_nonexistent_lock_directory_skips_cleanup_only(self) -> None:
        missing = str(self.lock_dir / "missing")
        result = self.run_dispatch({"agent_type": "coder"}, missing)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse(Path(missing).exists())
        self.assertEqual("coder", self.journal()[0]["from_actor"])

    def test_fails_open_without_agent_or_runtime(self) -> None:
        self.assertEqual(0, self.run_dispatch({"status": "done"}).returncode)
        self.assertEqual([], self.journal())
        shutil.rmtree(self.repo / "memory-bank")
        result = self.run_dispatch({"agent_type": "coder"})
        self.assertEqual(0, result.returncode, result.stderr)

    def failing_cli(self, message: str) -> None:
        (self.repo / "memory-bank" / "scripts" / "context.py").write_text(
            f"import sys\nsys.stderr.write({message!r} + '\\n')\nsys.exit(1)\n",
            encoding="utf-8",
        )

    def test_a_lost_completion_write_is_reported(self) -> None:
        # The flows read the channel as the record of who finished; a write
        # that failed in silence made a finished agent look unfinished.
        self.failing_cli("context: database is locked")
        result = self.run_dispatch({"agent_type": "coder"})
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn('completion of "coder" was NOT recorded', result.stdout)
        self.assertIn("database is locked", result.stdout)
        self.assertIn("NOT recorded", result.stderr)

    def test_a_crash_is_reported_by_its_exception_not_its_traceback_header(self) -> None:
        # An uncaught exception's first stderr line names nothing; its last
        # line names the error.
        self.failing_cli(
            "Traceback (most recent call last):\n"
            "  File \"/install/path/memory-bank/scripts/context.py\", line 1, in <module>\n"
            "UnicodeDecodeError: 'utf-8' codec can't decode byte 0xff in position 290\n"
        )
        result = self.run_dispatch({"agent_type": "coder"})
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("UnicodeDecodeError: 'utf-8' codec can't decode byte 0xff", result.stdout)
        self.assertNotIn("Traceback", result.stdout)
        self.assertNotIn("/install/path", result.stdout)

    def test_a_lost_completion_reaches_the_orchestrator_after_the_agent_returns(self) -> None:
        # Claude Code shows a SubagentStop hook's output to no conversation the
        # orchestrator reads; a PostToolUse hook on the Agent tool is its way
        # into the parent. The report waits for that hook, which hands it on
        # once, records nothing and releases nothing.
        self.failing_cli("context: database is locked")
        stopped = self.run_dispatch(
            {"hook_event_name": "SubagentStop", "agent_type": "coder", "session_id": "s-1"}
        )
        self.assertIn("NOT recorded", stopped.stdout)
        lock = self.lock_dir / f"claude-write-agent-lock-{repo_key(self.repo)}"
        lock.write_text("coder")
        returned = {
            "hook_event_name": "PostToolUse", "session_id": "s-1", "tool_name": "Agent",
            "tool_input": {"subagent_type": "coder", "prompt": "do it"},
        }
        relayed = self.run_dispatch(returned)
        self.assertEqual((0, ""), (relayed.returncode, relayed.stderr))
        output = json.loads(relayed.stdout)["hookSpecificOutput"]
        self.assertEqual("PostToolUse", output["hookEventName"])
        self.assertIn('completion of "coder" was NOT recorded', output["additionalContext"])
        self.assertIn("database is locked", output["additionalContext"])
        self.assertTrue(lock.exists(), "the relay released a lock")
        again = self.run_dispatch(returned)
        self.assertEqual((0, "", ""), (again.returncode, again.stdout, again.stderr))

    def test_cursor_reports_are_not_left_for_a_relay(self) -> None:
        # Cursor names the event subagentStop and has no relay to read them.
        self.failing_cli("context: database is locked")
        self.run_dispatch({"hook_event_name": "subagentStop", "subagent_type": "coder"})
        self.assertFalse((self.repo / "memory-bank" / "local" / "unrecorded-completions").exists())

    def test_no_channel_means_nothing_to_report(self) -> None:
        for message in (
            "context: Working task not found: feat/demo",
            "context: Agent messages require governed mode",
        ):
            with self.subTest(message=message):
                self.failing_cli(message)
                result = self.run_dispatch({"agent_type": "coder"})
                self.assertEqual((0, "", ""), (result.returncode, result.stdout, result.stderr))


class MirrorConsistencyTest(unittest.TestCase):
    def test_bash_validator_mirrors_differ_only_by_counter_namespace(self) -> None:
        # Documented invariant: bash-validator's per-mirror adaptations are
        # the counter directory its repetition guard shares with
        # loop-detection.sh, which is namespaced per tool so two hosts driving
        # one checkout cannot inflate each other's counts, and the Codex
        # mirror naming the debugger skill instead of the slash command.
        # Everything else - every pattern, threshold and message - is
        # identical, so a rule added to one host reaches all three.
        contents = {
            tool: hook_path(tool, "bash-validator.sh").read_text(encoding="utf-8")
            for tool in MIRRORS
        }
        for tool, text in contents.items():
            self.assertIn(f"/{MIRRORS[tool][2]}-loop-detection-", text)
        # The third: Cursor keeps the repetition warning user-visible (text,
        # exit 1), since it has no channel that shows the agent a message
        # about a command it lets run.
        warning = re.compile(
            r'(elif \[ "\$COUNT" -ge 6 \]; then\n  BV_WARNING=[^\n]*\n).*?(\nfi\n)', re.S
        )
        normalized = {
            tool: warning.sub(
                r"\1  WARN\2",
                text.replace(
                    f"/{MIRRORS[tool][2]}-loop-detection-", "/TOOL-loop-detection-"
                ).replace("systematic-debugger", "/debugger"),
            )
            for tool, text in contents.items()
        }
        self.assertEqual(normalized["claude"], normalized["cursor"])
        self.assertEqual(normalized["claude"], normalized["codex"])
        self.assertIn("additionalContext", contents["codex"])
        self.assertIn("exit 1", warning.search(contents["cursor"]).group(0))


class HookWiringTest(unittest.TestCase):
    """What Claude Code runs the edit counter and the dispatch relay on."""

    def post_tool_use(self) -> dict[str, list[str]]:
        settings = json.loads((EDITION_ROOT / ".claude" / "settings.json").read_text(encoding="utf-8"))
        wired: dict[str, list[str]] = {}
        for group in settings["hooks"]["PostToolUse"]:
            for hook in group["hooks"]:
                wired.setdefault(hook["command"].rsplit("/", 1)[-1], []).extend(
                    name.strip() for name in group.get("matcher", "").split("|")
                )
        return wired

    def test_every_edit_tool_reaches_the_edit_counter(self) -> None:
        # A Write that never reached loop-detection.sh never restarted a
        # command's repetition count either.
        matched = self.post_tool_use()["loop-detection.sh"]
        for tool in ("Edit", "Write", "MultiEdit", "NotebookEdit"):
            self.assertIn(tool, matched)

    def test_the_dispatch_relay_runs_when_an_agent_returns(self) -> None:
        self.assertIn("Agent", self.post_tool_use()["subagent-dispatch.sh"])


if __name__ == "__main__":
    unittest.main()
