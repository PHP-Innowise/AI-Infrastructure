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
  - Every hook script passes `bash -n` and carries the executable bit.
  - The seeded memory-bank passes its own scripts/validate.py.
  - No template placeholders remain in any generated file.

Exit code 0 = pass, non-zero = failures (printed to stderr).
"""

from __future__ import annotations

import argparse
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
]

# Edition -> (skills dir relative to target, has_agents, has_commands)
EDITION_LAYOUT = {
    "claude": (".claude/skills", ".claude/agents", ".claude/commands"),
    "cursor": (".cursor/skills", ".cursor/agents", ".cursor/commands"),
    "codex": (".agents/skills", None, None),
}


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


def validate_hooks(target: Path, editions: list, errors: list) -> None:
    hook_dirs = []
    for edition in editions:
        if edition == "claude":
            hook_dirs.append(target / ".claude/hooks")
        elif edition == "cursor":
            hook_dirs.append(target / ".cursor/hooks")
        elif edition == "codex":
            hook_dirs.append(target / ".codex/hooks")
    for hd in hook_dirs:
        if not hd.exists():
            continue
        for sh in hd.glob("*.sh"):
            result = subprocess.run(
                ["bash", "-n", str(sh)], capture_output=True, text=True
            )
            if result.returncode != 0:
                errors.append(f"{sh}: bash -n failed: {result.stderr.strip()}")
            mode = sh.stat().st_mode
            if not (mode & 0o111):
                errors.append(f"{sh}: not executable (chmod +x needed)")


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
        [sys.executable, str(validator), "--bank", str(bank)],
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

    for edition in editions:
        validate_edition(target, edition, errors)
    validate_hooks(target, editions, errors)
    validate_memory_bank(target, errors)
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
