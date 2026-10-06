#!/usr/bin/env python3
"""Prove that every hook-wiring and routing reference resolves.

The mirror and parity gates prove that copies agree with their canon; nothing
proved that the canon points at things that exist. A command can spawn an
agent that was renamed, a flow can name a command that only one tool has, a
hook can be wired to a script Git checks out without its executable bit - and
every other job stays green. This gate walks each edition (Laravel, Symfony,
PHP Core, WordPress) and Infrastructure-Creator and checks three things.

Hook wiring (``hook-wiring``)
    ``.claude/settings.json``, ``.cursor/hooks.json`` and ``.codex/hooks.json``
    are scanned line by line for ``(.claude|.cursor|.codex)/hooks/<name>.sh``.
    Only that tail is matched, so the command prefix (bare relative,
    ``"$CLAUDE_PROJECT_DIR"/...``, ``$(git rev-parse --show-toplevel)/...``)
    does not matter. Every referenced script must exist, be tracked in the Git
    index, and carry index mode 100755: Git records the executable bit, and a
    script checked out without it fails on every hook event. A ``*.sh`` under
    a tool's ``hooks/`` directory that no wiring file references is a
    warning, unless the allowlist gives a reason.

Routing (``routing``)
    Claude commands: ``spawns`` names an agent (frontmatter ``name``, file
    stem, or ``<name>-agent``); ``flow-next`` / ``flow-alternatives`` name a
    command or a skill (in Claude Code every skill is also ``/<skill>``);
    ``stages[].agents`` name agents. Command bodies (Claude and Cursor):
    ``subagent_type`` names an agent of the same tool, a ```/x``` code span
    names a command or skill of the same tool, a ``.<tool>/skills/<x>/`` path
    names an existing skill. Agents (Claude and Cursor) must carry a unique
    ``name``; a Claude agent's ``invokes`` and any "invoke the `x` skill"
    phrase in an agent body must name ``.agents/skills/<x>/SKILL.md``; slash
    code spans resolve as for commands. Every ``SKILL FLOW.md`` - its slash
    tokens, its backticked names and its Phase Map items - resolves in its
    own tool's namespace: Claude and Cursor accept a command or a skill, the
    Codex edition (``.agents/skills``) accepts a skill only, because Codex has
    no command layer. ``AGENTS.md`` slash code spans and its "`x` skill",
    "`x` agent", "`x` command" and "`x` hook" phrases must resolve too.

Reachability (``reachability``)
    Every skill in ``.agents/skills`` is named by some agent, command or flow
    (the references above), or is listed in the allowlist with a reason.

The allowlist (``scripts/check_routes_allowlist.json``) records deliberate
exceptions, each with a mandatory reason; an entry that no longer matches
anything is reported as a stale warning so the list cannot rot.

Usage:
    python3 scripts/check_routes.py                     # all editions
    python3 scripts/check_routes.py --edition Symfony   # repeatable
    python3 scripts/check_routes.py --json              # machine-readable

Exit status: 0 when no error is found (warnings allowed), 1 on any error,
2 on a usage or environment failure. Python 3.9+, standard library plus Git.
"""

from __future__ import annotations

import argparse
import json
import re
import shlex
import subprocess
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence, Set, Tuple

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_ALLOWLIST = REPO_ROOT / "scripts" / "check_routes_allowlist.json"

# Edition-relative paths, as policy_lock.py names them: "Cms/wordpress" is a
# path because the WordPress edition lives one level down.
EDITIONS = (
    "Laravel",
    "Symfony",
    "PHP Core",
    "Cms/wordpress",
    "Infrastructure-Creator",
)
EDITION_ALIASES = {
    "laravel": "Laravel",
    "symfony": "Symfony",
    "php core": "PHP Core",
    "php-core": "PHP Core",
    "cms/wordpress": "Cms/wordpress",
    "wordpress": "Cms/wordpress",
    "infrastructure-creator": "Infrastructure-Creator",
}

# tool -> (tool directory, wiring file relative to the edition root)
TOOLS = (
    ("claude", ".claude", ".claude/settings.json"),
    ("cursor", ".cursor", ".cursor/hooks.json"),
    ("codex", ".codex", ".codex/hooks.json"),
)
SKILL_FLOW = "SKILL FLOW.md"
EXECUTABLE_MODE = "100755"
NULL_VALUES = {"", "null", "none", "~"}

