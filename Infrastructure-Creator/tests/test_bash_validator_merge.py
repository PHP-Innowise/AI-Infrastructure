#!/usr/bin/env python3
"""infra-update's section merge of a team-edited bash-validator.sh.

A generated project's bash-validator.sh carries the generator's generic
section byte for byte, and the team's own rules go below it, in
BV_FRAMEWORK_RULES, where the section's comment and hook-forge step 2 put
them. Such a file no longer matches its manifest hash, so infra-update step
6 asks for a decision, and step 10's gate holds the section to the current
generator's: keeping a section an earlier release wrote rolls the whole
update back, and taking the staged version drops the team's rules.
merge_bash_validator.py offers the section merge instead - the target
version with only the section replaced by the staged one - and only a merge
the gate passes; a target whose section is current keeps the ordinary
choices.

Run from this directory: python3 test_bash_validator_merge.py
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
sys.path.insert(0, str(SCRIPTS))

import infra_ownership as ownership  # noqa: E402
import merge_bash_validator as merger  # noqa: E402
import validate_generated as validator  # noqa: E402

HELPER = SCRIPTS / "merge_bash_validator.py"
INFRA_UPDATE = ROOT / ".agents/skills/infra-update/SKILL.md"
GENERATOR_VALIDATOR = ROOT / ".claude/hooks/bash-validator.sh"
REPETITION_GUARD = ROOT / ".agents/skills/hook-forge/assets/bash-validator-repetition-guard.sh"
VERSION = (ROOT / "VERSION").read_text(encoding="utf-8").strip()
BASH = shutil.which("bash") or "/bin/bash"
VALIDATOR = ".claude/hooks/bash-validator.sh"
OPENING, CLOSING = validator.BASH_VALIDATOR_MARKERS
CALL = b'bv_validate "$cmd"\n'

SETTINGS = (
    b"# Acme Shop rules: no PHP console; Kubernetes deploys (profile section 5).\n"
    b"BV_CONSOLE_ENTRIES=''\n"
    b"BV_CONSOLE_PHP_SCRIPTS=0\n"
    b"BV_CONSOLE_VALUES=''\n"
    b"BV_CONSOLE_SHORT_VALUES=''\n"
)
# What hook-forge step 2 wrote for the detected risk, and the line the team
# added below the closing marker afterwards.
GENERATED_RULE = b'  "argv|kubectl||has:delete|destructive command - kubectl delete"\n'
TEAM_RULE = b'  "argv|helm|uninstall||destructive command - helm uninstall"\n'
# A generic rule the earlier release's section did not have yet.
NEW_GENERIC_RULE = b'  "argv|dropdb|||destructive command - dropdb"\n'


def generator_parts() -> tuple:
    """The current generator validator's payload decoding, section and call.

    The call runs from `bv_validate "$cmd"` through the `fi` of its block:
    step 2 puts the repetition guard between that `fi` and the `exit 0` that
    ends the generator's own file.
    """
    data = GENERATOR_VALIDATOR.read_bytes()
    start, end, problem = validator.bash_validator_section(data)
    if problem:
        raise AssertionError(f"{GENERATOR_VALIDATOR} {problem}")
    preamble = data[:start].replace(b"(Infrastructure-Creator)", b"(Acme Shop)", 1)
    ending = data[data.index(b"\n" + CALL) + 1:]
    return preamble, data[start:end], ending[:ending.rindex(b"\nexit 0")].rstrip(b"\n") + b"\n"


def repetition_guard() -> bytes:
    """hook-forge's repetition guard, from its opening marker through its closing one."""
    data = REPETITION_GUARD.read_bytes()
    start, end, problem = validator.marked_block(
        data, validator.REPETITION_GUARD_MARKERS, "repetition guard"
    )
    if problem:
        raise AssertionError(f"{REPETITION_GUARD} {problem}")
    return data[start:end]


def current_section() -> bytes:
    return generator_parts()[1]


def older_section() -> bytes:
    """The section as an earlier release copied it: without the dropdb rule."""
    section = current_section()
    if NEW_GENERIC_RULE not in section:
        raise AssertionError("the generic section has no dropdb rule to leave out")
    return section.replace(NEW_GENERIC_RULE, b"", 1)


