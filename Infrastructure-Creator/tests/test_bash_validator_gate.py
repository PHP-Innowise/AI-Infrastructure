#!/usr/bin/env python3
"""Every generated bash-validator.sh runs the generator's own shell parser.

hook-forge step 2 builds a target's validator around the generic section of
the generator's own .claude/hooks/bash-validator.sh - the parser that finds a
command inside chains, substitutions, heredocs, wrappers and launchers, and
the generic rules - copied byte for byte, and writes only the target's
console settings and rules below it. A validator written from scratch, or a
section edited or trimmed, still passes `bash -n` and still blocks the plain
command, so bootstrap-verifier compares the section byte for byte. Below the
section the hook must assign the two settings the section reads and every
variable it expands before it runs bv_validate, then exit 2 on a hit: the
hook runs under `set -u`, so an unset variable makes it exit 1 and the host
runs the command - on every command, for a rule copied without the helper
list it names. The repetition guard that follows the call has its own tests
in test_repetition_guard_gate.py; the validators built here carry it, as
step 2 says.

Run from this directory: python3 test_bash_validator_gate.py
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
SPEC = importlib.util.spec_from_file_location(
    "validate_generated_bash_validator", SCRIPTS / "validate_generated.py"
)
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)

HOOK_FORGE = ROOT / ".agents/skills/hook-forge/SKILL.md"
GENERATOR_VALIDATOR = ROOT / ".claude/hooks/bash-validator.sh"
REPETITION_GUARD = ROOT / ".agents/skills/hook-forge/assets/bash-validator-repetition-guard.sh"
BASH = shutil.which("bash") or "/bin/bash"
EDITIONS = ["claude", "cursor", "codex"]
OPENING, CLOSING = validator.BASH_VALIDATOR_MARKERS
CALL = b'bv_validate "$cmd"\n'

# What step 2 writes between the section and the call for a target with no
# PHP console whose profile found Kubernetes in section 5.
TARGET_RULES = (
    b"# Acme Shop rules: no PHP console; Kubernetes deploys (profile section 5).\n"
    b"BV_CONSOLE_ENTRIES=''\n"
    b"BV_CONSOLE_PHP_SCRIPTS=0\n"
    b"BV_CONSOLE_VALUES=''\n"
    b"BV_CONSOLE_SHORT_VALUES=''\n"
    b"BV_FRAMEWORK_RULES=(\n"
    b'  "argv|kubectl||has:delete|destructive command - kubectl delete"\n'
    b")\n"
)


def generator_parts() -> tuple:
    """The generator validator's payload decoding, generic section and call.

    The call runs from `bv_validate "$cmd"` through the `fi` of its block:
    step 2 puts the repetition guard between that `fi` and the `exit 0` that
    ends the generator's own file.
    """
    data = GENERATOR_VALIDATOR.read_bytes()
    start, end, problem = validator.bash_validator_section(data)
    if problem:
        raise AssertionError(f"{GENERATOR_VALIDATOR} {problem}")
    ending = data[data.index(b"\n" + CALL) + 1:]
    return data[:start], data[start:end], ending[:ending.rindex(b"\nexit 0")].rstrip(b"\n") + b"\n"


def generator_rules() -> bytes:
    """The generator's own settings and rules, between its section and call."""
    data = GENERATOR_VALIDATOR.read_bytes()
    _, end, _ = validator.bash_validator_section(data)
    return data[end:data.index(b"\n" + CALL) + 1].strip(b"\n") + b"\n"


def repetition_guard() -> bytes:
    """hook-forge's repetition guard, from its opening marker through its closing one."""
    data = REPETITION_GUARD.read_bytes()
    start, end, problem = validator.marked_block(
        data, validator.REPETITION_GUARD_MARKERS, "repetition guard"
    )
    if problem:
        raise AssertionError(f"{REPETITION_GUARD} {problem}")
    return data[start:end]


def built(rules: bytes = TARGET_RULES, *, host: str | None = "claude",
          section: bytes | None = None, call: bytes | None = None,
          guard: bytes | None = None) -> bytes:
    """A target's validator built the way hook-forge step 2 says.

    host is the edition the copy serves (BV_HOST); None leaves it unset.
    """
    preamble, generic, generator_call = generator_parts()
    preamble = preamble.replace(b"(Infrastructure-Creator)", b"(Acme Shop)", 1)
    return (
        preamble
        + (generic if section is None else section)
        + b"\n\n"
        + rules
        + (b"" if host is None else b"BV_HOST=" + host.encode() + b"\n")
        + b"\n"
        + (generator_call if call is None else call)
        + b"\n"
        + (repetition_guard() if guard is None else guard)
        + b"\n\nexit 0\n"
    )


