#!/usr/bin/env python3
"""scripts/check.py stays what CI runs, and its runner reports honestly.

Two halves:

* Drift. Every workflow in .github/workflows/ is read with a minimal YAML
  reader (standard library only; it understands the subset these workflows use
  and raises on anything else rather than guessing), and each `run:` step is
  split into its shell commands. Every command that checks the repository must
  have an entry in check.py's group for that job; every entry there must still
  be in CI, unless the files it needs are absent from this checkout (and CI has
  no such step: a guard that hides a step CI runs fails here) or it is declared
  local-only with a reason. Runner provisioning is excluded by the explicit
  CI_ONLY_SETUP list below, never by omission. Each job's setup-python
  version, leg by leg, must match the interpreter check.py pins for that leg.
* Runner. Fake groups made of tiny `python3 -c` commands exercise PASS, FAIL,
  SKIP (missing tool, absent file, other platform, unavailable interpreter,
  unresolvable changelog base), --strict, --fail-fast, exit codes, stop
  signals, the failure tail and the summary table. Nothing here runs a real
  gate.

Run: python3 -m unittest tests.test_check
"""

from __future__ import annotations

import contextlib
import io
import itertools
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Set, Tuple
from unittest import mock

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import check  # noqa: E402

PASS, FAIL, SKIP = check.PASS, check.FAIL, check.SKIP
WORKFLOWS = ROOT / ".github" / "workflows"


# --------------------------------------------------------------------------
# A minimal YAML reader for GitHub workflow files.
# --------------------------------------------------------------------------


class YamlError(ValueError):
    pass


KEY = re.compile(r"^([A-Za-z0-9_][A-Za-z0-9_.\-]*)\s*:(?:\s+(.*))?$")


def _is_item(content: str) -> bool:
    return content == "-" or content.startswith("- ")


def _strip_comment(text: str) -> str:
    quote = ""
    for index, char in enumerate(text):
        if quote:
            if char == quote:
                quote = ""
        elif char in "'\"":
            quote = char
        elif char == "#" and (index == 0 or text[index - 1] in " \t"):
            return text[:index]
    return text


def _split_flow(body: str) -> List[str]:
    items, current, quote = [], "", ""
    for char in body:
        if quote:
            current += char
            if char == quote:
                quote = ""
        elif char in "'\"":
            quote = char
            current += char
        elif char == ",":
            items.append(current)
            current = ""
        else:
            current += char
    items.append(current)
    return [item for item in items if item.strip()]


def _scalar(text: str, line: int):
    text = _strip_comment(text).strip()
    if not text:
        return None
    if text[0] == '"':
        if len(text) < 2 or not text.endswith('"'):
            raise YamlError(f"line {line}: unterminated double-quoted scalar")
        body = text[1:-1]
        return re.sub(r"\\(.)", lambda m: {"n": "\n", "t": "\t"}.get(m.group(1), m.group(1)), body)
    if text[0] == "'":
        if len(text) < 2 or not text.endswith("'"):
            raise YamlError(f"line {line}: unterminated single-quoted scalar")
        return text[1:-1].replace("''", "'")
    if text[0] == "[":
        if not text.endswith("]"):
            raise YamlError(f"line {line}: multi-line flow sequences are not supported")
        return [_scalar(item, line) for item in _split_flow(text[1:-1])]
    if text[0] in "{&*!":
        raise YamlError(f"line {line}: flow mappings, anchors, aliases and tags are not supported")
    return text


def _fold(lines: List[str]) -> str:
    """`>` folding: a single break between plain lines becomes a space."""
    out = ""
    pending = 0
    previous: Optional[str] = None
    for line in lines:
        if line == "":
            pending += 1
            continue
        if previous is None:
            out = "\n" * pending + line
        elif pending:
            out += "\n" * pending + line
        elif line[:1] in " \t" or previous[:1] in " \t":
            out += "\n" + line
        else:
            out += " " + line
        previous, pending = line, 0
    return out


class MiniYaml:
    """Block mappings and sequences, plain/quoted scalars, flow sequences, and
    `|`/`>` block scalars with `-`/`+` chomping. Everything else raises."""

    def __init__(self, text: str) -> None:
        self.lines = text.splitlines()
        self.index = 0

    @classmethod
    def load(cls, text: str):
        reader = cls(text)
        node = reader._block(0)
        if reader._peek() is not None:
            raise YamlError(f"line {reader.index + 1}: content after the document")
        return node

    def _peek(self) -> Optional[Tuple[int, str]]:
        while self.index < len(self.lines):
            line = self.lines[self.index]
            stripped = line.strip()
            if stripped and not stripped.startswith("#") and stripped != "---":
                indent = len(line) - len(line.lstrip(" "))
                if line[indent:indent + 1] == "\t":
                    raise YamlError(f"line {self.index + 1}: tab indentation")
                return indent, stripped
            self.index += 1
        return None

    def _block(self, minimum: int):
        peeked = self._peek()
        if peeked is None or peeked[0] < minimum:
            return None
        column, content = peeked
        return self._sequence(column) if _is_item(content) else self._mapping(column)

    def _mapping(self, column: int) -> Dict[str, object]:
        result: Dict[str, object] = {}
        while True:
            peeked = self._peek()
            if peeked is None or peeked[0] < column:
                return result
            if peeked[0] > column or _is_item(peeked[1]):
                raise YamlError(f"line {self.index + 1}: unexpected indentation or item")
            match = KEY.match(_strip_comment(peeked[1]).rstrip())
            if not match:
                raise YamlError(f"line {self.index + 1}: expected `key: value`")
            self.index += 1
            key = match.group(1)
            if key in result:
                raise YamlError(f"line {self.index}: duplicate key {key!r}")
            result[key] = self._value(match.group(2), column)

    def _value(self, rest: Optional[str], column: int):
        rest = (rest or "").strip()
        if rest and rest[0] in "|>":
            return self._block_scalar(rest, column)
        if _strip_comment(rest).strip():
            return _scalar(rest, self.index)
        peeked = self._peek()
        if peeked is None:
            return None
        if peeked[0] > column:
            return self._block(peeked[0])
        if peeked[0] == column and _is_item(peeked[1]):
            return self._sequence(column)
        return None

    def _sequence(self, column: int) -> List[object]:
        items: List[object] = []
        while True:
            peeked = self._peek()
            if peeked is None or peeked[0] < column:
                return items
            if peeked[0] > column or not _is_item(peeked[1]):
                raise YamlError(f"line {self.index + 1}: unexpected content in a sequence")
            content = peeked[1]
            self.index += 1
            rest = content[1:].lstrip(" ")
            if not rest:
                items.append(self._value("", column))
                continue
            item_column = column + len(content) - len(rest)
            match = None if rest[0] in "'\"[" else KEY.match(_strip_comment(rest).rstrip())
            if match is None:
                items.append(_scalar(rest, self.index))
                continue
            item = {match.group(1): self._value(match.group(2), item_column)}
            peeked = self._peek()
            if peeked is not None and peeked[0] == item_column:
                for key, value in self._mapping(item_column).items():
                    if key in item:
                        raise YamlError(f"line {self.index}: duplicate key {key!r}")
                    item[key] = value
            items.append(item)

    def _block_scalar(self, header: str, column: int) -> str:
        indicators = _strip_comment(header[1:]).strip()
        if any(char.isdigit() for char in indicators) or set(indicators) - set("+-"):
            raise YamlError(f"line {self.index}: unsupported block scalar header {header!r}")
        lines: List[str] = []
        indent: Optional[int] = None
        while self.index < len(self.lines):
            raw = self.lines[self.index]
            if not raw.strip():
                lines.append("")
                self.index += 1
                continue
            current = len(raw) - len(raw.lstrip(" "))
            if indent is None:
                if current <= column:
                    break
                indent = current
            if current < indent:
                break
            lines.append(raw[indent:])
            self.index += 1
        trailing = 0
        while lines and lines[-1] == "":
            lines.pop()
            trailing += 1
        text = "\n".join(lines) if header[0] == "|" else _fold(lines)
        if "-" in indicators or not text:
            return text
        if "+" in indicators:
            return text + "\n" * (trailing + 1)
        return text + "\n"


