#!/usr/bin/env python3
"""Generator-asset parity: the memory-seed payload against the canonical edition.

`Infrastructure-Creator/.agents/skills/memory-seed/assets/` is copied verbatim
into every project the generator builds, so it is a fourth copy of the memory
core on top of the three PHP editions. `context.py parity --cross-edition`
holds the three editions to each other and never looks at this one, which is
exactly how the copy drifts: an engine fix lands in the editions, CI stays
green, and the next generated project gets the old engine.

What this checker enforces:

* Every mapped asset file is byte-identical to its counterpart in the
  canonical edition (sha256).
* Every canonical file the asset is supposed to seed exists in the asset - a
  new module under `memory-bank/scripts/` cannot be forgotten here.
* Every asset file is either mapped or listed in ASSET_ONLY with a reason, so
  a new asset file cannot quietly become unchecked.
* `runtime.json.template` is a placeholder file and cannot match byte-for-byte,
  so its JSON keys are compared against the edition's real `runtime.json`
  instead: a new runtime setting reaches generated projects or fails here.
* Every `required_skeleton` path in `runtime-contract.json` that names a file
  under `memory-bank/scripts/` is actually present in the asset.

Exit status: 0 when the asset matches, 1 on any finding, 2 on a usage error.
Standard library only, no network, read-only unless --write is passed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
ASSET_ROOT = (
    REPO_ROOT
    / "Infrastructure-Creator"
    / ".agents"
    / "skills"
    / "memory-seed"
    / "assets"
)

# Any of the three PHP editions will do: they are held byte-identical for the
# whole core by `context.py parity --cross-edition`, which is a prerequisite of
# this check rather than a duplicate of it. The first present one wins so a
# partial checkout still gets a comparison.
CANONICAL_EDITIONS = ("Laravel", "Symfony", "PHP Core")

# How an asset path maps onto a path inside the canonical edition. Longest
# prefix wins, so "project-brain/" maps onto itself while "scripts/" and
# "templates/" move under memory-bank/. Ordered longest-first at use time.
ASSET_TO_EDITION_PREFIX = {
    "scripts/": "memory-bank/scripts/",
    "templates/": "memory-bank/templates/",
    "project-brain/": "project-brain/",
}

# Canonical globs the asset MUST carry a copy of. This is the direction the
# prefix map cannot check: a file that exists only in the edition would
# otherwise never be looked for.
REQUIRED_FROM_EDITION = (
    "memory-bank/scripts/*.py",
    "memory-bank/templates/*",
    "project-brain/PROTOCOL.md",
    "project-brain/README.md",
    "project-brain/.gitignore",
    "project-brain/schemas/*.json",
    "project-brain/scripts/*.py",
    "project-brain/templates/*",
    "project-brain/indexes/*.json",
    "project-brain/config/providers.json",
    "project-brain/config/telemetry.json",
)

# Asset files with no byte-identical counterpart in an edition, each with the
# reason it is legitimately asset-only.
ASSET_ONLY = {
    "runtime-contract.json": (
        "machine-readable contract for the generated project's runtime; the "
        "editions describe their own runtime in prose instead"
    ),
    "project-brain/config/runtime.json.template": (
        "placeholder source for the generated project's runtime.json "
        "({{TARGET_FRAMEWORK}}, {{CANONICAL_EDITION}}); compared by JSON key "
        "set against the edition's runtime.json below"
    ),
}

# Canonical files deliberately NOT seeded into a generated project.
EDITION_ONLY = {
    "project-brain/config/runtime.json": (
        "materialized in the target from runtime.json.template"
    ),
    "project-brain/tests/": "the generator emits no Python test suite into a target",
    "project-brain/.install/": "installer bookkeeping for the ready-made editions",
    "memory-bank/README.md": "written fresh per target by memory-seed",
    "memory-bank/INDEX.md": "written fresh per target by memory-seed",
    "memory-bank/chunks/": "durable memory is edition content, not a seed",
    "memory-bank/tests/": "the generator emits no Python test suite into a target",
    "memory-bank/local/": "ignored runtime state",
}

IGNORED_NAMES = frozenset({"__pycache__", ".DS_Store"})
IGNORED_SUFFIXES = (".pyc",)

# The template is a placeholder file, so these keys are expected to hold
# {{...}} markers rather than real values; every other key must be present on
# both sides.
TEMPLATE_REL = "project-brain/config/runtime.json.template"
TEMPLATE_TARGET_REL = "project-brain/config/runtime.json"


class Finding(Exception):
    """Not raised - Finding is the record type the checker reports."""


def is_ignored(path: Path, root: Path) -> bool:
    parts = path.relative_to(root).parts
    if any(part in IGNORED_NAMES for part in parts):
        return True
    return parts[-1].endswith(IGNORED_SUFFIXES)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def canonical_edition(repo_root: Path) -> Path:
    for name in CANONICAL_EDITIONS:
        candidate = repo_root / name
        if candidate.is_dir():
            return candidate
    raise SystemExit(
        "asset_parity: no canonical edition found next to "
        f"{repo_root}; expected one of {', '.join(CANONICAL_EDITIONS)}"
    )


def edition_path_for(asset_rel: str) -> str | None:
    """The edition-relative counterpart of an asset-relative path."""
    for prefix in sorted(ASSET_TO_EDITION_PREFIX, key=len, reverse=True):
        if asset_rel.startswith(prefix):
            return ASSET_TO_EDITION_PREFIX[prefix] + asset_rel[len(prefix) :]
    return None


def asset_path_for(edition_rel: str) -> str | None:
    """The asset-relative counterpart of an edition-relative path."""
    for asset_prefix, edition_prefix in sorted(
        ASSET_TO_EDITION_PREFIX.items(), key=lambda item: len(item[1]), reverse=True
    ):
        if edition_rel.startswith(edition_prefix):
            return asset_prefix + edition_rel[len(edition_prefix) :]
    return None


def edition_only_reason(edition_rel: str) -> str | None:
    for entry, reason in EDITION_ONLY.items():
        if edition_rel == entry or (entry.endswith("/") and edition_rel.startswith(entry)):
            return reason
    return None


def asset_files(asset_root: Path) -> list[str]:
    return sorted(
        path.relative_to(asset_root).as_posix()
        for path in asset_root.rglob("*")
        if path.is_file() and not path.is_symlink() and not is_ignored(path, asset_root)
    )


def compare_template(asset_root: Path, edition: Path) -> list[dict[str, str]]:
    """Compare the placeholder template's key set against the real runtime.json."""
    template = asset_root / TEMPLATE_REL
    target = edition / TEMPLATE_TARGET_REL
    if not template.is_file() or not target.is_file():
        return []
    try:
        template_keys = set(json.loads(template.read_text(encoding="utf-8")))
        target_keys = set(json.loads(target.read_text(encoding="utf-8")))
    except (OSError, ValueError) as error:
        return [{"path": TEMPLATE_REL, "reason": f"unreadable JSON ({error})"}]
    findings = []
    for missing in sorted(target_keys - template_keys):
        findings.append(
            {
                "path": TEMPLATE_REL,
                "reason": (
                    f"runtime setting {missing!r} exists in the edition's "
                    "runtime.json but not in the template, so generated "
                    "projects never get it"
                ),
            }
        )
    for extra in sorted(template_keys - target_keys):
        findings.append(
            {
                "path": TEMPLATE_REL,
                "reason": (
                    f"template declares {extra!r}, which the edition's "
                    "runtime.json does not have"
                ),
            }
        )
    return findings


