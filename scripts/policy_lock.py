#!/usr/bin/env python3
"""Content identity for the surface the model actually reads.

The model-facing surface is already gated on volume (`context_budget.py
--check`) and on mirror agreement (`parity`, `mirrors`), but on nothing that
says *what it is*. Consequences that were live when this was written: the
`model:` value in agent frontmatter was validated by nothing anywhere in the
repository, so `model: sonet` passed every job silently - the mirror builder
drops that line whatever it says, and the budget counts only the stem and the
description, so no byte moves; and `check_core_changelog.sh` covered the Python
core and the hooks but no skill, agent, command or policy document.

The lock is a per-edition manifest of sha256 digests plus the agent model
assignments, and a `policy_digest` over the sorted manifest. Two deliberate
choices:

* **Whole files, never projections.** The item this implements proposed
  hashing only the frontmatter of agents and commands and only two blocks of
  `settings.json`. A digest stored under a plain path key that is not
  `sha256(path)` makes every independent check a false positive - the exact
  failure the lock exists to remove. And an agent's *body* is its prompt: a
  body-only edit escapes the mirror check too, because the Cursor mirror
  copies bodies verbatim.
* **Enumerated from Git, never from disk.** `.cursor/rules/` contains a
  runtime-rendered, gitignored `working-memory.mdc` that changes every turn.
  A filesystem glob would bake it into the lock and `--check` would fail for
  ever after. The installer already encodes this reasoning: outside the index
  there is no way to tell distribution files from client data.

`policy_digest` identifies the surface, not a release. A surface change does
not require a `VERSION` bump; it requires the lock to be regenerated, which is
what CI enforces.

Exit status: 0 when the lock matches, 1 on drift or an invalid model, 2 on a
usage error. Standard library plus Git; read-only unless `--write` is passed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
EDITIONS = ("Laravel", "Symfony", "PHP Core", "Infrastructure-Creator")
LOCK_NAME = ".accelerator-policy-lock.json"
SCHEMA_VERSION = 1

# Every class of file the model reads, as pathspecs relative to an edition.
# `git ls-files` expands them against the index, so an untracked or ignored
# file cannot enter the lock.
SURFACE_PATHSPECS = (
    "AGENTS.md",
    "CLAUDE.md",
    ".claude/DOD.md",
    ".claude/GOLDEN-PRINCIPLES.md",
    ".claude/STABILIZATION.md",
    # The canonical skill tree in full, not only SKILL.md: an agent reads the
    # references and rule files under a skill exactly as it reads its body.
    ".agents/skills",
    # Canon in their own right rather than mirrors of `.agents`, and skipped
    # by the mirror class, so nothing else covers them.
    ".claude/skills/skill-creator/SKILL.md",
    ".cursor/skills/skill-creator/SKILL.md",
    ".claude/skills/SKILL FLOW.md",
    ".agents/skills/SKILL FLOW.md",
    ".claude/agents/*-agent.md",
    ".claude/commands",
    # Whole file. The `hooks` block decides whether automatic memory runs at
    # all, which is a larger policy statement than either `env` or
    # `permissions`.
    ".claude/settings.json",
    ".cursor/rules",
    ".cursor/hooks.json",
    ".codex/hooks.json",
    ".codex/config.toml",
)

# Per edition, because they legitimately differ: the three ready editions use
# haiku for four narrow agents, while Infrastructure-Creator's own agent-forge
# skill requires opus or sonnet and it ships no haiku agent at all.
MODEL_ALLOWLIST = {
    "Laravel": ("opus", "sonnet", "haiku"),
    "Symfony": ("opus", "sonnet", "haiku"),
    "PHP Core": ("opus", "sonnet", "haiku"),
    "Infrastructure-Creator": ("opus", "sonnet"),
}

FRONTMATTER_MODEL = re.compile(r"^model:\s*(?P<model>\S+)\s*$", re.M)
EDITION_SLUG = {
    "Laravel": "laravel",
    "Symfony": "symfony",
    "PHP Core": "php-core",
    "Infrastructure-Creator": "infrastructure-creator",
}


class PolicyLockError(Exception):
    """A usage or environment failure, distinct from a drift finding."""


def tracked_files(edition: str, repo_root: Path) -> list[str]:
    """Edition-relative paths of every tracked surface file, sorted."""
    pathspecs = [f"{edition}/{spec}" for spec in SURFACE_PATHSPECS]
    try:
        result = subprocess.run(
            ["git", "-C", str(repo_root), "ls-files", "-z", "--", *pathspecs],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    except OSError as error:
        raise PolicyLockError("git is required to enumerate the surface") from error
    if result.returncode != 0:
        raise PolicyLockError(
            f"git ls-files failed: {result.stderr.decode('utf-8', 'replace').strip()}"
        )
    prefix = f"{edition}/"
    paths = {
        entry[len(prefix):]
        for entry in result.stdout.decode("utf-8").split("\0")
        if entry.startswith(prefix)
    }
    # The lock itself is tracked and lives inside the edition; hashing it into
    # itself would make every regeneration change its own digest.
    paths.discard(LOCK_NAME)
    return sorted(paths)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def agent_models(edition_root: Path, paths: list[str]) -> dict[str, str]:
    """The declared model of every agent, keyed by agent file stem."""
    models: dict[str, str] = {}
    for relative in paths:
        if not relative.startswith(".claude/agents/") or not relative.endswith(
            "-agent.md"
        ):
            continue
        match = FRONTMATTER_MODEL.search(
            (edition_root / relative).read_text(encoding="utf-8")
        )
        if match is not None:
            models[Path(relative).stem] = match.group("model")
    return models


def edition_release(edition_root: Path) -> str:
    version = edition_root / "VERSION"
    if not version.is_file():
        return "unknown"
    return version.read_text(encoding="utf-8").strip()


def build_lock(edition: str, repo_root: Path) -> dict[str, object]:
    edition_root = repo_root / edition
    paths = tracked_files(edition, repo_root)
    if not paths:
        raise PolicyLockError(f"no surface files tracked for {edition}")
    files = {relative: digest(edition_root / relative) for relative in paths}
    manifest = json.dumps(files, sort_keys=True, separators=(",", ":"))
    return {
        "schema_version": SCHEMA_VERSION,
        "edition": edition,
        # Informational. `policy_digest` is the identity; the release is what
        # the edition last called itself, and the two move independently on
        # purpose.
        "release": edition_release(edition_root),
        "policy_digest": hashlib.sha256(manifest.encode("utf-8")).hexdigest(),
        "agent_models": agent_models(edition_root, paths),
        "files": files,
    }


def invalid_models(edition: str, lock: dict[str, object]) -> list[str]:
    allowed = MODEL_ALLOWLIST.get(edition, ("opus", "sonnet", "haiku"))
    models = lock.get("agent_models") or {}
    return [
        f"{agent}: {model!r} is not one of {', '.join(allowed)}"
        for agent, model in sorted(models.items())
        if model not in allowed
    ]


def lock_path(edition: str, repo_root: Path) -> Path:
    # Inside the edition, so the installer classifies it as a shared component
    # with no code change and it lands in the target beside VERSION - the only
    # place a session hook can find it without knowing its edition name.
    return repo_root / edition / LOCK_NAME


def compare(edition: str, current: dict, stored: dict) -> list[str]:
    findings: list[str] = []
    if stored.get("schema_version") != SCHEMA_VERSION:
        findings.append(
            f"lock schema_version {stored.get('schema_version')!r} "
            f"is not {SCHEMA_VERSION}"
        )
    current_files = current["files"]
    stored_files = stored.get("files") or {}
    for path in sorted(set(current_files) - set(stored_files)):
        findings.append(f"{path}: in the surface, absent from the lock")
    for path in sorted(set(stored_files) - set(current_files)):
        findings.append(f"{path}: in the lock, absent from the surface")
    for path in sorted(set(current_files) & set(stored_files)):
        if current_files[path] != stored_files[path]:
            findings.append(f"{path}: content changed")
    if current["agent_models"] != (stored.get("agent_models") or {}):
        findings.append("agent_models changed")
    if findings:
        return findings
    if current["policy_digest"] != stored.get("policy_digest"):
        findings.append(
            "policy_digest does not match a manifest that is otherwise identical"
        )
    return findings


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--check",
        action="store_true",
        help="compare the surface with the recorded lock (default)",
    )
    mode.add_argument(
        "--write", action="store_true", help="regenerate the lock for each edition"
    )
    parser.add_argument(
        "--edition",
        action="append",
        choices=EDITIONS,
        help="repeatable; defaults to every edition present",
    )
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    arguments = parser.parse_args(argv)

    editions = [
        edition
        for edition in (arguments.edition or EDITIONS)
        if (REPO_ROOT / edition).is_dir()
    ]
    if not editions:
        print("policy_lock: no editions found", file=sys.stderr)
        return 2

    report: dict[str, object] = {}
    failures = 0
    for edition in editions:
        try:
            current = build_lock(edition, REPO_ROOT)
        except (PolicyLockError, OSError) as error:
            print(f"policy_lock: {edition}: {error}", file=sys.stderr)
            return 2

        # Enforced on both paths on purpose. Validating only on --check lets a
        # regeneration record the typo and then agree with itself, which
        # defeats the whole point of validating it.
        bad_models = invalid_models(edition, current)
        path = lock_path(edition, REPO_ROOT)

        if arguments.write:
            if bad_models:
                for finding in bad_models:
                    print(f"  {edition}: {finding}")
                failures += 1
                report[edition] = {"written": False, "findings": bad_models}
                continue
            path.write_text(
                json.dumps(current, indent=2, sort_keys=True) + "\n", encoding="utf-8"
            )
            report[edition] = {
                "written": True,
                "policy_digest": current["policy_digest"],
                "files": len(current["files"]),
            }
            if not arguments.json:
                print(
                    f"WROTE\t{path.relative_to(REPO_ROOT)}\t"
                    f"{len(current['files'])} file(s)\t"
                    f"policy {current['policy_digest'][:8]}"
                )
            continue

        findings = list(bad_models)
        if not path.is_file():
            findings.append(f"{path.relative_to(REPO_ROOT)} is missing")
        else:
            try:
                stored = json.loads(path.read_text(encoding="utf-8"))
            except ValueError as error:
                findings.append(f"lock is not valid JSON: {error}")
                stored = {}
            if stored:
                findings.extend(compare(edition, current, stored))
        report[edition] = {
            "policy_digest": current["policy_digest"],
            "files": len(current["files"]),
            "findings": findings,
        }
        if findings:
            failures += 1
        if not arguments.json:
            if findings:
                print(f"FAIL\t{edition}\t{len(findings)} finding(s):")
                for finding in findings:
                    print(f"  {finding}")
            else:
                print(
                    f"ok\t{edition}\t{len(current['files'])} file(s)\t"
                    f"policy {current['policy_digest'][:8]}"
                )

    if arguments.json:
        print(json.dumps(report, ensure_ascii=False))
    elif failures and not arguments.write:
        print(
            "\nRegenerate with python3 scripts/policy_lock.py --write after an "
            "intentional surface change, and record it in that edition's "
            "CHANGELOG.md."
        )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