def validator_file(section: bytes, rules: bytes, *, preamble: bytes | None = None,
                   host: str = "claude") -> bytes:
    """A target validator built as hook-forge step 2 says.

    host is the edition the copy serves: BV_HOST names the repetition
    guard's counter directory and must match the edition it sits in.
    """
    generated_preamble, _, call = generator_parts()
    return (
        (generated_preamble if preamble is None else preamble)
        + section
        + b"\n\n"
        + SETTINGS
        + b"BV_FRAMEWORK_RULES=(\n"
        + rules
        + b")\n"
        + b"BV_HOST=" + host.encode() + b"\n\n"
        + call
        + b"\n"
        + repetition_guard()
        + b"\n\nexit 0\n"
    )


def line_of(data: bytes, needle: bytes) -> int:
    return data.count(b"\n", 0, data.index(needle)) + 1


class MergeFixture(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="bash-validator-merge-test-")
        self.addCleanup(temporary.cleanup)
        # A space in each path proves the quoting, not just the resolution.
        base = Path(temporary.name).resolve()
        self.target = base / "target project"
        self.staging = base / "update staging"
        self.target.mkdir()
        self.staging.mkdir()

    def place(self, root: Path, data: bytes, relative: str = VALIDATOR) -> Path:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o755)
        return path

    def helper(self, *extra: str, relative: str = VALIDATOR) -> subprocess.CompletedProcess:
        return subprocess.run(
            [
                sys.executable, str(HELPER),
                "--target", str(self.target),
                "--staging", str(self.staging),
                "--path", relative,
                *extra,
            ],
            capture_output=True,
            text=True,
            timeout=60,
        )

    def check(self, relative: str = VALIDATOR) -> dict:
        result = self.helper(relative=relative)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def gate(self, root: Path, relative: str = VALIDATOR) -> list:
        """Step 10's bash-validator check on the file at relative under root."""
        errors: list = []
        validator.validate_bash_validators(
            root, [merger.VALIDATORS[relative]], {relative: ""}, errors
        )
        return errors

    def run_hook(self, path: Path, command: str) -> subprocess.CompletedProcess:
        # The repetition guard keeps its counters under TMPDIR, keyed by the
        # repository the hook runs in: a private one per test.
        payload = {"tool_name": "Bash", "tool_input": {"command": command}}
        tmpdir = self.target.parent / "tmp"
        tmpdir.mkdir(exist_ok=True)
        return subprocess.run(
            [BASH, str(path)],
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            cwd=self.target,
            env={**os.environ, "TMPDIR": str(tmpdir)},
            timeout=30,
        )