# --------------------------------------------------------------------------
# From workflow steps to gate commands.
# --------------------------------------------------------------------------

# CI-only setup: commands that prepare the hosted runner rather than check the
# repository. They are dropped explicitly here - anything else a `run:` step
# does must have an entry in scripts/check.py. (`uses:` steps such as
# actions/checkout, actions/setup-python and actions/setup-node carry no
# `run:` and are never gates; check.py's interpreter shim stands in for
# setup-python.)
CI_ONLY_SETUP = (
    # Installs a system package (bubblewrap) on the hosted image.
    re.compile(r"^sudo apt-get\b"),
    # Lifts Ubuntu 24.04's AppArmor restriction on unprivileged user
    # namespaces: a host kernel setting, not a property of the repository.
    re.compile(r"^if \[ -e /proc/sys/kernel/apparmor_restrict_unprivileged_userns \]"),
    # Builds the optional graph runtime's venv and pip-installs third-party
    # dependencies; check.py skips the step that needs the venv when it is
    # absent (and fails it under --strict).
    re.compile(r"^python3 -m venv harness/\.venv$"),
    re.compile(r"^harness/\.venv/bin/python -m pip install\b"),
    # The same for the QA tooling's pinned workbook and schema libraries.
    re.compile(r"^python3 -m venv scripts/qa/\.venv$"),
    re.compile(r"^scripts/qa/\.venv/bin/python -m pip install\b"),
    # Prints the tool's version into the log; it checks nothing.
    re.compile(r"^shellcheck --version$"),
)

_QUOTED = re.compile(r"'[^']*'|\"(?:\\.|[^\"\\])*\"")
_OPENERS = {"if", "case", "do"}
_CLOSERS = {"fi", "esac", "done"}


def split_commands(script: str) -> List[str]:
    """A step's shell text as top-level commands: an if/for/while block or a
    backslash/pipe continuation stays one command. Comment and blank lines are
    dropped and whitespace runs collapsed, so both sides compare as text."""
    commands: List[str] = []
    current: List[str] = []
    depth = 0
    for raw in script.splitlines():
        line = re.sub(r"\s+", " ", raw.strip())
        if not line or line.startswith("#"):
            continue
        current.append(line)
        words = _QUOTED.sub(" ", line).replace(";", " ; ").split()
        depth += sum(word in _OPENERS for word in words) - sum(word in _CLOSERS for word in words)
        continued = line.endswith(("\\", "|", "&&", "||"))
        if depth <= 0 and not continued:
            commands.append("\n".join(current))
            current, depth = [], 0
    if current:
        commands.append("\n".join(current))
    return commands


_UNITTEST = re.compile(r"^(?:\S*/)?python3?\s+-m\s+unittest\s+(.+)$")


def unittest_modules(text: str) -> Optional[List[str]]:
    """Modules of a `python -m unittest a b c` line (not `discover`)."""
    if "\n" in text:
        return None
    match = _UNITTEST.match(text)
    if not match:
        return None
    arguments = shlex.split(match.group(1))
    if not arguments or arguments[0] == "discover":
        return None
    return [argument for argument in arguments if not argument.startswith("-")]


@dataclass(frozen=True)
class Gate:
    workflow: str
    job: str
    step: str
    cwd: str
    text: str


_MATRIX_EXPRESSION = re.compile(r"\$\{\{\s*matrix\.([A-Za-z0-9_-]+)\s*\}\}")
_MATRIX_CONDITION = re.compile(r"^\s*matrix\.([A-Za-z0-9_-]+)\s*(==|!=)\s*'([^']*)'\s*$")


def _legs(job: dict) -> List[Dict[str, str]]:
    matrix = (job.get("strategy") or {}).get("matrix") or {}
    if "include" in matrix or "exclude" in matrix:
        raise YamlError("matrix include/exclude is not supported by this reader")
    keys = sorted(matrix)
    for key in keys:
        if not isinstance(matrix[key], list):
            raise YamlError(f"matrix.{key} is not a list")
    return [dict(zip(keys, combination)) for combination in itertools.product(*(matrix[key] for key in keys))]


def _substitute(text: str, leg: Dict[str, str]) -> str:
    return _MATRIX_EXPRESSION.sub(lambda match: str(leg[match.group(1)]), text)


