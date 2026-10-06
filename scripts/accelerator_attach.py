#!/usr/bin/env python3
"""Lend an accelerator edition to a project for a session; copy nothing into it.

The accelerator stays in this clone. A session started through this module
gets the edition's policy, skills, agents, commands and hooks from the clone,
and keeps the accelerator's per-project state (Project Brain, Memory Bank,
local index) in a private directory outside the project. `git pull` in the
clone updates every attached project at once.

Run from anywhere; the project defaults to the current directory:

    python3 scripts/accelerator_attach.py detect [--project DIR]
    python3 scripts/accelerator_attach.py run claude [--edition NAME] [--project DIR] [-- CLAUDE ARGS]
    python3 scripts/accelerator_attach.py run codex  [...] [-- CODEX ARGS]
    python3 scripts/accelerator_attach.py run cursor [...] [-- CURSOR AGENT ARGS]
    python3 scripts/accelerator_attach.py env [--project DIR]        # shell exports for other launchers
    python3 scripts/accelerator_attach.py trust-codex-hooks [--project DIR]

How each tool receives the edition (all verified against the installed CLIs,
see docs/ATTACHED-MODE.md):

- Claude Code: `--add-dir <edition>` loads its skills, commands and subagents
  under their own names; `--append-system-prompt-file` carries AGENTS.md;
  `--settings` carries the edition's permissions, environment and hooks with
  absolute paths into the clone.
- Codex: `-c developer_instructions=...` carries AGENTS.md and the skill
  catalogue (Codex has no setting for an extra skills folder); `-c hooks.*`
  carries the hooks, which Codex runs only after `trust-codex-hooks` records
  their hashes once; `--add-dir <state>` lets the sandbox write the state.
- Cursor Agent: `--plugin-dir <edition>` loads `.cursor-plugin/plugin.json`,
  which points at the edition's own `.cursor` rules, skills, agents, commands
  and hooks; the policy file is named in the prompt preamble.

The Harness imports this module; the functions below are its contract.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional


REPOSITORY = Path(__file__).resolve().parents[1]
EDITIONS = {
    "Laravel": "Laravel",
    "Symfony": "Symfony",
    "PHP Core": "PHP Core",
    "WordPress": "Cms/wordpress",
}
TOOLS = ("claude", "codex", "cursor")
STATE_MARKER = "accelerator-attach.json"
# Codex hook events the edition wires, in .codex/hooks.json spelling.
CODEX_EVENTS = ("SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop")
SKILL_FRONTMATTER = re.compile(r"\A---\n(.*?)\n---", re.DOTALL)
WORDPRESS_PACKAGES = {"johnpbloch/wordpress", "johnpbloch/wordpress-core", "roots/wordpress", "roots/wordpress-no-content"}
WORDPRESS_TYPES = {"wordpress-plugin", "wordpress-theme", "wordpress-muplugin", "wordpress-core"}


class AttachError(Exception):
    """A user-facing reason the edition cannot be attached."""


@dataclass
class Overlay:
    """What one launch of one tool adds: arguments, settings, environment."""

    arguments: list[str] = field(default_factory=list)
    # Claude only: merged into the single --settings JSON of the launch.
    settings: dict[str, Any] = field(default_factory=dict)
    environment: dict[str, str] = field(default_factory=dict)
    # Cursor only: prepended to the first prompt, which is its only channel.
    prompt_prefix: str = ""


# ---------------------------------------------------------------------------
# Where things are

def edition_directory(edition: str, repository: Path = REPOSITORY) -> Path:
    if edition not in EDITIONS:
        raise AttachError(f"Unknown edition {edition!r}; choose one of: {', '.join(EDITIONS)}")
    directory = repository / EDITIONS[edition]
    if not (directory / "AGENTS.md").is_file():
        raise AttachError(f"The {edition} edition is missing from {repository}")
    return directory


def project_key(project: Path) -> str:
    """The project's identity, shared with the Harness's project registry."""
    return hashlib.sha256(os.path.normcase(str(project)).encode()).hexdigest()[:16]


def default_state_base() -> Path:
    """The Harness's own state directory, so the CLI and the browser share memory."""
    if os.name == "nt":
        base = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData/Local"))
    else:
        base = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local/state")
    return base / "ai-infrastructure-harness"


