#!/usr/bin/env python3
"""Structural QA gate for a freshly generated accelerator.

Run by bootstrap-verifier at the end of infra-generate against the TARGET
project. Dependency-free (standard library only).

Usage:
    python3 validate_generated.py --target /path/to/target [--editions claude,cursor,codex]

Checks:
  - Every generated SKILL.md / agent / command has valid frontmatter.
  - Every flow-next / flow-alternatives / related / invokes / spawns reference
    resolves to a skill/agent that actually exists in the same edition.
  - Every selected edition root exists and no unselected edition root exists.
  - Every hook script passes `bash -n` and carries the executable bit, and the
    expected hook set is complete per edition (working-memory-read.sh is
    deliberately absent from Cursor - it has no prompt-time hook event).
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
    matching sha256, every generator-owned file on disk is tracked (coverage;
    enforced for mode "full" only - merge generations track just the files
    they created), and AGENTS.md's first line is the version stamp matching
    the manifest (skipped when a merge-mode target's AGENTS.md pre-existed
    and is untracked).
  - No template placeholders remain in any generated file.

Exit code 0 = pass, non-zero = failures (printed to stderr).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

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
# (hook-forge step 6).
BASE_HOOKS = (
    "local-context.sh",
    "bash-validator.sh",
    "file-naming-validator.sh",
    "loop-detection.sh",
    "working-memory-write.sh",
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

# The upgrade contract infra-generate writes and infra-update consumes.
MANIFEST_NAME = ".infra-manifest.json"

# Runtime state the manifest must never track: it belongs to the target team
# from the moment it is seeded, and infra-update must never touch it. This
# list mirrors the manifest recipe in infra-generate's SKILL.md exactly.
MANIFEST_STATE_EXCLUDES = (
    "memory-bank/chunks/",
    "memory-bank/INDEX.md",
    "memory-bank/.memory-counter",
    "memory-bank/local/",
    "project-brain/indexes/",
    "project-brain/local/",
    "project-brain/archive/",
    "project-brain/control/",
    "project-brain/dynamic/",
)

# Roots whose non-state files count as generator-owned for manifest coverage.
MANIFEST_INCLUDE_ROOTS = (
    ".claude",
    ".cursor",
    ".agents",
    ".codex",
    "memory-bank",
    "project-brain",
)
MANIFEST_ROOT_FILES = ("AGENTS.md",)

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


def collect_skill_names(skills_dir: Path) -> set:
    names = set()
    if not skills_dir.exists():
        return names
    for child in skills_dir.iterdir():
        if child.is_dir() and (child / "SKILL.md").exists():
            names.add(child.name)
    return names


def check_placeholders(path: Path, errors: list) -> None:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return
    for pat in PLACEHOLDER_PATTERNS:
        if pat.search(text):
            errors.append(f"{path}: leftover placeholder matching /{pat.pattern}/")


def validate_edition(target: Path, edition: str, errors: list) -> None:
    skills_rel, agents_rel, commands_rel = EDITION_LAYOUT[edition]
    skills_dir = target / skills_rel
    if not skills_dir.exists():
        errors.append(f"[{edition}] selected but skills dir missing: {skills_rel}")
        return
    skill_names = collect_skill_names(skills_dir)
    if not skill_names:
        errors.append(f"[{edition}] no skills generated under {skills_rel}")
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
        check_placeholders(sp, errors)

    # Agents (editions that carry them).
    if agents_rel:
        agents_dir = target / agents_rel
        if agents_dir.exists():
            for af in agents_dir.glob("*.md"):
                if af.name == "README.md":
                    continue
                meta, _ = read_frontmatter(af)
                if not meta.get("name"):
                    errors.append(f"[{edition}] {af}: agent missing 'name'")
                invokes = meta.get("invokes")
                if invokes and invokes not in skill_names:
                    errors.append(
                        f"[{edition}] {af}: invokes '{invokes}' is not a generated skill"
                    )
                check_placeholders(af, errors)
            for required_skill in MEMORY_QUARTET:
                if required_skill in skill_names and not (
                    agents_dir / f"{required_skill}-agent.md"
                ).exists():
                    errors.append(
                        f"[{edition}] {required_skill}-agent.md missing for memory-layer skill"
                    )
        else:
            errors.append(f"[{edition}] selected but agents dir missing: {agents_rel}")

    # Commands (editions that carry them).
    if commands_rel:
        commands_dir = target / commands_rel
        if commands_dir.exists():
            for cf in commands_dir.glob("*.md"):
                if cf.name == "README.md":
                    continue
                meta, _ = read_frontmatter(cf)
                spawns = meta.get("spawns")
                if spawns:
                    agent_base = spawns.replace("-agent", "")
                    if agent_base not in skill_names:
                        errors.append(
                            f"[{edition}] {cf}: spawns '{spawns}' has no matching skill"
                        )
                check_placeholders(cf, errors)
            for required_skill in MEMORY_QUARTET:
                if required_skill in skill_names and not (
                    commands_dir / f"{required_skill}.md"
                ).exists():
                    errors.append(
                        f"[{edition}] {required_skill}.md command missing for memory-layer skill"
                    )
        else:
            errors.append(f"[{edition}] selected but commands dir missing: {commands_rel}")


def validate_edition_scope(target: Path, editions: list, errors: list) -> None:
    selected = set(editions)
    for edition, roots in EDITION_ROOTS.items():
        for root_rel in roots:
            exists = (target / root_rel).exists()
            if edition in selected and not exists:
                errors.append(
                    f"[{edition}] selected but edition root missing: {root_rel}"
                )
            elif edition not in selected and exists:
                errors.append(
                    f"[{edition}] unselected edition root exists: {root_rel}"
                )


def validate_hooks(target: Path, editions: list, errors: list) -> None:
    for edition in editions:
        hooks_rel, _ = EDITION_HOOK_WIRING[edition]
        hd = target / hooks_rel
        if not hd.exists():
            errors.append(f"[{edition}] selected but hooks dir missing: {hooks_rel}")
            continue
        for expected in REQUIRED_HOOKS[edition]:
            if not (hd / expected).exists():
                errors.append(f"[{edition}] required hook missing: {hooks_rel}/{expected}")
        if edition == "cursor" and (hd / "working-memory-read.sh").exists():
            errors.append(
                "[cursor] working-memory-read.sh generated, but Cursor has no "
                "prompt-time hook event to run it (documented divergence)"
            )
        if edition == "cursor":
            # Cursor's read path: the stop and sessionStart hooks must render
            # the capsule into the alwaysApply working-memory rule.
            for renderer in ("working-memory-write.sh", "local-context.sh"):
                script = hd / renderer
                if not script.exists():
                    continue  # already reported as a missing required hook
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
        for sh in hd.glob("*.sh"):
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


def validate_hook_wiring(target: Path, editions: list, errors: list) -> None:
    """Every wired hook must resolve to an existing executable script.

    A wiring entry that points at a missing or non-executable file is a dead
    hook: the host tool fails the call silently and the guardrail never runs.
    """
    for edition in editions:
        _, wiring_rel = EDITION_HOOK_WIRING[edition]
        wiring_path = target / wiring_rel
        if not wiring_path.exists():
            errors.append(f"[{edition}] hook wiring file missing: {wiring_rel}")
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
                if not script_path.exists():
                    errors.append(
                        f"{wiring_path}: wired hook does not exist: {script_token}"
                    )
                elif not (script_path.stat().st_mode & 0o111):
                    errors.append(
                        f"{wiring_path}: wired hook not executable: {script_token}"
                    )
    if "codex" in editions:
        config_toml = target / ".codex/config.toml"
        if not config_toml.exists():
            errors.append("[codex] .codex/config.toml missing ([features] hooks = true)")
        elif not re.search(
            r"^\s*hooks\s*=\s*true\s*$",
            config_toml.read_text(encoding="utf-8"),
            re.M,
        ):
            errors.append("[codex] .codex/config.toml does not enable hooks = true")


def validate_memory_runtime(target: Path, errors: list) -> None:
    """The context-brain runtime and project-brain skeleton, plus smoke runs."""
    scripts_dir = target / "memory-bank" / "scripts"
    missing_runtime = [
        name for name in RUNTIME_SCRIPTS if not (scripts_dir / name).exists()
    ]
    if missing_runtime:
        errors.append(
            "memory-bank/scripts/ runtime incomplete, missing: "
            + ", ".join(missing_runtime)
        )

    brain = target / "project-brain"
    if not brain.exists():
        errors.append("project-brain/ was not generated")
        return
    for rel in (
        "PROTOCOL.md",
        "config/runtime.json",
        "indexes/active.json",
        "indexes/archive.json",
        "scripts/validate.py",
    ):
        if not (brain / rel).exists():
            errors.append(f"project-brain/{rel} missing")
    schemas_dir = brain / "schemas"
    if not schemas_dir.is_dir() or not any(schemas_dir.glob("*.schema.json")):
        errors.append("project-brain/schemas/ has no *.schema.json files")

    runtime_json = brain / "config" / "runtime.json"
    if runtime_json.exists():
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


def manifest_tracks(rel: str) -> bool:
    """Whether a path is generator-owned per the manifest contract.

    Mirrors the include/exclude logic of the manifest recipe in
    infra-generate's SKILL.md - keep the two in sync.
    """
    if rel == MANIFEST_NAME or "__pycache__" in rel or rel.endswith(".pyc"):
        return False
    return not any(
        rel == entry.rstrip("/") or rel.startswith(entry)
        for entry in MANIFEST_STATE_EXCLUDES
    )


def validate_manifest(target: Path, errors: list) -> None:
    """The .infra-manifest.json upgrade contract, verified post-generation."""
    manifest_path = target / MANIFEST_NAME
    if not manifest_path.exists():
        errors.append(
            f"{MANIFEST_NAME} missing at the target root - infra-generate must "
            "write it (see 'Version Stamp & Generation Manifest' in its "
            "SKILL.md); without it infra-update can never upgrade this target"
        )
        return
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        errors.append(f"{MANIFEST_NAME}: unreadable or invalid JSON: {error}")
        return

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

    # Generation mode: "full" (overwrite/fresh - full coverage enforced) or
    # "merge" (only files the generator actually created are tracked; the
    # team's pre-existing files stay untracked, so coverage is NOT enforced).
    # Absent means "full" for manifests written before the mode field existed.
    mode = manifest.get("mode", "full")
    if mode not in ("full", "merge"):
        errors.append(f"{MANIFEST_NAME}: 'mode' must be 'full' or 'merge', got {mode!r}")
        mode = "full"

    files = manifest.get("files")
    if not isinstance(files, dict) or not files:
        errors.append(f"{MANIFEST_NAME}: 'files' must be a non-empty object")
        files = {}

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

    for rel, expected_sha in files.items():
        if rel == MANIFEST_NAME:
            errors.append(f"{MANIFEST_NAME}: must not track itself")
            continue
        if not manifest_tracks(rel):
            errors.append(
                f"{MANIFEST_NAME}: tracks runtime state it must never own: {rel}"
            )
            continue
        path = target / rel
        if not path.is_file():
            errors.append(f"{MANIFEST_NAME}: tracked file missing from target: {rel}")
            continue
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected_sha:
            errors.append(
                f"{MANIFEST_NAME}: sha256 mismatch for {rel} "
                "(manifest is stale - refresh it after content auto-fixes)"
            )

    # Coverage: every generator-owned file on disk must be tracked. Enforced
    # only for mode "full" - a merge generation deliberately leaves the team's
    # pre-existing files untracked, so untracked files under the roots are
    # expected there, not a coverage failure.
    if mode == "full":
        for rel in MANIFEST_ROOT_FILES:
            if (target / rel).is_file() and rel not in files:
                errors.append(f"{MANIFEST_NAME}: generated file not tracked: {rel}")
        for root in MANIFEST_INCLUDE_ROOTS:
            base = target / root
            if not base.is_dir():
                continue
            for path in sorted(base.rglob("*")):
                if not path.is_file() or path.is_symlink():
                    continue
                rel = path.relative_to(target).as_posix()
                if manifest_tracks(rel) and rel not in files:
                    errors.append(f"{MANIFEST_NAME}: generated file not tracked: {rel}")

    # The AGENTS.md version stamp must agree with the manifest. In merge mode
    # a pre-existing AGENTS.md is the team's (untracked, never touched), so
    # the stamp is only required when the manifest tracks the file.
    agents = target / "AGENTS.md"
    if agents.is_file() and (mode == "full" or "AGENTS.md" in files):
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


def validate_memory_bank(target: Path, errors: list) -> None:
    bank = target / "memory-bank"
    validator = bank / "scripts" / "validate.py"
    if not bank.exists():
        errors.append("memory-bank/ was not generated")
        return
    if not validator.exists():
        errors.append("memory-bank/scripts/validate.py missing")
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

    target = Path(args.target)
    if not target.exists():
        print(f"target not found: {target}", file=sys.stderr)
        return 2
    editions = [e.strip() for e in args.editions.split(",") if e.strip()]
    for e in editions:
        if e not in EDITION_LAYOUT:
            print(f"unknown edition: {e}", file=sys.stderr)
            return 2

    errors: list = []

    if not (target / "AGENTS.md").exists():
        errors.append("target AGENTS.md was not generated")

    validate_edition_scope(target, editions, errors)
    for edition in editions:
        validate_edition(target, edition, errors)
    validate_hooks(target, editions, errors)
    validate_hook_wiring(target, editions, errors)
    validate_memory_bank(target, errors)
    validate_memory_runtime(target, errors)
    validate_manifest(target, errors)
    check_placeholders(target / "AGENTS.md", errors)

    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        print(f"\n{len(errors)} problem(s) found.", file=sys.stderr)
        return 1

    print("generated accelerator OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