def _applies(condition: Optional[str], leg: Dict[str, str]) -> bool:
    """Only matrix filters select legs; any other condition (an event name,
    say) decides whether CI runs the job at all, and the gate still counts."""
    match = _MATRIX_CONDITION.match(condition or "")
    if not match:
        return True
    equal = str(leg.get(match.group(1))) == match.group(3)
    return equal if match.group(2) == "==" else not equal


def _cwd(path: str) -> str:
    path = path.strip().rstrip("/")
    return path if path not in ("", "./") else "."


def load_workflows(directory: Path = WORKFLOWS) -> Dict[str, dict]:
    paths = sorted(list(directory.glob("*.yml")) + list(directory.glob("*.yaml")))
    return {path.name: MiniYaml.load(path.read_text(encoding="utf-8")) for path in paths}


def ci_gates(workflows: Dict[str, dict]) -> Dict[Tuple[str, str], List[Gate]]:
    gates: Dict[Tuple[str, str], List[Gate]] = {}
    for workflow, document in workflows.items():
        for job_id, job in (document.get("jobs") or {}).items():
            if job.get("defaults"):
                raise YamlError(f"{workflow} > {job_id}: job defaults are not supported by this reader")
            found = gates.setdefault((workflow, job_id), [])
            for leg in _legs(job):
                for step in job.get("steps") or []:
                    if "run" not in step or not _applies(step.get("if"), leg):
                        continue
                    cwd = _cwd(_substitute(step.get("working-directory") or ".", leg))
                    for text in split_commands(_substitute(step["run"], leg)):
                        if any(pattern.search(text) for pattern in CI_ONLY_SETUP):
                            continue
                        gate = Gate(workflow, job_id, step.get("name") or "", cwd, text)
                        if gate not in found:
                            found.append(gate)
    return gates


# setup-python pins scripts/check.py deliberately does not reproduce, and why.
UNMIRRORED_PYTHON = {
    ("ci.yml", "harness-fleet", "3.11"): (
        "the graph tests run on harness/.venv's own interpreter; the Fleet step on the one running check.py"
    ),
    ("ci.yml", "windows-harness", "3.13"): "Windows-only: it skips wherever check.py runs",
}


def ci_python_legs(workflows: Dict[str, dict]) -> Dict[Tuple[str, str], List[Optional[str]]]:
    """Each job's setup-python version, one per leg (matrix values resolved);
    None for a leg with no setup-python step."""
    legs: Dict[Tuple[str, str], List[Optional[str]]] = {}
    for workflow, document in workflows.items():
        for job_id, job in (document.get("jobs") or {}).items():
            found = legs.setdefault((workflow, job_id), [])
            for leg in _legs(job):
                versions = [
                    _substitute(str((step.get("with") or {}).get("python-version") or ""), leg)
                    for step in job.get("steps") or []
                    if str(step.get("uses") or "").startswith("actions/setup-python@") and _applies(step.get("if"), leg)
                ]
                found.append(versions[-1] if versions else None)
    return legs


def entry_text(command: check.Command) -> str:
    return "\n".join(split_commands(command.ci))


def _modules_by_cwd(pairs) -> Dict[str, Set[str]]:
    modules: Dict[str, Set[str]] = {}
    for cwd, text in pairs:
        found = unittest_modules(text)
        if found:
            modules.setdefault(cwd, set()).update(found)
    return modules


# --------------------------------------------------------------------------
# Drift tests.
# --------------------------------------------------------------------------


class DriftTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.gates = ci_gates(load_workflows())
        cls.groups = check.build_groups(ROOT)
        cls.by_job = {(group.workflow, group.job): group for group in cls.groups}

    def test_every_workflow_job_has_a_group(self) -> None:
        missing = [f"{workflow} > {job}" for workflow, job in self.gates if (workflow, job) not in self.by_job]
        self.assertEqual(missing, [], "CI jobs with no group in scripts/check.py")

    def test_every_ci_gate_has_an_entry(self) -> None:
        problems = []
        for (workflow, job), gates in self.gates.items():
            group = self.by_job.get((workflow, job))
            if group is None:
                continue  # reported by test_every_workflow_job_has_a_group
            entries = {(command.cwd, entry_text(command)) for command in group.commands}
            modules = _modules_by_cwd(entries)
            for gate in gates:
                wanted = unittest_modules(gate.text)
                if wanted is not None:
                    absent = [module for module in wanted if module not in modules.get(gate.cwd, set())]
                    if absent:
                        problems.append(f"{workflow} > {job} > {gate.step}: modules {absent} (cwd {gate.cwd})")
                elif (gate.cwd, gate.text) not in entries:
                    problems.append(f"{workflow} > {job} > {gate.step}: {gate.text!r} (cwd {gate.cwd})")
        self.assertEqual(
            problems,
            [],
            "CI runs these and scripts/check.py does not - add them to the group, "
            "or, for pure runner setup, to CI_ONLY_SETUP in tests/test_check.py",
        )

    def in_ci(self, group: check.Group, command: check.Command) -> bool:
        keys = {(gate.cwd, gate.text) for gate in self.gates.get((group.workflow, group.job), [])}
        text = entry_text(command)
        wanted = unittest_modules(text)
        if wanted:
            modules = _modules_by_cwd(keys)
            return all(module in modules.get(command.cwd, set()) for module in wanted)
        return (command.cwd, text) in keys

    def test_every_entry_is_in_ci(self) -> None:
        problems = []
        for group in self.groups:
            for command in group.commands:
                text = entry_text(command)
                in_ci = self.in_ci(group, command)
                where = f"{group.name} > {command.label}"
                if command.local_only:
                    if in_ci:
                        problems.append(f"{where}: CI runs it now - drop local_only")
                    continue
                absent = [path for path in command.paths if not (ROOT / path).exists()]
                if absent:
                    # The guard is for a branch whose CI has no such step.
                    # Where CI runs it, the missing file fails CI while the
                    # entry skips here as "not present".
                    if in_ci:
                        problems.append(f"{where}: CI runs it, but {absent} is absent - it skips here and fails there")
                    continue
                if not in_ci:
                    problems.append(f"{where}: {text!r} is not in {group.workflow} > {group.job}")
        self.assertEqual(problems, [], "scripts/check.py runs these and CI does not, or hides what CI runs")

    def test_preflight_knows_which_entries_ci_runs(self) -> None:
        # check.py decides "absent here, but CI runs it" (fail under
        # --strict) by a text match on the job; this holds that match to the
        # parsed workflow.
        runner = check.Runner(root=ROOT, out=io.StringIO())
        for group in self.groups:
            for command in group.commands:
                with self.subTest(group=group.name, command=command.label):
                    self.assertEqual(runner.ci_runs(group, command), self.in_ci(group, command))

    def test_python_pins_match_ci(self) -> None:
        legs = ci_python_legs(load_workflows())
        problems = []
        for (workflow, job, version), reason in UNMIRRORED_PYTHON.items():
            if version not in (legs.get((workflow, job)) or []):
                problems.append(f"UNMIRRORED_PYTHON {workflow} > {job} {version}: CI no longer pins it ({reason})")
        for (workflow, job), versions in legs.items():
            group = self.by_job.get((workflow, job))
            if group is None:
                continue  # reported by test_every_workflow_job_has_a_group
            # check.py's pin per leg: "" is the interpreter running check.py,
            # the stand-in for `3.x` and for a job with no setup-python.
            wanted = sorted(
                "" if version in (None, "3.x") or (workflow, job, version) in UNMIRRORED_PYTHON else version
                for version in versions
            )
            pinned = sorted(
                ",".join(sorted({command.python for _, command in lane if command.python}))
                for lane in check.lanes_of(group)
            )
            if pinned != wanted:
                shown = lambda pins: [pin or "3.x" for pin in pins]  # noqa: E731
                problems.append(
                    f"{workflow} > {job}: CI runs Python {shown(wanted)} (one per leg), check.py {shown(pinned)}"
                )
        self.assertEqual(problems, [], "setup-python versions and check.py `python=` pins differ")

    def test_modules_ci_names_exist(self) -> None:
        # A guarded entry skips when its file is absent; CI would fail
        # instead. This keeps the guard from hiding a deleted test module.
        missing = []
        for gates in self.gates.values():
            for gate in gates:
                for module in unittest_modules(gate.text) or ():
                    if not (ROOT / gate.cwd / check.module_path(module)).exists():
                        missing.append(f"{gate.workflow} > {gate.job}: {module}")
        self.assertEqual(missing, [])

    def test_entries_are_single_commands(self) -> None:
        for group in self.groups:
            for command in group.commands:
                with self.subTest(group=group.name, command=command.label):
                    self.assertEqual(len(split_commands(command.ci)), 1)

    def test_names_are_unique(self) -> None:
        names = [group.name for group in self.groups]
        jobs = [(group.workflow, group.job) for group in self.groups]
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual(len(jobs), len(set(jobs)))

    def test_ci_tests_matrix_is_the_suite_list(self) -> None:
        tests = load_workflows()["ci.yml"]["jobs"]["tests"]
        self.assertEqual(tuple(tests["strategy"]["matrix"]["suite"]), check.EDITION_TEST_SUITES)

    def test_every_suite_runs_every_test_file(self) -> None:
        group = next(group for group in self.groups if group.name == "tests")
        for suite in check.EDITION_TEST_SUITES:
            with self.subTest(suite=suite):
                files = sorted(path.name for path in (ROOT / suite).glob("test_*.py"))
                self.assertTrue(files)
                ran = [command.argv[1] for command in group.commands if command.cwd == suite]
                self.assertEqual(ran, files)


class MiniYamlTest(unittest.TestCase):
    def test_subset(self) -> None:
        document = MiniYaml.load(
            textwrap_dedent(
                """
                name: CI  # trailing comment
                on:
                  push:
                    branches: [main, "release/x"]
                  pull_request:
                jobs:
                  build:
                    runs-on: [self-hosted, Windows]
                    strategy:
                      matrix:
                        suite:
                          - "a b/tests"
                          - 'it''s'
                    steps:
                      - uses: actions/checkout@v4
                      - name: Literal
                        if: matrix.suite == 'a b/tests'
                        run: |
                          # a comment inside the script
                          for f in x; do
                            echo "$f"
                          done
                      - name: Folded
                        run: >-
                          python3
                          tool.py
                          --flag
                      - run: echo one # not a comment? it is
                """
            )
        )
        self.assertEqual(document["on"], {"push": {"branches": ["main", "release/x"]}, "pull_request": None})
        build = document["jobs"]["build"]
        self.assertEqual(build["runs-on"], ["self-hosted", "Windows"])
        self.assertEqual(build["strategy"]["matrix"]["suite"], ["a b/tests", "it's"])
        steps = build["steps"]
        self.assertEqual(steps[0], {"uses": "actions/checkout@v4"})
        self.assertEqual(steps[1]["if"], "matrix.suite == 'a b/tests'")
        self.assertEqual(
            steps[1]["run"], '# a comment inside the script\nfor f in x; do\n  echo "$f"\ndone\n'
        )
        self.assertEqual(steps[2]["run"], "python3 tool.py --flag")
        self.assertEqual(steps[3]["run"], "echo one")

    def test_refuses_what_it_does_not_understand(self) -> None:
        for text in ("a: {b: 1}\n", "a: &anchor x\n", "a:\n\tb: 1\n", "a: 1\na: 2\n", "a: |2\n  x\n"):
            with self.subTest(text=text):
                with self.assertRaises(YamlError):
                    MiniYaml.load(text)

    def test_reads_the_real_workflow(self) -> None:
        ci = load_workflows()["ci.yml"]
        reliability = ci["jobs"]["infrastructure-creator-reliability"]["steps"]
        folded = next(step["run"] for step in reliability if step.get("name") == "Validate canonical reference contracts")
        self.assertNotIn("\n", folded)
        self.assertTrue(folded.startswith("python3 Infrastructure-Creator/"))

    def test_split_commands(self) -> None:
        script = textwrap_dedent(
            r"""
            sudo apt-get update -qq && sudo apt-get install -y bubblewrap
            # a comment line
            if [ -e /proc/x ]; then
              sudo sysctl -w x=0
            fi
            git ls-files -z -- '*.json' | while IFS= read -r -d '' f; do
              python3 -m json.tool "$f" > /dev/null || { echo "bad: $f" >&2; exit 1; }
            done
            if git ls-files -z -- 'a' \
                                  'b' \
              | xargs -0 -r grep -nE 'do|done|if'; then
              exit 1
            fi
            python3   -m unittest tests.a tests.b
            """
        )
        commands = split_commands(script)
        self.assertEqual(len(commands), 5)
        self.assertTrue(commands[1].startswith("if [ -e /proc/x ]") and commands[1].endswith("fi"))
        self.assertTrue(commands[2].endswith("done"))
        self.assertEqual(commands[3].count("\n"), 4)
        self.assertEqual(commands[4], "python3 -m unittest tests.a tests.b")
        self.assertEqual(unittest_modules(commands[4]), ["tests.a", "tests.b"])
        self.assertIsNone(unittest_modules('python3 -m unittest discover -s x -p "test_*.py"'))
        self.assertEqual(unittest_modules("python -m unittest tests.x -v"), ["tests.x"])

    def test_matrix_legs_and_conditions(self) -> None:
        workflow = MiniYaml.load(
            textwrap_dedent(
                """
                jobs:
                  parity:
                    strategy:
                      matrix:
                        edition: ["A", "B"]
                    steps:
                      - working-directory: ${{ matrix.edition }}
                        run: tool parity
                      - if: matrix.edition == 'A'
                        run: tool once
                      - run: |
                          sudo apt-get install -y x
                          shellcheck --version
                """
            )
        )
        gates = ci_gates({"w.yml": workflow})[("w.yml", "parity")]
        self.assertEqual([(gate.cwd, gate.text) for gate in gates], [("A", "tool parity"), (".", "tool once"), ("B", "tool parity")])


