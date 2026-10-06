#!/usr/bin/env python3
"""Run locally what CI runs: one command, one exit code.

Each group below is one job of `.github/workflows/*.yml`; its commands are
that job's `run:` steps in workflow order, each an argv plus the step's working
directory. It always runs from the repository root, wherever it is invoked.

    python3 scripts/check.py                  # every group, groups in parallel
    python3 scripts/check.py --group lint     # one group (repeatable)
    python3 scripts/check.py --list           # what would run, and what it needs
    python3 scripts/check.py --fail-fast      # stop everything at the first failure
    python3 scripts/check.py --jobs 4         # how many jobs (or matrix legs) run at once
    python3 scripts/check.py --strict         # a missing tool fails instead of skipping

Why it exists: the copy-paste loops in the documentation exited 0 when a test
failed, and the local command list had drifted from the workflow - the harness
regression tests, for one, ran only in CI. `tests/test_check.py` parses the
workflows and fails when a CI gate has no entry here, or when an entry here is
no longer in CI, so the two cannot drift apart silently again.

Fidelity, and its deliberate limits:

* A step that needs a shell runs the way GitHub Actions runs it, as
  `bash --noprofile --norc -eo pipefail -c <step>`, so a loop stops at its
  first failing command.
* `python3` (and `python`) inside every command - including the interpreters
  tests spawn themselves - is the interpreter running this script: the local
  stand-in for `actions/setup-python`. A job CI pins to 3.9 runs under
  `python3.9`.
* Steps that only provision the runner (apt-get, sysctl, venv creation, pip
  install) are not repeated; `tests/test_check.py` lists them explicitly.
* A tool CI's runner provides but this machine lacks (shellcheck, php, pwsh,
  bwrap, python3.9, the harness venv) skips with the reason; `--strict` makes
  it a failure. An entry whose files are not in this checkout skips as
  "not present" in either mode: there is nothing to run, and CI does not run it
  either.
* Windows-only jobs are listed and skip elsewhere. The runner supports Linux
  and macOS for now.
* Jobs run in parallel, and so do the legs of a matrix job, which CI also runs
  as separate jobs; the steps of one job (or leg) run in order. `--jobs 1`
  runs everything one after another.
* The `tests` job's file loop is expanded into one command per test file, so a
  failure names its file. Unlike CI, a failing command does not cancel the rest
  of its job: one run shows every failure. `--fail-fast` stops everything at
  the first one.
* CI's per-job `timeout-minutes` is reported, not enforced: a busy laptop
  running every group at once is not the runner it was sized for.

Exit status: 0 when nothing failed (skips allowed), 1 on any failure, 2 on a
usage error or an unsupported platform, 130 when interrupted. Standard library
only; it writes nothing in the repository (logs and interpreter shims live in a
temporary directory, kept only when something failed).
"""

from __future__ import annotations

import argparse
import concurrent.futures
import os
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import textwrap
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Set, TextIO, Tuple

ROOT = Path(__file__).resolve().parent.parent

PASS, FAIL, SKIP = "PASS", "FAIL", "SKIP"
TAIL_LINES = 60

# How GitHub Actions invokes a `run:` step: `bash --noprofile --norc -eo
# pipefail {0}` on Linux/macOS runners, and for `shell: pwsh` steps PowerShell.
GHA_BASH = ("bash", "--noprofile", "--norc", "-eo", "pipefail", "-c")
GHA_PWSH = ("pwsh", "-NoProfile", "-NonInteractive", "-Command")

HARNESS_PY = "harness/.venv/bin/python"
TOOL_HINTS = {
    "shellcheck": "GitHub's ubuntu runner preinstalls it; install it from your package manager",
    "php": "GitHub's ubuntu runner preinstalls it; install php-cli",
    "pwsh": "install PowerShell 7",
    "bwrap": "install bubblewrap",
    "python3.9": "CI pins this job to Python 3.9; put a python3.9 on PATH (for example `uv python install 3.9`)",
    HARNESS_PY: "python3 -m venv harness/.venv && harness/.venv/bin/python -m pip install -e harness",
}


@dataclass(frozen=True)
class Command:
    """One CI `run:` command.

    `ci` is the command text as the workflow writes it; tests/test_check.py
    matches it against the workflow. `paths` are repository files the command
    needs: when one is absent the command skips as "not present". `tools` are
    executables beyond argv[0] (a bare name is looked up on PATH, a path is
    taken relative to the repository root). `python` pins an interpreter
    version ("" is the one running this script). `local_only` gives the reason
    for a gate CI does not run (yet); every other command must exist in CI.
    """

    name: str
    argv: Tuple[str, ...]
    ci: str
    cwd: str = "."
    leg: str = ""
    tools: Tuple[str, ...] = ()
    paths: Tuple[str, ...] = ()
    python: str = ""
    platform: str = ""
    env: Tuple[Tuple[str, str], ...] = ()
    local_only: str = ""
    # What actually runs, for output, when that differs from `ci`.
    shown: str = ""

    @property
    def label(self) -> str:
        return f"{self.leg} > {self.name}" if self.leg else self.name

    @property
    def display(self) -> str:
        text = self.shown or self.ci
        return text if "\n" not in text else text.splitlines()[0] + " ..."


