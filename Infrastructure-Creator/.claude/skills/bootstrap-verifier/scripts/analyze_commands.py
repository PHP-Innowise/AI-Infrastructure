#!/usr/bin/env python3
"""Statically classify target project commands without executing them.

The public API is ``CommandAnalyzer``, ``analyze_command()``, and
``analyze_target()``. Results are immutable dataclasses with ``to_dict()``
helpers so callers can use the module directly or consume deterministic JSON
from the CLI.
"""

from __future__ import annotations

import argparse
import json
import re
import shlex
import sys
from dataclasses import asdict, dataclass
from enum import Enum
from pathlib import Path
from typing import Iterable


class CommandAnalysisError(ValueError):
    """Raised when target command metadata cannot be analyzed safely."""


class Risk(str, Enum):
    """Risk categories emitted by the analyzer."""

    NON_MUTATING = "non_mutating"
    WORKSPACE_MUTATION = "workspace_mutation"
    DESTRUCTIVE_DATABASE_DEPLOY = "destructive_database_deploy"
    EXTERNAL_PROVIDER_NETWORK = "external_provider_network"


@dataclass(frozen=True)
class Finding:
    code: str
    category: str
    message: str


@dataclass(frozen=True)
class CommandAnalysis:
    command: str
    categories: tuple[str, ...]
    findings: tuple[Finding, ...]
    expanded_commands: tuple[str, ...]
    aliases: tuple[str, ...]
    verification_safe: bool

    def to_dict(self) -> dict:
        return asdict(self)


@dataclass(frozen=True)
class TargetAnalysis:
    target: str
    scripts: dict[str, CommandAnalysis]
    commands: tuple[CommandAnalysis, ...]

    def to_dict(self) -> dict:
        return {
            "target": self.target,
            "scripts": {
                name: analysis.to_dict()
                for name, analysis in sorted(self.scripts.items())
            },
            "commands": [analysis.to_dict() for analysis in self.commands],
        }


@dataclass
class _Accumulator:
    findings: list[Finding]
    expanded: list[str]
    aliases: list[str]
    walk_count: int = 0