def state_directory(project: Path, base: Optional[Path] = None) -> Path:
    return Path(base or default_state_base()) / "attached" / project_key(project)


def resolve_project(value: Optional[str]) -> Path:
    project = Path(value or os.getcwd()).expanduser().resolve()
    if not project.is_dir():
        raise AttachError(f"The project is not a directory: {project}")
    if project == REPOSITORY or REPOSITORY in project.parents or project in REPOSITORY.parents:
        raise AttachError("Attach the accelerator to a project outside this clone, and not to a folder that contains it.")
    return project


# ---------------------------------------------------------------------------
# Which edition fits the project

def _read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, UnicodeDecodeError):
        return {}
    return value if isinstance(value, dict) else {}


def _composer_packages(project: Path) -> tuple[set[str], str]:
    composer = _read_json(project / "composer.json")
    names: set[str] = set()
    for section in ("require", "require-dev"):
        value = composer.get(section)
        if isinstance(value, dict):
            names.update(name.lower() for name in value if isinstance(name, str))
    lock = _read_json(project / "composer.lock")
    for section in ("packages", "packages-dev"):
        for package in lock.get(section) or []:
            if isinstance(package, dict) and isinstance(package.get("name"), str):
                names.add(package["name"].lower())
    kind = composer.get("type") if isinstance(composer.get("type"), str) else ""
    return names, kind.lower()


def _has_header(paths: list[Path], header: str) -> bool:
    for path in paths[:40]:
        try:
            with path.open("rb") as handle:
                head = handle.read(8192).decode("utf-8", "ignore")
        except OSError:
            continue
        if header in head:
            return True
    return False


def detect_edition(project: Path) -> tuple[Optional[str], str]:
    """The edition the project's own files point to, and the evidence."""
    names, kind = _composer_packages(project)
    if "laravel/framework" in names:
        return "Laravel", "composer requires laravel/framework"
    if (project / "artisan").is_file():
        return "Laravel", "an artisan file at the project root"
    if "symfony/framework-bundle" in names:
        return "Symfony", "composer requires symfony/framework-bundle"
    if (project / "bin/console").is_file() and (project / "config/bundles.php").is_file():
        return "Symfony", "bin/console and config/bundles.php"
    if names & WORDPRESS_PACKAGES or kind in WORDPRESS_TYPES:
        return "WordPress", "composer declares WordPress"
    if (project / "wp-config.php").is_file() or (project / "wp-content").is_dir():
        return "WordPress", "a WordPress installation"
    root_php = sorted(project.glob("*.php"))
    if _has_header([project / "style.css"], "Theme Name:") or _has_header(root_php, "Plugin Name:"):
        return "WordPress", "a WordPress theme or plugin header"
    if (project / "composer.json").is_file() or root_php:
        return "PHP Core", "a PHP project without a dedicated edition"
    return None, "no composer.json and no PHP files at the project root"


# ---------------------------------------------------------------------------
# What every tool gets

def environment(edition: str, project: Path, state: Path, repository: Path = REPOSITORY) -> dict[str, str]:
    """The variables the edition's runtime and hooks read to find the three roots."""
    return {
        "ACCELERATOR_HOME": str(edition_directory(edition, repository)),
        "ACCELERATOR_STATE_DIR": str(state),
        "ACCELERATOR_PROJECT_DIR": str(project),
        "ACCELERATOR_EDITION": edition,
    }


def prepare_state(edition: str, project: Path, state: Path) -> None:
    """Create the private state directory and say whose it is.

    The runtime fills in the Project Brain and Memory Bank layout itself on
    first use; this only records the project, for a person looking at it.
    """
    state.mkdir(parents=True, exist_ok=True)
    marker = state / STATE_MARKER
    record = {"project": str(project), "edition": edition}
    current = _read_json(marker)
    if {key: current.get(key) for key in record} != record:
        marker.write_text(json.dumps({**record, "since": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}, indent=2) + "\n", encoding="utf-8")