HOOK_REF = re.compile(r"(\.claude|\.cursor|\.codex)/hooks/([A-Za-z0-9._-]+\.sh)")
# A launcher that locates the hooks directory itself and execs the script
# named as its first argument - the Codex form:
# sh -c '... exec "$d/.codex/hooks/$1"' sh local-context.sh
HOOK_LAUNCHER = re.compile(r"(\.claude|\.cursor|\.codex)/hooks/\$(?:1|\{1\})")
HOOK_SCRIPT_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]*\.sh")
COMMAND_VALUE = re.compile(r'"command"\s*:\s*("(?:[^"\\]|\\.)*")')
NAME = r"[a-z][a-z0-9]*(?:-[a-z0-9]+)*"
NAME_RE = re.compile(rf"^{NAME}$")
FRONTMATTER_KEY = re.compile(r"^([A-Za-z0-9_-]+):\s*(.*)$")
STAGE_AGENTS = re.compile(r"agents:\s*\[([^\]]*)\]")
CODE_SPAN = re.compile(r"`([^`\n]+)`")
# A code span that is a slash invocation: `/name`, `/name args`, `/name [ctx]`.
SLASH_SPAN = re.compile(rf"^/({NAME})(?:[\s\[].*)?$")
# A bare slash token in SKILL FLOW.md, where the main flow is an unquoted
# diagram. The boundaries keep paths (`specs/x`, `<edition>/hooks`, `/tmp/x`)
# and file names (`/x.md`) out.
SLASH_TOKEN = re.compile(
    rf"(?:(?<=^)|(?<=[\s`(\[\"',]))/({NAME})(?![A-Za-z0-9_/{{<>-])(?!\.[A-Za-z0-9])"
)
# A backticked bare name in SKILL FLOW.md (`documentation-generator`); slash
# forms are already covered by SLASH_TOKEN.
BACKTICK_NAME = re.compile(rf"`({NAME})`")
SUBAGENT_TYPE = re.compile(r"subagent_type[*:\s]*`([^`]+)`")
SKILL_PATH = re.compile(r"\.(claude|cursor|agents)/skills/([A-Za-z0-9_-]+)/")
INVOKE_SKILL = re.compile(
    rf"\b(?:invoke|execute)\s+(?:the\s+)?(?:`({NAME})`|({NAME}-{NAME}))\s+skill\b",
    re.IGNORECASE,
)
AGENTS_MD_KIND = re.compile(rf"`({NAME})`\s+(skill|agent|command|hook)\b")
PHASE_MAP_HEADING = re.compile(r"^#{2,}\s+Phase Map\s*$", re.IGNORECASE)


class RoutesError(Exception):
    """A usage or environment failure, distinct from a finding."""


@dataclass
class Finding:
    edition: str
    severity: str  # "error" | "warning"
    check: str  # "hook-wiring" | "routing" | "reachability" | "allowlist"
    path: str  # repository-relative, POSIX separators
    line: Optional[int]
    message: str
    target: str = ""  # the unresolved name or path, for allowlist matching

    def location(self) -> str:
        return f"{self.path}:{self.line}" if self.line else self.path


@dataclass
class Report:
    editions: List[str]
    findings: List[Finding] = field(default_factory=list)
    allowlisted: List[Finding] = field(default_factory=list)

    @property
    def errors(self) -> List[Finding]:
        return [item for item in self.findings if item.severity == "error"]

    @property
    def warnings(self) -> List[Finding]:
        return [item for item in self.findings if item.severity == "warning"]

    def to_json(self) -> dict:
        return {
            "editions": self.editions,
            "errors": len(self.errors),
            "warnings": len(self.warnings),
            "allowlisted": len(self.allowlisted),
            "findings": [asdict(item) for item in self.findings],
            "allowlisted_findings": [asdict(item) for item in self.allowlisted],
        }


# --------------------------------------------------------------------------
# Parsing helpers
# --------------------------------------------------------------------------


