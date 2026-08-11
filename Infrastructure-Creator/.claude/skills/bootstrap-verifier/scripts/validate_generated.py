#!/usr/bin/env python3
"""Structural QA gate for a freshly generated accelerator.

Run by bootstrap-verifier at the end of infra-generate against the TARGET
project. Dependency-free (standard library only).

Usage:
    python3 validate_generated.py --target /path/to/target [--editions claude,cursor,codex]

Checks:
  - Every manifest-owned SKILL.md / agent / command has valid frontmatter.
  - Every flow-next / flow-alternatives / related / invokes / spawns reference
    resolves to a skill/agent that actually exists in the same edition.
  - Every selected edition root exists and no unselected edition root exists.
  - Every hook script passes `bash -n` and carries the executable bit, and the
    expected hook set is complete per edition (working-memory-read.sh is
    deliberately absent from Cursor - it has no prompt-time hook event).
  - Every flow command's stages name agents that were generated, no parallel
    stage holds two write-capable agents (the gate runs those one at a time),
    and a multi-stage flow declares at least one checkpoint.
  - Every hook wiring file (.claude/settings.json, .cursor/hooks.json,
    .codex/hooks.json) references only hook scripts that exist and are
    executable - every .sh token in a wired command is resolved, so an
    interpreter-prefixed "bash .claude/hooks/x.sh" cannot hide a dead hook.
  - The seeded memory-bank passes its own scripts/validate.py.
  - The context-brain runtime is present (context.py, brain_runtime.py,
    context_retrieval.py, validate.py under memory-bank/scripts/) and the
    project-brain/ skeleton is seeded with a substituted runtime.json whose
    canonical_edition points at a skills tree that actually exists.
  - Smoke: `python3 memory-bank/scripts/context.py status` and `... validate`
    both exit 0 inside the generated tree.
  - Every selected edition includes the memory quartet skills (memory-bank,
    project-brain, checkpoint, memory) and their agent/command wrappers where
    that edition carries those layers.
  - The upgrade contract: .infra-manifest.json exists, parses, carries a
    semver generator_version and a TASK reference, a valid mode (full/merge),
    a well-formed optional decisions map (standing kept/merged decisions),
    tracks no runtime state and not itself, every tracked file exists with a
    matching sha256, and a tracked AGENTS.md has a version stamp matching the
    manifest. Manifest membership exclusively defines ownership in both modes.
  - No template placeholders remain in any manifest-owned generated text file;
    unmanifested team files are never opened by structural/placeholder checks.

Exit code 0 = pass, non-zero = failures (printed to stderr).
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from infra_ownership import (  # noqa: E402
    MANIFEST_NAME,
    OwnershipError,
    confined_target_path,
    load_manifest,
    may_be_manifest_owned,
    resolve_target,
    sha256_file,
)

PLACEHOLDER_PATTERNS = [
    re.compile(r"\{skill-name\}"),
    re.compile(r"\{NNNN\}"),
    re.compile(r"\bTODO\b"),
    re.compile(r"\bFIXME\b"),
    re.compile(r"\bYYYY-MM-DD\b"),
    re.compile(r"\[target[_ ]name\]"),
    re.compile(r"\bTASK-\{N\}"),
    re.compile(r"\{\{[A-Z_]+\}\}"),
]

# Always-generated skills that operate the shared memory layer; every selected
# edition must carry all four (plus wrappers where the edition has those layers).
MEMORY_QUARTET = ("memory-bank", "project-brain", "checkpoint", "memory")

# The context-brain runtime memory-seed installs verbatim.
RUNTIME_SCRIPTS = (
    "context.py",
    "brain_runtime.py",
    "context_retrieval.py",
    "validate.py",
)

# Hook contract per edition. Cursor deliberately lacks working-memory-read.sh:
# it has no UserPromptSubmit-equivalent event to wire it to. Its read path is
# the alwaysApply rule .cursor/rules/working-memory.mdc that the Cursor
# copies of working-memory-write.sh and local-context.sh render instead
# (hook-forge step 6). subagent-gate.sh is present in every edition but is
# tool-owned - each edition ships a variant matching its host's gate
# contract, so byte-identity across editions is NOT expected for it.
BASE_HOOKS = (
    "local-context.sh",
    "bash-validator.sh",
    "file-naming-validator.sh",
    "loop-detection.sh",
    "working-memory-write.sh",
    "subagent-gate.sh",
    # Ships in every edition; wired on Claude/Cursor and deliberately
    # unregistered on Codex, where multi-agent is off and nothing stops.
    "subagent-dispatch.sh",
)
REQUIRED_HOOKS = {
    "claude": BASE_HOOKS + ("working-memory-read.sh",),
    "cursor": BASE_HOOKS,
    "codex": BASE_HOOKS + ("working-memory-read.sh",),
}

# Edition -> (hooks dir, wiring file) relative to the target root.
EDITION_HOOK_WIRING = {
    "claude": (".claude/hooks", ".claude/settings.json"),
    "cursor": (".cursor/hooks", ".cursor/hooks.json"),
    "codex": (".codex/hooks", ".codex/hooks.json"),
}

# Edition -> (skills dir relative to target, has_agents, has_commands)
EDITION_LAYOUT = {
    "claude": (".claude/skills", ".claude/agents", ".claude/commands"),
    "cursor": (".cursor/skills", ".cursor/agents", ".cursor/commands"),
    "codex": (".agents/skills", None, None),
}

EDITION_ROOTS = {
    "claude": (".claude",),
    "cursor": (".cursor",),
    "codex": (".agents", ".codex"),
}

# First line of a generated AGENTS.md, e.g.:
# <!-- Generated by Infrastructure-Creator v1.4.0 | TASK-003 | 2026-08-02 -->
# The generator name is not pinned so stack-adapter siblings can reuse this
# script verbatim.
AGENTS_STAMP_RE = re.compile(
    r"^<!--\s*Generated by\s+.+?\s+v(\d+\.\d+\.\d+)\s*\|\s*(TASK-\d+)\s*\|\s*\d{4}-\d{2}-\d{2}\s*-->"
)


def read_frontmatter(path: Path) -> tuple[dict, str]:
    text = path.read_text(encoding="utf-8")
    if not text.startswith("---"):
        return {}, text
    end = text.find("\n---", 3)
    if end == -1:
        return {}, text
    block = text[3:end].strip("\n")
    meta: dict = {}
    key = None
    for line in block.splitlines():
        m = re.match(r"^([A-Za-z0-9_-]+):\s*(.*)$", line)
        if m:
            key = m.group(1)
            val = m.group(2).strip()
            if val.startswith("[") and val.endswith("]"):
                items = [x.strip() for x in val[1:-1].split(",") if x.strip()]
                meta[key] = items
            else:
                meta[key] = val
    return meta, text


STAGE_RE = re.compile(r"^\s*-\s*\{(?P<body>.*)\}\s*$")
AGENTS_RE = re.compile(r"agents:\s*\[(?P<names>[^\]]*)\]")


def validate_flow(
    edition: str, path: Path, text: str, roster: set, write_agents: set, errors: list
) -> None:
    """Check a flow command's stages against the roster this run generated.

    Flows are the one command class that spawns more than one agent, so a
    stage naming an agent that was never generated is a dead flow. A single
    write-capable agent in a parallel stage is harmless - it takes the write
    lock and the read-only agents beside it are unaffected - but two of them
    deadlock the stage against each other, which is what is flagged here.
    """
    end = text.find("\n---", 3)
    block = text[3:end] if end != -1 else ""
    stages = [m.group("body") for m in
              (STAGE_RE.match(line) for line in block.splitlines()) if m]
    if not stages:
        errors.append(f"[{edition}] {path}: flow declares no stages")
        return
    for body in stages:
        found = AGENTS_RE.search(body)
        names = ([n.strip() for n in found.group("names").split(",") if n.strip()]
                 if found else [])
        if not names:
            errors.append(f"[{edition}] {path}: flow stage names no agents: {body}")
            continue
        for name in names:
            if name not in roster:
                errors.append(
                    f"[{edition}] {path}: flow stage agent '{name}' was not generated"
                )
        if "parallel: true" in body:
            writers = [n for n in names if n in write_agents]
            if len(writers) > 1:
                errors.append(
                    f"[{edition}] {path}: parallel stage holds write-capable "
                    f"agents {writers}; the gate runs them one at a time, so "
                    f"the stage blocks itself"
                )
    # A single-stage flow is one fan-out whose result the user reads
    # immediately; a multi-stage flow runs agents back to back and must
    # hand control back somewhere.
    if len(stages) > 1 and not any("checkpoint:" in body for body in stages):
        errors.append(
            f"[{edition}] {path}: multi-stage flow declares no checkpoint"
        )


def is_owned(files: dict, rel: str) -> bool:
    return rel in files


def collect_skill_names(skills_rel: str, files: dict) -> set:
    """Collect only manifest-owned skill descriptors."""
    prefix = f"{skills_rel}/"
    suffix = "/SKILL.md"
    return {
        rel[len(prefix) : -len(suffix)]
        for rel in files
        if rel.startswith(prefix)
        and rel.endswith(suffix)
        and "/" not in rel[len(prefix) : -len(suffix)]
    }


def owned_children(files: dict, directory: str, suffix: str) -> list[str]:
    prefix = f"{directory}/"
    return sorted(
        rel
        for rel in files
        if rel.startswith(prefix)
        and "/" not in rel[len(prefix) :]
        and rel.endswith(suffix)
    )


def check_placeholders(path: Path, errors: list) -> None:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return
    for pat in PLACEHOLDER_PATTERNS:
        if pat.search(text):
            errors.append(f"{path}: leftover placeholder matching /{pat.pattern}/")


def validate_owned_placeholders(target: Path, files: dict, errors: list) -> None:
    """Scan every owned text file and no unmanifested team file."""
    for rel in sorted(files):
        path = target / rel
        if path.is_file():
            check_placeholders(path, errors)


def validate_edition(target: Path, edition: str, files: dict, errors: list) -> None:
    skills_rel, agents_rel, commands_rel = EDITION_LAYOUT[edition]
    skills_dir = target / skills_rel
    if not skills_dir.exists():
        errors.append(f"[{edition}] selected but skills dir missing: {skills_rel}")
        return
    skill_names = collect_skill_names(skills_rel, files)
    if not skill_names:
        errors.append(f"[{edition}] no manifest-owned skills under {skills_rel}")
    for required_skill in MEMORY_QUARTET:
        if required_skill not in skill_names:
            errors.append(
                f"[{edition}] memory-layer skill '{required_skill}' missing under {skills_rel}"
            )

    for name in skill_names:
        sp = skills_dir / name / "SKILL.md"
        meta, _ = read_frontmatter(sp)
        if not meta.get("name"):
            errors.append(f"[{edition}] {sp}: missing 'name' in frontmatter")
        if not meta.get("description"):
            errors.append(f"[{edition}] {sp}: missing 'description' in frontmatter")
        for ref_key in ("flow-next", "flow-alternatives", "related"):
            refs = meta.get(ref_key, [])
            if isinstance(refs, str):
                refs = [] if refs in ("", "null", "none") else [refs]
            for ref in refs:
                if ref in ("null", "none", ""):
                    continue
                if ref not in skill_names:
                    errors.append(
                        f"[{edition}] {sp}: {ref_key} -> '{ref}' does not resolve to a generated skill"
                    )
    # Agents (editions that carry them).
    roster: set = set()
    write_agents: set = set()
    if agents_rel:
        agents_dir = target / agents_rel
        if agents_dir.exists():
            for rel in owned_children(files, agents_rel, ".md"):
                af = target / rel
                if af.name == "README.md":
                    continue
                meta, _ = read_frontmatter(af)
                if not meta.get("name"):
                    errors.append(f"[{edition}] {af}: agent missing 'name'")
                else:
                    roster.add(meta["name"])
                    if str(meta.get("writes", "")).lower() == "true":
                        write_agents.add(meta["name"])
                invokes = meta.get("invokes")
                if invokes and invokes not in skill_names:
                    errors.append(
                        f"[{edition}] {af}: invokes '{invokes}' is not a generated skill"
                    )
            for required_skill in MEMORY_QUARTET:
                required_rel = f"{agents_rel}/{required_skill}-agent.md"
                if required_skill in skill_names and not is_owned(files, required_rel):
                    errors.append(
                        f"[{edition}] {required_rel} is not manifest-owned"
                    )
        else:
            errors.append(f"[{edition}] selected but agents dir missing: {agents_rel}")

    # Commands (editions that carry them).
    if commands_rel:
        commands_dir = target / commands_rel
        if commands_dir.exists():
            for rel in owned_children(files, commands_rel, ".md"):
                cf = target / rel
                if cf.name == "README.md":
                    continue
                meta, text = read_frontmatter(cf)
                spawns = meta.get("spawns")
                if spawns:
                    agent_base = spawns.replace("-agent", "")
                    if agent_base not in skill_names:
                        errors.append(
                            f"[{edition}] {cf}: spawns '{spawns}' has no matching skill"
                        )
                elif meta.get("flow") or cf.name.startswith("flow-"):
                    validate_flow(
                        edition, cf, text, roster or skill_names, write_agents, errors
                    )
            for required_skill in MEMORY_QUARTET:
                required_rel = f"{commands_rel}/{required_skill}.md"
                if required_skill in skill_names and not is_owned(files, required_rel):
                    errors.append(
                        f"[{edition}] {required_rel} is not manifest-owned"
                    )
        else:
            errors.append(f"[{edition}] selected but commands dir missing: {commands_rel}")


def validate_edition_scope(
    target: Path, editions: list, files: dict, errors: list
) -> None:
    selected = set(editions)
    for edition, roots in EDITION_ROOTS.items():
        for root_rel in roots:
            exists = (target / root_rel).exists()
            if edition in selected and not exists:
                errors.append(
                    f"[{edition}] selected but edition root missing: {root_rel}"
                )
            elif edition not in selected and any(
                rel == root_rel or rel.startswith(f"{root_rel}/") for rel in files
            ):
                errors.append(
                    f"[{edition}] manifest owns file(s) under unselected root: {root_rel}"
                )


def validate_hooks(target: Path, editions: list, files: dict, errors: list) -> None:
    for edition in editions:
        hooks_rel, _ = EDITION_HOOK_WIRING[edition]
        hd = target / hooks_rel
        if not hd.exists():
            errors.append(f"[{edition}] selected but hooks dir missing: {hooks_rel}")
            continue
        for expected in REQUIRED_HOOKS[edition]:
            expected_rel = f"{hooks_rel}/{expected}"
            if not is_owned(files, expected_rel):
                errors.append(f"[{edition}] required hook is not manifest-owned: {expected_rel}")
        read_hook_rel = f"{hooks_rel}/working-memory-read.sh"
        if edition == "cursor" and is_owned(files, read_hook_rel):
            errors.append(
                "[cursor] working-memory-read.sh generated, but Cursor has no "
                "prompt-time hook event to run it (documented divergence)"
            )
        if edition == "cursor":
            # Cursor's read path: the stop and sessionStart hooks must render
            # the capsule into the alwaysApply working-memory rule.
            for renderer in ("working-memory-write.sh", "local-context.sh"):
                renderer_rel = f"{hooks_rel}/{renderer}"
                if not is_owned(files, renderer_rel):
                    continue  # already reported as a missing required hook
                script = target / renderer_rel
                text = script.read_text(encoding="utf-8", errors="replace")
                if (
                    ".cursor/rules/working-memory.mdc" not in text
                    or "alwaysApply: true" not in text
                ):
                    errors.append(
                        f"[cursor] {hooks_rel}/{renderer} does not render the "
                        "alwaysApply working-memory rule "
                        "(.cursor/rules/working-memory.mdc) that serves as "
                        "Cursor's read path (hook-forge step 6)"
                    )
        for rel in owned_children(files, hooks_rel, ".sh"):
            sh = target / rel
            result = subprocess.run(
                ["bash", "-n", str(sh)], capture_output=True, text=True
            )
            if result.returncode != 0:
                errors.append(f"{sh}: bash -n failed: {result.stderr.strip()}")
            mode = sh.stat().st_mode
            if not (mode & 0o111):
                errors.append(f"{sh}: not executable (chmod +x needed)")


def collect_wired_commands(node) -> list:
    """Every 'command' string anywhere inside a parsed wiring JSON document."""
    commands: list = []
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "command" and isinstance(value, str):
                commands.append(value)
            else:
                commands.extend(collect_wired_commands(value))
    elif isinstance(node, list):
        for item in node:
            commands.extend(collect_wired_commands(item))
    return commands


def validate_hook_wiring(
    target: Path, editions: list, files: dict, errors: list
) -> None:
    """Every wired hook must resolve to an existing executable script.

    A wiring entry that points at a missing or non-executable file is a dead
    hook: the host tool fails the call silently and the guardrail never runs.
    """
    for edition in editions:
        _, wiring_rel = EDITION_HOOK_WIRING[edition]
        wiring_path = target / wiring_rel
        if not is_owned(files, wiring_rel):
            errors.append(f"[{edition}] hook wiring is not manifest-owned: {wiring_rel}")
            continue
        try:
            document = json.loads(wiring_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"{wiring_path}: unreadable or invalid JSON: {error}")
            continue
        commands = collect_wired_commands(document)
        if not commands:
            errors.append(f"{wiring_path}: no hook commands wired")
        for command in commands:
            # Check every token ending in .sh, not just the first: a wiring of
            # the form "bash .claude/hooks/x.sh" (against hook-forge's bare-path
            # rule) must not smuggle a dead hook past this check.
            script_tokens = [tok for tok in command.split() if tok.endswith(".sh")]
            if not script_tokens:
                continue  # non-script command (e.g. a notifier); not ours to check
            for script_token in script_tokens:
                script_path = target / script_token
                if not is_owned(files, script_token):
                    errors.append(
                        f"{wiring_path}: wired hook is not manifest-owned: {script_token}"
                    )
                elif not script_path.exists():
                    errors.append(
                        f"{wiring_path}: wired hook does not exist: {script_token}"
                    )
                elif not (script_path.stat().st_mode & 0o111):
                    errors.append(
                        f"{wiring_path}: wired hook not executable: {script_token}"
                    )
    if "codex" in editions:
        config_rel = ".codex/config.toml"
        config_toml = target / config_rel
        if not is_owned(files, config_rel):
            errors.append(
                "[codex] .codex/config.toml is not manifest-owned "
                "([features] hooks = true)"
            )
        elif not re.search(
            r"^\s*hooks\s*=\s*true\s*$",
            config_toml.read_text(encoding="utf-8"),
            re.M,
        ):
            errors.append("[codex] .codex/config.toml does not enable hooks = true")


def validate_memory_runtime(target: Path, files: dict, errors: list) -> None:
    """The context-brain runtime and project-brain skeleton, plus smoke runs."""
    scripts_dir = target / "memory-bank" / "scripts"
    missing_runtime = []
    for name in RUNTIME_SCRIPTS:
        rel = f"memory-bank/scripts/{name}"
        if not is_owned(files, rel):
            missing_runtime.append(name)
    if missing_runtime:
        errors.append(
            "memory-bank/scripts/ runtime incomplete, missing: "
            + ", ".join(missing_runtime)
        )

    brain = target / "project-brain"
    if not any(rel.startswith("project-brain/") for rel in files):
        errors.append("project-brain/ has no manifest-owned generated files")
        return
    for rel in (
        "PROTOCOL.md",
        "config/runtime.json",
        "scripts/validate.py",
    ):
        owned_rel = f"project-brain/{rel}"
        if not is_owned(files, owned_rel):
            errors.append(f"{owned_rel} is not manifest-owned")
    if not any(
        rel.startswith("project-brain/schemas/") and rel.endswith(".schema.json")
        for rel in files
    ):
        errors.append("project-brain/schemas/ has no manifest-owned *.schema.json files")

    runtime_json = brain / "config" / "runtime.json"
    if is_owned(files, "project-brain/config/runtime.json") and runtime_json.exists():
        raw = runtime_json.read_text(encoding="utf-8")
        if "{{" in raw:
            errors.append(
                "project-brain/config/runtime.json: unsubstituted template placeholder"
            )
        try:
            config = json.loads(raw)
        except json.JSONDecodeError as error:
            errors.append(f"project-brain/config/runtime.json: invalid JSON: {error}")
        else:
            framework = config.get("framework")
            if not isinstance(framework, str) or not framework.strip():
                errors.append(
                    "project-brain/config/runtime.json: 'framework' must be a "
                    "non-empty slug (use 'generic' when none is confirmed)"
                )
            canonical = config.get("canonical_edition")
            if not isinstance(canonical, str) or not canonical.strip():
                errors.append(
                    "project-brain/config/runtime.json: 'canonical_edition' "
                    "must name a generated edition root (.agents/.claude/.cursor)"
                )
            elif not (target / canonical / "skills").is_dir():
                errors.append(
                    f"project-brain/config/runtime.json: canonical_edition "
                    f"{canonical!r} has no skills tree in the target - "
                    "'context.py parity' would report every mirror file as "
                    "drift (memory-seed must substitute a generated edition)"
                )

    if missing_runtime:
        return  # smoke runs cannot succeed without the runtime
    context_cli = scripts_dir / "context.py"
    for arguments, label in ((["status"], "status"), (["validate"], "validate")):
        try:
            result = subprocess.run(
                [sys.executable, str(context_cli), *arguments],
                capture_output=True,
                text=True,
                cwd=str(target),
                timeout=120,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            errors.append(f"context.py {label} smoke run failed to execute: {error}")
            continue
        if result.returncode != 0:
            detail = (result.stderr.strip() or result.stdout.strip())[:400]
            errors.append(f"context.py {label} exited {result.returncode}: {detail}")


def validate_manifest(target: Path, errors: list) -> dict:
    """The .infra-manifest.json upgrade contract, verified post-generation."""
    manifest_path = target / MANIFEST_NAME
    if not manifest_path.exists():
        errors.append(
            f"{MANIFEST_NAME} missing at the target root - infra-generate must "
            "write it (see 'Version Stamp & Generation Manifest' in its "
            "SKILL.md); without it infra-update can never upgrade this target"
        )
        return {}
    try:
        manifest = load_manifest(target)
    except (OSError, OwnershipError) as error:
        errors.append(str(error))
        return {}

    if manifest.get("manifest_version") != 1:
        errors.append(
            f"{MANIFEST_NAME}: manifest_version must be 1, "
            f"got {manifest.get('manifest_version')!r}"
        )

    version = manifest.get("generator_version")
    if not isinstance(version, str) or not re.fullmatch(r"\d+\.\d+\.\d+", version):
        errors.append(
            f"{MANIFEST_NAME}: generator_version must be semver, got {version!r}"
        )
    task = manifest.get("task")
    if not isinstance(task, str) or not re.fullmatch(r"TASK-\d+", task):
        errors.append(f"{MANIFEST_NAME}: task must be a TASK-<number> id, got {task!r}")
    for key in ("generator", "profile"):
        value = manifest.get(key)
        if not isinstance(value, str) or not value.strip():
            errors.append(f"{MANIFEST_NAME}: '{key}' must be a non-empty string")

    # Generation mode describes collision handling, not ownership breadth.
    # In both modes, only explicit manifest membership establishes ownership.
    mode = manifest.get("mode", "full")
    if mode not in ("full", "merge"):
        errors.append(f"{MANIFEST_NAME}: 'mode' must be 'full' or 'merge', got {mode!r}")
        mode = "full"

    files = manifest.get("files")
    if not isinstance(files, dict) or not files:
        errors.append(f"{MANIFEST_NAME}: 'files' must be a non-empty object")
        files = {}

    manifest_editions = manifest.get("editions")
    if (
        not isinstance(manifest_editions, list)
        or not manifest_editions
        or any(item not in EDITION_LAYOUT for item in manifest_editions)
        or len(set(manifest_editions)) != len(manifest_editions)
    ):
        errors.append(
            f"{MANIFEST_NAME}: 'editions' must be a non-empty unique list "
            "containing only claude, cursor, and/or codex"
        )

    # Optional decisions map (written only by infra-update): records standing
    # per-file human decisions (kept/merged) that outlive the run, so a later
    # update never auto-replaces a file the user explicitly chose to keep.
    decisions = manifest.get("decisions", {})
    if not isinstance(decisions, dict):
        errors.append(f"{MANIFEST_NAME}: 'decisions' must be an object when present")
        decisions = {}
    for rel, entry in decisions.items():
        if rel not in files:
            errors.append(
                f"{MANIFEST_NAME}: decisions entry for untracked file: {rel}"
            )
        if not isinstance(entry, dict):
            errors.append(f"{MANIFEST_NAME}: decisions[{rel!r}] must be an object")
            continue
        if entry.get("decision") not in ("kept", "merged"):
            errors.append(
                f"{MANIFEST_NAME}: decisions[{rel!r}].decision must be "
                f"'kept' or 'merged', got {entry.get('decision')!r}"
            )
        if not re.fullmatch(r"[0-9a-f]{64}", str(entry.get("rejected_sha256", ""))):
            errors.append(
                f"{MANIFEST_NAME}: decisions[{rel!r}].rejected_sha256 must be "
                "a sha256 hex digest of the declined staged version"
            )
        if not re.fullmatch(r"TASK-\d+", str(entry.get("task", ""))):
            errors.append(
                f"{MANIFEST_NAME}: decisions[{rel!r}].task must be a "
                "TASK-<number> id"
            )

    for rel, expected_sha in files.items():
        if not isinstance(rel, str) or not rel or Path(rel).is_absolute() or ".." in Path(rel).parts:
            errors.append(f"{MANIFEST_NAME}: invalid target-relative file path: {rel!r}")
            continue
        if rel == MANIFEST_NAME:
            errors.append(f"{MANIFEST_NAME}: must not track itself")
            continue
        if not may_be_manifest_owned(rel):
            errors.append(
                f"{MANIFEST_NAME}: tracks runtime state it must never own: {rel}"
            )
            continue
        if not re.fullmatch(r"[0-9a-f]{64}", str(expected_sha)):
            errors.append(
                f"{MANIFEST_NAME}: files[{rel!r}] must be a sha256 hex digest"
            )
            continue
        try:
            path = confined_target_path(target, rel)
        except OwnershipError as error:
            errors.append(f"{MANIFEST_NAME}: unsafe tracked path {rel}: {error}")
            continue
        if not path.is_file():
            errors.append(f"{MANIFEST_NAME}: tracked file missing from target: {rel}")
            continue
        actual = sha256_file(path)
        if actual != expected_sha:
            errors.append(
                f"{MANIFEST_NAME}: sha256 mismatch for {rel} "
                "(manifest is stale - refresh it after content auto-fixes)"
            )

    # A team-owned AGENTS.md is never opened. A manifest-owned AGENTS.md must
    # carry the matching generation stamp in both full and merge modes.
    agents = target / "AGENTS.md"
    if "AGENTS.md" in files and agents.is_file():
        lines = agents.read_text(encoding="utf-8").splitlines()
        match = AGENTS_STAMP_RE.match(lines[0]) if lines else None
        if not match:
            errors.append(
                "AGENTS.md: first line must be the generation version stamp "
                "(<!-- Generated by ... vX.Y.Z | TASK-<number> | date -->)"
            )
        else:
            if isinstance(version, str) and match.group(1) != version:
                errors.append(
                    f"AGENTS.md stamp version {match.group(1)} != "
                    f"manifest generator_version {version}"
                )
            if isinstance(task, str) and match.group(2) != task:
                errors.append(
                    f"AGENTS.md stamp task {match.group(2)} != manifest task {task}"
                )

    # Right after a generation/update run the manifest must match the
    # generator that ran. When this script still sits inside its generator
    # tree (VERSION four levels up), require equality; standalone copies skip.
    version_file = Path(__file__).resolve().parents[4] / "VERSION"
    if version_file.is_file() and isinstance(version, str):
        current = version_file.read_text(encoding="utf-8").strip()
        if re.fullmatch(r"\d+\.\d+\.\d+", current) and current != version:
            errors.append(
                f"{MANIFEST_NAME}: generator_version {version} != "
                f"generator's VERSION {current}"
            )
    return manifest


def validate_memory_bank(target: Path, files: dict, errors: list) -> None:
    bank = target / "memory-bank"
    validator = bank / "scripts" / "validate.py"
    validator_rel = "memory-bank/scripts/validate.py"
    if not is_owned(files, validator_rel):
        errors.append(f"{validator_rel} is not manifest-owned")
        return
    result = subprocess.run(
        [sys.executable, str(validator), str(bank)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        errors.append(f"memory-bank validate.py failed: {result.stderr.strip()}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate a generated accelerator.")
    parser.add_argument("--target", required=True, help="path to the target project")
    parser.add_argument(
        "--editions",
        default="claude,cursor,codex",
        help="comma-separated editions that should exist",
    )
    args = parser.parse_args()

    target = resolve_target(args.target)
    if not target.exists():
        print(f"target not found: {target}", file=sys.stderr)
        return 2
    editions = [e.strip() for e in args.editions.split(",") if e.strip()]
    for e in editions:
        if e not in EDITION_LAYOUT:
            print(f"unknown edition: {e}", file=sys.stderr)
            return 2

    errors: list = []
    manifest = validate_manifest(target, errors)
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        print(f"\n{len(errors)} problem(s) found.", file=sys.stderr)
        return 1

    files = manifest["files"]
    if set(editions) != set(manifest.get("editions", [])):
        errors.append(
            f"{MANIFEST_NAME}: CLI editions {editions!r} do not match "
            f"manifest editions {manifest.get('editions')!r}"
        )

    validate_edition_scope(target, editions, files, errors)
    for edition in editions:
        validate_edition(target, edition, files, errors)
    validate_hooks(target, editions, files, errors)
    validate_hook_wiring(target, editions, files, errors)
    validate_memory_bank(target, files, errors)
    validate_memory_runtime(target, files, errors)
    validate_owned_placeholders(target, files, errors)

    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        print(f"\n{len(errors)} problem(s) found.", file=sys.stderr)
        return 1

    print("generated accelerator OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
