#!/usr/bin/env python3
"""Generated Stop hooks write their line in the native result journal.

The seeded runtime appends one metadata line per prompt, Stop and save to
ignored `memory-bank/local/native-results.ndjson`, so a measurement of real
sessions can count from disk how far automatic saves get. The one line it
cannot write is the Stop hook's own: a save request killed at the hook's
budget, or failed before its own line, leaves nothing - so the shell appends
the request's exit status (124 when the budget killed it), or the Harness
skip. The editions ship that block; hook-forge documents it for generated
hooks and bootstrap-verifier requires it once the seeded runtime journals.

These tests hold the documented fragment, the verifier and the seeded asset
to that: the fragment passes both native contracts for every host and writes
exactly one line of status and nothing else, never through a link; a hook
without it, or rewritten, fails the gate; a project seeded before the journal
keeps its hooks. The fragment is our own documentation and runs here in a
temporary project; no generated hook is executed.

Run from this directory: python3 test_native_journal_generator.py
"""

from __future__ import annotations

import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
SPEC = importlib.util.spec_from_file_location(
    "validate_generated_native_journal", SCRIPTS / "validate_generated.py"
)
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)

HOOK_FORGE = ROOT / ".agents/skills/hook-forge/SKILL.md"
SEEDED_RUNTIME = ROOT / ".agents/skills/memory-seed/assets/scripts/native_results.py"
JOURNAL = "memory-bank/local/native-results.ndjson"
HOSTS = ("claude", "codex", "cursor")
DIAGNOSTIC = "lacks the native result journal line (hook-forge step 6)"
BASH = shutil.which("bash")
STAMP = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ")

# What a generated working-memory-write.sh holds before its Stop block: the
# root, the state directory beside it, the runtime entry, the budget and the
# payload. TEST_PYTHON and TEST_STATE_DIR let a test pick the interpreter and
# an attached state; the contract does not read them.
PREAMBLE = (
    "#!/bin/bash\n"
    "set -u\n"
    'ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)\n'
    "STATE_DIR=$ROOT_DIR\n"
    '[ -n "${TEST_STATE_DIR:-}" ] && STATE_DIR=$TEST_STATE_DIR\n'
    'CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context_cli.py"\n'
    '[ -f "$CONTEXT_CLI" ] || CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"\n'
    'BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"\n'
    'PYTHON=("${TEST_PYTHON:-python3}")\n'
    "HOOK_STDIN=$(cat 2>/dev/null)\n"
    'TASK_ID="${CONTEXT_TASK_ID:-feature-x}"\n'
)

# The Stop block a hook generated before the journal carries: the bounded
# save request alone. It passes the adapter contract and nothing more.
ADAPTER_ONLY = """\
if [ "${CONTEXT_CAPSULE_DELIVERED:-}" != "1" ] && [ -f "$ROOT_DIR/memory-bank/scripts/native_results.py" ]; then
  if command -v timeout > /dev/null 2>&1; then
    NATIVE_RESPONSE=$(printf '%s' "$HOOK_STDIN" | timeout "$BUDGET_SECONDS" "${PYTHON[@]}" "$CONTEXT_CLI" native-stop \\
      --host claude --task-id "$TASK_ID" 2>/dev/null)
  else
    NATIVE_RESPONSE=$(printf '%s' "$HOOK_STDIN" | "${PYTHON[@]}" "$CONTEXT_CLI" native-stop \\
      --host claude --task-id "$TASK_ID" 2>/dev/null)
  fi
  NATIVE_STATUS=$?
  [ "$NATIVE_STATUS" -eq 0 ] && [ -n "$NATIVE_RESPONSE" ] && printf '%s\\n' "$NATIVE_RESPONSE"
fi
"""

# Stands in for the runtime: reads the payload, notes that it ran, sleeps
# and exits as the test asks.
FAKE_CLI = (
    "import os, sys, time\n"
    "sys.stdin.read()\n"
    "with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'ran'), 'a') as ran:\n"
    "    ran.write(' '.join(sys.argv[1:]) + '\\n')\n"
    "time.sleep(float(os.environ.get('FAKE_NATIVE_SLEEP', '0')))\n"
    "sys.exit(int(os.environ.get('FAKE_NATIVE_STATUS', '0')))\n"
)


def documented_fragments() -> list:
    """Every bash block of hook-forge that defines the journal function."""
    text = HOOK_FORGE.read_text(encoding="utf-8")
    return [block + "\n" for block in re.findall(r"```bash\n(.*?)\n```", text, re.S)
            if "native_journal() {" in block]