_SHELL_COMPOSITION = re.compile(r"(?:&&|\|\||[;&|<>`]|\$\(|[\r\n])")
_ENV_ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=.*$")
_SHORT_OPTION_CLUSTER = re.compile(r"-[A-Za-z]+")
_MUTATING_FLAGS = {
    "--fix",
    "--fix-dry-run=false",
    "--write",
    "-w",
    "--apply",
    "--update",
}
_SAFE_FORMAT_FLAGS = {"--check", "--dry-run", "--test", "--diff"}
_PACKAGE_MANAGERS = {"npm", "pnpm", "yarn", "bun", "composer"}
_PACKAGE_RUNNERS = {"bunx", "npx"}
_SHELL_INTERPRETERS = {"bash", "dash", "fish", "sh", "zsh"}
# Flags that make a shell print a report and exit instead of starting an
# interactive session reading commands from stdin.
_SHELL_REPORT_FLAGS = frozenset({"--help", "--version"})
# General-purpose interpreters execute arbitrary code. Report/lint modes
# (php -v, php -l), python -m with a re-classified module, and script
# arguments whose basename is itself a known executable (php artisan,
# php vendor/bin/phpunit) stay classified; inline code flags, unknown
# script files, and stdin/REPL sessions are verification blockers.
_GENERAL_INTERPRETERS = {"node", "php", "python", "python3"}
_INTERPRETER_INLINE_FLAGS = {
    "node": frozenset(
        {
            "--eval",
            "--interactive",
            "--print",
            "--require",
            "-e",
            "-i",
            "-p",
            "-r",
        }
    ),
    "php": frozenset({"-F", "-R", "-a", "-f", "-r"}),
    "python": frozenset({"-c", "-i"}),
    "python3": frozenset({"-c", "-i"}),
}
_INTERPRETER_INLINE_LETTERS = {
    "node": frozenset("eipr"),
    "php": frozenset("FRafr"),
    "python": frozenset("ci"),
    "python3": frozenset("ci"),
}
# Interpreter options that consume the following token as a value without
# executing code, so the value is never mistaken for a script argument.
_INTERPRETER_VALUE_FLAGS = {
    "node": frozenset({"--conditions"}),
    "php": frozenset({"-c", "-d"}),
    "python": frozenset({"-W", "-X"}),
    "python3": frozenset({"-W", "-X"}),
}
# Flags that make an interpreter print a report (or run its built-in test
# runner) and exit instead of reading code from stdin.
_INTERPRETER_REPORT_FLAGS = {
    "node": frozenset({"--help", "--test", "--version", "-h", "-v"}),
    "php": frozenset(
        {"--help", "--ini", "--modules", "--version", "-h", "-i", "-m", "-v"}
    ),
    "python": frozenset({"--help", "--version", "-V", "-h"}),
    "python3": frozenset({"--help", "--version", "-V", "-h"}),
}
_PHP_LINT_FLAGS = frozenset({"--syntax-check", "-l"})
_FORMATTER_EXECUTABLES = {"black", "php-cs-fixer", "pint", "prettier", "ruff"}
_DESTRUCTIVE_FILESYSTEM_COMMANDS = {"rm", "rmdir", "shred", "unlink"}
_FILESYSTEM_MUTATION_COMMANDS = {"cp", "install", "ln", "mkdir", "mv", "touch"}
_MAX_ALIAS_EXPANSIONS = 512
# Wrapper executables that run another command; each maps to the set of its
# options that consume a following value token. Wrappers are unwrapped by
# basename so absolute forms such as /usr/bin/env are treated identically.
_WRAPPER_VALUE_FLAGS = {
    "env": frozenset({"-C", "-S", "-u", "--chdir", "--split-string", "--unset"}),
    "nice": frozenset({"-n", "--adjustment"}),
    "nohup": frozenset(),
    "stdbuf": frozenset({"-e", "-i", "-o", "--error", "--input", "--output"}),
    "sudo": frozenset({"-g", "--group", "-p", "--prompt", "-u", "--user"}),
    "timeout": frozenset({"-k", "--kill-after", "-s", "--signal"}),
}
_GIT_VALUE_OPTIONS = {"-C", "-c", "--git-dir", "--work-tree"}
_DEPENDENCY_ACTIONS = {
    "add",
    "create",
    "create-project",
    "install",
    "publish",
    "remove",
    "require",
    "self-update",
    "update",
}
_NODE_DEPENDENCY_SHORTHANDS = {"ci", "i", "up", "upgrade"}
_NODE_EXECUTE_ACTIONS = {"dlx", "exec", "x"}
# Builtin subcommands of pnpm/yarn/bun that never resolve to a package.json
# script when invoked bare; anything else bare is treated as a script alias
# and fails closed when unknown.
_NODE_PACKAGE_BUILTINS = {
    "add",
    "audit",
    "bin",
    "cache",
    "ci",
    "config",
    "create",
    "dedupe",
    "dlx",
    "doctor",
    "exec",
    "fetch",
    "global",
    "help",
    "i",
    "import",
    "info",
    "init",
    "install",
    "licenses",
    "link",
    "list",
    "ls",
    "node",
    "outdated",
    "pack",
    "patch",
    "patch-commit",
    "prune",
    "publish",
    "rebuild",
    "remove",
    "rm",
    "run",
    "run-script",
    "setup",
    "store",
    "un",
    "uninstall",
    "unlink",
    "up",
    "update",
    "upgrade",
    "version",
    "versions",
    "why",
    "workspace",
    "workspaces",
    "x",
}
# Curated allow-list of executables known to be read-only (or read-only unless
# an explicitly classified flag/action is present). Anything not derivable
# from a classification set below fails closed as UNKNOWN_EXECUTABLE.
_KNOWN_READONLY_EXECUTABLES = {
    "@putenv",
    "ava",
    "behat",
    "cat",
    "date",
    "deptrac",
    "diff",
    "echo",
    "eslint",
    "false",
    "find",
    "flake8",
    "gofmt",
    "grep",
    "head",
    "jest",
    "ls",
    "mocha",
    "mypy",
    "node",
    "parallel-lint",
    "paratest",
    "pest",
    "php",
    "phpcs",
    "phploc",
    "phpmd",
    "phpspec",
    "phpstan",
    "phpunit",
    "printf",
    "psalm",
    "pwd",
    "pylint",
    "pytest",
    "python",
    "python3",
    "sort",
    "stylelint",
    "tail",
    "test",
    "true",
    "tsc",
    "uniq",
    "unittest",
    "vitest",
    "wc",
    "which",
}
_PROVIDER_COMMANDS = {
    "aws",
    "az",
    "gcloud",
    "heroku",
    "stripe",
    "shopify",
    "firebase",
    "gh",
    "vercel",
    "netlify",
    "openai",
    "kubectl",
    "helm",
    "terraform",
    "tofu",
    "pulumi",
}
_NETWORK_COMMANDS = {
    "curl",
    "wget",
    "ssh",
    "scp",
    "sftp",
    "rsync",
    "nc",
    "ncat",
    "telnet",
}
_COMPOSER_BUILTINS = {
    "about",
    "archive",
    "audit",
    "browse",
    "check-platform-reqs",
    "clear-cache",
    "config",
    "create-project",
    "depends",
    "diagnose",
    "dump-autoload",
    "exec",
    "fund",
    "global",
    "help",
    "init",
    "install",
    "licenses",
    "list",
    "outdated",
    "prohibits",
    "reinstall",
    "remove",
    "repository",
    "require",
    "run-script",
    "search",
    "self-update",
    "show",
    "status",
    "suggests",
    "update",
    "validate",
}
# Every executable the classifier understands. Executables outside this union
# are verification blockers (UNKNOWN_EXECUTABLE) because their effects cannot
# be attested statically.
_KNOWN_EXECUTABLES = frozenset(
    _KNOWN_READONLY_EXECUTABLES
    | _FORMATTER_EXECUTABLES
    | _SHELL_INTERPRETERS
    | _PACKAGE_MANAGERS
    | _PACKAGE_RUNNERS
    | _NETWORK_COMMANDS
    | _PROVIDER_COMMANDS
    | _DESTRUCTIVE_FILESYSTEM_COMMANDS
    | _FILESYSTEM_MUTATION_COMMANDS
    | {
        "artisan",
        "cap",
        "console",
        "deploy",
        "deployer",
        "git",
        "mysql",
        "psql",
        "symfony",
    }
)
# Public projection of every executable name this module recognises, including
# the wrapper executables that are unwrapped before classification. Callers
# that must decide whether a token found in prose is plausibly a command at
# all (rather than a class name, a path, or a constant) gate on this set.
RUNNER_EXECUTABLES = frozenset(_KNOWN_EXECUTABLES | set(_WRAPPER_VALUE_FLAGS))
# Public: executables that run code handed to them as an argument. A caller
# scanning prose re-reads their arguments as a nested command so a payload
# quoted behind `bash -c` or `php -r` is classified rather than hidden.
CODE_HOST_EXECUTABLES = frozenset(_SHELL_INTERPRETERS | _GENERAL_INTERPRETERS)