def preamble(edition: str, project: Path, state: Path, repository: Path = REPOSITORY) -> str:
    home = edition_directory(edition, repository)
    return f"""# {edition} accelerator: attached, not installed

The {edition} accelerator is attached to this project from its own clone at
`{home}` (ACCELERATOR_HOME). Nothing of it is installed in the project at
`{project}`, and nothing of it may be copied there.

- Paths that the policy, skills, agents and commands give for the accelerator's
  own files - `AGENTS.md`, `.claude/`, `.agents/`, `.cursor/`, `.codex/`,
  `project-brain/PROTOCOL.md`, `project-brain/templates/`, `memory-bank/README.md`,
  `memory-bank/scripts/` - are relative to ACCELERATOR_HOME. Never edit them.
- The accelerator's state for this project - Project Brain records, Memory Bank
  chunks, the local index - is in `{state}` (ACCELERATOR_STATE_DIR). Change it
  only through the context runtime: `python3 "$ACCELERATOR_HOME/memory-bank/scripts/context.py" ...`,
  run from the project; it finds the state and the project by itself.
- Work products a workflow asks for - `specs/`, `tasks/TASK-NNN/`, `codebase/` -
  belong to the project and are written into it as usual.
"""


def policy(edition: str, repository: Path = REPOSITORY) -> str:
    return (edition_directory(edition, repository) / "AGENTS.md").read_text(encoding="utf-8")


def _frontmatter_value(text: str, key: str) -> str:
    match = SKILL_FRONTMATTER.match(text.replace("\r\n", "\n"))
    if not match:
        return ""
    lines = match.group(1).split("\n")
    for index, line in enumerate(lines):
        if line.startswith(f"{key}:"):
            value = line[len(key) + 1:].strip()
            if value in ("|", ">", "|-", ">-"):
                continuation = []
                for following in lines[index + 1:]:
                    if following and not following[0].isspace():
                        break
                    continuation.append(following.strip())
                value = " ".join(part for part in continuation if part)
            return value.strip().strip("\"'")
    return ""


def skill_catalogue(edition: str, tree: str = ".agents/skills", repository: Path = REPOSITORY) -> list[dict[str, str]]:
    """Name, description and path of every skill the edition ships for a tool."""
    root = edition_directory(edition, repository) / tree
    skills = []
    for path in sorted(root.glob("*/SKILL.md")):
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        name = _frontmatter_value(text, "name") or path.parent.name
        skills.append({"name": name, "description": _frontmatter_value(text, "description"), "path": str(path)})
    return skills


# ---------------------------------------------------------------------------
# Claude Code

def _claude_settings(edition: str, state: Path, repository: Path) -> dict[str, Any]:
    home = edition_directory(edition, repository)
    source = _read_json(home / ".claude" / "settings.json")
    settings: dict[str, Any] = {}
    if isinstance(source.get("env"), dict):
        settings["env"] = {str(key): str(value) for key, value in source["env"].items()}
    permissions = source.get("permissions") if isinstance(source.get("permissions"), dict) else {}
    home_rule = "//" + home.as_posix().lstrip("/")
    state_rule = "//" + state.as_posix().lstrip("/")
    settings["permissions"] = {
        "allow": [*permissions.get("allow", []), f"Read({home_rule}/**)", f"Read({state_rule}/**)"],
        # The clone is shared by every attached project: never edited from one.
        "deny": [*permissions.get("deny", []), f"Edit({home_rule}/**)", f"Write({home_rule}/**)"],
    }
    quoted_home = shlex.quote(home.as_posix())
    hooks: dict[str, Any] = {}
    for event, groups in (source.get("hooks") or {}).items():
        rewritten = []
        for group in groups if isinstance(groups, list) else []:
            group = json.loads(json.dumps(group))
            for hook in group.get("hooks", []):
                if isinstance(hook.get("command"), str):
                    hook["command"] = hook["command"].replace('"${CLAUDE_PROJECT_DIR}"', quoted_home)
            rewritten.append(group)
        hooks[event] = rewritten
    if hooks:
        settings["hooks"] = hooks
    return settings


def claude_overlay(edition: str, project: Path, state: Path, repository: Path = REPOSITORY) -> Overlay:
    home = edition_directory(edition, repository)
    prepare_state(edition, project, state)
    launch = state / "launch"
    launch.mkdir(parents=True, exist_ok=True)
    system_prompt = launch / "claude-system-prompt.md"
    content = preamble(edition, project, state, repository) + "\n" + policy(edition, repository)
    if not system_prompt.is_file() or system_prompt.read_text(encoding="utf-8") != content:
        system_prompt.write_text(content, encoding="utf-8")
    return Overlay(
        arguments=["--add-dir", str(home), "--append-system-prompt-file", str(system_prompt)],
        settings=_claude_settings(edition, state, repository),
        environment=environment(edition, project, state, repository),
    )