@dataclass(frozen=True)
class Group:
    """One CI job: `job` in `.github/workflows/<workflow>`."""

    name: str
    title: str
    job: str
    workflow: str = "ci.yml"
    commands: Tuple[Command, ...] = ()
    timeout_minutes: float = 0
    platform: str = ""
    env: Tuple[Tuple[str, str], ...] = ()
    note: str = ""
    # A matrix job: CI runs each leg (the commands sharing a `leg`) as a job of
    # its own, so each leg is a lane of its own here; steps within a leg stay
    # in order.
    matrix: bool = False
    # Scheduling hint only: heavier groups start first so the slowest one is
    # not the last to begin.
    weight: int = 0


# --------------------------------------------------------------------------
# The groups. Keep them in step with .github/workflows/*.yml:
# `python3 -m unittest tests.test_check` says what is missing on either side.
# --------------------------------------------------------------------------


def step(name: str, line: str, **options) -> Command:
    """A `run:` line that needs no shell: split into argv exactly once."""
    return Command(name=name, argv=tuple(shlex.split(line)), ci=line, **options)


def shell(name: str, body: str, **options) -> Command:
    """A `run:` command that needs bash (a pipe, a loop, a conditional)."""
    body = textwrap.dedent(body).strip("\n")
    return Command(name=name, argv=GHA_BASH + (body,), ci=body, **options)


def pwsh(name: str, line: str, **options) -> Command:
    """A `shell: pwsh` step."""
    tools = ("pwsh",) + tuple(options.pop("tools", ()))
    return Command(name=name, argv=GHA_PWSH + (line,), ci=line, tools=tools, **options)


def module_path(module: str) -> str:
    return module.replace(".", "/") + ".py"


def unittests(name: str, modules: Sequence[str], interpreter: str = "python3", **options) -> Command:
    """`<interpreter> -m unittest <modules>`, one process, as CI runs it."""
    return step(name, f"{interpreter} -m unittest " + " ".join(modules), **options)


def guarded(modules: Sequence[str], *extra: str) -> Tuple[str, ...]:
    return tuple(module_path(module) for module in modules) + tuple(extra)


EDITION_TEST_SUITES = (
    "Laravel/memory-bank/tests",
    "Laravel/project-brain/tests",
    "Symfony/memory-bank/tests",
    "Symfony/project-brain/tests",
    "PHP Core/memory-bank/tests",
    "PHP Core/project-brain/tests",
    "Cms/wordpress/memory-bank/tests",
    "Cms/wordpress/project-brain/tests",
    "Infrastructure-Creator/tests",
)
PARITY_EDITIONS = ("Laravel", "Symfony", "PHP Core", "Cms/wordpress")
TESTS_LOOP = 'for test_file in test_*.py; do\n  python3 "$test_file"\ndone'

SHELL_FILES = "git ls-files -z -- '*.sh' 'collect' 'kit3' 'harness-server'"

KIT3_TESTS = (
    "tests.test_registry",
    "tests.test_open_source_kit",
    "tests.test_kit_fetcher",
    "tests.test_kit3",
    "tests.test_kit3_catalog",
)
HARNESS_TESTS = (
    "tests.test_harness_providers",
    "tests.test_harness_sessions",
    "tests.test_harness_web",
    "tests.test_harness_process_guard",
    "tests.test_harness_skills",
    "tests.test_harness_fleet",
    "tests.test_harness_knowledge",
    "tests.test_harness_task_context",
    "tests.test_harness_setup",
    "tests.test_harness_creator",
    "tests.test_harness_results",
    "tests.test_harness_delivery",
    "tests.test_harness_clash",
)