def for_host(text: str, host: str) -> str:
    """Codex and Cursor change only the host, as hook-forge says."""
    return text.replace("--host claude", "--host " + host)


def journal_contract(source: str, edition: str) -> bool:
    contract = getattr(validator, "native_journal_hook_contract", None)
    return bool(contract and contract(source, edition))


class FragmentCase(unittest.TestCase):
    def fragment(self, host: str = "claude") -> str:
        found = documented_fragments()
        self.assertEqual(1, len(found), "hook-forge documents one native_journal fragment")
        return for_host(found[0], host)

    def hook(self, host: str = "claude") -> str:
        return PREAMBLE + self.fragment(host) + "exit 0\n"


class DocumentedFragmentTest(FragmentCase):
    def test_step_6_requires_the_line_and_names_its_guarantees(self) -> None:
        text = HOOK_FORGE.read_text(encoding="utf-8")
        bullet = next((line for line in text.splitlines()
                       if line.lstrip().startswith("- **Native result capture:**")), "")
        self.assertIn("native-results.ndjson", bullet)
        self.assertIn("`stop-hook`", bullet)
        flat = " ".join(text.split())
        for guarantee in ("124", "capsule-delivered", "linked journal file",
                          "linked state directory", "one physical line"):
            self.assertTrue(guarantee in flat, guarantee)

    def test_fragment_passes_both_native_contracts_for_every_host(self) -> None:
        for host in HOSTS:
            with self.subTest(host=host):
                source = self.hook(host)
                # Still exactly two native-stop code lines: the journal adds none.
                self.assertTrue(validator.native_capture_hook_contract(
                    source, host, "working-memory-write.sh"))
                self.assertTrue(journal_contract(source, host))
                self.assertFalse(journal_contract(source, next(h for h in HOSTS if h != host)))

    @unittest.skipUnless(BASH, "bash checks the fragment's syntax")
    def test_fragment_is_valid_bash(self) -> None:
        result = subprocess.run([BASH, "-n"], input=self.hook(), text=True,
                                capture_output=True, check=False)
        self.assertEqual(0, result.returncode, result.stderr)