def check_runtime_contract(asset_root: Path) -> list[dict[str, str]]:
    """Every core module the contract promises a target must exist in the asset."""
    contract = asset_root / "runtime-contract.json"
    if not contract.is_file():
        return [{"path": "runtime-contract.json", "reason": "missing"}]
    try:
        skeleton = json.loads(contract.read_text(encoding="utf-8"))["path_contracts"][
            "required_skeleton"
        ]
    except (OSError, ValueError, KeyError) as error:
        return [
            {
                "path": "runtime-contract.json",
                "reason": f"path_contracts.required_skeleton unreadable ({error})",
            }
        ]
    findings = []
    for entry in skeleton:
        if not entry.startswith("memory-bank/scripts/") or entry.endswith("/"):
            continue
        asset_rel = asset_path_for(entry)
        if asset_rel and not (asset_root / asset_rel).is_file():
            findings.append(
                {
                    "path": asset_rel,
                    "reason": (
                        f"runtime-contract.json requires {entry} in every "
                        "generated project, but the asset does not ship it"
                    ),
                }
            )
    return findings


def collect_findings(repo_root: Path, asset_root: Path) -> list[dict[str, str]]:
    edition = canonical_edition(repo_root)
    findings: list[dict[str, str]] = []

    for asset_rel in asset_files(asset_root):
        if asset_rel in ASSET_ONLY:
            continue
        edition_rel = edition_path_for(asset_rel)
        if edition_rel is None:
            findings.append(
                {
                    "path": asset_rel,
                    "reason": (
                        "asset file maps onto no canonical path and is not "
                        "listed in ASSET_ONLY with a reason"
                    ),
                }
            )
            continue
        counterpart = edition / edition_rel
        if not counterpart.is_file():
            findings.append(
                {
                    "path": asset_rel,
                    "reason": f"no counterpart at {edition.name}/{edition_rel}",
                }
            )
            continue
        if digest(asset_root / asset_rel) != digest(counterpart):
            findings.append(
                {
                    "path": asset_rel,
                    "reason": f"content differs from {edition.name}/{edition_rel}",
                }
            )

    for pattern in REQUIRED_FROM_EDITION:
        for path in sorted(edition.glob(pattern)):
            if not path.is_file() or path.is_symlink() or is_ignored(path, edition):
                continue
            edition_rel = path.relative_to(edition).as_posix()
            if edition_only_reason(edition_rel):
                continue
            asset_rel = asset_path_for(edition_rel)
            if asset_rel is None or not (asset_root / asset_rel).is_file():
                findings.append(
                    {
                        "path": asset_rel or edition_rel,
                        "reason": (
                            f"{edition.name}/{edition_rel} is seeded into every "
                            "generated project but is missing from the asset"
                        ),
                    }
                )

    findings.extend(compare_template(asset_root, edition))
    findings.extend(check_runtime_contract(asset_root))
    return sorted(findings, key=lambda item: (item["path"], item["reason"]))


