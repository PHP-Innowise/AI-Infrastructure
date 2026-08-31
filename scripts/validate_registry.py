#!/usr/bin/env python3
"""Validate the Kit 3 risk registry and recompute every status.

Each `install/open-source-kit/registry/<id>.json` records how one tool answered
the twelve gates. Eight gates are binary - answered pass/fail/unknown - and four
are scored 0-5.

The registry describes; it does not forbid. Nothing here refuses a tool: the
selector installs nothing either way, so a refusal would only block writing the
choice down, and an install that happens anyway would then be absent from the
audit trail. What the registry produces is a dossier - what was checked, what
was found, and what would resolve each open item - and the team decides.

The status is stored in the file so it is greppable and reviewable, but it is
never trusted: this script recomputes it from the gates and fails when the two
disagree. A stored `clear` cannot outrank a failing gate.

    python3 scripts/validate_registry.py            # report every entry
    python3 scripts/validate_registry.py --check    # CI gate, non-zero on any error
    python3 scripts/validate_registry.py --id graphify
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
KIT_DIR = ROOT / "install" / "open-source-kit"
REGISTRY_DIR = KIT_DIR / "registry"
CATALOG = KIT_DIR / "resources.json"

BINARY_GATES = (
    "license",
    "data_egress",
    "pinning",
    "uninstall",
    "collisions",
    "auto_update",
    "maintenance_ownership",
    "measurability",
)
SCORED_GATES = (
    "automation_depth",
    "token_efficiency",
    "integration_coverage",
    "trust_signals",
)
GATE_STATUSES = {"pass", "fail", "unknown"}
# Descriptive, not prescriptive: each says what the gates found, never whether
# the tool is allowed.
STATUSES = {"clear", "open_questions", "known_risks"}
RELATIONS = {"duplicates", "replaces", "conflicts", "complements"}
DEFAULT_STATES = {"enabled", "disabled"}
TIERS = {1, 2, 3}

TOP_LEVEL_FIELDS = (
    "schema_version",
    "id",
    "catalog_id",
    "name",
    "url",
    "reviewed_date",
    "reviewed_by",
    "status",
    "status_summary",
    "tier",
    "default_state",
    "install",
    "binary_gates",
    "scored_gates",
    "intersection_map",
    "lifecycle",
)
INSTALL_FIELDS = ("method", "pinned_ref", "namespace_prefix", "command")
LIFECYCLE_FIELDS = ("uninstall_command", "residue", "manifest_tracked")


class RegistryError(Exception):
    """Raised when the registry directory itself cannot be read."""


def stated(value: object) -> bool:
    """True when a field carries actual text.

    Whitespace is not evidence: `"   "` is truthy in Python but says nothing,
    and these fields exist to make a reviewer commit to a claim in writing.
    """
    return isinstance(value, str) and bool(value.strip())


def compute_status(binary_gates: dict) -> str:
    """Summarise what the binary gates found. A description, not a decision.

    A failure is a risk we found and wrote down. An unknown is a question
    nobody has answered yet - reported as its own state rather than folded into
    the clear one, because absence of evidence is not evidence of safety.
    """
    statuses = {name: gate.get("status") for name, gate in binary_gates.items()}
    if any(status == "fail" for status in statuses.values()):
        return "known_risks"
    if any(status == "unknown" for status in statuses.values()):
        return "open_questions"
    return "clear"


def validate_entry(data: dict, catalog_ids: set[str]) -> list[str]:
    """Return every problem found in one registry entry."""
    errors: list[str] = []
    entry_id = data.get("id", "<no id>")

    def err(message: str) -> None:
        errors.append(f"{entry_id}: {message}")

    for field in TOP_LEVEL_FIELDS:
        if field not in data:
            err(f"missing required field `{field}`")
    if errors:
        return errors

    if data["schema_version"] != 1:
        err(f"unsupported schema_version {data['schema_version']}")
    if data["tier"] not in TIERS:
        err(f"tier must be one of {sorted(TIERS)}, got {data['tier']!r}")
    if data["default_state"] not in DEFAULT_STATES:
        err(f"default_state must be one of {sorted(DEFAULT_STATES)}")
    if data["status"] not in STATUSES:
        err(f"status must be one of {sorted(STATUSES)}")
    if data["catalog_id"] not in catalog_ids:
        err(
            f"catalog_id {data['catalog_id']!r} is not in "
            "install/open-source-kit/resources.json"
        )

    # Type-guard the blocks before indexing them. A hand-edited file that puts a
    # string where an object belongs must produce a diagnostic, not a traceback:
    # in --check this runs in CI, where a crash reads as tooling failure rather
    # than as the invalid input it is.
    for block in ("install", "binary_gates", "scored_gates", "lifecycle"):
        if not isinstance(data[block], dict):
            err(f"`{block}` must be an object, got {type(data[block]).__name__}")
    if errors:
        return errors

    for field in INSTALL_FIELDS:
        if field not in data["install"]:
            err(f"install block missing `{field}`")
    for field in LIFECYCLE_FIELDS:
        if field not in data["lifecycle"]:
            err(f"lifecycle block missing `{field}`")

    binary = data["binary_gates"]
    missing = set(BINARY_GATES) - set(binary)
    unexpected = set(binary) - set(BINARY_GATES)
    if missing:
        err(f"binary_gates missing: {sorted(missing)}")
    if unexpected:
        err(f"binary_gates has unknown gates: {sorted(unexpected)}")

    for name, gate in binary.items():
        if name not in BINARY_GATES:
            continue
        if not isinstance(gate, dict):
            err(f"gate `{name}` must be an object, got {type(gate).__name__}")
            continue
        status = gate.get("status")
        if status not in GATE_STATUSES:
            err(f"gate `{name}` status must be one of {sorted(GATE_STATUSES)}")
            continue
        if not stated(gate.get("evidence")):
            err(f"gate `{name}` has no evidence")
        # A gate that is not passing must say what would change that. Without
        # it a rejection is a dead end rather than a work item.
        if status in {"fail", "unknown"} and not stated(gate.get("resolves_by")):
            err(f"gate `{name}` is {status} but has no `resolves_by`")

    scored = data["scored_gates"]
    missing_scored = set(SCORED_GATES) - set(scored)
    unexpected_scored = set(scored) - set(SCORED_GATES)
    if missing_scored:
        err(f"scored_gates missing: {sorted(missing_scored)}")
    if unexpected_scored:
        err(f"scored_gates has unknown gates: {sorted(unexpected_scored)}")

    for name, gate in scored.items():
        if name not in SCORED_GATES:
            continue
        if not isinstance(gate, dict):
            err(f"scored gate `{name}` must be an object, got {type(gate).__name__}")
            continue
        score = gate.get("score")
        # `bool` is a subclass of `int`, so True would otherwise pass as 1.
        if isinstance(score, bool) or not isinstance(score, int) or not 0 <= score <= 5:
            err(f"scored gate `{name}` score must be an int 0-5, got {score!r}")
        if not stated(gate.get("evidence")):
            err(f"scored gate `{name}` has no evidence")

    # Star count is excluded from scoring by policy; when a disqualified signal
    # is recorded it must be a list of stated findings, not a bare flag.
    trust = scored.get("trust_signals")
    disqualified = trust.get("disqualified_signals") if isinstance(trust, dict) else None
    if disqualified is not None and (
        not isinstance(disqualified, list)
        or not all(stated(item) for item in disqualified)
    ):
        err("trust_signals.disqualified_signals must be a list of non-empty strings")

    # The hidden cost of Kit 3 is two systems doing one job, so an entry with no
    # intersection analysis is unreviewed whatever its gates say.
    intersections = data["intersection_map"]
    if not isinstance(intersections, list) or not intersections:
        err("intersection_map must be a non-empty list")
    else:
        for index, row in enumerate(intersections):
            if not isinstance(row, dict):
                err(f"intersection_map[{index}] must be an object")
                continue
            for field in ("ours", "relation", "detail"):
                if not stated(row.get(field)):
                    err(f"intersection_map[{index}] missing `{field}`")
            if row.get("relation") not in RELATIONS:
                err(
                    f"intersection_map[{index}] relation must be one of "
                    f"{sorted(RELATIONS)}, got {row.get('relation')!r}"
                )

    # Cross-check: unmeasured cost scores 0 and must also fail measurability.
    # Otherwise an entry could quietly claim an unmeasured tool is admissible.
    token_score = scored.get("token_efficiency", {}).get("score")
    measurability = binary.get("measurability", {}).get("status")
    if token_score == 0 and measurability == "pass":
        err(
            "token_efficiency scores 0 (unmeasured) but the measurability gate "
            "passes - one of the two is wrong"
        )

    if not errors:
        computed = compute_status(binary)
        if data["status"] != computed:
            err(
                f"stored status {data['status']!r} disagrees with the gates, "
                f"which compute {computed!r}"
            )

    return errors


def load_catalog_ids() -> set[str]:
    try:
        data = json.loads(CATALOG.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RegistryError(f"catalog unreadable: {error}") from error
    resources = data.get("resources", [])
    if not isinstance(resources, list):
        raise RegistryError("catalog `resources` must be a list")
    # Same rule as everywhere else in this file: a malformed catalog is bad
    # input and must say so, not surface as a bare KeyError traceback.
    ids = set()
    for index, entry in enumerate(resources):
        if not isinstance(entry, dict) or not entry.get("id"):
            raise RegistryError(f"catalog resource #{index} has no usable `id`")
        ids.add(entry["id"])
    return ids


def registry_files(entry_id: str | None) -> list[Path]:
    if not REGISTRY_DIR.is_dir():
        raise RegistryError(f"registry directory not found: {REGISTRY_DIR}")
    paths = sorted(REGISTRY_DIR.glob("*.json"))
    if entry_id is not None:
        paths = [path for path in paths if path.stem == entry_id]
        if not paths:
            raise RegistryError(f"no registry entry with id: {entry_id}")
    return paths


def describe(data: dict) -> str:
    binary = data["binary_gates"]
    failing = [name for name in BINARY_GATES if binary[name]["status"] == "fail"]
    unknown = [name for name in BINARY_GATES if binary[name]["status"] == "unknown"]
    lines = [
        f"{data['status'].upper():<14}\t{data['id']:<14}\t{data['name']}",
        f"  tier {data['tier']}, default {data['default_state']}, "
        f"pinned_ref {data['install']['pinned_ref'] or 'NONE'}",
    ]
    if failing:
        lines.append(f"  risks found:    {', '.join(failing)}")
    if unknown:
        lines.append(f"  open questions: {', '.join(unknown)}")
    scores = ", ".join(
        f"{name}={data['scored_gates'][name]['score']}" for name in SCORED_GATES
    )
    lines.append(f"  scores:         {scores}")
    lines.append(f"  intersections:  {len(data['intersection_map'])} recorded")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="exit non-zero on any error")
    parser.add_argument("--id", help="validate only this entry")
    args = parser.parse_args()

    try:
        catalog_ids = load_catalog_ids()
        paths = registry_files(args.id)
    except RegistryError as error:
        print(f"validate-registry: {error}", file=sys.stderr)
        return 1

    all_errors: list[str] = []
    entries = 0
    for path in paths:
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            all_errors.append(f"{path.name}: unreadable: {error}")
            continue
        entries += 1
        if not isinstance(data, dict):
            all_errors.append(
                f"{path.name}: must contain a JSON object, got {type(data).__name__}"
            )
            continue
        if data.get("id") != path.stem:
            all_errors.append(
                f"{path.name}: id {data.get('id')!r} does not match its filename"
            )
        # A malformed entry must be reported, never crash the run: --check is a
        # CI gate, where a traceback reads as broken tooling rather than as bad
        # input.
        try:
            errors = validate_entry(data, catalog_ids)
        except Exception as error:  # noqa: BLE001 - diagnostic of last resort
            errors = [f"{path.name}: could not be validated: {error!r}"]
        all_errors.extend(errors)
        if not errors and not args.check:
            print(describe(data))
            print()

    # An empty directory is not a passing gate. If entries are dropped in a bad
    # merge or renamed to another extension, --check must fail rather than
    # report VALID over nothing.
    if args.check and entries == 0 and args.id is None:
        all_errors.append(f"no registry entries found in {REGISTRY_DIR}")

    if all_errors:
        print(f"INVALID\t{len(all_errors)} problem(s) across {entries} entr(ies)", file=sys.stderr)
        for error in all_errors:
            print(f"  {error}", file=sys.stderr)
        return 1

    print(f"VALID\t{entries} registry entr(ies)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