def merge_claude_settings(base: dict[str, Any], overlay: dict[str, Any]) -> dict[str, Any]:
    """One --settings value from the launcher's own and the edition's.

    Environment entries the launcher sets win: they carry per-launch choices
    (agent counts, effort) the edition's defaults must not undo.
    """
    merged = {**overlay, **{key: value for key, value in base.items() if key not in ("env", "permissions", "hooks")}}
    merged["env"] = {**overlay.get("env", {}), **base.get("env", {})}
    permissions: dict[str, list[str]] = {}
    for source in (overlay.get("permissions", {}), base.get("permissions", {})):
        for key, rules in source.items():
            permissions[key] = list(dict.fromkeys([*permissions.get(key, []), *rules]))
    if permissions:
        merged["permissions"] = permissions
    hooks = {event: list(groups) for event, groups in overlay.get("hooks", {}).items()}
    for event, groups in base.get("hooks", {}).items():
        hooks.setdefault(event, []).extend(groups)
    if hooks:
        merged["hooks"] = hooks
    return merged


# ---------------------------------------------------------------------------
# Codex

def _toml(value: Any) -> str:
    """Inline TOML for a `-c key=value` override."""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return json.dumps(value)
    if isinstance(value, str):
        # A JSON string is a valid TOML basic string.
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return "[" + ",".join(_toml(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{" + ",".join(f"{json.dumps(str(key)) if not re.fullmatch(r'[A-Za-z0-9_-]+', str(key)) else key}={_toml(item)}" for key, item in value.items()) + "}"
    raise TypeError(f"cannot express {type(value).__name__} in TOML")


def codex_hooks(edition: str, repository: Path = REPOSITORY) -> dict[str, list[dict[str, Any]]]:
    """The edition's .codex/hooks.json with each command pointing into the clone."""
    home = edition_directory(edition, repository)
    source = _read_json(home / ".codex" / "hooks.json").get("hooks") or {}
    hooks: dict[str, list[dict[str, Any]]] = {}
    for event in CODEX_EVENTS:
        groups = []
        for group in source.get(event) or []:
            handlers = []
            for handler in group.get("hooks", []):
                match = re.search(r"([A-Za-z0-9_.-]+\.sh)\s*$", str(handler.get("command", "")))
                if not match:
                    continue
                script = home / ".codex" / "hooks" / match.group(1)
                # bash runs the script on every platform Codex supports, and
                # needs no executable bit (NTFS has none).
                handlers.append({**handler, "command": "bash " + shlex.quote(script.as_posix())})
            if handlers:
                groups.append({**{key: value for key, value in group.items() if key != "hooks"}, "hooks": handlers})
        if groups:
            hooks[event] = groups
    return hooks


def codex_hook_arguments(edition: str, repository: Path = REPOSITORY) -> list[str]:
    arguments: list[str] = []
    for event, groups in codex_hooks(edition, repository).items():
        arguments += ["-c", f"hooks.{event}={_toml(groups)}"]
    return arguments


def codex_instructions(edition: str, project: Path, state: Path, repository: Path = REPOSITORY) -> str:
    skills = skill_catalogue(edition, ".agents/skills", repository)
    lines = [
        preamble(edition, project, state, repository),
        "## Accelerator skills",
        "",
        "Each skill below is a SKILL.md file in the accelerator. When the task matches",
        "a skill's description, read that whole file and follow it; per the policy,",
        "run only the one selected skill.",
        "",
    ]
    lines += [f"- {skill['name']}: {skill['description']} (file: {skill['path']})" for skill in skills]
    lines += ["", "## Accelerator policy (AGENTS.md)", "", policy(edition, repository)]
    return "\n".join(lines)


def codex_overlay(edition: str, project: Path, state: Path, repository: Path = REPOSITORY) -> Overlay:
    prepare_state(edition, project, state)
    arguments = ["-c", "developer_instructions=" + _toml(codex_instructions(edition, project, state, repository))]
    arguments += codex_hook_arguments(edition, repository)
    # Writable for the context runtime the model runs in the sandbox.
    arguments += ["--add-dir", str(state)]
    return Overlay(arguments=arguments, environment=environment(edition, project, state, repository))


def _app_server(executable: str, arguments: list[str], cwd: Path, requests: list[dict[str, Any]], timeout: float = 60) -> list[dict[str, Any]]:
    """Send numbered requests to `codex app-server` and return their responses."""
    process = subprocess.Popen([executable, *arguments, "app-server"], cwd=cwd, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    responses: dict[int, dict[str, Any]] = {}
    try:
        def send(message: dict[str, Any]) -> None:
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()

        def wait(identifier: int) -> dict[str, Any]:
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                line = process.stdout.readline()
                if not line:
                    break
                try:
                    message = json.loads(line)
                except ValueError:
                    continue
                if message.get("id") == identifier:
                    return message
            raise AttachError("Codex did not answer; check that `codex` runs and is signed in.")

        send({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"clientInfo": {"name": "accelerator-attach", "version": "1"}}})
        if "error" in wait(0):
            raise AttachError("Codex refused the app-server handshake.")
        send({"jsonrpc": "2.0", "method": "initialized"})
        for number, request in enumerate(requests, start=1):
            send({"jsonrpc": "2.0", "id": number, **request})
            responses[number] = wait(number)
    finally:
        try:
            process.stdin.close()
            process.wait(timeout=10)
        except (OSError, subprocess.TimeoutExpired):
            process.kill()
    return [responses[number] for number in sorted(responses)]


def codex_hook_trust(executable: str, edition: str, project: Path, repository: Path = REPOSITORY) -> list[dict[str, str]]:
    """The edition's session hooks as Codex sees them: key, hash, trust status."""
    [listing] = _app_server(executable, codex_hook_arguments(edition, repository), project,
                            [{"method": "hooks/list", "params": {"cwds": [str(project)]}}])
    if "error" in listing:
        raise AttachError("Codex could not list hooks.")
    entries = []
    for scope in (listing.get("result") or {}).get("data") or []:
        for hook in scope.get("hooks") or []:
            if hook.get("source") == "sessionFlags":
                entries.append({"key": hook["key"], "hash": hook["currentHash"], "status": hook["trustStatus"],
                                "command": hook.get("command", "")})
    return entries


def trust_codex_hooks(executable: str, edition: str, project: Path, repository: Path = REPOSITORY) -> list[dict[str, str]]:
    """Record the edition's hook hashes as trusted in the user's Codex config.

    Codex runs a hook only after its definition was approved once; the
    approval is a hash in ~/.codex/config.toml (hooks.state), the same record
    Codex's own /hooks review writes. Moving the clone changes the commands
    and so needs a new approval; a `git pull` that edits only scripts does not.
    """
    entries = codex_hook_trust(executable, edition, project, repository)
    pending = {entry["key"]: {"trusted_hash": entry["hash"]} for entry in entries if entry["status"] != "trusted"}
    if pending:
        [written] = _app_server(executable, [], project, [{"method": "config/batchWrite", "params": {
            "edits": [{"keyPath": "hooks.state", "mergeStrategy": "upsert", "value": pending}]}}])
        if "error" in written:
            raise AttachError(f"Codex did not record the hook approvals: {written['error'].get('message', 'unknown error')}")
    return codex_hook_trust(executable, edition, project, repository)


# ---------------------------------------------------------------------------
# Cursor Agent

def cursor_overlay(edition: str, project: Path, state: Path, repository: Path = REPOSITORY) -> Overlay:
    home = edition_directory(edition, repository)
    if not (home / ".cursor-plugin" / "plugin.json").is_file():
        raise AttachError(f"The {edition} edition has no .cursor-plugin/plugin.json to load.")
    prepare_state(edition, project, state)
    prefix = (preamble(edition, project, state, repository)
              + f"\nRead and follow the accelerator policy in `{home / 'AGENTS.md'}` before acting.\n\n")
    return Overlay(arguments=["--plugin-dir", str(home)], environment=environment(edition, project, state, repository),
                   prompt_prefix=prefix)


def apply_overlay(tool: str, command: list[str], overlay: Any, first_turn: bool = True) -> list[str]:
    """Merge an overlay - an Overlay or its dataclasses.asdict form - into argv
    built the Harness way: Claude with one `--settings`, Codex global flags
    before `exec`, Cursor's prompt after `--`."""
    if isinstance(overlay, Overlay):
        overlay = {"arguments": overlay.arguments, "settings": overlay.settings, "prompt_prefix": overlay.prompt_prefix}
    command = list(command)
    arguments = list(overlay.get("arguments") or [])
    if tool == "claude":
        index = command.index("--settings")
        settings = merge_claude_settings(json.loads(command[index + 1]), overlay.get("settings") or {})
        command[index + 1] = json.dumps(settings, separators=(",", ":"))
        command[index + 2:index + 2] = arguments
    elif tool == "codex":
        index = command.index("exec")
        command[index:index] = arguments
    elif tool == "cursor":
        index = command.index("--")
        command[index:index] = arguments
        if first_turn and overlay.get("prompt_prefix"):
            command[-1] = overlay["prompt_prefix"] + command[-1]
    else:
        raise AttachError(f"Unknown tool {tool!r}; choose one of: {', '.join(TOOLS)}")
    return command


def overlay(tool: str, edition: str, project: Path, state: Path, repository: Path = REPOSITORY) -> Overlay:
    if tool == "claude":
        return claude_overlay(edition, project, state, repository)
    if tool == "codex":
        return codex_overlay(edition, project, state, repository)
    if tool == "cursor":
        return cursor_overlay(edition, project, state, repository)
    raise AttachError(f"Unknown tool {tool!r}; choose one of: {', '.join(TOOLS)}")


# ---------------------------------------------------------------------------
# Command line

def _choose_edition(project: Path, requested: Optional[str]) -> str:
    if requested:
        edition_directory(requested)
        return requested
    edition, evidence = detect_edition(project)
    if edition is None:
        raise AttachError(f"No edition fits {project}: {evidence}. Pass --edition.")
    return edition


def _executable(tool: str) -> str:
    return {"claude": "claude", "codex": "codex", "cursor": "cursor-agent"}[tool]


def main(argv: Optional[list[str]] = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    # Everything after `--` belongs to the launched CLI, untouched.
    passthrough: list[str] = []
    if "--" in argv:
        index = argv.index("--")
        argv, passthrough = argv[:index], argv[index + 1:]
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("detect", "env", "trust-codex-hooks"):
        command = commands.add_parser(name)
        command.add_argument("--project")
        command.add_argument("--edition", choices=list(EDITIONS))
        command.add_argument("--state-base", type=Path)
    run = commands.add_parser("run")
    run.add_argument("tool", choices=TOOLS)
    run.add_argument("--project")
    run.add_argument("--edition", choices=list(EDITIONS))
    run.add_argument("--state-base", type=Path)
    run.add_argument("--executable")
    for command in (commands.choices["trust-codex-hooks"],):
        command.add_argument("--executable", default="codex")
    options = parser.parse_args(argv)
    if passthrough and options.command != "run":
        parser.error("arguments after -- are passed only to `run`")
    try:
        project = resolve_project(options.project)
        if options.command == "detect":
            edition, evidence = detect_edition(project)
            print(json.dumps({"project": str(project), "edition": edition, "evidence": evidence}))
            return 0 if edition else 1
        edition = _choose_edition(project, options.edition)
        state = state_directory(project, options.state_base)
        if options.command == "env":
            for key, value in environment(edition, project, state).items():
                print(f"export {key}={shlex.quote(value)}")
            return 0
        if options.command == "trust-codex-hooks":
            for entry in trust_codex_hooks(options.executable, edition, project):
                print(f"{entry['status']:9} {entry['key']}")
            return 0
        attached = overlay(options.tool, edition, project, state)
        extra = passthrough
        command = [options.executable or _executable(options.tool)]
        if options.tool == "claude":
            command += [*attached.arguments, "--settings", json.dumps(attached.settings), *extra]
        elif options.tool == "codex":
            command += [*attached.arguments, *extra]
        else:
            command += [*attached.arguments, *extra]
            if attached.prompt_prefix and not extra:
                command.append(attached.prompt_prefix + "Say which accelerator is attached and wait for my task.")
        print(f"Attaching {edition} from {edition_directory(edition)} to {project} (state: {state}).", file=sys.stderr)
        os.chdir(project)
        os.execvpe(command[0], command, {**os.environ, **attached.environment})
    except AttachError as error:
        print(f"accelerator_attach: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