# Branches that carry the native-Windows and System-orchestration work run
# more modules in the harness step and add the `system-orchestration`,
# `windows-harness` and `windows-creator` jobs. Those entries are guarded by
# the files they need: where the files are absent they skip as "not present"
# (and CI has no such step); where they exist CI runs them, and
# tests/test_check.py then holds them to that workflow like every other entry.
HARNESS_TESTS_NEWER = (
    "tests.test_harness_run_activity",
    "tests.test_harness_commands",
    "tests.test_harness_process_runtime",
    "tests.test_harness_launcher",
    "tests.test_harness_memory_use",
    "tests.test_harness_context_usage",
    "tests.test_harness_memory_draft",
    "tests.test_portable_fs",
    "tests.test_windows_security",
    "tests.test_windows_commands",
    "tests.test_windows_creator",
)
SYSTEM_TESTS = (
    "tests.test_ai_system",
    "tests.test_ai_system_providers",
    "tests.test_ai_system_execution",
    "tests.test_ai_system_portable",
    "tests.test_harness_system_orchestration",
    "tests.test_harness_system_editor",
    "tests.test_harness_system_discovery",
    "tests.test_harness_agent_activity",
)
SYSTEM_CATALOG = "docs/examples/ai-system/system.json"
WINDOWS_HARNESS_TESTS = (
    "tests.test_harness_windows",
    "tests.test_harness_launcher",
    "tests.test_harness_process_runtime",
    "tests.test_portable_fs",
    "tests.test_windows_security",
    "tests.test_windows_commands",
    "tests.test_windows_creator",
    "tests.test_windows_discovery",
    "tests.test_ai_system_portable",
)
WINDOWS_CREATOR_TESTS = ("tests.test_windows_creator", "tests.test_windows_discovery")


def _suite_commands(root: Path, suite: str) -> List[Command]:
    """The `tests` job's loop, one command per file so a failure names it."""
    files = sorted(path.name for path in (root / suite).glob("test_*.py"))
    if not files:
        # CI's loop would then run `python3 "test_*.py"` and fail; so does this.
        return [shell("Run unit tests", TESTS_LOOP, cwd=suite, leg=suite)]
    return [
        Command(name=name, argv=("python3", name), ci=TESTS_LOOP, cwd=suite, leg=suite, shown=f"python3 {name}")
        for name in files
    ]


def _parity_commands(edition: str) -> List[Command]:
    commands = [
        step("Mirror parity", "python3 memory-bank/scripts/context.py parity", cwd=edition, leg=edition),
        step(
            "Cross-edition core parity",
            "python3 memory-bank/scripts/context.py parity --cross-edition",
            cwd=edition,
            leg=edition,
        ),
    ]
    if edition == "Laravel":
        # CI: `if: matrix.edition == 'Laravel'` - repository-scoped, run once.
        commands += [
            step("Generator-asset parity", "python3 scripts/asset_parity.py --check", leg=edition),
            unittests("Generator-asset parity regression tests", ["tests.test_asset_parity"], leg=edition),
        ]
    return commands


def _system_commands(python: str) -> List[Command]:
    leg = f"Python {python or '3.x'}"
    needs = guarded(SYSTEM_TESTS, "scripts/ai_system.py", SYSTEM_CATALOG)
    return [
        step(
            "Sandbox probe (bubblewrap)",
            "bwrap --unshare-pid --ro-bind / / --proc /proc --dev /dev true",
            leg=leg,
            platform="linux",
            paths=needs,
        ),
        unittests(
            "Test universal system planning and execution",
            SYSTEM_TESTS,
            leg=leg,
            python=python,
            paths=needs,
        ),
        step(
            "Validate synthetic service catalog",
            f"python3 scripts/ai_system.py validate --system {SYSTEM_CATALOG}",
            leg=leg,
            python=python,
            paths=needs,
        ),
    ]