class OlderSectionTest(MergeFixture):
    """A section from an earlier release above the team's own rule."""

    def setUp(self) -> None:
        super().setUp()
        self.team_version = validator_file(older_section(), GENERATED_RULE + TEAM_RULE)
        self.staged_version = validator_file(current_section(), GENERATED_RULE)
        self.target_file = self.place(self.target, self.team_version)
        self.staged_file = self.place(self.staging, self.staged_version)
        self.expected = validator_file(current_section(), GENERATED_RULE + TEAM_RULE)

    def test_keep_fails_the_gate_and_take_staged_drops_the_team_rule(self) -> None:
        errors = self.gate(self.target)
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("the generic section differs", errors[0])
        self.assertEqual(self.gate(self.staging), [])
        self.assertNotIn(TEAM_RULE, self.staged_version)

    def test_only_take_staged_or_the_section_merge_is_offered(self) -> None:
        report = self.check()
        opening = line_of(self.team_version, OPENING)
        closing = line_of(self.team_version, CLOSING)
        self.assertEqual(report, {
            "path": VALIDATOR,
            "section": "differs",
            "choices": ["take-staged", "section-merge"],
            "staged_sha256": ownership.sha256_file(self.staged_file),
            "merged_sha256": merger.sha256(self.expected),
            "replaced_lines": [opening, closing],
            "above_section": "same",
            "problems": [],
            "written": False,
        })
        # Checking writes nothing anywhere.
        self.assertEqual(self.staged_file.read_bytes(), self.staged_version)
        self.assertEqual(self.target_file.read_bytes(), self.team_version)

    def test_the_section_merge_is_the_staged_section_plus_the_team_rule(self) -> None:
        report = self.check()
        result = self.helper("--write", "--staged-sha256", report["staged_sha256"])
        self.assertEqual(result.returncode, 0, result.stderr)
        written = json.loads(result.stdout)
        self.assertTrue(written["written"])
        self.assertEqual(written["merged_sha256"], report["merged_sha256"])
        merged = self.staged_file.read_bytes()
        self.assertEqual(merged, self.expected)
        # Byte for byte: the target above the opening marker and below the
        # closing one, the staged bytes in between.
        start, end, _ = validator.bash_validator_section(self.team_version)
        merged_start, merged_end, _ = validator.bash_validator_section(merged)
        self.assertEqual(merged[:merged_start], self.team_version[:start])
        self.assertEqual(merged[merged_end:], self.team_version[end:])
        self.assertEqual(merged[merged_start:merged_end], current_section())
        self.assertIn(TEAM_RULE, merged[merged_end:])
        self.assertEqual(self.gate(self.staging), [])
        # Still the executable hook, and the target untouched until step 9.
        self.assertEqual(self.staged_file.stat().st_mode & 0o777, 0o755)
        self.assertEqual(self.target_file.read_bytes(), self.team_version)
        self.assertEqual(
            sorted(path.name for path in self.staged_file.parent.iterdir()),
            ["bash-validator.sh"],
        )

    def test_the_merged_hook_runs_the_new_rule_and_the_team_rule(self) -> None:
        report = self.check()
        self.helper("--write", "--staged-sha256", report["staged_sha256"])
        for command, kept, merged in (
            ("dropdb shop", 0, 2),
            ("helm uninstall shop", 2, 2),
            ("kubectl -n prod delete deploy web", 2, 2),
            ("ls -la", 0, 0),
        ):
            with self.subTest(command=command):
                self.assertEqual(
                    self.run_hook(self.target_file, command).returncode, kept
                )
                result = self.run_hook(self.staged_file, command)
                self.assertEqual(result.returncode, merged, result.stderr)

    def test_a_repeated_write_refuses_and_leaves_the_merge(self) -> None:
        report = self.check()
        first = self.helper("--write", "--staged-sha256", report["staged_sha256"])
        self.assertEqual(first.returncode, 0, first.stderr)
        second = self.helper("--write", "--staged-sha256", report["staged_sha256"])
        self.assertEqual(second.returncode, 2)
        self.assertIn("no longer holds", second.stderr)
        self.assertEqual(self.staged_file.read_bytes(), self.expected)

    def test_write_needs_the_checked_staged_sha256(self) -> None:
        result = self.helper("--write")
        self.assertEqual(result.returncode, 2)
        self.assertIn("--staged-sha256", result.stderr)
        stale = self.helper("--write", "--staged-sha256", "0" * 64)
        self.assertEqual(stale.returncode, 2)
        self.assertIn("no longer holds", stale.stderr)
        self.assertEqual(self.staged_file.read_bytes(), self.staged_version)

    def test_a_newer_payload_decoding_is_reported_and_kept_as_the_target_has_it(self) -> None:
        preamble, _, _ = generator_parts()
        older = preamble.replace(b"set -uo pipefail\n", b"set -uo pipefail\n# older decoding\n", 1)
        self.assertNotEqual(older, preamble)
        self.place(self.target, validator_file(older_section(), GENERATED_RULE + TEAM_RULE, preamble=older))
        report = self.check()
        self.assertEqual(report["choices"], ["take-staged", "section-merge"])
        self.assertEqual(report["above_section"], "differs")
        self.helper("--write", "--staged-sha256", report["staged_sha256"])
        self.assertTrue(self.staged_file.read_bytes().startswith(older))


class CurrentSectionTest(MergeFixture):
    def test_a_current_section_stays_an_ordinary_decision(self) -> None:
        team_version = validator_file(current_section(), GENERATED_RULE + TEAM_RULE)
        target_file = self.place(self.target, team_version)
        staged_version = validator_file(current_section(), GENERATED_RULE)
        staged_file = self.place(self.staging, staged_version)
        report = self.check()
        self.assertEqual(report["section"], "current")
        self.assertEqual(report["choices"], ["keep", "take-staged", "manual-merge"])
        self.assertIsNone(report["merged_sha256"])
        # Keeping it passes step 10.
        self.assertEqual(self.gate(self.target), [])
        result = self.helper("--write", "--staged-sha256", report["staged_sha256"])
        self.assertEqual(result.returncode, 2)
        self.assertIn("an ordinary decision", result.stderr)
        self.assertEqual(staged_file.read_bytes(), staged_version)
        self.assertEqual(target_file.read_bytes(), team_version)