def wired_hook_scripts(command: str) -> List[str]:
    """Tool-tree-relative hook scripts one wiring command runs.

    Two forms name a script: a path ending in ``.<tool>/hooks/<name>.sh``
    under any prefix (bare, ``"${CLAUDE_PROJECT_DIR}"/``, a git toplevel), and
    a launcher that execs ``.<tool>/hooks/$1`` with the script name passed as
    an argument. An inline snippet such as the Notification hook names none.
    """
    found = [f"{match.group(1)}/hooks/{match.group(2)}" for match in HOOK_REF.finditer(command)]
    launcher = HOOK_LAUNCHER.search(command)
    if launcher:
        try:
            words = shlex.split(command)
        except ValueError:
            words = []
        found += [
            f"{launcher.group(1)}/hooks/{word}"
            for word in words[1:]
            if HOOK_SCRIPT_NAME.fullmatch(word)
        ]
    return list(dict.fromkeys(found))


def read_lines(path: Path) -> List[str]:
    return path.read_text(encoding="utf-8").splitlines()


def parse_frontmatter(lines: Sequence[str]) -> Tuple[Dict[str, Tuple[str, int]], int]:
    """Return ``{key: (raw value, 1-based line)}`` and the first body index.

    A deliberately small YAML subset - top-level ``key: value`` lines with
    indented continuation lines appended - which is all the command and agent
    frontmatter in this repository uses. No frontmatter, or an unterminated
    block, yields an empty mapping and a body starting at line 1.
    """
    if not lines or lines[0].strip() != "---":
        return {}, 0
    values: Dict[str, List] = {}
    key = None
    for index in range(1, len(lines)):
        line = lines[index]
        if line.strip() == "---":
            return {k: (v[0], v[1]) for k, v in values.items()}, index + 1
        match = FRONTMATTER_KEY.match(line)
        if match and not line[:1].isspace():
            key = match.group(1)
            values[key] = [match.group(2), index + 1]
        elif key is not None:
            values[key][0] += "\n" + line
    return {}, 0


def scalar(raw: str) -> str:
    return raw.strip().strip("'\"").strip()


def name_list(raw: str) -> List[str]:
    """Names from ``[a, b]`` (one or several lines), ``a, b``, a YAML block
    list (``- a``), or a scalar; ``null`` and ``[]`` yield nothing."""
    lines = [line.strip() for line in raw.strip().splitlines() if line.strip()]
    if lines and all(line.startswith("-") for line in lines):
        items = [line[1:] for line in lines]
    else:
        text = " ".join(lines)
        if text.startswith("[") and text.endswith("]"):
            text = text[1:-1]
        items = text.split(",")
    names = []
    for item in items:
        value = item.strip().strip("'\"").strip()
        if value and value.lower() not in NULL_VALUES:
            names.append(value)
    return names


def body_lines(lines: Sequence[str], start: int) -> Iterable[Tuple[int, str]]:
    """Body lines outside fenced code blocks, with 1-based line numbers."""
    fenced = False
    for index in range(start, len(lines)):
        line = lines[index]
        stripped = line.lstrip()
        if stripped.startswith("```") or stripped.startswith("~~~"):
            fenced = not fenced
            continue
        if not fenced:
            yield index + 1, line


# --------------------------------------------------------------------------
# Edition model
# --------------------------------------------------------------------------


@dataclass
class Agent:
    stem: str
    name: str
    path: Path
    lines: List[str]
    frontmatter: Dict[str, Tuple[str, int]]
    body_start: int