def sync(repo_root: Path, asset_root: Path, findings: list[dict[str, str]]) -> int:
    """Copy the canonical bytes over the asset for content/missing findings."""
    edition = canonical_edition(repo_root)
    copied = 0
    for finding in findings:
        asset_rel = finding["path"]
        reason = finding["reason"]
        if not (reason.startswith("content differs") or "missing from the asset" in reason):
            print(f"  skip {asset_rel}: {reason} (needs a human decision)")
            continue
        edition_rel = edition_path_for(asset_rel)
        if edition_rel is None:
            print(f"  skip {asset_rel}: no canonical counterpart")
            continue
        source = edition / edition_rel
        if not source.is_file():
            print(f"  skip {asset_rel}: {edition.name}/{edition_rel} does not exist")
            continue
        destination = asset_root / asset_rel
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        print(f"  wrote {asset_rel} from {edition.name}/{edition_rel}")
        copied += 1
    return copied


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Check the memory-seed generator asset against the canonical "
            "edition, so an engine fix reaches generated projects too."
        )
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--check",
        action="store_true",
        help="report drift and exit non-zero if any remains (default)",
    )
    mode.add_argument(
        "--write",
        action="store_true",
        help=(
            "copy canonical bytes over drifted asset files, then re-check. "
            "Findings that need a human decision are reported, not written."
        ),
    )
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    arguments = parser.parse_args(argv)

    if not ASSET_ROOT.is_dir():
        print(f"asset_parity: asset tree not found at {ASSET_ROOT}", file=sys.stderr)
        return 2

    findings = collect_findings(REPO_ROOT, ASSET_ROOT)

    if arguments.write and findings:
        print(f"Generator-asset parity: syncing {len(findings)} finding(s).")
        sync(REPO_ROOT, ASSET_ROOT, findings)
        findings = collect_findings(REPO_ROOT, ASSET_ROOT)

    if arguments.json:
        print(
            json.dumps(
                {
                    "asset": ASSET_ROOT.relative_to(REPO_ROOT).as_posix(),
                    "canonical_edition": canonical_edition(REPO_ROOT).name,
                    "findings": findings,
                },
                ensure_ascii=False,
            )
        )
    elif findings:
        print(f"Generator-asset parity drift ({len(findings)} finding(s)):")
        for finding in findings:
            print(f"  {finding['path']}: {finding['reason']}")
        print(
            "\nFix by bringing the asset to canon "
            "(python3 scripts/asset_parity.py --write), or record a justified "
            "divergence in ASSET_ONLY / EDITION_ONLY in this script."
        )
    else:
        print(
            "Generator-asset parity passed "
            f"(canonical edition: {canonical_edition(REPO_ROOT).name})."
        )

    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