def generator_line(prefix: bytes) -> bytes:
    """The generator validator's line that starts with prefix, newline included."""
    data = GENERATOR_VALIDATOR.read_bytes()
    start = data.index(b"\n" + prefix) + 1
    return data[start:data.index(b"\n", start) + 1]


def laravel_rules(*, helper: bool = True) -> bytes:
    """A Laravel target's settings with two of the generator's own lines.

    The config:show line expands BV_LARAVEL_SECRET_CONFIG, so step 2 copies
    that helper list with it.
    """
    return (
        b"BV_CONSOLE_ENTRIES='artisan'\n"
        b"BV_CONSOLE_PHP_SCRIPTS=0\n"
        b"BV_CONSOLE_VALUES='--env'\n"
        b"BV_CONSOLE_SHORT_VALUES=''\n"
        + (generator_line(b"BV_LARAVEL_SECRET_CONFIG=") if helper else b"")
        + b"BV_FRAMEWORK_RULES=(\n"
        + generator_line(b'  "console|migrate:fresh|')
        + generator_line(b'  "console|config:show|')
        + b")\n"
    )


def line_of(data: bytes, needle: bytes) -> int:
    return data.count(b"\n", 0, data.index(needle)) + 1


class GateFixture(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="bash-validator-gate-")
        self.addCleanup(temporary.cleanup)
        # A space in the path proves the quoting, not just the resolution.
        self.target = Path(temporary.name).resolve() / "target project"
        self.target.mkdir()
        # The repetition guard counts what a hook lets through in
        # ${TMPDIR:-/tmp}, keyed by the repository the hook runs in: the hooks
        # here run in the target with a TMPDIR of their own, so no count
        # reaches this checkout's counters or outlives the test.
        self.tmpdir = Path(temporary.name).resolve() / "tmp"
        self.tmpdir.mkdir()
        self.files: dict = {}

    def relative(self, edition: str) -> str:
        return f"{validator.EDITION_HOOK_WIRING[edition][0]}/bash-validator.sh"

    def write(self, edition: str, data: bytes, *, owned: bool = True) -> Path:
        path = self.target / self.relative(edition)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o755)
        if owned:
            self.files[self.relative(edition)] = {"sha256": "x"}
        return path

    def errors(self, editions: list = EDITIONS) -> list:
        errors: list = []
        validator.validate_bash_validators(self.target, list(editions), self.files, errors)
        return errors

    def assert_one_error(self, edition: str, *fragments: str) -> str:
        errors = self.errors()
        self.assertEqual(len(errors), 1, errors)
        self.assertTrue(errors[0].startswith(f"[{edition}] {self.relative(edition)}"), errors)
        for fragment in fragments:
            self.assertIn(fragment, errors[0])
        return errors[0]

    def run_hook(self, path: Path, command: str, **fields) -> subprocess.CompletedProcess:
        """Run a hook on a shell command; fields join the payload (session_id)."""
        payload = {"tool_name": "Bash", "tool_input": {"command": command}, **fields}
        return self.run_payload(path, payload)

    def run_payload(self, path: Path, payload: dict) -> subprocess.CompletedProcess:
        return subprocess.run(
            [BASH, str(path)],
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            cwd=self.target,
            env={**os.environ, "TMPDIR": str(self.tmpdir)},
            timeout=30,
        )