class Edition:
    def __init__(self, repo_root: Path, name: str) -> None:
        self.repo_root = repo_root
        self.name = name
        self.root = repo_root / name
        self.skills = self._skill_names(self.root / ".agents" / "skills")
        self.tool_skills = {
            tool: self.skills | self._skill_names(self.root / directory / "skills")
            for tool, directory, _ in TOOLS
        }
        self.tool_skills["agents"] = set(self.skills)
        self.commands = {
            tool: self._markdown_stems(self.root / directory / "commands")
            for tool, directory, _ in TOOLS
        }
        self.agents = {
            tool: self._load_agents(self.root / directory / "agents")
            for tool, directory, _ in TOOLS
        }

    @staticmethod
    def _skill_names(directory: Path) -> Set[str]:
        if not directory.is_dir():
            return set()
        return {
            child.name
            for child in directory.iterdir()
            if child.is_dir() and (child / "SKILL.md").is_file()
        }

    @staticmethod
    def _markdown_files(directory: Path) -> List[Path]:
        if not directory.is_dir():
            return []
        return sorted(
            child
            for child in directory.iterdir()
            if child.is_file() and child.suffix == ".md" and child.name != "README.md"
        )

    def _markdown_stems(self, directory: Path) -> Dict[str, Path]:
        return {path.stem: path for path in self._markdown_files(directory)}

    def _load_agents(self, directory: Path) -> Dict[str, Agent]:
        agents = {}
        for path in self._markdown_files(directory):
            lines = read_lines(path)
            frontmatter, body_start = parse_frontmatter(lines)
            name = scalar(frontmatter["name"][0]) if "name" in frontmatter else ""
            agents[path.stem] = Agent(path.stem, name, path, lines, frontmatter, body_start)
        return agents

    def rel(self, path: Path) -> str:
        return path.relative_to(self.repo_root).as_posix()

    def agent_resolves(self, tool: str, value: str) -> bool:
        for agent in self.agents.get(tool, {}).values():
            if value in (agent.stem, agent.name) or (
                agent.name and value == f"{agent.name}-agent"
            ):
                return True
        return False

    def namespace(self, tool: str) -> Set[str]:
        """Names a ``/x`` invocation can reach in ``tool``."""
        return set(self.commands.get(tool, {})) | self.tool_skills.get(tool, set())


# --------------------------------------------------------------------------
# Checks
# --------------------------------------------------------------------------