def build_groups(root: Path = ROOT, base_ref: Optional[str] = None) -> List[Group]:
    """Every CI job as a group, in workflow order."""
    changelog_argv = ("bash", "scripts/check_core_changelog.sh") + ((base_ref,) if base_ref else ())
    return [
        Group(
            name="tests",
            matrix=True,
            title="tests (matrix: suite)",
            job="tests",
            timeout_minutes=10,
            weight=90,
            commands=tuple(c for suite in EDITION_TEST_SUITES for c in _suite_commands(root, suite)),
        ),
        Group(
            name="parity",
            matrix=True,
            title="parity (matrix: edition)",
            job="parity",
            timeout_minutes=10,
            weight=40,
            commands=tuple(c for edition in PARITY_EDITIONS for c in _parity_commands(edition)),
        ),
        Group(
            name="mirrors",
            title="mirrors",
            job="mirrors",
            timeout_minutes=5,
            weight=30,
            commands=(
                step("Verify tool mirrors against canon", "python3 scripts/build_mirrors.py --check"),
                unittests("Mirror executor regression tests", ["tests.test_build_mirrors"]),
            ),
        ),
        Group(
            name="infrastructure-creator-reliability",
            title="Infrastructure-Creator reliability (Python 3.9)",
            job="infrastructure-creator-reliability",
            timeout_minutes=15,
            weight=80,
            commands=(
                step(
                    "Run complete reliability suite",
                    'python3 -m unittest discover -s Infrastructure-Creator/tests -p "test_*.py"',
                    python="3.9",
                ),
                step(
                    "Validate canonical reference contracts",
                    "python3"
                    " Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_reference_catalogs.py"
                    " --references-dir"
                    " Infrastructure-Creator/.agents/skills/skill-forge/references",
                    python="3.9",
                ),
                step(
                    "Verify Infrastructure-Creator mirrors",
                    "python3 scripts/build_mirrors.py --check --edition Infrastructure-Creator",
                    python="3.9",
                ),
            ),
        ),
        Group(
            name="installation",
            title="installation",
            job="installation",
            timeout_minutes=10,
            weight=100,
            commands=(
                step(
                    "Verify deterministic installation inventories",
                    "python3 scripts/install_accelerator.py --verify-inventories",
                ),
                unittests("Test selected-tool clean installations", ["tests.test_installation"]),
                unittests("Preserve framework-specific skill semantics", ["tests.test_framework_semantics"]),
                unittests("Optional developer tooling stays out of the editions", ["tests.test_collect_context"]),
                step("Kit 3 admission registry is well-formed", "python3 scripts/validate_registry.py --check"),
                unittests("Kit 3 registry and catalog regression tests", KIT3_TESTS),
                unittests("Native browser harness regression tests", HARNESS_TESTS),
                unittests(
                    "Native browser harness regression tests (newer modules)",
                    HARNESS_TESTS_NEWER,
                    paths=guarded(HARNESS_TESTS_NEWER),
                ),
            ),
        ),
        Group(
            name="harness-fleet",
            title="Offline Fleet runtime",
            job="harness-fleet",
            timeout_minutes=10,
            weight=60,
            note="CI first builds harness/.venv with `pip install -e harness`; locally that venv must exist",
            commands=(
                step(
                    "Graph and checkpoint regression tests",
                    f"{HARNESS_PY} -m unittest discover -s harness/tests -p 'test_*.py'",
                ),
                unittests("Browser Fleet integration without model calls", ["tests.test_harness_fleet"]),
            ),
        ),
        Group(
            name="lint",
            title="lint",
            job="lint",
            timeout_minutes=5,
            weight=70,
            commands=(
                shell("Bash syntax check (bash -n)", f"{SHELL_FILES} | xargs -0 -r -n1 bash -n"),
                shell(
                    "ShellCheck (severity gate error)",
                    f"{SHELL_FILES} | xargs -0 -r shellcheck -S error",
                    tools=("shellcheck",),
                ),
                shell(
                    "Validate JSON",
                    """
                    git ls-files -z -- '*.json' | while IFS= read -r -d '' f; do
                      python3 -m json.tool "$f" > /dev/null || { echo "Invalid JSON: $f" >&2; exit 1; }
                    done
                    """,
                ),
                shell(
                    "Cursor rule render carries no per-turn invalidator",
                    r"""
                    if git ls-files -z -- '*/.cursor/hooks/working-memory-write.sh' \
                                          '*/.cursor/hooks/local-context.sh' \
                      | xargs -0 -r grep -nE '\bdate[[:space:]]+[-+]|\$\(date|\$RANDOM|uuidgen'; then
                      echo "Per-turn invalidator in the Cursor rule render (above)." >&2
                      exit 1
                    fi
                    """,
                ),
                step(
                    "PHP snippets are valid PHP",
                    "python3 scripts/check_php_snippets.py --require-php",
                    tools=("php",),
                ),
                step("Context budget", "python3 scripts/context_budget.py --check"),
                step("Stabilization rules are well-formed", "python3 scripts/check_stabilization.py"),
                unittests("Stabilization validator regression tests", ["tests.test_check_stabilization"]),
                step("Policy lock matches the surface", "python3 scripts/policy_lock.py --check"),
                unittests("Policy lock regression tests", ["tests.test_policy_lock"]),
                unittests("Routing eval scoring and fixtures", ["tests.test_routing_eval"]),
                step(
                    "Command and agent routes resolve",
                    "python3 scripts/check_routes.py",
                    paths=("scripts/check_routes.py",),
                    local_only="added by the routing-check change; wire it into ci.yml's lint job",
                ),
                unittests(
                    "check.py mirrors the workflows",
                    ["tests.test_check"],
                    local_only="ci.yml does not run tests/test_check.py yet; wire it into the lint job",
                ),
            ),
        ),
        Group(
            name="changelog",
            title="core-changelog",
            job="changelog",
            timeout_minutes=5,
            weight=10,
            note="CI runs it on pull requests against the PR base; locally against origin/main (or --base)",
            commands=(
                Command(
                    name="Shared-core changes require a root CHANGELOG entry",
                    argv=changelog_argv,
                    ci='bash scripts/check_core_changelog.sh "origin/${{ github.base_ref }}"',
                    shown=" ".join(shlex.quote(part) for part in changelog_argv),
                ),
            ),
        ),
        Group(
            name="links",
            title="links",
            job="links",
            timeout_minutes=5,
            weight=20,
            commands=(step("Check relative markdown links", "python3 scripts/check_links.py"),),
        ),
        Group(
            name="system-orchestration",
            matrix=True,
            title="system orchestration (matrix: Python 3.9, 3.x)",
            job="system-orchestration",
            timeout_minutes=10,
            weight=50,
            note="CI installs bubblewrap and lifts the AppArmor userns restriction first",
            commands=tuple(_system_commands("3.9") + _system_commands("")),
        ),
        Group(
            name="windows-harness",
            title="Windows Harness (Python 3.13)",
            job="windows-harness",
            timeout_minutes=15,
            platform="windows",
            commands=(
                pwsh(
                    "Native Windows Harness acceptance and boundary tests",
                    "python -m unittest " + " ".join(WINDOWS_HARNESS_TESTS),
                    paths=guarded(WINDOWS_HARNESS_TESTS),
                ),
            ),
        ),
        Group(
            name="windows-creator",
            title="Windows Creator sandbox",
            job="creator",
            workflow="windows-creator.yml",
            timeout_minutes=20,
            platform="windows",
            env=(("HARNESS_WINDOWS_CREATOR_SANDBOX", "1"),),
            note="workflow_dispatch on a self-hosted Windows runner with Codex's elevated sandbox",
            commands=(
                pwsh(
                    "Verify real elevated sandbox boundaries for Creator and AI discovery",
                    "python -m unittest " + " ".join(WINDOWS_CREATOR_TESTS) + " -v",
                    paths=guarded(WINDOWS_CREATOR_TESTS),
                ),
            ),
        ),
    ]