class NoSectionMergeTest(MergeFixture):
    """Where no merge would pass step 10, only take staged is offered."""

    def setUp(self) -> None:
        super().setUp()
        self.staged_version = validator_file(current_section(), GENERATED_RULE)
        self.staged_file = self.place(self.staging, self.staged_version)

    def assert_take_staged_only(self, section: str, fragment: str) -> None:
        report = self.check()
        self.assertEqual(report["section"], section)
        self.assertEqual(report["choices"], ["take-staged"])
        self.assertIsNone(report["merged_sha256"])
        self.assertTrue(any(fragment in problem for problem in report["problems"]), report)
        result = self.helper("--write", "--staged-sha256", report["staged_sha256"])
        self.assertEqual(result.returncode, 2)
        self.assertIn("no section merge to write", result.stderr)
        self.assertEqual(self.staged_file.read_bytes(), self.staged_version)

    def test_a_deleted_validator(self) -> None:
        self.assert_take_staged_only("missing", "has no .claude/hooks/bash-validator.sh")

    def test_a_validator_without_the_section(self) -> None:
        self.place(self.target, b"#!/bin/bash\ncase \"$1\" in *'push --force'*) exit 2 ;; esac\nexit 0\n")
        self.assert_take_staged_only("unmarked", f"has no `{OPENING.decode()}` marker")

    def test_a_section_closed_twice(self) -> None:
        self.place(self.target, validator_file(older_section() + b"\n" + CLOSING, TEAM_RULE))
        self.assert_take_staged_only("unmarked", f"has the `{CLOSING.decode()}` marker 2 times")

    def test_team_code_below_the_section_the_gate_refuses(self) -> None:
        # Under set -u an unassigned list ends the hook with exit 1 on every
        # command, which the host runs: the merge would fail step 10 too.
        rule = b'  "argv|helm|uninstall|arg:$TEAM_RELEASES|destructive command - helm uninstall"\n'
        self.place(self.target, validator_file(older_section(), GENERATED_RULE + rule))
        self.assert_take_staged_only("differs", "expands $TEAM_RELEASES")


class RefusalTest(MergeFixture):
    def test_a_staged_validator_that_fails_the_gate_is_repaired_first(self) -> None:
        self.place(self.target, validator_file(older_section(), TEAM_RULE))
        self.place(self.staging, validator_file(older_section(), GENERATED_RULE))
        result = self.helper()
        self.assertEqual(result.returncode, 2)
        self.assertIn("repair staging through hook-forge first", result.stderr)
        self.assertIn("the generic section differs", result.stderr)

    def test_only_an_editions_validator_is_merged(self) -> None:
        for relative in (".claude/hooks/local-context.sh", "bash-validator.sh",
                         "../.claude/hooks/bash-validator.sh", "./.claude/hooks/bash-validator.sh"):
            with self.subTest(path=relative):
                result = self.helper(relative=relative)
                self.assertEqual(result.returncode, 2)
                self.assertIn("ERROR:", result.stderr)

    def test_every_edition_is_covered(self) -> None:
        self.assertEqual(
            merger.VALIDATORS,
            {
                ".claude/hooks/bash-validator.sh": "claude",
                ".cursor/hooks/bash-validator.sh": "cursor",
                ".codex/hooks/bash-validator.sh": "codex",
            },
        )
        relative = ".codex/hooks/bash-validator.sh"
        self.place(self.target, validator_file(older_section(), TEAM_RULE, host="codex"), relative)
        self.place(self.staging, validator_file(current_section(), b"", host="codex"), relative)
        self.assertEqual(self.check(relative)["choices"], ["take-staged", "section-merge"])

    def test_a_staged_validator_behind_a_link_is_refused(self) -> None:
        real = self.place(self.staging, validator_file(current_section(), b""), "elsewhere.sh")
        self.place(self.target, validator_file(older_section(), TEAM_RULE))
        link = self.staging / VALIDATOR
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(real)
        result = self.helper()
        self.assertEqual(result.returncode, 2)
        self.assertIn("symlink", result.stderr)