class Checker:
    def __init__(self, edition: Edition, index_modes: Dict[str, str]) -> None:
        self.edition = edition
        self.index_modes = index_modes
        self.findings: List[Finding] = []
        self.reached: Set[str] = set()
        self._seen: Set[Tuple[str, Optional[int], str, str]] = set()

    def add(
        self,
        severity: str,
        check: str,
        path: Path,
        line: Optional[int],
        message: str,
        target: str = "",
    ) -> None:
        rel = self.edition.rel(path)
        key = (rel, line, check, target or message)
        if key in self._seen:
            return
        self._seen.add(key)
        self.findings.append(
            Finding(self.edition.name, severity, check, rel, line, message, target)
        )

    def note_skill(self, name: str) -> None:
        if name in self.edition.skills:
            self.reached.add(name)

    # -- hook wiring -------------------------------------------------------

    def check_hooks(self) -> None:
        edition = self.edition
        wired: Set[str] = set()
        for tool, directory, wiring in TOOLS:
            wiring_path = edition.root / wiring
            if not wiring_path.is_file():
                continue
            text = wiring_path.read_text(encoding="utf-8")
            try:
                json.loads(text)
            except ValueError as error:
                line = getattr(error, "lineno", None)
                self.add("error", "hook-wiring", wiring_path, line, f"invalid JSON: {error}")
            for number, line in enumerate(text.splitlines(), 1):
                for value in COMMAND_VALUE.finditer(line):
                    try:
                        command = json.loads(value.group(1))
                    except ValueError:
                        continue
                    for relative in wired_hook_scripts(command):
                        wired.add(relative)
                        self._check_hook_script(wiring_path, number, relative)
        for tool, directory, _ in TOOLS:
            hooks_dir = edition.root / directory / "hooks"
            if not hooks_dir.is_dir():
                continue
            for script in sorted(hooks_dir.glob("*.sh")):
                relative = f"{directory}/hooks/{script.name}"
                if relative not in wired:
                    self.add(
                        "warning",
                        "hook-wiring",
                        script,
                        None,
                        f"hook script is present for {tool} but no wiring file references it",
                        relative,
                    )

    def _check_hook_script(self, wiring_path: Path, line: int, relative: str) -> None:
        script = self.edition.root / relative
        index_key = self.edition.rel(script)
        mode = self.index_modes.get(index_key)
        if not script.is_file() and mode is None:
            self.add(
                "error", "hook-wiring", wiring_path, line,
                f"wired hook script {relative} does not exist", relative,
            )
        elif mode is None:
            self.add(
                "error", "hook-wiring", wiring_path, line,
                f"wired hook script {relative} is not tracked in Git", relative,
            )
        elif not script.is_file():
            self.add(
                "error", "hook-wiring", wiring_path, line,
                f"wired hook script {relative} is tracked but missing on disk", relative,
            )
        elif mode != EXECUTABLE_MODE:
            self.add(
                "error", "hook-wiring", wiring_path, line,
                f"wired hook script {relative} has index mode {mode}, expected "
                f"{EXECUTABLE_MODE} (git update-index --chmod=+x)", relative,
            )

    # -- routing: commands -------------------------------------------------

    def check_commands(self) -> None:
        edition = self.edition
        for tool in ("claude", "cursor"):
            for stem, path in sorted(edition.commands.get(tool, {}).items()):
                lines = read_lines(path)
                frontmatter, body_start = parse_frontmatter(lines)
                if tool == "claude":
                    self._command_frontmatter(path, lines, frontmatter, body_start)
                self._command_body(tool, path, lines, body_start)

    def _command_frontmatter(
        self,
        path: Path,
        lines: Sequence[str],
        frontmatter: Dict[str, Tuple[str, int]],
        body_start: int,
    ) -> None:
        edition = self.edition
        if "spawns" in frontmatter:
            raw, line = frontmatter["spawns"]
            for value in name_list(raw):
                if not edition.agent_resolves("claude", value):
                    self.add(
                        "error", "routing", path, line,
                        f"spawns '{value}' names no .claude/agents agent", value,
                    )
        for key in ("flow-next", "flow-alternatives"):
            if key not in frontmatter:
                continue
            raw, line = frontmatter[key]
            for value in name_list(raw):
                self._resolve_invocation("claude", path, line, value, f"{key} '{value}'")
        for index in range(1, max(body_start - 1, 1)):
            for match in STAGE_AGENTS.finditer(lines[index]):
                for value in name_list(match.group(1)):
                    if not edition.agent_resolves("claude", value):
                        self.add(
                            "error", "routing", path, index + 1,
                            f"flow stage agent '{value}' names no .claude/agents agent", value,
                        )

    def _command_body(self, tool: str, path: Path, lines: Sequence[str], start: int) -> None:
        edition = self.edition
        for number, line in body_lines(lines, start):
            for match in SUBAGENT_TYPE.finditer(line):
                value = match.group(1).strip()
                if not edition.agent_resolves(tool, value):
                    self.add(
                        "error", "routing", path, number,
                        f"subagent_type '{value}' names no .{tool}/agents agent", value,
                    )
            self._skill_paths(path, number, line)
            self._slash_spans(tool, path, number, line)

    def _skill_paths(self, path: Path, number: int, line: str) -> None:
        for match in SKILL_PATH.finditer(line):
            tree, value = match.group(1), match.group(2)
            if value in self.edition.tool_skills.get(tree, set()):
                self.note_skill(value)
            else:
                self.add(
                    "error", "routing", path, number,
                    f"path .{tree}/skills/{value}/ names no skill", value,
                )

    def _slash_spans(self, tool: str, path: Path, number: int, line: str) -> None:
        for span in CODE_SPAN.finditer(line):
            match = SLASH_SPAN.match(span.group(1))
            if match:
                name = match.group(1)
                self._resolve_invocation(tool, path, number, name, f"`/{name}`")

    def _resolve_invocation(
        self, tool: str, path: Path, line: int, value: str, label: str
    ) -> None:
        """A name invoked as ``/value`` (or offered as the next one) in ``tool``."""
        name = value.lstrip("/")
        if name in self.edition.namespace(tool):
            self.note_skill(name)
            return
        if tool == "agents":
            where = "no skill in .agents/skills (Codex has no command layer)"
        else:
            where = f"no .{tool}/commands command and no skill"
        self.add("error", "routing", path, line, f"{label} resolves to {where}", name)

    # -- routing: agents ---------------------------------------------------

    def check_agents(self) -> None:
        edition = self.edition
        for tool in ("claude", "cursor"):
            names: Dict[str, Agent] = {}
            for stem, agent in sorted(edition.agents.get(tool, {}).items()):
                if not agent.name:
                    self.add(
                        "error", "routing", agent.path, 1,
                        "agent has no frontmatter name; commands and flows cannot route to it",
                        stem,
                    )
                elif agent.name in names:
                    self.add(
                        "error", "routing", agent.path,
                        agent.frontmatter["name"][1],
                        f"agent name '{agent.name}' is also used by "
                        f"{edition.rel(names[agent.name].path)}",
                        agent.name,
                    )
                else:
                    names[agent.name] = agent
                if tool == "claude":
                    if "invokes" not in agent.frontmatter:
                        self.add(
                            "error", "routing", agent.path, 1,
                            "Claude agent has no 'invokes' skill", stem,
                        )
                    else:
                        raw, line = agent.frontmatter["invokes"]
                        for value in name_list(raw):
                            self._agent_skill(agent.path, line, value, "invokes")
                for number, line in body_lines(agent.lines, agent.body_start):
                    for match in INVOKE_SKILL.finditer(line):
                        value = (match.group(1) or match.group(2)).lower()
                        self._agent_skill(agent.path, number, value, "invoked skill")
                    self._skill_paths(agent.path, number, line)
                    self._slash_spans(tool, agent.path, number, line)

    def _agent_skill(self, path: Path, line: int, value: str, label: str) -> None:
        if value in self.edition.skills:
            self.note_skill(value)
        else:
            self.add(
                "error", "routing", path, line,
                f"{label} '{value}' has no .agents/skills/{value}/SKILL.md", value,
            )

    # -- routing: SKILL FLOW.md --------------------------------------------

    def check_skill_flows(self) -> None:
        # "agents" is the Codex edition of the flow: Codex reads .agents/skills.
        for tool in ("agents", "claude", "cursor"):
            path = self.edition.root / f".{tool}" / "skills" / SKILL_FLOW
            if not path.is_file():
                continue
            lines = read_lines(path)
            in_phase_map = False
            for number, line in enumerate(lines, 1):
                if line.startswith("#"):
                    in_phase_map = bool(PHASE_MAP_HEADING.match(line))
                for match in SLASH_TOKEN.finditer(line):
                    name = match.group(1)
                    self._resolve_invocation(tool, path, number, name, f"`/{name}`")
                for match in BACKTICK_NAME.finditer(line):
                    value = match.group(1)
                    self._resolve_invocation(tool, path, number, value, f"`{value}`")
                if in_phase_map and line.lstrip().startswith("|"):
                    self._phase_map_row(tool, path, number, line)

    def _phase_map_row(self, tool: str, path: Path, number: int, line: str) -> None:
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) < 2 or set("".join(cells)) <= set("-: "):
            return
        for cell in cells[1:]:
            for item in cell.split(","):
                value = item.strip().strip("`").strip()
                bare = value.lstrip("/")
                if bare and NAME_RE.match(bare):
                    label = f"Phase Map item '{bare}'"
                    self._resolve_invocation(tool, path, number, bare, label)

    # -- routing: AGENTS.md ------------------------------------------------

    def check_agents_md(self) -> None:
        edition = self.edition
        path = edition.root / "AGENTS.md"
        if not path.is_file():
            return
        lines = read_lines(path)
        hook_names: Set[str] = set()
        for _, directory, _ in TOOLS:
            hooks_dir = edition.root / directory / "hooks"
            if hooks_dir.is_dir():
                hook_names |= {script.stem for script in hooks_dir.glob("*.sh")}
        commands = set(edition.commands["claude"]) | set(edition.commands["cursor"])
        for number, line in body_lines(lines, 0):
            for span in CODE_SPAN.finditer(line):
                match = SLASH_SPAN.match(span.group(1))
                if not match:
                    continue
                name = match.group(1)
                if name not in commands and name not in edition.skills:
                    self.add(
                        "error", "routing", path, number,
                        f"/{name} names no command and no skill", name,
                    )
            for match in AGENTS_MD_KIND.finditer(line):
                name, kind = match.group(1), match.group(2)
                if kind == "skill":
                    ok = name in edition.skills
                elif kind == "agent":
                    ok = edition.agent_resolves("claude", name) or edition.agent_resolves(
                        "cursor", name
                    )
                elif kind == "command":
                    ok = name in commands
                else:
                    ok = name in hook_names
                if not ok:
                    self.add(
                        "error", "routing", path, number,
                        f"`{name}` {kind} does not exist in this edition", name,
                    )

    # -- reachability ------------------------------------------------------

    def check_reachability(self) -> None:
        edition = self.edition
        for skill in sorted(edition.skills - self.reached):
            self.add(
                "error",
                "reachability",
                edition.root / ".agents" / "skills" / skill / "SKILL.md",
                None,
                f"skill '{skill}' is named by no command, agent or flow",
                skill,
            )

    def run(self) -> List[Finding]:
        self.check_hooks()
        self.check_agents()
        self.check_commands()
        self.check_skill_flows()
        self.check_agents_md()
        self.check_reachability()
        return self.findings