@unittest.skipUnless(BASH and os.name == "posix", "runs the documented fragment in bash")
class JournalLineTest(FragmentCase):
    """The documented fragment, run in a temporary project."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="native-journal-")
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root = self.base / "target project"
        self.scripts = self.root / "memory-bank/scripts"
        self.scripts.mkdir(parents=True)
        (self.scripts / "context_cli.py").write_text(FAKE_CLI, encoding="utf-8")
        (self.scripts / "native_results.py").write_text("", encoding="utf-8")

    def run_hook(self, host: str = "claude", **environment: str) -> subprocess.CompletedProcess:
        script = self.root / f".{host}/hooks/working-memory-write.sh"
        script.parent.mkdir(parents=True, exist_ok=True)
        script.write_text(self.hook(host), encoding="utf-8")
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("CONTEXT_", "ACCELERATOR_", "FAKE_", "TEST_"))}
        env.update(TEST_PYTHON=sys.executable, **environment)
        result = subprocess.run([BASH, str(script)], input='{"session_id":"s"}', text=True,
                                capture_output=True, env=env, timeout=60, check=False)
        self.assertEqual(0, result.returncode, result.stderr)
        return result

    def lines(self, state: Path | None = None) -> list:
        journal = (state or self.root) / JOURNAL
        if not journal.exists():
            return []
        return [json.loads(line) for line in journal.read_text(encoding="utf-8").splitlines()]

    def assertLine(self, line: dict, host: str, **expected) -> None:
        self.assertRegex(line.pop("at"), STAMP)
        self.assertEqual({"host": host, "stage": "stop-hook", **expected}, line)

    def test_each_request_leaves_its_exit_status(self) -> None:
        for status in ("0", "3"):
            self.run_hook(FAKE_NATIVE_STATUS=status)
        first, second = self.lines()
        self.assertLine(first, "claude", event="exited", status=0)
        self.assertLine(second, "claude", event="exited", status=3)
        ran = (self.scripts / "ran").read_text(encoding="utf-8").splitlines()
        self.assertEqual(["native-stop --host claude --task-id feature-x"] * 2, ran)

    @unittest.skipUnless(shutil.which("timeout"), "the budget is timeout's")
    def test_a_request_killed_at_the_budget_is_124(self) -> None:
        self.run_hook(CONTEXT_HOOK_BUDGET="1", FAKE_NATIVE_SLEEP="10")
        [line] = self.lines()
        self.assertLine(line, "claude", event="exited", status=124)

    def test_a_harness_turn_is_journaled_as_skipped(self) -> None:
        self.run_hook(CONTEXT_CAPSULE_DELIVERED="1")
        [line] = self.lines()
        self.assertLine(line, "claude", event="skipped", code="capsule-delivered")
        self.assertFalse((self.scripts / "ran").exists())

    def test_a_runtime_without_native_results_leaves_no_line(self) -> None:
        (self.scripts / "native_results.py").unlink()
        self.run_hook()
        self.assertEqual([], self.lines())

    def test_codex_and_cursor_name_themselves(self) -> None:
        for host in ("codex", "cursor"):
            self.run_hook(host)
        codex, cursor = self.lines()
        self.assertLine(codex, "codex", event="exited", status=0)
        self.assertLine(cursor, "cursor", event="exited", status=0)

    def test_the_line_carries_a_status_and_nothing_else(self) -> None:
        self.run_hook(CONTEXT_TASK_ID="secret-task")
        text = (self.root / JOURNAL).read_text(encoding="utf-8")
        for absent in ("secret-task", "session_id", str(self.root)):
            self.assertNotIn(absent, text)

    def test_never_written_through_a_linked_journal(self) -> None:
        outside = self.base / "outside.ndjson"
        outside.write_text("kept\n", encoding="utf-8")
        journal = self.root / JOURNAL
        journal.parent.mkdir(parents=True)
        journal.symlink_to(outside)
        self.run_hook()
        self.assertEqual("kept\n", outside.read_text(encoding="utf-8"))

    def test_never_written_through_a_linked_attached_state(self) -> None:
        # Attached, the state directory is the launcher's: a link at the
        # state directory, its memory-bank or memory-bank/local would carry
        # this project's line into whatever it points at.
        outside = self.base / "shared state"
        (outside / "memory-bank/local").mkdir(parents=True)
        for linked in ("", "memory-bank", "memory-bank/local"):
            with self.subTest(linked=linked or "state directory"):
                state = self.base / ("state-" + (linked.replace("/", "-") or "linked"))
                link = state / linked if linked else state
                link.parent.mkdir(parents=True, exist_ok=True)
                link.symlink_to(outside / linked if linked else outside)
                self.run_hook(TEST_STATE_DIR=str(state))
                self.assertEqual([], list((outside / "memory-bank/local").iterdir()))
        attached = self.base / "attached state"
        self.run_hook(TEST_STATE_DIR=str(attached))
        [line] = self.lines(attached)
        self.assertLine(line, "claude", event="exited", status=0)
        self.assertEqual([], self.lines())

    def test_an_unwritable_journal_never_fails_the_hook(self) -> None:
        local = self.root / "memory-bank/local"
        local.write_text("not a directory\n", encoding="utf-8")
        self.run_hook()
        self.assertEqual("not a directory\n", local.read_text(encoding="utf-8"))


class VerifierRequiresTheJournalTest(FragmentCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="native-journal-gate-")
        self.addCleanup(temporary.cleanup)
        self.target = Path(temporary.name).resolve()
        runtime = self.target / "memory-bank/scripts/native_results.py"
        runtime.parent.mkdir(parents=True)
        shutil.copyfile(SEEDED_RUNTIME, runtime)
        self.files = {"memory-bank/scripts/native_results.py": {}}

    def write_hooks(self, body) -> None:
        for edition in HOSTS:
            hooks_rel, _ = validator.EDITION_HOOK_WIRING[edition]
            relative = f"{hooks_rel}/working-memory-write.sh"
            script = self.target / relative
            script.parent.mkdir(parents=True, exist_ok=True)
            script.write_text(PREAMBLE + for_host(body, edition) + "exit 0\n", encoding="utf-8")
            script.chmod(0o755)
            self.files[relative] = {}

    def journal_errors(self) -> list:
        errors: list = []
        validator.validate_hooks(self.target, list(HOSTS), self.files, errors)
        return [error for error in errors if "native result journal" in error]

    def test_a_stop_hook_without_the_line_fails(self) -> None:
        # Every project generated from now on is held to the line: the
        # runtime memory-seed copies journals.
        self.assertIn("native-results.ndjson", SEEDED_RUNTIME.read_text(encoding="utf-8"))
        self.write_hooks(ADAPTER_ONLY)
        for edition in HOSTS:
            source = PREAMBLE + for_host(ADAPTER_ONLY, edition)
            self.assertTrue(validator.native_capture_hook_contract(
                source, edition, "working-memory-write.sh"))
        errors = self.journal_errors()
        self.assertEqual(3, len(errors), errors)
        for edition, error in zip(HOSTS, errors):
            hooks_rel, _ = validator.EDITION_HOOK_WIRING[edition]
            self.assertEqual(f"[{edition}] {hooks_rel}/working-memory-write.sh {DIAGNOSTIC}", error)

    def test_the_documented_fragment_passes(self) -> None:
        self.write_hooks(self.fragment())
        self.assertEqual([], self.journal_errors())

    def test_a_project_seeded_before_the_journal_keeps_its_hooks(self) -> None:
        self.write_hooks(ADAPTER_ONLY)
        (self.target / "memory-bank/scripts/native_results.py").write_text(
            "PROFILE = 'native-result-challenge-v1'\n", encoding="utf-8")
        self.assertEqual([], self.journal_errors())
        # Once the runtime journals, the same hooks fail.
        shutil.copyfile(SEEDED_RUNTIME, self.target / "memory-bank/scripts/native_results.py")
        self.assertEqual(3, len(self.journal_errors()))

    def test_unowned_files_are_never_read(self) -> None:
        self.write_hooks(ADAPTER_ONLY)
        self.assertEqual(3, len(self.journal_errors()))
        owned = dict(self.files)
        del self.files["memory-bank/scripts/native_results.py"]
        self.assertEqual([], self.journal_errors())
        self.files = {key: value for key, value in owned.items()
                      if not key.endswith("working-memory-write.sh")}
        self.assertEqual([], self.journal_errors())

    def test_rewritten_journals_are_rejected(self) -> None:
        base = self.hook()
        self.assertTrue(journal_contract(base, "claude"))
        write = next(line for line in base.splitlines()
                     if line.lstrip().startswith("printf") and '>> "$directory/native-results.ndjson"' in line)
        exited = "  native_journal '\"event\":\"exited\",\"status\":'\"$NATIVE_STATUS\"\n"
        skipped = "  native_journal '\"event\":\"skipped\",\"code\":\"capsule-delivered\"'\n"
        elif_arm = 'elif [ -f "$ROOT_DIR/memory-bank/scripts/native_results.py" ]; then\n'
        function = base[base.index("native_journal() {"):base.index("}\n", base.index("native_journal() {")) + 2]
        mutations = {
            "other_host": base.replace("NATIVE_HOST=(--host claude)", "NATIVE_HOST=(--host codex)"),
            "journal_link_followed": base.replace('    [ -L "$entry" ] && return 0\n', ""),
            "attached_links_followed": base.replace(
                '  [ "$STATE_DIR" = "$ROOT_DIR" ] || entries+=("${STATE_DIR%/}" "$STATE_DIR/memory-bank" "$directory")\n', ""),
            "append_split_over_two_lines": base.replace(
                write, write.replace(' >> "$directory/', ' \\\n>> "$directory/')),
            "clock_dropped": base.replace('"$(date -u +%Y-%m-%dT%H:%M:%SZ)"', '"unknown"'),
            "other_file": base.replace('>> "$directory/native-results.ndjson"', '>> "$directory/stop.ndjson"'),
            "status_before_the_request": base.replace(exited, "").replace(
                "  NATIVE_STATUS=$?\n", exited + "  NATIVE_STATUS=$?\n"),
            "skip_on_every_stop": base.replace(elif_arm + skipped, "").replace(
                "exit 0\n", skipped.lstrip() + "exit 0\n"),
            "no_skip_arm": base.replace(elif_arm + skipped, ""),
            "no_state_dir": base.replace("STATE_DIR=$ROOT_DIR\n", ""),
            "defined_after_use": base.replace(function, "").replace("exit 0\n", function + "exit 0\n"),
            "payload_journaled": base.replace(exited, exited + '  native_journal "$HOOK_STDIN"\n'),
            "second_writer": base.replace(
                "exit 0\n", 'printf \'%s\\n\' "$HOOK_STDIN" >> "$STATE_DIR/memory-bank/local/native-results.ndjson"\nexit 0\n'),
        }
        for name, text in mutations.items():
            with self.subTest(mutation=name):
                self.assertNotEqual(base, text)
                self.assertFalse(journal_contract(text, "claude"))


if __name__ == "__main__":
    unittest.main()