class UpdateLifecycleTest(MergeFixture):
    """Generate, edit below the section, update: merge, publish, rerun."""

    def manifest(self, root: Path, *, decisions: dict | None = None,
                 file_hashes: dict | None = None, task: str = "TASK-001") -> dict:
        return ownership.build_manifest(
            root,
            [VALIDATOR],
            generator="Infrastructure-Creator",
            generator_version=VERSION,
            task=task,
            profile=f"tasks/{task}/infra-scan-project-profile.md",
            editions=["claude"],
            mode="full",
            decisions=decisions,
            file_hashes=file_hashes,
            generated_at="2026-10-29T08:00:00Z",
        )

    def test_the_merge_publishes_passes_the_gate_and_stands_on_the_next_run(self) -> None:
        # An earlier release generated the validator, then the team added its rule.
        hook = self.place(self.target, validator_file(older_section(), GENERATED_RULE))
        ownership.write_manifest(self.target, self.manifest(self.target))
        hook.write_bytes(validator_file(older_section(), GENERATED_RULE + TEAM_RULE))
        staged_version = validator_file(current_section(), GENERATED_RULE)
        staged = self.place(self.staging, staged_version)

        manifest = ownership.load_manifest(self.target)
        rows = ownership.classify_update(self.target, self.staging, manifest)
        self.assertEqual(rows, [{
            "path": VALIDATOR,
            "classification": "requires-decision",
            "reason": "tracked-file-modified",
        }])

        report = self.check()
        self.assertEqual(report["choices"], ["take-staged", "section-merge"])
        written = self.helper("--write", "--staged-sha256", report["staged_sha256"])
        self.assertEqual(written.returncode, 0, written.stderr)

        # Step 8: the refreshed manifest hashes the merged staged bytes and
        # records the decision against the staged version it was built from.
        decisions = {VALIDATOR: {
            "decision": "merged",
            "rejected_sha256": report["staged_sha256"],
            "task": "TASK-002",
        }}
        hashes = ownership.hash_write_plan_sources(self.staging, [VALIDATOR], self.target, {})
        refreshed = self.manifest(
            self.staging, decisions=decisions, file_hashes=hashes, task="TASK-002"
        )
        self.assertEqual(refreshed["files"][VALIDATOR], report["merged_sha256"])
        # Step 9 publishes the staged bytes, mode included.
        shutil.copy2(staged, hook)
        ownership.write_manifest(self.target, refreshed)

        # Step 10 passes on the updated target.
        errors: list = []
        validator.validate_manifest(self.target, errors)
        validator.validate_bash_validators(self.target, ["claude"], refreshed["files"], errors)
        self.assertEqual(errors, [])
        self.assertIn(TEAM_RULE, hook.read_bytes())

        # The next run regenerates the same staged version: the decision stands.
        staged.write_bytes(staged_version)
        rerun = ownership.classify_update(self.target, self.staging, refreshed)
        self.assertEqual(rerun[0]["classification"], "standing-decision-honored")
        self.assertEqual(rerun[0]["reason"], "merged")


class InfraUpdateContractTest(unittest.TestCase):
    def test_step_6_routes_a_validator_through_the_helper(self) -> None:
        text = INFRA_UPDATE.read_text(encoding="utf-8")
        step_6 = text[text.index("6. **Present decisions.**"):text.index("7. **Compose shared")]
        for needle in (
            "`bash-validator.sh`",
            f"`{OPENING.decode()}`",
            f"`{CLOSING.decode()}`",
            "merge_bash_validator.py",
            "`choices`",
            "**section merge**",
            "--write --staged-sha256",
            "`merged`",
            "`rejected_sha256`",
            "`above_section: differs`",
            "never defaults to keep",
        ):
            with self.subTest(needle=needle):
                self.assertIn(needle, step_6)
        guardrails = text[text.index("## Guardrails"):]
        self.assertIn("`bash-validator.sh`", guardrails)
        self.assertIn("section merge", guardrails)


if __name__ == "__main__":
    unittest.main()