class ExactSectionTest(GateFixture):
    def test_the_generators_own_validator_is_well_formed(self) -> None:
        data = GENERATOR_VALIDATOR.read_bytes()
        start, end, problem = validator.bash_validator_section(data)
        self.assertEqual(problem, "")
        self.assertEqual(
            validator.bash_validator_call(data[:end].decode("utf-8"), data[end:].decode("utf-8")),
            "",
        )
        self.assertEqual(validator.GENERATOR_BASH_VALIDATOR, GENERATOR_VALIDATOR)

    def test_a_validator_built_as_step_2_says_passes_in_every_edition(self) -> None:
        for edition in EDITIONS:
            self.write(edition, built(host=edition))
        self.assertEqual(self.errors(), [])

    def test_the_generators_own_rules_pass_in_a_built_validator(self) -> None:
        # Every framework line with the helper list it expands, as a target
        # running all four frameworks would take them.
        for edition in EDITIONS:
            self.write(edition, built(generator_rules(), host=edition))
        self.assertEqual(self.errors(), [])

    def test_the_built_validator_runs_the_shipped_parser_and_the_target_rule(self) -> None:
        hook = self.write("claude", built())
        blocked = {
            # The rule step 2 authored, also after a global option.
            "kubectl -n prod delete deploy web": "kubectl delete",
            # Generic rules and parsing the target gets from the section.
            "git push --force origin main": "git force push",
            "echo ${X:-$(gh repo delete acme/shop --yes)}": "gh repo delete",
            "cat <<EOF\n$(gh repo delete acme/shop --yes)\nEOF": "gh repo delete",
            "docker compose exec app sh -c 'mysql -e \"DROP TABLE users\"'": "DROP TABLE",
        }
        for command, category in blocked.items():
            with self.subTest(command=command):
                result = self.run_hook(hook, command)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn(category, result.stderr)
        # No console was detected, so no console rule was authored.
        for command in ("kubectl get pods", "php artisan migrate:fresh", "ls -la"):
            with self.subTest(command=command):
                result = self.run_hook(hook, command)
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_framework_lines_copied_with_their_helper_list_pass_and_block(self) -> None:
        hook = self.write("claude", built(laravel_rules()))
        self.assertEqual(self.errors(["claude"]), [])
        for command, expected in (
            ("php artisan mi:fr", 2),
            ("php artisan config:show database", 2),
            ("php artisan route:list", 0),
        ):
            with self.subTest(command=command):
                result = self.run_hook(hook, command)
                self.assertEqual(result.returncode, expected, result.stderr)


class EditedSectionTest(GateFixture):
    def test_a_trimmed_rule_is_rejected_at_its_line(self) -> None:
        rule = b'  "argv|dropdb|||destructive command - dropdb"\n'
        _, generic, _ = generator_parts()
        complete = built()
        self.write("cursor", built(section=generic.replace(rule, b"", 1), host="cursor"))
        self.assert_one_error(
            "cursor",
            f"line {line_of(complete, rule)}:",
            "the generic section differs",
            str(GENERATOR_VALIDATOR),
            "hook-forge step 2",
        )

    def test_a_one_byte_edit_is_rejected_at_its_line(self) -> None:
        _, generic, _ = generator_parts()
        self.assertIn(b"BV_MAX_NEST=100\n", generic)
        complete = built()
        self.write("claude", built(section=generic.replace(b"BV_MAX_NEST=100\n", b"BV_MAX_NEST=10\n", 1)))
        self.assert_one_error("claude", f"line {line_of(complete, b'BV_MAX_NEST=100')}:")

    def test_carriage_returns_and_trailing_blanks_are_edits(self) -> None:
        _, generic, _ = generator_parts()
        for name, section in (
            ("crlf", generic.replace(b"\n", b"\r\n")),
            ("trailing blank", generic.replace(b"BV_MAX_DEPTH=8\n", b"BV_MAX_DEPTH=8 \n", 1)),
        ):
            with self.subTest(name=name):
                self.write("codex", built(section=section, host="codex"))
                self.assert_one_error("codex", "the generic section differs")


class MissingMarkersTest(GateFixture):
    def test_a_validator_written_from_scratch_is_rejected(self) -> None:
        scratch = (
            b"#!/bin/bash\n"
            b"input=\n"
            b"IFS= read -r -d '' input || :\n"
            b"case \"$input\" in *'git push --force'*) exit 2 ;; esac\n"
            b"exit 0\n"
        )
        self.write("claude", scratch)
        self.assert_one_error(
            "claude",
            f"has no `{OPENING.decode()}` marker",
            f"build it around the generic section of {GENERATOR_VALIDATOR}",
        )

    def test_a_missing_opening_or_closing_marker_is_rejected(self) -> None:
        for marker in (OPENING, CLOSING):
            with self.subTest(marker=marker.decode()):
                self.write("cursor", built(host="cursor").replace(marker, b"# section", 1))
                self.assert_one_error("cursor", f"has no `{marker.decode()}` marker")

    def test_a_second_section_is_rejected(self) -> None:
        _, generic, _ = generator_parts()
        self.write("codex", built(section=generic + b"\n" + generic, host="codex"))
        self.assert_one_error("codex", f"has the `{OPENING.decode()}` marker 2 times")

    def test_a_section_closed_before_it_opens_is_rejected(self) -> None:
        swapped = (
            built()
            .replace(OPENING, b"\0", 1)
            .replace(CLOSING, OPENING, 1)
            .replace(b"\0", CLOSING, 1)
        )
        self.write("claude", swapped)
        self.assert_one_error("claude", "closes the generic section before it opens it")


