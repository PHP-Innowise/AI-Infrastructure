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
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

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


def track_dir(tool: str, repo: Path) -> Path:
    return Path("/tmp/{}-loop-detection-{}".format(MIRRORS[tool][2], repo_key(repo)))


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
                shutil.rmtree(track_dir(tool, repo), ignore_errors=True)
        self._tmp.cleanup()


class BashValidatorTest(unittest.TestCase):
    HOOK = "bash-validator.sh"

    @staticmethod
    def payload(command: str) -> dict:
        return {"tool_name": "Bash", "tool_input": {"command": command}}

    def test_safe_command_passes(self) -> None:
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(tool, self.HOOK, self.payload("ls -la src/"))
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
                result = run_hook(tool, self.HOOK, self.payload(command))
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, "")

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
                    self.assertEqual(result.returncode, 1, "count={}".format(count))
                    self.assertIn("WARNING", result.stdout)
                    self.assertEqual(result.stderr, "")
                result = self.edit(tool, self.repo_a)
                self.assertEqual(result.returncode, 2)
                self.assertIn("BLOCKED", result.stderr)
                self.assertEqual(result.stdout, "")

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
                for repo in (self.repo_a, self.repo_b):
                    primed = run_hook(
                        tool, "loop-detection.sh", edit_payload("/work/App.php"), cwd=repo
                    )
                    self.assertEqual(primed.returncode, 0, primed.stderr)
                dir_a = track_dir(tool, self.repo_a)
                dir_b = track_dir(tool, self.repo_b)
                self.assertEqual(len(counter_files(dir_a)), 1)
                self.assertEqual(len(counter_files(dir_b)), 1)

                tmp_root = self.cache_root / tool
                tmp_root.mkdir(exist_ok=True)
                result = run_hook(
                    tool,
                    self.HOOK,
                    "",
                    cwd=self.repo_a,
                    env={"TMPDIR": str(tmp_root)},
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(counter_files(dir_a), [])
                self.assertEqual(len(counter_files(dir_b)), 1)


class WorkingMemoryRuleTest(unittest.TestCase):
    """Cursor's read path: hooks render the capsule into an alwaysApply rule.

    Cursor has no UserPromptSubmit-equivalent event, so its mirrors of the
    Stop and sessionStart hooks render the freshest Task Capsule into
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

    def test_failed_render_preserves_previous_rule(self) -> None:
        self.rule_file().write_text("previous capsule\n", encoding="utf-8")
        self.cli.write_text(self.STUB_FAILURE, encoding="utf-8")
        for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
            with self.subTest(hook=hook):
                result = self.run_cursor_hook(hook)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    self.rule_file().read_text(encoding="utf-8"),
                    "previous capsule\n",
                )
                # No temp litter: an interrupted render must not accumulate.
                self.assertEqual(
                    [path.name for path in self.rules_dir.iterdir()],
                    ["working-memory.mdc"],
                )

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
        self.rule_file().write_text("previous capsule\n", encoding="utf-8")
        self.cli.write_text(self.STUB_MALFORMED, encoding="utf-8")
        for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
            with self.subTest(hook=hook):
                result = self.run_cursor_hook(hook)
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertEqual(
                    "previous capsule\n",
                    self.rule_file().read_text(encoding="utf-8"),
                )

    @unittest.skipUnless(shutil.which("timeout"), "native timeout unavailable")
    def test_timed_out_render_preserves_previous_rule(self) -> None:
        self.rule_file().write_text("previous capsule\n", encoding="utf-8")
        self.cli.write_text(self.STUB_TIMEOUT, encoding="utf-8")
        result = self.run_cursor_hook(
            "local-context.sh", budget="0.05"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(
            "previous capsule\n",
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
        self.clear_write_locks()
        self.addCleanup(self.clear_write_locks)

    @staticmethod
    def clear_write_locks() -> None:
        key = repo_key(EDITION_ROOT)
        for tool in ("claude", "cursor"):
            Path(f"/tmp/{tool}-write-agent-lock-{key}").unlink(missing_ok=True)

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
                result = run_hook(
                    "claude", "subagent-gate.sh", self.spawn_payload(builtin)
                )
                self.assertEqual(2, result.returncode)
                self.assertIn("BLOCKED", result.stderr)

    def test_claude_blocks_via_legacy_task_tool_name(self) -> None:
        payload = {
            "tool_name": "Task",
            "tool_input": {"subagent_type": "general-purpose"},
        }
        result = run_hook("claude", "subagent-gate.sh", payload)
        self.assertEqual(2, result.returncode)

    def test_claude_allows_every_roster_agent(self) -> None:
        names = self.roster("claude")
        self.assertTrue(names)
        for name in names:
            with self.subTest(agent=name):
                result = run_hook(
                    "claude", "subagent-gate.sh", self.spawn_payload(name)
                )
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
                result = run_hook("claude", "subagent-gate.sh", payload)
                self.assertEqual(0, result.returncode)

    def test_cursor_denies_builtins_with_permission_json(self) -> None:
        for builtin in self.BUILTIN_CURSOR:
            with self.subTest(agent=builtin):
                result = run_hook(
                    "cursor", "subagent-gate.sh", {"subagent_type": builtin}
                )
                self.assertEqual(0, result.returncode, result.stderr)
                verdict = json.loads(result.stdout)
                self.assertEqual("deny", verdict["permission"])
                self.assertIn("user_message", verdict)

    def test_cursor_allows_every_roster_agent(self) -> None:
        names = self.roster("cursor")
        self.assertTrue(names)
        for name in names:
            with self.subTest(agent=name):
                result = run_hook(
                    "cursor", "subagent-gate.sh", {"subagent_type": name}
                )
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertEqual(
                    "allow", json.loads(result.stdout)["permission"]
                )
            self.clear_write_locks()

    def test_cursor_typeless_payload_allows(self) -> None:
        result = run_hook("cursor", "subagent-gate.sh", {"task": "x"})
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
                result = run_hook(
                    "codex",
                    "subagent-gate.sh",
                    {"tool_name": tool, "tool_input": {}},
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
                result = run_hook("codex", "subagent-gate.sh", payload)
                self.assertEqual(0, result.returncode)


class WriteLockMixin:
    """Lock paths as the gate and dispatch hooks compute them.

    Both derive the key from the git toplevel above the hook script, which
    inside this monorepo is the repository root.
    """

    def lock_path(self, tool: str) -> Path:
        return Path(f"/tmp/{tool}-write-agent-lock-{repo_key(EDITION_ROOT)}")

    def clear_locks(self) -> None:
        for tool in ("claude", "cursor"):
            self.lock_path(tool).unlink(missing_ok=True)


class SubagentWriteLockTest(WriteLockMixin, unittest.TestCase):
    """`writes: true` agents are serialized by a TTL lock in both gates."""

    def setUp(self) -> None:
        self.clear_locks()
        self.addCleanup(self.clear_locks)

    @staticmethod
    def spawn(tool: str, agent: str):
        if tool == "claude":
            payload = {"tool_name": "Agent", "tool_input": {"subagent_type": agent}}
        else:
            payload = {"subagent_type": agent}
        return run_hook(tool, "subagent-gate.sh", payload)

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

    def test_same_agent_respawn_refreshes_the_lock(self) -> None:
        self.assertEqual(0, self.spawn("claude", "coder").returncode)
        again = self.spawn("claude", "coder")
        self.assertEqual(0, again.returncode, again.stderr)
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
        """
        trap = EDITION_ROOT / ".claude" / "agents" / "zz-parser-trap-agent.md"
        trap.write_text(
            "---\nname: zz-parser-trap\ndescription: read-only fixture\n"
            "phase: understanding\n---\n\n# Trap\n\nProse.\n\n---\n"
            "writes: true\n---\n\nMore prose.\n",
            encoding="utf-8",
        )
        self.addCleanup(trap.unlink)
        result = self.spawn("claude", "zz-parser-trap")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse(
            self.lock_path("claude").exists(),
            "a body horizontal rule must not make an agent write-capable",
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
        scripts = self.repo / "memory-bank" / "scripts"
        scripts.mkdir(parents=True)
        for source in (EDITION_ROOT / "memory-bank" / "scripts").glob("*.py"):
            shutil.copy(source, scripts / source.name)
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
        self.addCleanup(self._tmp.cleanup)
        self.addCleanup(self.repo_locks_cleanup)

    def repo_locks_cleanup(self) -> None:
        key = repo_key(self.repo)
        for tool in ("claude", "cursor"):
            Path(f"/tmp/{tool}-write-agent-lock-{key}").unlink(missing_ok=True)

    def cli(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(self.repo / "memory-bank" / "scripts" / "context.py"),
             "--root", str(self.repo), *arguments],
            text=True, capture_output=True, timeout=HOOK_TIMEOUT,
        )

    def run_dispatch(self, payload) -> subprocess.CompletedProcess[str]:
        stdin = payload if isinstance(payload, str) else json.dumps(payload)
        return subprocess.run(
            [BASH, str(self.hook)], input=stdin, text=True,
            capture_output=True, cwd=str(self.repo), timeout=HOOK_TIMEOUT,
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
        mine = Path(f"/tmp/claude-write-agent-lock-{key}")
        mine.write_text("coder")
        other = Path(f"/tmp/cursor-write-agent-lock-{key}")
        other.write_text("refactorer")
        self.assertEqual(0, self.run_dispatch({"agent_type": "coder"}).returncode)
        self.assertFalse(mine.exists())
        self.assertTrue(other.exists())

    def test_fails_open_without_agent_or_runtime(self) -> None:
        self.assertEqual(0, self.run_dispatch({"status": "done"}).returncode)
        self.assertEqual([], self.journal())
        shutil.rmtree(self.repo / "memory-bank")
        result = self.run_dispatch({"agent_type": "coder"})
        self.assertEqual(0, result.returncode, result.stderr)


class MirrorConsistencyTest(unittest.TestCase):
    def test_bash_validator_mirrors_are_byte_identical(self) -> None:
        # Documented invariant: bash-validator has zero per-mirror
        # adaptations inside an edition.
        contents = {
            tool: hook_path(tool, "bash-validator.sh").read_bytes() for tool in MIRRORS
        }
        self.assertEqual(contents["claude"], contents["cursor"])
        self.assertEqual(contents["claude"], contents["codex"])


if __name__ == "__main__":
    unittest.main()
