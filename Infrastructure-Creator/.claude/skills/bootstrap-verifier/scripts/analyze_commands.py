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


_SHELL_COMPOSITION = re.compile(r"(?:&&|\|\||[;&|<>`]|\$\(|[\r\n])")
_ENV_ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=.*$")
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
        categories = {
            finding.category
            for finding in accumulator.findings
            if finding.category in {risk.value for risk in Risk}
        }
        if not categories:
            categories.add(Risk.NON_MUTATING.value)
        ordered = tuple(risk.value for risk in Risk if risk.value in categories)
        unsafe_codes = {
            "SHELL_COMPOSITION",
            "TOKENIZE_ERROR",
            "UNKNOWN_ALIAS",
            "ALIAS_CYCLE",
            "SHELL_INTERPRETER",
        }
        verification_safe = (
            categories == {Risk.NON_MUTATING.value}
            and not any(
                finding.code in unsafe_codes
                for finding in accumulator.findings
            )
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
            key = ("composer", subcommand)
            return key, tokens[index + 1 :], (
                key in self._scripts or subcommand not in _COMPOSER_BUILTINS
            )

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
                key = ("package", subcommand)
                if key in self._scripts:
                    return key, tokens[index + 1 :], True
        return None

    def _classify_leaf(
        self, tokens: list[str], command: str, accumulator: _Accumulator
    ) -> None:
        working = list(tokens)
        while working and _ENV_ASSIGNMENT.match(working[0]):
            working.pop(0)
        if working and working[0] == "env":
            working.pop(0)
            while working and (
                working[0].startswith("-") or _ENV_ASSIGNMENT.match(working[0])
            ):
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

        executable = Path(working[0]).name.lower()
        lowered = [token.lower() for token in working[1:]]
        token_set = set(lowered)

        if executable in {"bash", "dash", "fish", "sh", "zsh"} and (
            "-c" in token_set or "--command" in token_set
        ):
            accumulator.findings.append(
                Finding(
                    "SHELL_INTERPRETER",
                    "verification_blocker",
                    f"{command!r} delegates interpretation to a shell",
                )
            )

        if token_set & _MUTATING_FLAGS:
            self._add(
                accumulator,
                "WORKSPACE_WRITE_FLAG",
                Risk.WORKSPACE_MUTATION,
                f"{command!r} uses a write/fix flag",
            )

        formatter = executable in {
            "pint",
            "php-cs-fixer",
            "prettier",
            "black",
            "ruff",
            "gofmt",
        }
        if formatter and not token_set.intersection(_SAFE_FORMAT_FLAGS):
            if executable != "prettier" or "--write" in token_set:
                self._add(
                    accumulator,
                    "FORMAT_WRITES",
                    Risk.WORKSPACE_MUTATION,
                    f"{executable} defaults to writing unless a "
                    "check/dry-run mode is used",
                )

        if executable == "git" and lowered:
            action = lowered[0]
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
        artisan_action = (
            lowered[1]
            if executable == "php"
            and len(lowered) >= 2
            and Path(lowered[0]).name == "artisan"
            else ""
        )
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
        artisan_database_action = (
            artisan_action in database_actions
            or (
                "migrat" in artisan_action
                and not artisan_action.endswith(":status")
            )
        )
        database_or_deploy = (
            executable in {"deploy", "cap", "deployer"}
            or database_action
            or artisan_database_action
            or artisan_action in {"deploy", "release", "rollback"}
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
        if artisan_action and any(
            term in artisan_action for term in provider_terms
        ) and any(action in artisan_action for action in provider_actions):
            self._add(
                accumulator,
                "PROVIDER_OPERATION",
                Risk.EXTERNAL_PROVIDER_NETWORK,
                f"artisan action {artisan_action!r} can invoke a provider",
            )

        if executable in _PACKAGE_MANAGERS and lowered:
            action = lowered[0]
            if action in {
                "install",
                "update",
                "require",
                "remove",
                "add",
                "publish",
                "create",
                "create-project",
                "self-update",
            }:
                self._add(
                    accumulator,
                    "DEPENDENCY_WRITE",
                    Risk.WORKSPACE_MUTATION,
                    f"{executable} {action} can change dependencies or generated files",
                )
                self._add(
                    accumulator,
                    "DEPENDENCY_NETWORK",
                    Risk.EXTERNAL_PROVIDER_NETWORK,
                    f"{executable} {action} can access a package registry",
                )

        if executable in {"rm", "rmdir", "shred", "unlink"}:
            self._add(
                accumulator,
                "DESTRUCTIVE_FILESYSTEM",
                Risk.DESTRUCTIVE_DATABASE_DEPLOY,
                f"{executable} removes workspace data",
            )
        elif executable in {"cp", "install", "ln", "mkdir", "mv", "touch"}:
            self._add(
                accumulator,
                "FILESYSTEM_MUTATION",
                Risk.WORKSPACE_MUTATION,
                f"{executable} can write workspace data",
            )

    @staticmethod
    def _add(
        accumulator: _Accumulator, code: str, risk: Risk, message: str
    ) -> None:
        finding = Finding(code, risk.value, message)
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