class SectionRunsTest(GateFixture):
    def test_an_unset_setting_is_rejected_and_lets_commands_through(self) -> None:
        for name in validator.BASH_VALIDATOR_SETTINGS:
            with self.subTest(setting=name):
                rules = b"\n".join(
                    line for line in TARGET_RULES.split(b"\n")
                    if not line.startswith(name.encode() + b"=")
                )
                if name == "BV_CONSOLE_PHP_SCRIPTS":
                    # Read only once an entry point is set.
                    rules = rules.replace(b"BV_CONSOLE_ENTRIES=''", b"BV_CONSOLE_ENTRIES='artisan'")
                hook = self.write("claude", built(rules))
                self.assert_one_error("claude", f"runs bv_validate with {name} not assigned")
                result = self.run_hook(hook, "php bin/console d:d:d")
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(f"{name}: unbound variable", result.stderr)

    def test_a_setting_counts_only_where_it_always_runs(self) -> None:
        variants = {
            "inside a condition": TARGET_RULES.replace(
                b"BV_CONSOLE_ENTRIES=''\n",
                b'if [ -f artisan ]; then\n  BV_CONSOLE_ENTRIES=artisan\nfi\n',
            ),
            "in a comment": TARGET_RULES.replace(
                b"BV_CONSOLE_ENTRIES=''\n", b"# BV_CONSOLE_ENTRIES=''\n"
            ),
        }
        for name, rules in variants.items():
            with self.subTest(variant=name):
                self.write("cursor", built(rules, host="cursor"))
                self.assert_one_error("cursor", "runs bv_validate with BV_CONSOLE_ENTRIES not assigned")

    def test_settings_assigned_after_the_call_are_rejected(self) -> None:
        _, _, ending = generator_parts()
        self.write("codex", built(b"", call=ending.replace(CALL, CALL + TARGET_RULES, 1), host="codex"))
        self.assert_one_error(
            "codex",
            "runs bv_validate with BV_CONSOLE_ENTRIES and BV_CONSOLE_PHP_SCRIPTS not assigned",
        )

    def test_a_section_nothing_runs_is_rejected(self) -> None:
        # The repetition guard still follows; an exit before it is its own
        # error (test_repetition_guard_gate.py).
        for name, call in (
            ("no call", b""),
            ("call in a comment", b'# bv_validate "$cmd"\n'),
        ):
            with self.subTest(variant=name):
                self.write("claude", built(call=call))
                self.assert_one_error("claude", "never runs bv_validate below the generic section")

    def test_a_rule_copied_without_its_helper_list_is_rejected_and_lets_all_through(self) -> None:
        hook = self.write("cursor", built(laravel_rules(helper=False), host="cursor"))
        self.assert_one_error(
            "cursor",
            "expands $BV_LARAVEL_SECRET_CONFIG below the generic section before any line assigns it",
        )
        # Even the plainest generic rule: the array assignment dies first.
        result = self.run_hook(hook, "git push --force origin main")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("BV_LARAVEL_SECRET_CONFIG: unbound variable", result.stderr)

    def test_a_helper_list_assigned_after_its_rule_is_rejected(self) -> None:
        helper = generator_line(b"BV_LARAVEL_SECRET_CONFIG=")
        self.write("codex", built(laravel_rules().replace(helper, b"", 1) + helper, host="codex"))
        self.assert_one_error("codex", "expands $BV_LARAVEL_SECRET_CONFIG")

    def test_a_call_on_a_variable_nothing_assigns_is_rejected(self) -> None:
        _, _, ending = generator_parts()
        self.write("claude", built(call=ending.replace(CALL, b'bv_validate "$COMMAND"\n', 1)))
        self.assert_one_error("claude", "expands $COMMAND below the generic section")

    def test_a_default_or_single_quoted_dollar_needs_no_assignment(self) -> None:
        rules = TARGET_RULES + b"BV_NOTE='$HOME stays text'\nBV_EXTRA=\"${BV_OPTIONAL-}\"\n"
        self.write("claude", built(rules))
        self.assertEqual(self.errors(), [])

    def test_a_hit_that_does_not_exit_2_is_rejected(self) -> None:
        # The repetition guard below the call exits 2 on its own refusal; that
        # is no exit for a command the rules refused.
        _, _, ending = generator_parts()
        self.assertIn(b"  exit 2\n", ending)
        self.assertIn(b"  exit 2\n", repetition_guard())
        self.write("claude", built(call=ending.replace(b"  exit 2\n", b"  exit 1\n", 1)))
        self.assert_one_error("claude", "never exits 2 after bv_validate")