# --------------------------------------------------------------------------
# The runner.
# --------------------------------------------------------------------------


@dataclass
class Result:
    group: Group
    command: Command
    status: str
    seconds: float = 0.0
    reason: str = ""
    returncode: Optional[int] = None
    log: Optional[Path] = None
    # time.monotonic() when the command started; 0.0 when it never ran.
    started: float = 0.0


@dataclass
class Report:
    results: List[Result] = field(default_factory=list)
    wall_seconds: float = 0.0
    log_dir: Optional[Path] = None
    interrupted: bool = False

    @property
    def failed(self) -> List[Result]:
        return [result for result in self.results if result.status == FAIL]

    @property
    def exit_code(self) -> int:
        if self.interrupted:
            return 130
        return 1 if self.failed else 0


def find_python(version: str) -> Optional[str]:
    """The interpreter for a pinned version, or the one running this script."""
    if not version or version == "%d.%d" % sys.version_info[:2]:
        return sys.executable
    return shutil.which("python" + version)


def platform_matches(name: str) -> bool:
    if name == "windows":
        return os.name == "nt"
    if name == "linux":
        return sys.platform.startswith("linux")
    return True


def default_jobs(lanes: int) -> int:
    cpus = os.cpu_count() or 2
    return max(1, min(lanes, max(2, cpus // 2)))


def lanes_of(group: Group) -> List[List[Tuple[int, Command]]]:
    """The group's commands as lanes, each run in order: one lane per job,
    or one per leg for a matrix job. Indexes keep the group's order."""
    indexed = list(enumerate(group.commands))
    if not group.matrix:
        return [indexed] if indexed else []
    legs: Dict[str, List[Tuple[int, Command]]] = {}
    for index, command in indexed:
        legs.setdefault(command.leg, []).append((index, command))
    return list(legs.values())


class Runner:
    """Runs lanes (a job, or one leg of a matrix job) in parallel and the
    commands of one lane in order."""

    def __init__(
        self,
        root: Path = ROOT,
        jobs: Optional[int] = None,
        fail_fast: bool = False,
        strict: bool = False,
        out: Optional[TextIO] = None,
    ) -> None:
        self.root = root
        self.jobs = jobs
        self.fail_fast = fail_fast
        self.strict = strict
        self.out = out if out is not None else sys.stdout
        self._stop = threading.Event()
        self._stop_reason = ""
        self._lock = threading.Lock()
        self._running: Dict[int, subprocess.Popen] = {}
        # Processes this runner terminated: their exit is a cancellation, not a
        # failure, whichever thread happened to observe it first.
        self._killed: Set[int] = set()
        self._shims: Dict[str, Path] = {}
        self._work: Optional[Path] = None

    # -- preflight ---------------------------------------------------------

    def _has_tool(self, tool: str) -> bool:
        if "/" in tool:
            return os.access(str(self.root / tool), os.X_OK)
        return shutil.which(tool) is not None

    def preflight(self, group: Group, command: Command) -> Optional[Tuple[str, str]]:
        """(status, reason) when the command cannot run here, else None."""
        platform = command.platform or group.platform
        if platform and not platform_matches(platform):
            return SKIP, f"{platform.capitalize()}-only"
        missing = [path for path in command.paths if not (self.root / path).exists()]
        if missing:
            more = f" (+{len(missing) - 1} more)" if len(missing) > 1 else ""
            return SKIP, f"not present: {missing[0]}{more}"
        absent: List[str] = []
        if find_python(command.python) is None:
            absent.append("python" + command.python)
        program = command.argv[0]
        for tool in ((program,) if program not in ("python3", "python") else ()) + command.tools:
            if tool not in absent and not self._has_tool(tool):
                absent.append(tool)
        if absent:
            tool = absent[0]
            hint = TOOL_HINTS.get(tool)
            reason = f"{tool} not installed" + (f" ({hint})" if hint else "")
            if self.strict:
                return FAIL, reason + " [--strict]"
            return SKIP, reason
        return None

    # -- execution ---------------------------------------------------------

    def _shim(self, interpreter: str) -> Path:
        """A PATH directory whose python3/python exec the chosen interpreter."""
        with self._lock:
            if interpreter not in self._shims:
                assert self._work is not None
                directory = self._work / "bin" / str(len(self._shims))
                directory.mkdir(parents=True)
                for name in ("python3", "python"):
                    shim = directory / name
                    shim.write_text(f'#!/bin/sh\nexec {shlex.quote(interpreter)} "$@"\n', encoding="utf-8")
                    shim.chmod(0o755)
                self._shims[interpreter] = directory
            return self._shims[interpreter]

    def _terminate(self, process: subprocess.Popen) -> None:
        """Stop the command and everything it spawned (its own session)."""
        with self._lock:
            self._killed.add(process.pid)
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(process.pid, sig)
            except (ProcessLookupError, PermissionError):
                return
            try:
                process.wait(timeout=5)
                return
            except subprocess.TimeoutExpired:
                continue

    def _execute(self, group: Group, command: Command, index: int) -> Result:
        assert self._work is not None
        interpreter = find_python(command.python)
        assert interpreter is not None
        env = dict(os.environ)
        env["PATH"] = str(self._shim(interpreter)) + os.pathsep + env.get("PATH", "")
        env.update(dict(group.env))
        env.update(dict(command.env))
        log = self._work / "logs" / f"{group.name}-{index:03d}.log"
        started = time.monotonic()
        with open(log, "wb") as handle:
            handle.write(f"$ {command.shown or command.ci}\n  (cwd: {command.cwd})\n\n".encode("utf-8"))
            handle.flush()
            try:
                process = subprocess.Popen(
                    list(command.argv),
                    cwd=str(self.root / command.cwd),
                    env=env,
                    stdin=subprocess.DEVNULL,
                    stdout=handle,
                    stderr=subprocess.STDOUT,
                    start_new_session=True,
                )
            except OSError as error:
                return Result(
                    group, command, FAIL, time.monotonic() - started, f"could not start: {error}", log=log, started=started
                )
            with self._lock:
                self._running[process.pid] = process
            try:
                while True:
                    try:
                        returncode = process.wait(timeout=0.2)
                        break
                    except subprocess.TimeoutExpired:
                        if self._stop.is_set():
                            self._terminate(process)
                            returncode = process.wait()
                            break
            finally:
                with self._lock:
                    self._running.pop(process.pid, None)
                    cancelled = process.pid in self._killed
        seconds = time.monotonic() - started
        if cancelled:
            return Result(group, command, SKIP, seconds, f"cancelled: {self._stop_reason}", returncode, log, started)
        if returncode == 0:
            return Result(group, command, PASS, seconds, "", 0, log, started)
        return Result(group, command, FAIL, seconds, f"exit {returncode}", returncode, log, started)

    def _run_lane(self, group: Group, lane: Sequence[Tuple[int, Command]]) -> List[Tuple[int, Result]]:
        results: List[Tuple[int, Result]] = []
        for index, command in lane:
            if self._stop.is_set():
                result = Result(group, command, SKIP, reason=f"not run: {self._stop_reason}")
            else:
                verdict = self.preflight(group, command)
                if verdict is not None:
                    result = Result(group, command, verdict[0], reason=verdict[1])
                else:
                    result = self._execute(group, command, index)
            if result.status == FAIL and self.fail_fast:
                self.stop("stopped after a failure (--fail-fast)")
            results.append((index, result))
            self._print_live(result)
        return results

    def _print_live(self, result: Result) -> None:
        reason = f"  ({result.reason})" if result.reason else ""
        timing = f"{result.seconds:7.1f}s" if result.status != SKIP or result.seconds else "       -"
        line = f"{result.status}  {timing}  {result.group.name}: {result.command.label}{reason}"
        with self._lock:
            print(line, file=self.out, flush=True)

    def stop(self, reason: str) -> None:
        with self._lock:
            if not self._stop.is_set():
                self._stop_reason = reason
                self._stop.set()

    def interrupt(self) -> None:
        self.stop("interrupted")
        with self._lock:
            processes = list(self._running.values())
        for process in processes:
            self._terminate(process)

    def run(self, groups: Sequence[Group]) -> Report:
        report = Report()
        self._work = Path(tempfile.mkdtemp(prefix="check-py-"))
        (self._work / "logs").mkdir()
        lanes = [(group, lane) for group in groups for lane in lanes_of(group)]
        jobs = self.jobs or default_jobs(len(lanes))
        ordered = sorted(lanes, key=lambda pair: -pair[0].weight)
        print(
            f"check.py: {len(groups)} group(s) in {len(lanes)} lane(s), "
            f"{sum(len(g.commands) for g in groups)} command(s), {jobs} lane(s) at a time; "
            f"python3 = {sys.executable} ({sys.version.split()[0]})",
            file=self.out,
            flush=True,
        )
        started = time.monotonic()
        executor = concurrent.futures.ThreadPoolExecutor(max_workers=jobs)
        futures = {executor.submit(self._run_lane, group, lane): group for group, lane in ordered}
        try:
            pending = set(futures)
            while pending:
                _, pending = concurrent.futures.wait(pending, timeout=0.5)
        except KeyboardInterrupt:
            report.interrupted = True
            self.interrupt()
        finally:
            executor.shutdown(wait=True)
        report.wall_seconds = time.monotonic() - started
        collected: Dict[str, Dict[int, Result]] = {}
        for future, group in futures.items():
            if future.done():
                collected.setdefault(group.name, {}).update(future.result())
        for group in groups:
            results = collected.get(group.name, {})
            report.results.extend(results[index] for index in sorted(results))
        shutil.rmtree(self._work / "bin", ignore_errors=True)
        if any(result.log is not None for result in report.failed):
            report.log_dir = self._work / "logs"
        else:
            shutil.rmtree(self._work, ignore_errors=True)
        return report


# --------------------------------------------------------------------------
# Output.
# --------------------------------------------------------------------------


def tail(path: Optional[Path], lines: int = TAIL_LINES) -> Tuple[List[str], int]:
    if path is None or not path.exists():
        return [], 0
    text = path.read_bytes().decode("utf-8", errors="replace").splitlines()
    return text[-lines:], len(text)


def print_failures(report: Report, out: TextIO) -> None:
    for result in report.failed:
        shown, total = tail(result.log)
        print(file=out)
        print(f"---- FAIL {result.group.name}: {result.command.label} ({result.reason})", file=out)
        print(f"     $ {result.command.display}   (cwd: {result.command.cwd})", file=out)
        if result.log is not None and shown:
            print(f"     last {len(shown)} of {total} lines; full log: {result.log}", file=out)
            for line in shown:
                print(f"     | {line}", file=out)


def group_status(results: Sequence[Result]) -> str:
    statuses = {result.status for result in results}
    if FAIL in statuses:
        return FAIL
    if PASS in statuses:
        return PASS
    return SKIP


def print_summary(groups: Sequence[Group], report: Report, out: TextIO) -> None:
    rows = []
    for group in groups:
        results = [result for result in report.results if result.group.name == group.name]
        if not results:
            continue
        ran = [result for result in results if result.started]
        seconds = max(r.started + r.seconds for r in ran) - min(r.started for r in ran) if ran else 0.0
        counts = {status: sum(1 for result in results if result.status == status) for status in (PASS, FAIL, SKIP)}
        # CI's limit applies per job, which for a matrix is per leg.
        per_lane: Dict[str, float] = {}
        for result in results:
            lane = result.command.leg if group.matrix else ""
            per_lane[lane] = per_lane.get(lane, 0.0) + result.seconds
        over = [lane for lane, total in per_lane.items() if group.timeout_minutes and total > group.timeout_minutes * 60]
        flag = ""
        if over:
            legs = ", ".join(lane for lane in over if lane)
            flag = f"over CI's timeout-minutes: {group.timeout_minutes:g}" + (f" ({legs})" if legs else "")
        rows.append((group.name, group_status(results), counts[PASS], counts[FAIL], counts[SKIP], seconds, flag))
    width = max([len("group")] + [len(row[0]) for row in rows])
    print(file=out)
    print(f"{'group':<{width}}  result  pass  fail  skip      wall", file=out)
    print(f"{'-' * width}  ------  ----  ----  ----  --------", file=out)
    for name, status, passed, failed, skipped, seconds, flag in rows:
        note = f"  ! {flag}" if flag else ""
        print(f"{name:<{width}}  {status:<6}  {passed:>4}  {failed:>4}  {skipped:>4}  {seconds:7.1f}s{note}", file=out)
    total = {status: sum(1 for result in report.results if result.status == status) for status in (PASS, FAIL, SKIP)}
    verdict = "INTERRUPTED" if report.interrupted else ("FAILED" if report.failed else "OK")
    print(
        f"\n{verdict}: {total[PASS]} passed, {total[FAIL]} failed, {total[SKIP]} skipped "
        f"in {report.wall_seconds:.1f}s wall time",
        file=out,
    )
    if report.log_dir is not None:
        print(f"logs of failed commands: {report.log_dir}", file=out)


def print_listing(groups: Sequence[Group], runner: Runner, out: TextIO) -> None:
    for group in groups:
        facts = [f"{group.workflow} > {group.job}"]
        if group.timeout_minutes:
            facts.append(f"timeout {group.timeout_minutes:g} min")
        if group.platform:
            facts.append(f"{group.platform}-only")
        print(f"{group.name}  ({group.title}; {'; '.join(facts)})", file=out)
        if group.note:
            print(f"    note: {group.note}", file=out)
        for command in group.commands:
            verdict = runner.preflight(group, command)
            state = f"  [{verdict[0].lower()}: {verdict[1]}]" if verdict else ""
            where = f"  (cwd: {command.cwd})" if command.cwd != "." else ""
            pin = f"  [python {command.python}]" if command.python else ""
            local = f"  [local only: {command.local_only}]" if command.local_only else ""
            print(f"  - {command.label}{state}{pin}{local}", file=out)
            print(f"      $ {command.display}{where}", file=out)


# --------------------------------------------------------------------------
# CLI.
# --------------------------------------------------------------------------


def parse_args(argv: Optional[Sequence[str]], names: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="check.py",
        description="Run locally what CI runs (.github/workflows/*.yml), group by group.",
    )
    parser.add_argument(
        "--group",
        action="append",
        metavar="NAME",
        help="run only this group (repeatable): " + ", ".join(names),
    )
    parser.add_argument("--list", action="store_true", help="list groups and commands, run nothing")
    parser.add_argument("--fail-fast", action="store_true", help="stop everything at the first failure")
    parser.add_argument(
        "--jobs", type=int, metavar="N", help="jobs or matrix legs to run at once (default: half the CPUs, at least 2)"
    )
    parser.add_argument("--strict", action="store_true", help="a missing tool (shellcheck, php, ...) fails instead of skipping")
    parser.add_argument("--base", metavar="REF", help="base ref for the changelog group (CI: the PR base; default origin/main)")
    arguments = parser.parse_args(argv)
    if arguments.jobs is not None and arguments.jobs < 1:
        parser.error("--jobs must be at least 1")
    unknown = [name for name in arguments.group or () if name not in names]
    if unknown:
        parser.error(f"unknown group {', '.join(unknown)}; choose from: {', '.join(names)}")
    return arguments


def main(
    argv: Optional[Sequence[str]] = None,
    groups: Optional[Sequence[Group]] = None,
    root: Path = ROOT,
    out: Optional[TextIO] = None,
) -> int:
    out = out if out is not None else sys.stdout
    available = list(groups) if groups is not None else build_groups(root)
    arguments = parse_args(argv, [group.name for group in available])
    if groups is None and arguments.base:
        available = build_groups(root, base_ref=arguments.base)
    selected = [group for group in available if not arguments.group or group.name in arguments.group]
    runner = Runner(root=root, jobs=arguments.jobs, fail_fast=arguments.fail_fast, strict=arguments.strict, out=out)
    if arguments.list:
        print_listing(selected, runner, out)
        return 0
    if os.name == "nt":
        print("check.py runs on Linux and macOS for now; use --list to see the groups.", file=sys.stderr)
        return 2
    report = runner.run(selected)
    print_failures(report, out)
    print_summary(selected, report, out)
    return report.exit_code


if __name__ == "__main__":
    sys.exit(main())