def textwrap_dedent(text: str) -> str:
    return textwrap.dedent(text).lstrip("\n")


# --------------------------------------------------------------------------
# Runner tests: fake commands only.
# --------------------------------------------------------------------------


def fake(name: str, code: str, **options) -> check.Command:
    return check.Command(name=name, argv=("python3", "-c", code), ci=f"python3 -c {shlex.quote(code)}", **options)


OK = "pass"
BOOM = "import sys\nfor i in range(100): print('boom', i)\nsys.exit(3)"

# A command that starts a child of its own, records both pids, and sleeps.
SLEEPER = textwrap_dedent(
    """
    import os, subprocess, sys, time
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)"])
    with open("pids.tmp", "w") as handle:
        handle.write("%d %d" % (os.getpid(), child.pid))
    os.replace("pids.tmp", "pids")
    time.sleep(120)
    """
)
# check.main() over one sleeper group, in a process of its own to signal.
DRIVER = textwrap_dedent(
    """
    import signal, sys
    from pathlib import Path
    # Whatever the test runner inherited (a background job ignores SIGINT,
    # nohup SIGHUP), start from the defaults an interactive run has.
    signal.signal(signal.SIGTERM, signal.SIG_DFL)
    signal.signal(signal.SIGHUP, signal.SIG_DFL)
    signal.signal(signal.SIGINT, signal.default_int_handler)
    sys.path.insert(0, sys.argv[1])
    import check
    sleeper = check.Command(name="sleeper", argv=("python3", "sleeper.py"), ci="python3 sleeper.py")
    group = check.Group("g", "g", "g", commands=(sleeper,))
    sys.exit(check.main([], groups=[group], root=Path(sys.argv[2])))
    """
)


def alive(pid: int) -> bool:
    """Whether the process exists and is not a zombie awaiting its reaper."""
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    if Path("/proc/self/stat").exists():
        try:
            return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] != "Z"
        except (OSError, IndexError):
            return False
    state = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return bool(state) and not state.startswith("Z")


def kill_all(pids: List[int]) -> None:
    for pid in pids:
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass


@unittest.skipIf(os.name == "nt", "check.py runs on Linux and macOS")
class RunnerTest(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="test-check-"))
        self.addCleanup(shutil.rmtree, self.root, True)

    def run_groups(self, groups, **options) -> Tuple[check.Report, str]:
        out = io.StringIO()
        report = check.Runner(root=self.root, out=out, **options).run(groups)
        self.addCleanup(lambda: report.log_dir and shutil.rmtree(report.log_dir.parent, True))
        return report, out.getvalue()

    def statuses(self, report: check.Report) -> Dict[str, str]:
        return {result.command.name: result.status for result in report.results}

    def test_pass_fail_and_exit_code(self) -> None:
        group = check.Group("g", "g", "g", commands=(fake("ok", OK), fake("bad", BOOM), fake("after", OK)))
        report, out = self.run_groups([group], jobs=1)
        self.assertEqual(self.statuses(report), {"ok": PASS, "bad": FAIL, "after": PASS})
        self.assertEqual(report.exit_code, 1)
        failed = report.failed[0]
        self.assertEqual((failed.reason, failed.returncode), ("exit 3", 3))
        shown, total = check.tail(failed.log)
        self.assertEqual(len(shown), check.TAIL_LINES)
        self.assertGreater(total, check.TAIL_LINES)
        self.assertEqual(shown[-1], "boom 99")
        self.assertRegex(out, r"(?m)^FAIL\s+[\d.]+s  g: bad  \(exit 3\)$")

    def test_all_pass_exits_zero_and_keeps_no_logs(self) -> None:
        report, _ = self.run_groups([check.Group("g", "g", "g", commands=(fake("ok", OK),))])
        self.assertEqual(report.exit_code, 0)
        self.assertIsNone(report.log_dir)

    def test_missing_tool_skips_unless_strict(self) -> None:
        command = fake("needs", OK, tools=("check-py-no-such-tool",))
        group = check.Group("g", "g", "g", commands=(command,))
        report, _ = self.run_groups([group])
        self.assertEqual(report.results[0].status, SKIP)
        self.assertIn("check-py-no-such-tool not installed", report.results[0].reason)
        self.assertEqual(report.exit_code, 0)
        report, _ = self.run_groups([group], strict=True)
        self.assertEqual(report.results[0].status, FAIL)
        self.assertEqual(report.exit_code, 1)
        self.assertIsNone(report.log_dir)  # nothing ran, so there is no log to keep

    def test_tool_given_as_a_path_is_checked_relative_to_the_root(self) -> None:
        group = check.Group("g", "g", "g", commands=(fake("venv", OK, tools=("venv/bin/python",)),))
        self.assertEqual(self.run_groups([group])[0].results[0].status, SKIP)
        (self.root / "venv" / "bin").mkdir(parents=True)
        os.symlink(sys.executable, str(self.root / "venv" / "bin" / "python"))
        self.assertEqual(self.run_groups([group])[0].results[0].status, PASS)

    def test_absent_file_skips_even_when_strict(self) -> None:
        group = check.Group("g", "g", "g", commands=(fake("routes", OK, paths=("scripts/check_routes.py",)),))
        for strict in (False, True):
            report, _ = self.run_groups([group], strict=strict)
            self.assertEqual(report.results[0].status, SKIP)
            self.assertEqual(report.results[0].reason, "not present: scripts/check_routes.py")
            self.assertEqual(report.exit_code, 0)

    def write_workflow(self, text: str) -> None:
        workflows = self.root / ".github" / "workflows"
        workflows.mkdir(parents=True, exist_ok=True)
        (workflows / "ci.yml").write_text(textwrap_dedent(text), encoding="utf-8")

    def test_absent_file_of_a_step_ci_runs_fails_under_strict(self) -> None:
        # On a branch whose CI runs the guarded step, a missing file is a
        # breakage CI reports, not a feature this branch lacks.
        self.write_workflow(
            """
            jobs:
              other:
                steps:
                  - run: python3 -m unittest tests.test_gone
              g:
                steps:
                  # python3 -m unittest tests.test_commented
                  - run: python3 -m unittest tests.test_gone tests.test_kept
                  - run: python3 tool.py --check
            """
        )
        guarded = check.unittests("gone", ["tests.test_gone", "tests.test_kept"], paths=("tests/test_gone.py",))
        tool = check.step("tool", "python3 tool.py --check", paths=("tool.py",))
        report, _ = self.run_groups([check.Group("g", "g", "g", commands=(guarded, tool))])
        self.assertEqual([result.status for result in report.results], [SKIP, SKIP])
        self.assertIn("ci.yml > g runs this step and fails there", report.results[0].reason)
        self.assertIn("ci.yml > g runs this step and fails there", report.results[1].reason)
        report, _ = self.run_groups([check.Group("g", "g", "g", commands=(guarded, tool))], strict=True)
        self.assertEqual([result.status for result in report.results], [FAIL, FAIL])
        self.assertEqual(report.exit_code, 1)
        # A job that does not carry the step - here, only another job or a
        # comment names the module - keeps the plain skip, --strict or not.
        for name, modules in (("other", ["tests.test_gone", "tests.test_kept"]), ("g", ["tests.test_commented"])):
            with self.subTest(job=name, modules=modules):
                command = check.unittests("x", modules, paths=("tests/test_gone.py",))
                report, _ = self.run_groups([check.Group(name, name, name, commands=(command,))], strict=True)
                self.assertEqual((report.results[0].status, report.results[0].reason), (SKIP, "not present: tests/test_gone.py"))

    def test_workflow_job_text(self) -> None:
        self.write_workflow(
            """
            on:
              push:
            jobs:
              a:
                steps:
                  - name: A  # trailing comment
                    run: |
                      # a comment line
                      python3   one.py
                      --flag
              b:
                steps:
                  - run: python3 two.py
            """
        )
        path = self.root / ".github" / "workflows" / "ci.yml"
        self.assertEqual(check.workflow_job_text(path, "a"), "steps: - name: A # trailing comment run: | python3 one.py --flag")
        self.assertEqual(check.workflow_job_text(path, "b"), "steps: - run: python3 two.py")
        self.assertEqual(check.workflow_job_text(path, "push"), "")
        self.assertEqual(check.workflow_job_text(self.root / "missing.yml", "a"), "")
        runner = check.Runner(root=self.root, out=io.StringIO())
        group = check.Group("a", "a", "a")
        self.assertTrue(runner.ci_runs(group, check.step("one", "python3 one.py --flag")))
        self.assertFalse(runner.ci_runs(group, check.step("one", "python3 one.p")))  # whole words only
        self.assertFalse(runner.ci_runs(group, check.step("two", "python3 two.py")))  # job b's, not a's

    def git(self, *arguments: str) -> None:
        subprocess.run(
            ["git", "-c", "user.name=check", "-c", "user.email=check@example.invalid", "-c", "commit.gpgsign=false", *arguments],
            cwd=str(self.root), check=True, capture_output=True,
        )

    def test_unresolvable_changelog_base_never_passes(self) -> None:
        # check_core_changelog.sh exits 0 when it cannot resolve a merge base;
        # check.py must not report that as PASS.
        self.git("-c", "init.defaultBranch=trunk", "init", "-q")
        self.git("commit", "-q", "--allow-empty", "-m", "base")

        def changelog(base_ref: str, explicit: bool) -> check.Group:
            command = fake("changelog", OK, base_ref=base_ref, base_explicit=explicit)
            return check.Group("changelog", "changelog", "changelog", commands=(command,))

        cases = (
            # (base, explicit, strict) -> status
            (("origin/main", False, False), SKIP),  # neither origin/main nor a local main
            (("origin/main", False, True), FAIL),
            (("feature/x", True, False), FAIL),  # a typo or an unfetched branch
            (("feature/x", True, True), FAIL),
            (("trunk", True, False), PASS),
        )
        for (base, explicit, strict), status in cases:
            with self.subTest(base=base, explicit=explicit, strict=strict):
                report, _ = self.run_groups([changelog(base, explicit)], strict=strict)
                self.assertEqual(report.results[0].status, status, report.results[0].reason)
                if status != PASS:
                    self.assertIn(f"cannot resolve a merge base with {base}", report.results[0].reason)
        self.git("branch", "main")  # the script falls back to a local main for origin/main
        self.assertEqual(self.run_groups([changelog("origin/main", False)], strict=True)[0].results[0].status, PASS)

    def test_changelog_base_comes_from_base_or_the_scripts_default(self) -> None:
        def command(groups: List[check.Group]) -> check.Command:
            return next(group for group in groups if group.name == "changelog").commands[0]

        with mock.patch.dict(os.environ, {"GITHUB_BASE_REF": ""}):
            default = command(check.build_groups(self.root))
            self.assertEqual((default.base_ref, default.base_explicit, default.argv[-1]), ("origin/main", False, "scripts/check_core_changelog.sh"))
            explicit = command(check.build_groups(self.root, base_ref="feature/x"))
            self.assertEqual((explicit.base_ref, explicit.base_explicit, explicit.argv[-1]), ("feature/x", True, "feature/x"))
        with mock.patch.dict(os.environ, {"GITHUB_BASE_REF": "release"}):
            self.assertEqual(command(check.build_groups(self.root)).base_ref, "origin/release")
        # End to end: an explicit base that does not resolve fails the run.
        self.git("-c", "init.defaultBranch=trunk", "init", "-q")
        self.git("commit", "-q", "--allow-empty", "-m", "base")
        out = io.StringIO()
        code = check.main(["--group", "changelog", "--base", "origin/no-such-branch"], root=self.root, out=out)
        self.assertEqual(code, 1, out.getvalue())
        self.assertIn("cannot resolve a merge base with origin/no-such-branch", out.getvalue())

    def test_other_platform_skips(self) -> None:
        windows = check.Group("w", "w", "w", platform="windows", commands=(fake("a", OK),))
        one = check.Group("o", "o", "o", commands=(fake("b", OK, platform="windows"),))
        report, _ = self.run_groups([windows, one], strict=True)
        self.assertEqual(self.statuses(report), {"a": SKIP, "b": SKIP})
        self.assertEqual(report.results[0].reason, "Windows-only")

    def test_unavailable_pinned_interpreter_skips_unless_strict(self) -> None:
        group = check.Group("g", "g", "g", commands=(fake("old", OK, python="0.1"),))
        report, _ = self.run_groups([group])
        self.assertEqual((report.results[0].status, report.results[0].reason[:24]), (SKIP, "python0.1 not installed"))
        self.assertEqual(self.run_groups([group], strict=True)[0].results[0].status, FAIL)

    def test_python3_everywhere_is_this_interpreter(self) -> None:
        code = (
            "import os, subprocess, sys\n"
            "nested = subprocess.check_output(['python3', '-c', 'import sys; print(sys.executable)'], text=True)\n"
            "shell = subprocess.check_output(['sh', '-c', 'python -c \"import sys; print(sys.executable)\"'], text=True)\n"
            "open('seen.txt', 'w').write('\\n'.join([sys.executable, nested.strip(), shell.strip(), os.getcwd(), os.environ['CHECK_ENV']]))\n"
        )
        (self.root / "sub").mkdir()
        group = check.Group("g", "g", "g", env=(("CHECK_ENV", "set"),), commands=(fake("which", code, cwd="sub"),))
        report, _ = self.run_groups([group])
        self.assertEqual(report.exit_code, 0, report.results[0].reason)
        seen = (self.root / "sub" / "seen.txt").read_text().splitlines()
        real = os.path.realpath(sys.executable)
        self.assertEqual([os.path.realpath(path) for path in seen[:3]], [real] * 3)
        self.assertEqual(os.path.realpath(seen[3]), os.path.realpath(str(self.root / "sub")))
        self.assertEqual(seen[4], "set")

    def test_shell_step_stops_at_its_first_failure(self) -> None:
        loop = check.shell("loop", "for x in 1 2; do\n  false\n  echo reached > reached.txt\ndone")
        report, _ = self.run_groups([check.Group("g", "g", "g", commands=(loop,))])
        self.assertEqual(report.results[0].status, FAIL)
        self.assertFalse((self.root / "reached.txt").exists())

    def test_fail_fast_cancels_running_and_pending_commands(self) -> None:
        failing = check.Group("a", "a", "a", weight=1, commands=(fake("bad", BOOM), fake("next", OK)))
        slow = check.Group("b", "b", "b", commands=(fake("slow", "import time; time.sleep(60)"),))
        started = time.monotonic()
        report, _ = self.run_groups([failing, slow], jobs=2, fail_fast=True)
        self.assertLess(time.monotonic() - started, 30)
        statuses = self.statuses(report)
        self.assertEqual((statuses["bad"], statuses["next"], statuses["slow"]), (FAIL, SKIP, SKIP))
        reasons = {result.command.name: result.reason for result in report.results}
        self.assertIn("--fail-fast", reasons["next"])
        self.assertIn("--fail-fast", reasons["slow"])
        self.assertEqual(report.exit_code, 1)

    def test_stop_signals_end_every_command_and_remove_the_work_dir(self) -> None:
        # Every command runs in a session of its own, so a signal that ends
        # check.py alone (kill, timeout, a closed terminal) used to leave the
        # commands running as orphans, and /tmp/check-py-* behind.
        (self.root / "sleeper.py").write_text(SLEEPER, encoding="utf-8")
        (self.root / "driver.py").write_text(DRIVER, encoding="utf-8")
        for signum in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
            with self.subTest(signal=signum.name):
                tmp = Path(tempfile.mkdtemp(prefix="test-check-tmp-"))
                self.addCleanup(shutil.rmtree, tmp, True)
                pids_file = self.root / "pids"
                if pids_file.exists():
                    pids_file.unlink()
                process = subprocess.Popen(
                    [sys.executable, str(self.root / "driver.py"), str(ROOT / "scripts"), str(self.root)],
                    cwd=str(self.root),
                    env=dict(os.environ, TMPDIR=str(tmp)),
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                )
                self.addCleanup(lambda p=process: p.poll() is None and p.kill())
                deadline = time.monotonic() + 60
                while not pids_file.exists() and process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.05)
                self.assertTrue(pids_file.exists(), process.communicate(timeout=30)[0] if process.poll() is not None else "no pids")
                pids = [int(pid) for pid in pids_file.read_text().split()]
                self.addCleanup(kill_all, pids)
                process.send_signal(signum)
                out, _ = process.communicate(timeout=60)
                self.assertEqual(process.returncode, 130, out)
                self.assertIn("INTERRUPTED", out)
                self.assertIn(f"cancelled: interrupted ({signum.name})", out)
                deadline = time.monotonic() + 15
                while any(alive(pid) for pid in pids) and time.monotonic() < deadline:
                    time.sleep(0.05)
                self.assertEqual([pid for pid in pids if alive(pid)], [], "a command outlived check.py")
                self.assertEqual(sorted(path.name for path in tmp.glob("check-py-*")), [])

    def test_signal_handlers_are_restored_and_an_ignored_signal_stays_ignored(self) -> None:
        before = {signum: signal.getsignal(signum) for signum in check.STOP_SIGNALS}
        self.addCleanup(lambda: [signal.signal(signum, handler) for signum, handler in before.items()])
        signal.signal(signal.SIGHUP, signal.SIG_IGN)  # as under nohup
        runner = check.Runner(root=self.root, out=io.StringIO())
        previous = runner._trap_signals()
        try:
            self.assertIs(signal.getsignal(signal.SIGHUP), signal.SIG_IGN)
            self.assertEqual(signal.getsignal(signal.SIGTERM), runner._on_signal)
            self.assertNotIn(signal.SIGHUP, previous)
        finally:
            for signum, handler in previous.items():
                signal.signal(signum, handler)
        signal.signal(signal.SIGHUP, before[signal.SIGHUP])
        report, _ = self.run_groups([check.Group("g", "g", "g", commands=(fake("ok", OK),))])
        self.assertEqual(report.exit_code, 0)
        self.assertEqual({signum: signal.getsignal(signum) for signum in check.STOP_SIGNALS}, before)

    def test_matrix_legs_run_in_parallel_and_jobs_in_order(self) -> None:
        stamp = (
            "import sys, time; open(sys.argv[1], 'w').write(str(time.time())); "
            "time.sleep(1); open(sys.argv[1], 'a').write(' ' + str(time.time()))"
        )

        def leg(name: str, leg_name: str) -> check.Command:
            return check.Command(name=name, argv=("python3", "-c", stamp, name), ci=name, leg=leg_name)

        matrix = check.Group("m", "m", "m", matrix=True, commands=(leg("m1", "A"), leg("m2", "B")))
        plain = check.Group("p", "p", "p", commands=(leg("p1", "A"), leg("p2", "B")))
        report, _ = self.run_groups([matrix, plain], jobs=4)
        self.assertEqual(report.exit_code, 0)
        spans = {
            name: [float(value) for value in (self.root / name).read_text().split()]
            for name in ("m1", "m2", "p1", "p2")
        }
        self.assertTrue(spans["m1"][0] < spans["m2"][1] and spans["m2"][0] < spans["m1"][1], spans)
        self.assertLessEqual(spans["p1"][1], spans["p2"][0], spans)
        self.assertEqual([result.command.name for result in report.results], ["m1", "m2", "p1", "p2"])
        self.assertEqual(len(check.lanes_of(matrix)), 2)
        self.assertEqual(len(check.lanes_of(plain)), 1)

    def test_suite_loop_expands_per_file_and_fails_like_ci_when_empty(self) -> None:
        suite = self.root / "suite"
        suite.mkdir()
        self.assertEqual(len(check._suite_commands(self.root, "suite")), 1)
        empty = check.Group("t", "t", "t", commands=tuple(check._suite_commands(self.root, "suite")))
        self.assertEqual(self.run_groups([empty])[0].results[0].status, FAIL)
        for name in ("test_b.py", "test_a.py", "helper.py"):
            (suite / name).write_text("print('ran')\n")
        commands = check._suite_commands(self.root, "suite")
        self.assertEqual([command.argv for command in commands], [("python3", "test_a.py"), ("python3", "test_b.py")])
        self.assertTrue(all(command.ci == check.TESTS_LOOP for command in commands))

    def test_main_prints_tail_then_summary(self) -> None:
        groups = [
            check.Group("first", "first", "first", timeout_minutes=0.0001, commands=(fake("slow-ok", "import time; time.sleep(0.2)"),)),
            check.Group("second", "second", "second", commands=(fake("bad", BOOM), fake("skipped", OK, tools=("check-py-no-such-tool",)))),
        ]
        out = io.StringIO()
        code = check.main([], groups=groups, root=self.root, out=out)
        text = out.getvalue()
        self.assertEqual(code, 1)
        self.assertIn("---- FAIL second: bad (exit 3)", text)
        self.assertIn("     | boom 99", text)
        self.assertNotIn("     | boom 39\n", text)  # only the last 60 lines
        table = text[text.index("group "):]
        self.assertRegex(table, r"(?m)^first\s+PASS\s+1\s+0\s+0\s+[\d.]+s  ! over CI's timeout-minutes")
        self.assertRegex(table, r"(?m)^second\s+FAIL\s+0\s+1\s+1\s")
        self.assertIn("FAILED: 1 passed, 1 failed, 1 skipped", table)
        self.assertLess(text.index("---- FAIL"), text.index("group "))
        log_dir = re.search(r"logs of failed commands: (.+)", text).group(1)
        shutil.rmtree(Path(log_dir).parent, True)

    def test_main_group_selection_list_and_usage_errors(self) -> None:
        groups = [check.Group("one", "one", "one", commands=(fake("a", OK),)), check.Group("two", "two", "two", commands=(fake("b", OK),))]
        out = io.StringIO()
        self.assertEqual(check.main(["--group", "two"], groups=groups, root=self.root, out=out), 0)
        self.assertIn("two: b", out.getvalue())
        self.assertNotIn("one: a", out.getvalue())
        out = io.StringIO()
        self.assertEqual(check.main(["--list"], groups=groups, root=self.root, out=out), 0)
        self.assertIn("one  (one; ci.yml > one)", out.getvalue())
        for argv in (["--group", "three"], ["--jobs", "0"]):
            with self.subTest(argv=argv), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as raised:
                    check.main(argv, groups=groups, root=self.root, out=io.StringIO())
                self.assertEqual(raised.exception.code, 2)

    def test_default_jobs(self) -> None:
        self.assertEqual(check.default_jobs(1), 1)
        self.assertGreaterEqual(check.default_jobs(12), 2)
        self.assertLessEqual(check.default_jobs(3), 3)


if __name__ == "__main__":
    unittest.main()