class ScopeTest(GateFixture):
    def test_only_selected_editions_are_checked_each_by_name(self) -> None:
        self.write("claude", built())
        self.write("codex", b"#!/bin/sh\nexit 0\n")
        self.assertEqual(self.errors(["claude"]), [])
        errors = self.errors(["claude", "codex"])
        self.assertEqual(len(errors), 1, errors)
        self.assertTrue(errors[0].startswith("[codex] .codex/hooks/bash-validator.sh"), errors)

    def test_an_unowned_validator_is_never_opened(self) -> None:
        # A directory where the hook would be: reading it would be an error.
        (self.target / self.relative("claude")).mkdir(parents=True)
        self.write("cursor", b"#!/bin/sh\nexit 0\n", owned=False)
        self.assertEqual(self.errors(), [])

    def test_the_generators_own_validator_must_be_readable_and_whole(self) -> None:
        self.write("claude", built())
        unmarked = self.target / "generator-validator.sh"
        unmarked.write_bytes(b"#!/bin/bash\nexit 0\n")
        for reference, fragment in (
            (self.target / "missing.sh", "cannot be read"),
            (unmarked, f"has no `{OPENING.decode()}` marker"),
        ):
            with self.subTest(reference=reference.name), \
                    mock.patch.object(validator, "GENERATOR_BASH_VALIDATOR", reference):
                errors = self.errors()
                self.assertEqual(len(errors), 1, errors)
                self.assertTrue(errors[0].startswith(
                    f"bash-validator.sh not checked: the generator's own {reference} {fragment}"
                ), errors)

    def test_main_runs_the_gate(self) -> None:
        self.write("claude", b"#!/bin/sh\nexit 0\n")
        manifest = {"files": self.files, "editions": ["claude"]}
        stderr = io.StringIO()
        with contextlib.ExitStack() as stack:
            # Every other gate passes: only this one is left to fail.
            for name in dir(validator):
                if name.startswith("validate_") and name != "validate_bash_validators" \
                        and callable(getattr(validator, name)):
                    stack.enter_context(mock.patch.object(
                        validator, name,
                        return_value=manifest if name == "validate_manifest" else None,
                    ))
            stack.enter_context(mock.patch.object(sys, "argv", [
                "validate_generated.py", "--target", str(self.target), "--editions", "claude",
            ]))
            stack.enter_context(contextlib.redirect_stderr(stderr))
            code = validator.main()
        self.assertEqual(code, 1)
        self.assertIn(
            f"ERROR: [claude] .claude/hooks/bash-validator.sh has no `{OPENING.decode()}` marker",
            stderr.getvalue(),
        )


class HookForgeContractTest(unittest.TestCase):
    def test_hook_forge_names_the_source_the_markers_the_call_and_the_settings(self) -> None:
        text = HOOK_FORGE.read_text(encoding="utf-8")
        for needle in (
            "`.claude/hooks/bash-validator.sh`",
            f"`{OPENING.decode()}`",
            f"`{CLOSING.decode()}`",
            '`bv_validate "$cmd"`',
            *(f"`{name}`" for name in validator.BASH_VALIDATOR_SETTINGS),
        ):
            with self.subTest(needle=needle):
                self.assertIn(needle, text)

    def test_hook_forge_names_every_helper_list_the_generators_rules_expand(self) -> None:
        data = GENERATOR_VALIDATOR.read_bytes()
        _, end, _ = validator.bash_validator_section(data)
        rules = data[end:data.index(b"\n" + CALL)].decode("utf-8")
        helpers = set(re.findall(r"\$(BV_[A-Z_]+)", rules))
        self.assertTrue(helpers)
        text = HOOK_FORGE.read_text(encoding="utf-8")
        for name in sorted(helpers):
            with self.subTest(helper=name):
                self.assertIn(f"`{name}`", text)


if __name__ == "__main__":
    unittest.main()