# --------------------------------------------------------------------------
# Git index and allowlist
# --------------------------------------------------------------------------


def index_modes(repo_root: Path, editions: Sequence[str]) -> Dict[str, str]:
    """``{repo-relative path: index mode}`` for every tracked hook file."""
    pathspecs = [
        f"{edition}/{directory}/hooks" for edition in editions for _, directory, _ in TOOLS
    ]
    try:
        result = subprocess.run(
            ["git", "-C", str(repo_root), "ls-files", "-s", "-z", "--", *pathspecs],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    except OSError as error:
        raise RoutesError("git is required to read hook index modes") from error
    if result.returncode != 0:
        raise RoutesError(
            "git ls-files failed: " + result.stderr.decode("utf-8", "replace").strip()
        )
    modes = {}
    for entry in result.stdout.decode("utf-8").split("\0"):
        if not entry:
            continue
        meta, _, path = entry.partition("\t")
        modes[path] = meta.split(" ", 1)[0]
    return modes


ALLOWLIST_SECTIONS = {
    # section: (fields that identify the entry, check it suppresses)
    "unwired_hooks": (("path",), "hook-wiring"),
    "unreachable_skills": (("skill",), "reachability"),
    "references": (("file", "target"), "routing"),
}


def load_allowlist(path: Path) -> dict:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {}
    except ValueError as error:
        raise RoutesError(f"{path}: invalid JSON: {error}") from error
    if not isinstance(data, dict):
        raise RoutesError(f"{path}: top level must be an object")
    return data


def validate_allowlist(data: dict, label: str, editions: Sequence[str] = ()) -> List[Finding]:
    """Structural problems in the allowlist, as errors: an entry without a
    reason is exactly the silent exemption the file exists to prevent, and an
    entry naming an unknown edition would never match and never go stale."""
    problems = []
    known = set(EDITIONS) | set(editions) | {"*"}
    for section, entries in data.items():
        if section.startswith("_") or section == "version":
            continue
        if section not in ALLOWLIST_SECTIONS:
            problems.append(f"unknown section '{section}'")
            continue
        if not isinstance(entries, list):
            problems.append(f"section '{section}' must be a list")
            continue
        keys = ALLOWLIST_SECTIONS[section][0]
        for position, entry in enumerate(entries):
            where = f"{section}[{position}]"
            if not isinstance(entry, dict):
                problems.append(f"{where} must be an object")
                continue
            for required in ("edition", "reason") + keys:
                value = entry.get(required)
                if not isinstance(value, str) or not value.strip():
                    problems.append(f"{where} needs a non-empty '{required}'")
            edition = entry.get("edition")
            if isinstance(edition, str) and edition.strip() and edition not in known:
                problems.append(f"{where} names unknown edition '{edition}'")
    return [
        Finding("(allowlist)", "error", "allowlist", label, None, problem)
        for problem in problems
    ]


def entry_matches(section: str, entry: dict, finding: Finding) -> bool:
    check = ALLOWLIST_SECTIONS[section][1]
    if finding.check != check:
        return False
    if entry.get("edition") not in ("*", finding.edition):
        return False
    if section == "unwired_hooks":
        return finding.severity == "warning" and finding.target == entry["path"]
    if section == "unreachable_skills":
        return finding.target == entry["skill"]
    prefix = f"{finding.edition}/"
    relative = finding.path[len(prefix):] if finding.path.startswith(prefix) else finding.path
    return relative == entry["file"] and finding.target == entry["target"]


def apply_allowlist(
    findings: List[Finding],
    data: dict,
    label: str,
    editions: Sequence[str],
    filtered: bool = False,
) -> Tuple[List[Finding], List[Finding]]:
    """Split findings into (kept, suppressed) and add a stale warning for
    every entry that matched nothing in a run that could have matched it."""
    kept, suppressed = [], []
    used: Set[Tuple[str, int]] = set()
    sections = [
        (section, position, entry)
        for section in ALLOWLIST_SECTIONS
        for position, entry in enumerate(data.get(section, []) or [])
        if isinstance(entry, dict)
    ]
    for finding in findings:
        matched = False
        for section, position, entry in sections:
            if entry_matches(section, entry, finding):
                used.add((section, position))
                matched = True
        (suppressed if matched else kept).append(finding)
    for section, position, entry in sections:
        if (section, position) in used:
            continue
        edition = entry.get("edition")
        if (edition == "*" and filtered) or (edition != "*" and edition not in editions):
            continue  # not every edition it covers was checked; staleness is unknown
        kept.append(
            Finding(
                "(allowlist)",
                "warning",
                "allowlist",
                label,
                None,
                f"stale entry {section}[{position}] matches nothing: "
                + json.dumps({k: v for k, v in entry.items() if k != "reason"}, sort_keys=True),
            )
        )
    return kept, suppressed


# --------------------------------------------------------------------------
# Driver
# --------------------------------------------------------------------------


def check_repo(
    repo_root: Path,
    editions: Sequence[str] = EDITIONS,
    allowlist: Optional[dict] = None,
    allowlist_label: str = "scripts/check_routes_allowlist.json",
    filtered: bool = False,
) -> Report:
    """Run every check over ``editions`` under ``repo_root``.

    ``filtered`` marks a run narrowed by ``--edition``: wildcard allowlist
    entries are then not reported stale, since the editions they would match
    were not checked.
    """
    report = Report(list(editions))
    allowlist = allowlist or {}
    findings = validate_allowlist(allowlist, allowlist_label, editions)
    modes = index_modes(repo_root, editions)
    for name in editions:
        if not (repo_root / name).is_dir():
            findings.append(
                Finding(name, "error", "routing", name, None, "edition directory not found")
            )
            continue
        findings.extend(Checker(Edition(repo_root, name), modes).run())
    kept, suppressed = apply_allowlist(
        findings, allowlist, allowlist_label, editions, filtered
    )
    report.findings = kept
    report.allowlisted = suppressed
    return report


def render(report: Report) -> str:
    out = []
    groups = list(report.editions)
    if any(item.edition == "(allowlist)" for item in report.findings):
        groups.append("(allowlist)")
    for group in groups:
        items = [item for item in report.findings if item.edition == group]
        allowed = sum(1 for item in report.allowlisted if item.edition == group)
        suffix = f" ({allowed} allowlisted)" if allowed else ""
        if not items:
            out.append(f"{group}: ok{suffix}")
            continue
        out.append(f"{group}:{suffix}")
        items.sort(key=lambda item: (item.severity != "error", item.path, item.line or 0))
        for item in items:
            out.append(f"  {item.severity:<7} {item.location()}  [{item.check}] {item.message}")
    out.append(
        f"check_routes: {len(report.errors)} error(s), {len(report.warnings)} warning(s), "
        f"{len(report.allowlisted)} allowlisted across {len(report.editions)} edition(s)"
    )
    return "\n".join(out)


def resolve_edition(value: str) -> str:
    if value in EDITIONS:
        return value
    alias = EDITION_ALIASES.get(value.strip().lower())
    if alias:
        return alias
    raise argparse.ArgumentTypeError(
        f"unknown edition '{value}' (choose from: {', '.join(EDITIONS)})"
    )


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--edition",
        action="append",
        type=resolve_edition,
        help="limit to one edition (repeatable); default: all",
    )
    parser.add_argument("--json", action="store_true", help="print a JSON report")
    parser.add_argument(
        "--allowlist",
        type=Path,
        default=DEFAULT_ALLOWLIST,
        help="allowlist file (default: scripts/check_routes_allowlist.json)",
    )
    parser.add_argument("--root", type=Path, default=REPO_ROOT, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)

    editions = list(dict.fromkeys(args.edition or EDITIONS))
    try:
        allowlist = load_allowlist(args.allowlist)
        try:
            label = args.allowlist.resolve().relative_to(args.root.resolve()).as_posix()
        except ValueError:
            label = str(args.allowlist)
        report = check_repo(args.root, editions, allowlist, label, bool(args.edition))
    except RoutesError as error:
        print(f"check_routes: {error}", file=sys.stderr)
        return 2
    if args.json:
        print(json.dumps(report.to_json(), indent=2, sort_keys=True))
    else:
        print(render(report))
    return 1 if report.errors else 0


if __name__ == "__main__":
    sys.exit(main())
