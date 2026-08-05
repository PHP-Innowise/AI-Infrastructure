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
import tempfile
import unittest
from pathlib import Path

EDITION_ROOT = Path(__file__).resolve().parents[2]

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
        ["bash", str(hook_path(tool, name))],
        input=stdin,
        capture_output=True,
        text=True,
        cwd=str(cwd) if cwd is not None else str(EDITION_ROOT),
        env=merged_env,
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


def run_hook_restricted(tool: str, name: str, payload, path_value: str):
    """Run a hook under a minimal PATH (regression: no external `cat`)."""
    bash = shutil.which("bash")
    if bash is None:
        raise unittest.SkipTest("bash not found")
    stdin = payload if isinstance(payload, str) else json.dumps(payload)
    env = dict(os.environ)
    env["PATH"] = path_value
    return subprocess.run(
        [bash, str(hook_path(tool, name))],
        input=stdin,
        capture_output=True,
        text=True,
        cwd=str(EDITION_ROOT),
        env=env,
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

    def test_stdin_read_does_not_need_external_cat(self) -> None:
        # TC-030 regression: stdin is read with the bash builtin. With no
        # `cat` in PATH the validator used to lose the payload and silently
        # pass every command - including destructive ones.
        with tempfile.TemporaryDirectory(prefix="no-cat-") as tmp:
            path_value = restricted_bin(Path(tmp), ("grep", "python3"))
            for tool in MIRRORS:
                with self.subTest(tool=tool):
                    result = run_hook_restricted(
                        tool,
                        self.HOOK,
                        self.payload("git reset --hard HEAD~1"),
                        path_value,
                    )
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertIn("BLOCKED", result.stderr)
                    self.assertNotIn("command not found", result.stderr)

    def test_no_extractor_warning_survives_missing_cat(self) -> None:
        # TC-030 regression: with neither `cat` nor any JSON extractor in
        # PATH, the fail-open warning must still reach stderr instead of
        # being lost together with the unread payload.
        with tempfile.TemporaryDirectory(prefix="no-extractor-") as tmp:
            path_value = restricted_bin(Path(tmp), ("grep",))
            for tool in MIRRORS:
                with self.subTest(tool=tool):
                    result = run_hook_restricted(
                        tool,
                        self.HOOK,
                        self.payload("git reset --hard HEAD~1"),
                        path_value,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("no JSON extractor available", result.stderr)
                    self.assertEqual(result.stdout, "")

    def test_block_message_names_the_matching_pattern(self) -> None:
        # The combined single-pass scan must still report which concrete
        # pattern matched, not just that something did.
        for tool in MIRRORS:
            with self.subTest(tool=tool):
                result = run_hook(
                    tool, self.HOOK, self.payload("git reset --hard HEAD~1")
                )
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("matches pattern '", result.stderr)
                self.assertIn("reset", result.stderr)


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
                self.assertIn("as of end of previous turn", cursor_text)
                for tool in ("claude", "codex"):
                    text = hook_path(tool, hook).read_text(encoding="utf-8")
                    self.assertNotIn("working-memory.mdc", text)


class CursorCapsuleRenderTest(unittest.TestCase):
    """Functional render tests against a throwaway edition tree.

    The hooks resolve every path from their own location, so copying the
    Cursor mirrors into a fake edition with a stub context.py exercises the
    real render logic without touching this edition's index or rule file.
    """

    TASK_ID = "TASK-STUB"

    STUB_CAPSULE = (
        "import sys\n"
        "if len(sys.argv) > 1 and sys.argv[1] == \"context\":\n"
        "    print(\"working: TASK-STUB — stub goal\")\n"
        "    print(\"semantic:\")\n"
        "    print(\"  docs/A.md — Stub doc\")\n"
    )
    STUB_SILENT = "import sys\n"

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

    def run_cursor_hook(self, name: str):
        env = dict(os.environ)
        env["CONTEXT_TASK_ID"] = self.TASK_ID
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
        self.assertIn("as of end of previous turn", rendered)
        self.assertIn("task: TASK-STUB", rendered)
        self.assertIn("stub goal", rendered)
        self.assertIn("not authoritative", rendered)

    def test_session_start_renders_rule_without_printing_it(self) -> None:
        self.cli.write_text(self.STUB_CAPSULE, encoding="utf-8")
        result = self.run_cursor_hook("local-context.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("stub goal", result.stdout)
        self.assertIn("stub goal", self.rule_file().read_text(encoding="utf-8"))

    def test_failed_render_preserves_previous_rule_for_same_task(self) -> None:
        # A transient render failure must not downgrade a capsule already
        # rendered for the current task (one turn stale beats a placeholder).
        previous = (
            "# Working Memory (auto-rendered)\n\n"
            "Task Capsule as of end of previous turn"
            " (task: TASK-STUB, rendered: earlier).\n"
            "previous capsule\n"
        )
        self.rule_file().write_text(previous, encoding="utf-8")
        self.cli.write_text(self.STUB_SILENT, encoding="utf-8")
        for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
            with self.subTest(hook=hook):
                result = self.run_cursor_hook(hook)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    self.rule_file().read_text(encoding="utf-8"), previous
                )
                # No temp litter: an interrupted render must not accumulate.
                self.assertEqual(
                    [path.name for path in self.rules_dir.iterdir()],
                    ["working-memory.mdc"],
                )

    def test_cold_start_renders_warming_up_placeholder(self) -> None:
        # DEF-001 / TC-004 regression: before the working task auto-provisions
        # (the flush-after boundary) the capsule render is empty, and a fresh
        # Cursor session used to run its first turns with no rule at all.
        self.cli.write_text(self.STUB_SILENT, encoding="utf-8")
        for hook in WorkingMemoryRuleTest.RENDER_HOOKS:
            with self.subTest(hook=hook):
                if self.rule_file().exists():
                    self.rule_file().unlink()
                result = self.run_cursor_hook(hook)
                self.assertEqual(result.returncode, 0, result.stderr)
                rendered = self.rule_file().read_text(encoding="utf-8")
                self.assertTrue(rendered.startswith("---\n"))
                self.assertIn("alwaysApply: true", rendered)
                self.assertIn("warming up", rendered)
                self.assertIn("(task: TASK-STUB,", rendered)
                self.assertIn("auto-provisions", rendered)
                self.assertIn("not authoritative", rendered)
                # The placeholder goes to the rule file only, never stdout.
                self.assertNotIn("warming up", result.stdout)
                self.assertEqual(
                    [path.name for path in self.rules_dir.iterdir()],
                    ["working-memory.mdc"],
                )

    def test_placeholder_folds_in_last_turn_report(self) -> None:
        # The placeholder carries the last turn report from ignored local
        # state when one exists, so a cold start still says what the write
        # hook last did.
        self.cli.write_text(self.STUB_SILENT, encoding="utf-8")
        report_dir = self.root / "memory-bank" / "local"
        report_dir.mkdir(parents=True)
        (report_dir / "last-turn-report.json").write_text(
            '{"task_id": "TASK-STUB", "flushed": false, "pending": 2}\n',
            encoding="utf-8",
        )
        result = self.run_cursor_hook("working-memory-write.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        rendered = self.rule_file().read_text(encoding="utf-8")
        self.assertIn("warming up", rendered)
        self.assertIn("Last turn report", rendered)
        self.assertIn('"pending": 2', rendered)

    def test_stale_other_task_rule_replaced_by_placeholder(self) -> None:
        # A rule left over from a different task (branch switch) must not be
        # served as this task's memory when the fresh render is empty.
        self.rule_file().write_text(
            "Task Capsule as of end of previous turn"
            " (task: OTHER-TASK, rendered: earlier).\n",
            encoding="utf-8",
        )
        self.cli.write_text(self.STUB_SILENT, encoding="utf-8")
        result = self.run_cursor_hook("local-context.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        rendered = self.rule_file().read_text(encoding="utf-8")
        self.assertIn("warming up", rendered)
        self.assertIn("(task: TASK-STUB,", rendered)
        self.assertNotIn("OTHER-TASK", rendered)

    def test_capsule_replaces_placeholder_once_available(self) -> None:
        self.cli.write_text(self.STUB_SILENT, encoding="utf-8")
        self.run_cursor_hook("working-memory-write.sh")
        self.assertIn(
            "warming up", self.rule_file().read_text(encoding="utf-8")
        )
        self.cli.write_text(self.STUB_CAPSULE, encoding="utf-8")
        result = self.run_cursor_hook("working-memory-write.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        rendered = self.rule_file().read_text(encoding="utf-8")
        self.assertIn("auto-rendered", rendered)
        self.assertIn("stub goal", rendered)
        self.assertNotIn("warming up", rendered)


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