def _load_json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise CommandAnalysisError(f"cannot read {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise CommandAnalysisError(f"invalid JSON in {path}: {error}") from error
    if not isinstance(value, dict):
        raise CommandAnalysisError(f"{path} must contain a JSON object")
    return value


def _normalise_script_commands(
    source: Path, ecosystem: str, name: str, value: object
) -> tuple[str, ...]:
    if isinstance(value, str):
        return (value,)
    if ecosystem == "composer" and isinstance(value, list):
        if all(isinstance(item, str) for item in value):
            return tuple(value)
    expected = "a string or string list" if ecosystem == "composer" else "a string"
    raise CommandAnalysisError(
        f"{source}: script {name!r} must be {expected}, got "
        f"{type(value).__name__}"
    )


class CommandAnalyzer:
    """Read script metadata from a target and classify commands statically."""

    def __init__(self, target: str | Path):
        self.target = Path(target).expanduser().resolve()
        if not self.target.is_dir():
            raise CommandAnalysisError(f"target is not a directory: {self.target}")
        self._scripts: dict[tuple[str, str], tuple[str, ...]] = {}
        self._load_scripts()

    @property
    def scripts(self) -> dict[str, tuple[str, ...]]:
        """Return script bodies keyed as ``composer:name`` or ``package:name``."""
        return {
            f"{ecosystem}:{name}": commands
            for (ecosystem, name), commands in sorted(self._scripts.items())
        }

    def _load_scripts(self) -> None:
        for filename, ecosystem in (
            ("composer.json", "composer"),
            ("package.json", "package"),
        ):
            path = self.target / filename
            if not path.is_file():
                continue
            document = _load_json(path)
            scripts = document.get("scripts", {})
            if not isinstance(scripts, dict):
                raise CommandAnalysisError(f"{path}: 'scripts' must be an object")
            for name, value in scripts.items():
                if not isinstance(name, str) or not name:
                    raise CommandAnalysisError(
                        f"{path}: script names must be non-empty strings"
                    )
                self._scripts[(ecosystem, name)] = _normalise_script_commands(
                    path, ecosystem, name, value
                )

    def analyze(
        self,
        command: str,
        *,
        verification: bool = False,
        ecosystem: str | None = None,
    ) -> CommandAnalysis:
        """Analyze one command and optionally enforce verification safety.

        ``ecosystem`` is needed only for Composer's internal ``@alias`` form.
        The method never executes the command.
        """
        if not isinstance(command, str) or not command.strip():
            raise CommandAnalysisError("command must be a non-empty string")
        if ecosystem not in (None, "composer", "package"):
            raise CommandAnalysisError(f"unknown ecosystem: {ecosystem}")
        accumulator = _Accumulator([], [], [])
        self._walk(command, ecosystem, (), accumulator)
        risk_values = {risk.value for risk in Risk}
        categories = {
            finding.category
            for finding in accumulator.findings
            if finding.category in risk_values
        }
        blocked = any(
            finding.category == "verification_blocker"
            for finding in accumulator.findings
        )
        if not categories and not blocked:
            categories.add(Risk.NON_MUTATING.value)
        ordered = tuple(risk.value for risk in Risk if risk.value in categories)
        if blocked:
            ordered = (*ordered, "verification_blocker")
        verification_safe = (
            not blocked and categories == {Risk.NON_MUTATING.value}
        )
        if verification and not verification_safe:
            accumulator.findings.append(
                Finding(
                    "UNSAFE_VERIFICATION",
                    "verification_blocker",
                    "verification commands must be non-mutating, local, "
                    "composition-free, and fully resolved",
                )
            )
        return CommandAnalysis(
            command=command,
            categories=ordered,
            findings=tuple(accumulator.findings),
            expanded_commands=tuple(accumulator.expanded),
            aliases=tuple(accumulator.aliases),
            verification_safe=verification_safe,
        )

    def analyze_scripts(
        self, *, verification: bool = False
    ) -> dict[str, CommandAnalysis]:
        """Analyze every target script alias with transitive expansion."""
        results: dict[str, CommandAnalysis] = {}
        for ecosystem, name in sorted(self._scripts):
            invocation = (
                f"composer run-script {shlex.quote(name)}"
                if ecosystem == "composer"
                else f"npm run {shlex.quote(name)}"
            )
            results[f"{ecosystem}:{name}"] = self.analyze(
                invocation, verification=verification
            )
        return results

    def _walk(
        self,
        command: str,
        ecosystem: str | None,
        stack: tuple[tuple[str, str], ...],
        accumulator: _Accumulator,
    ) -> None:
        if accumulator.walk_count >= _MAX_ALIAS_EXPANSIONS:
            self._add_blocker(
                accumulator,
                "EXPANSION_LIMIT",
                f"alias expansion exceeded {_MAX_ALIAS_EXPANSIONS} commands; "
                "refusing to analyze further",
            )
            return
        accumulator.walk_count += 1
        composition = _SHELL_COMPOSITION.search(command)
        if composition:
            accumulator.findings.append(
                Finding(
                    "SHELL_COMPOSITION",
                    "verification_blocker",
                    f"shell composition token {composition.group(0)!r} "
                    "cannot be analyzed safely",
                )
            )
            return
        try:
            tokens = shlex.split(command, posix=True)
        except ValueError as error:
            accumulator.findings.append(
                Finding("TOKENIZE_ERROR", "verification_blocker", str(error))
            )
            return
        if not tokens:
            accumulator.findings.append(
                Finding("TOKENIZE_ERROR", "verification_blocker", "empty command")
            )
            return

        alias = self._resolve_alias(tokens, ecosystem)
        if alias is not None:
            alias_key, trailing, explicit_alias = alias
            alias_label = f"{alias_key[0]}:{alias_key[1]}"
            accumulator.aliases.append(alias_label)
            if alias_key in stack:
                chain = " -> ".join(
                    f"{kind}:{name}" for kind, name in (*stack, alias_key)
                )
                accumulator.findings.append(
                    Finding(
                        "ALIAS_CYCLE",
                        "verification_blocker",
                        f"script alias cycle detected: {chain}",
                    )
                )
                return
            bodies = self._scripts.get(alias_key)
            if bodies is None:
                if explicit_alias:
                    accumulator.findings.append(
                        Finding(
                            "UNKNOWN_ALIAS",
                            "verification_blocker",
                            f"script alias {alias_label!r} is not defined",
                        )
                    )
                    return
            else:
                if alias_key[0] == "package":
                    pre_alias = ("package", f"pre{alias_key[1]}")
                    if pre_alias in self._scripts:
                        self._walk(
                            f"npm run {shlex.quote(pre_alias[1])}",
                            "package",
                            (*stack, alias_key),
                            accumulator,
                        )
                for body in bodies:
                    expanded = body
                    if trailing:
                        expanded = f"{expanded} {shlex.join(trailing)}"
                    self._walk(
                        expanded,
                        alias_key[0],
                        (*stack, alias_key),
                        accumulator,
                    )
                if alias_key[0] == "package":
                    post_alias = ("package", f"post{alias_key[1]}")
                    if post_alias in self._scripts:
                        self._walk(
                            f"npm run {shlex.quote(post_alias[1])}",
                            "package",
                            (*stack, alias_key),
                            accumulator,
                        )
                return

        accumulator.expanded.append(command)
        self._classify_leaf(tokens, command, accumulator)

    def _resolve_alias(
        self, tokens: list[str], ecosystem: str | None
    ) -> tuple[tuple[str, str], list[str], bool] | None:
        first = Path(tokens[0]).name
        if first.startswith("@") and ecosystem == "composer":
            if first in {"@php", "@composer", "@putenv"}:
                return None
            return ("composer", first[1:]), tokens[1:], True

        if first == "composer" and len(tokens) >= 2:
            index = 1
            while index < len(tokens) and tokens[index].startswith("-"):
                index += 1
            if index >= len(tokens):
                return None
            subcommand = tokens[index]
            if subcommand in {"run", "run-script"}:
                alias_index = index + 1
                while (
                    alias_index < len(tokens)
                    and tokens[alias_index].startswith("-")
                ):
                    alias_index += 1
                if alias_index >= len(tokens):
                    return ("composer", ""), [], True
                return (
                    ("composer", tokens[alias_index]),
                    tokens[alias_index + 1 :],
                    True,
                )
            if subcommand in _COMPOSER_BUILTINS:
                # Composer always runs the builtin for a direct invocation;
                # name-colliding scripts are reachable only via run-script.
                return None
            return ("composer", subcommand), tokens[index + 1 :], True

        if first in {"npm", "pnpm", "yarn", "bun"} and len(tokens) >= 2:
            index = 1
            while index < len(tokens) and tokens[index].startswith("-"):
                index += 1
            if index >= len(tokens):
                return None
            subcommand = tokens[index]
            if subcommand in {"run", "run-script"}:
                alias_index = index + 1
                while (
                    alias_index < len(tokens)
                    and tokens[alias_index].startswith("-")
                ):
                    alias_index += 1
                if alias_index >= len(tokens):
                    return ("package", ""), [], True
                return (
                    ("package", tokens[alias_index]),
                    tokens[alias_index + 1 :],
                    True,
                )
            if subcommand in {"test", "start", "stop", "restart"}:
                return ("package", subcommand), tokens[index + 1 :], True
            if first in {"pnpm", "yarn", "bun"}:
                if subcommand in _NODE_PACKAGE_BUILTINS:
                    return None
                # Bare invocations resolve package.json scripts; unknown
                # scripts fail closed as UNKNOWN_ALIAS like `npm run`.
                return ("package", subcommand), tokens[index + 1 :], True
        return None

    def _classify_leaf(
        self, tokens: list[str], command: str, accumulator: _Accumulator
    ) -> None:
        working = list(tokens)
        while working:
            if _ENV_ASSIGNMENT.match(working[0]):
                working.pop(0)
                continue
            wrapper = Path(working[0]).name.lower()
            if wrapper not in _WRAPPER_VALUE_FLAGS:
                break
            if wrapper == "sudo":
                self._add_blocker(
                    accumulator,
                    "SUDO_EXECUTION",
                    f"{command!r} escalates privileges with sudo",
                )
            value_flags = _WRAPPER_VALUE_FLAGS[wrapper]
            working.pop(0)
            while working and working[0].startswith("-"):
                flag = working.pop(0)
                if flag in value_flags and working:
                    working.pop(0)
            if wrapper == "timeout" and working:
                working.pop(0)
        if working and working[0] == "@php":
            working[0] = "php"
        if working and working[0] == "@composer":
            working[0] = "composer"
        if not working:
            accumulator.findings.append(
                Finding(
                    "TOKENIZE_ERROR",
                    "verification_blocker",
                    "command contains no executable after environment assignments",
                )
            )
            return
        if working[0].startswith("$"):
            self._add_blocker(
                accumulator,
                "COMMAND_INDIRECTION",
                f"{command!r} resolves its executable from a shell variable",
            )
            return

        executable = Path(working[0]).name.lower()
        if executable == "xargs":
            self._add_blocker(
                accumulator,
                "COMMAND_INDIRECTION",
                f"{command!r} builds its final command through xargs",
            )
            return
        arguments = working[1:]
        lowered = [token.lower() for token in arguments]
        token_set = set(arguments)

        if executable not in _KNOWN_EXECUTABLES:
            self._add_blocker(
                accumulator,
                "UNKNOWN_EXECUTABLE",
                f"executable {executable!r} is not a known read-only tool",
            )

        if executable in _GENERAL_INTERPRETERS and self._classify_interpreter(
            executable, arguments, command, accumulator
        ):
            return

        if executable in _SHELL_INTERPRETERS:
            delegates = "--command" in token_set or any(
                "c" in flag
                for flag in arguments
                if _SHORT_OPTION_CLUSTER.fullmatch(flag)
            )
            script_argument = next(
                (token for token in arguments if not token.startswith("-")), ""
            )
            if delegates:
                accumulator.findings.append(
                    Finding(
                        "SHELL_INTERPRETER",
                        "verification_blocker",
                        f"{command!r} delegates interpretation to a shell",
                    )
                )
            elif script_argument:
                accumulator.findings.append(
                    Finding(
                        "SHELL_INTERPRETER",
                        "verification_blocker",
                        f"{command!r} runs unanalyzed shell script "
                        f"{script_argument!r}",
                    )
                )
            elif not token_set & _SHELL_REPORT_FLAGS:
                accumulator.findings.append(
                    Finding(
                        "SHELL_INTERPRETER",
                        "verification_blocker",
                        f"{command!r} starts an interactive shell reading "
                        "commands from stdin",
                    )
                )

        if token_set & _MUTATING_FLAGS:
            self._add(
                accumulator,
                "WORKSPACE_WRITE_FLAG",
                Risk.WORKSPACE_MUTATION,
                f"{command!r} uses a write/fix flag",
            )

        first_action = next(
            (token for token in lowered if not token.startswith("-")), ""
        )
        formatter = executable in _FORMATTER_EXECUTABLES
        if (
            formatter
            and first_action != "check"
            and not token_set.intersection(_SAFE_FORMAT_FLAGS)
        ):
            if executable != "prettier" or "--write" in token_set:
                self._add(
                    accumulator,
                    "FORMAT_WRITES",
                    Risk.WORKSPACE_MUTATION,
                    f"{executable} defaults to writing unless a "
                    "check/dry-run mode is used",
                )

        if executable == "git" and lowered:
            index = 0
            while index < len(arguments) and arguments[index].startswith("-"):
                if arguments[index] in _GIT_VALUE_OPTIONS:
                    index += 2
                else:
                    index += 1
            action = (
                arguments[index].lower() if index < len(arguments) else ""
            )
            if action in {"reset", "clean"}:
                self._add(
                    accumulator,
                    "DESTRUCTIVE_GIT",
                    Risk.DESTRUCTIVE_DATABASE_DEPLOY,
                    f"git {action} can destroy workspace state",
                )
            elif action in {
                "add",
                "commit",
                "checkout",
                "switch",
                "merge",
                "rebase",
                "restore",
                "stash",
                "tag",
            }:
                self._add(
                    accumulator,
                    "GIT_MUTATION",
                    Risk.WORKSPACE_MUTATION,
                    f"git {action} mutates repository state",
                )
            if action in {"clone", "fetch", "pull", "push", "submodule"}:
                self._add(
                    accumulator,
                    "GIT_NETWORK",
                    Risk.EXTERNAL_PROVIDER_NETWORK,
                    f"git {action} can access a remote",
                )

        action = lowered[0] if lowered else ""
        console_action = ""
        if (
            executable == "php"
            and len(lowered) >= 2
            and Path(lowered[0]).name in {"artisan", "console"}
        ):
            console_action = lowered[1]
        elif executable in {"artisan", "console"} and lowered:
            console_action = lowered[0]
        elif (
            executable == "symfony"
            and len(lowered) >= 2
            and lowered[0] == "console"
        ):
            console_action = lowered[1]
        database_actions = {
            "db:wipe",
            "db:seed",
            "schema:drop",
            "schema:update",
            "database:drop",
        }
        provider_mutations = {
            "terraform": {"apply", "destroy", "import"},
            "tofu": {"apply", "destroy", "import"},
            "kubectl": {"apply", "create", "delete", "patch", "replace", "scale"},
            "helm": {"install", "rollback", "uninstall", "upgrade"},
            "serverless": {"deploy", "remove", "rollback"},
            "sam": {"deploy"},
            "docker": {"push"},
        }
        database_action = (
            action in database_actions
            or (
                "migrat" in action
                and not action.endswith(":status")
                and action != "migrations:status"
            )
        )
        console_database_action = (
            any(term in console_action for term in database_actions)
            or (
                "migrat" in console_action
                and not console_action.endswith(":status")
            )
        )
        database_or_deploy = (
            executable in {"deploy", "cap", "deployer"}
            or database_action
            or console_database_action
            or console_action in {"deploy", "release", "rollback"}
            or action in provider_mutations.get(executable, set())
            or (executable in {"mysql", "psql"} and bool(lowered))
        )
        if database_or_deploy:
            self._add(
                accumulator,
                "DESTRUCTIVE_DATABASE_DEPLOY",
                Risk.DESTRUCTIVE_DATABASE_DEPLOY,
                f"{command!r} can mutate database, infrastructure, or deployment state",
            )

        if executable in _NETWORK_COMMANDS or executable in _PROVIDER_COMMANDS:
            self._add(
                accumulator,
                "EXTERNAL_OR_PROVIDER",
                Risk.EXTERNAL_PROVIDER_NETWORK,
                f"{executable} may contact an external service or provider",
            )
        provider_terms = (
            "aws",
            "s3",
            "stripe",
            "shopify",
            "firebase",
            "openai",
            "meilisearch",
            "storyblok",
            "akeneo",
        )
        provider_actions = ("sync", "send", "publish", "upload", "download")
        if console_action and any(
            term in console_action for term in provider_terms
        ) and any(action in console_action for action in provider_actions):
            self._add(
                accumulator,
                "PROVIDER_OPERATION",
                Risk.EXTERNAL_PROVIDER_NETWORK,
                f"console action {console_action!r} can invoke a provider",
            )

        if executable in _PACKAGE_MANAGERS:
            positional = [
                token for token in lowered if not token.startswith("-")
            ]
            pm_action = positional[0] if positional else ""
            if pm_action == "global" and len(positional) >= 2:
                pm_action = positional[1]
            dependency_actions = set(_DEPENDENCY_ACTIONS)
            execute_actions = set()
            if executable != "composer":
                dependency_actions |= _NODE_DEPENDENCY_SHORTHANDS
                execute_actions = _NODE_EXECUTE_ACTIONS
            if pm_action in dependency_actions:
                self._add(
                    accumulator,
                    "DEPENDENCY_WRITE",
                    Risk.WORKSPACE_MUTATION,
                    f"{executable} {pm_action} can change dependencies "
                    "or generated files",
                )
                self._add(
                    accumulator,
                    "DEPENDENCY_NETWORK",
                    Risk.EXTERNAL_PROVIDER_NETWORK,
                    f"{executable} {pm_action} can access a package registry",
                )
            if pm_action in execute_actions:
                self._add(
                    accumulator,
                    "DEPENDENCY_EXECUTE",
                    Risk.WORKSPACE_MUTATION,
                    f"{executable} {pm_action} can download and execute "
                    "package code",
                )
                self._add(
                    accumulator,
                    "DEPENDENCY_NETWORK",
                    Risk.EXTERNAL_PROVIDER_NETWORK,
                    f"{executable} {pm_action} can access a package registry",
                )

        if executable in _PACKAGE_RUNNERS:
            self._add(
                accumulator,
                "DEPENDENCY_EXECUTE",
                Risk.WORKSPACE_MUTATION,
                f"{executable} can download and execute package code",
            )
            self._add(
                accumulator,
                "DEPENDENCY_NETWORK",
                Risk.EXTERNAL_PROVIDER_NETWORK,
                f"{executable} can access a package registry",
            )

        if executable in _DESTRUCTIVE_FILESYSTEM_COMMANDS:
            self._add(
                accumulator,
                "DESTRUCTIVE_FILESYSTEM",
                Risk.DESTRUCTIVE_DATABASE_DEPLOY,
                f"{executable} removes workspace data",
            )
        elif executable in _FILESYSTEM_MUTATION_COMMANDS:
            self._add(
                accumulator,
                "FILESYSTEM_MUTATION",
                Risk.WORKSPACE_MUTATION,
                f"{executable} can write workspace data",
            )

    def _classify_interpreter(
        self,
        executable: str,
        arguments: list[str],
        command: str,
        accumulator: _Accumulator,
    ) -> bool:
        """Classify a general-purpose interpreter invocation.

        Returns True when the invocation was fully handled here (blocked,
        or re-dispatched to classify the known tool it runs); False when
        the caller should continue classifying the interpreter itself
        (report and lint modes such as ``php -v`` or ``php -l``).
        """
        if executable == "php" and _PHP_LINT_FLAGS.intersection(arguments):
            return False
        inline_flags = _INTERPRETER_INLINE_FLAGS[executable]
        inline_letters = _INTERPRETER_INLINE_LETTERS[executable]
        value_flags = _INTERPRETER_VALUE_FLAGS[executable]
        index = 0
        while index < len(arguments):
            token = arguments[index]
            if not token.startswith("-"):
                if Path(token).name.lower() in _KNOWN_EXECUTABLES:
                    self._classify_leaf(
                        arguments[index:], command, accumulator
                    )
                else:
                    self._add_blocker(
                        accumulator,
                        "INTERPRETER_EXECUTION",
                        f"{command!r} runs unanalyzed {executable} "
                        f"script {token!r}",
                    )
                return True
            base = token.split("=", 1)[0]
            if base in inline_flags or (
                _SHORT_OPTION_CLUSTER.fullmatch(token)
                and inline_letters.intersection(token[1:])
            ):
                self._add_blocker(
                    accumulator,
                    "INTERPRETER_EXECUTION",
                    f"{command!r} passes code to {executable} "
                    f"via {token!r}",
                )
                return True
            if executable in {"python", "python3"} and base == "-m":
                if index + 1 < len(arguments):
                    self._classify_leaf(
                        arguments[index + 1 :], command, accumulator
                    )
                else:
                    self._add_blocker(
                        accumulator,
                        "INTERPRETER_EXECUTION",
                        f"{command!r} names no module to run",
                    )
                return True
            if base in value_flags and "=" not in token:
                index += 2
                continue
            index += 1
        if not set(arguments) & _INTERPRETER_REPORT_FLAGS[executable]:
            self._add_blocker(
                accumulator,
                "INTERPRETER_EXECUTION",
                f"{command!r} starts an interactive {executable} session "
                "reading code from stdin",
            )
            return True
        return False

    @staticmethod
    def _add(
        accumulator: _Accumulator, code: str, risk: Risk, message: str
    ) -> None:
        finding = Finding(code, risk.value, message)
        if finding not in accumulator.findings:
            accumulator.findings.append(finding)

    @staticmethod
    def _add_blocker(
        accumulator: _Accumulator, code: str, message: str
    ) -> None:
        finding = Finding(code, "verification_blocker", message)
        if finding not in accumulator.findings:
            accumulator.findings.append(finding)


def analyze_command(
    target: str | Path,
    command: str,
    *,
    verification: bool = False,
    ecosystem: str | None = None,
) -> CommandAnalysis:
    """Convenience API for analyzing one target command."""
    return CommandAnalyzer(target).analyze(
        command, verification=verification, ecosystem=ecosystem
    )


def analyze_target(
    target: str | Path,
    commands: Iterable[str] = (),
    *,
    verification: bool = False,
) -> TargetAnalysis:
    """Analyze all target aliases and any additional commands."""
    analyzer = CommandAnalyzer(target)
    return TargetAnalysis(
        target=str(analyzer.target),
        scripts=analyzer.analyze_scripts(verification=verification),
        commands=tuple(
            analyzer.analyze(command, verification=verification)
            for command in commands
        ),
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Statically classify composer/npm script command risk."
    )
    parser.add_argument("--target", required=True, help="target project root")
    parser.add_argument(
        "--command",
        action="append",
        default=[],
        help="additional command to analyze; repeatable",
    )
    parser.add_argument(
        "--verification",
        action="store_true",
        help="exit 1 if any analyzed script or command is unsafe for verification",
    )
    parser.add_argument(
        "--no-scripts",
        action="store_true",
        help="analyze only explicit --command values",
    )
    args = parser.parse_args(argv)
    try:
        analyzer = CommandAnalyzer(args.target)
        scripts = (
            {}
            if args.no_scripts
            else analyzer.analyze_scripts(verification=args.verification)
        )
        commands = tuple(
            analyzer.analyze(command, verification=args.verification)
            for command in args.command
        )
        result = TargetAnalysis(str(analyzer.target), scripts, commands)
    except CommandAnalysisError as error:
        print(json.dumps({"error": str(error)}, sort_keys=True))
        return 2
    print(json.dumps(result.to_dict(), indent=2, sort_keys=True))
    analyses = [*scripts.values(), *commands]
    if args.verification and any(not item.verification_safe for item in analyses):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
