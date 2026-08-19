#!/usr/bin/env python3
"""Reject stub skill-reference catalogs before a generator is accepted."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REQUIRED_CONCEPTS = {
    "selection": r"\b(select|selection|trigger|evidence-gated)\b",
    "evidence": r"\bevidence\b",
    "scope": r"\bscope\b",
    "procedure": r"\bprocedure\b",
    "decisions": r"\bdecision\b",
    "verification": r"\b(verif(?:y|ication)|tests?)\b",
    "outputs": r"\boutput\b",
    "failure": r"\bfail(?:ure|ing|s)?\b",
    "boundaries": r"\b(boundar(?:y|ies)|sibling)\b",
    "negative-example": r"\b(bad|negative|invalid)\b",
}


def heading_slug(value: str) -> str:
    value = re.sub(r"[`*_]", "", value.strip().lower())
    value = re.sub(r"[^a-z0-9 -]", "", value)
    return re.sub(r"-+", "-", re.sub(r"\s+", "-", value)).strip("-")


def _valid_falsifier(candidate: dict) -> bool:
    falsifier = candidate.get("falsifier", None)
    if falsifier is None:
        return "falsifier" not in candidate or bool(
            isinstance(candidate.get("falsifier_absent"), str)
            and candidate["falsifier_absent"].strip()
        )
    if "falsifier_absent" in candidate:
        return False
    if not isinstance(falsifier, dict) or set(falsifier) - {
        "surface", "requires", "probes", "at_least"
    }:
        return False
    if not isinstance(falsifier.get("surface"), str) or not falsifier["surface"].strip():
        return False
    if falsifier.get("requires") not in {"any", "all"}:
        return False
    at_least = falsifier.get("at_least", 1)
    if not isinstance(at_least, int) or isinstance(at_least, bool) or at_least < 1:
        return False
    probes = falsifier.get("probes")
    if not isinstance(probes, list) or not probes:
        return False
    for probe in probes:
        if not isinstance(probe, dict) or set(probe) - {"paths", "pattern"}:
            return False
        paths = probe.get("paths")
        if not isinstance(paths, list) or not paths or not all(
            isinstance(item, str) and item.strip() for item in paths
        ):
            return False
        if "pattern" in probe:
            if not isinstance(probe["pattern"], str) or not probe["pattern"].strip():
                return False
            try:
                re.compile(probe["pattern"])
            except re.error:
                return False
    return True


def _valid_roles(value) -> bool:
    """A candidate declares its mandatory reasoning roles, or declares none."""
    if value is None:
        return True
    return (
        isinstance(value, list)
        and bool(value)
        and all(isinstance(item, str) and item.strip() for item in value)
    )


def validate(directory: Path, forbidden: list[str] | None = None) -> list[str]:
    errors: list[str] = []
    directory = directory.expanduser().resolve()
    files = sorted(directory.glob("*.md")) if directory.is_dir() else []
    if len(files) < 6:
        errors.append(f"expected at least six reference catalogs, found {len(files)}")
    headings: dict[str, set[str]] = {}
    for path in files:
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as error:
            errors.append(f"{path.name}: cannot read catalog: {error}")
            continue
        headings[path.name] = {
            heading_slug(match.group(1))
            for match in re.finditer(r"^#{1,6}\s+(.+?)\s*$", text, re.M)
        }
        if len(text.split()) < 180:
            errors.append(f"{path.name}: reference catalog is a stub")
        for concept, pattern in REQUIRED_CONCEPTS.items():
            if not re.search(pattern, text, re.I):
                errors.append(f"{path.name}: missing contract concept {concept}")
        if re.search(r"\{\{[^}]+\}\}|\[Stack\]", text, re.I):
            errors.append(f"{path.name}: unresolved placeholder")
        for term in forbidden or []:
            if term.strip() and re.search(rf"\b{re.escape(term.strip())}\b", text, re.I):
                errors.append(f"{path.name}: forbidden source-stack term {term.strip()!r}")
    registry_path = directory / "candidate-registry.json"
    try:
        registry = json.loads(registry_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        errors.append(f"candidate-registry.json: missing or invalid: {error}")
        registry = {}
    candidates = registry.get("candidates") if isinstance(registry, dict) else None
    if (
        not isinstance(registry, dict)
        or registry.get("schema_version") != "1.1"
        or not isinstance(candidates, list)
    ):
        errors.append("candidate-registry.json: invalid schema")
    else:
        if not candidates:
            errors.append(
                "candidate-registry.json: candidates must list at least one entry"
            )
        seen: set[str] = set()
        for index, candidate in enumerate(candidates):
            if (
                not isinstance(candidate, dict)
                # `roles` is the optional machine-readable form of the
                # catalog's obligations for this candidate; it is filled in
                # tranches, so an entry without it stays valid.
                # `escalates_on_rejection` marks a risk-bearing family whose
                # rejection must be escalated and adversarially reviewed.
                or not {"id", "catalog", "category", "mode"} <= set(candidate)
                or set(candidate) - {
                    "id", "catalog", "category", "mode", "roles",
                    # Whether a rejection has to reach a person, and the signal
                    # whose presence in a target makes "no surface here" false -
                    # or a stated reason why no target artifact could show it.
                    "escalates_on_rejection", "falsifier", "falsifier_absent",
                }
                or not _valid_roles(candidate.get("roles"))
                or not isinstance(candidate.get("escalates_on_rejection", False), bool)
                or not _valid_falsifier(candidate)
                or candidate.get("mode") not in {"static", "family", "runtime-fixed"}
                or not all(
                    isinstance(candidate.get(field), str)
                    and candidate[field].strip()
                    for field in ("id", "catalog", "category")
                )
            ):
                errors.append(
                    f"candidate-registry.json: candidates[{index}] is invalid"
                )
                continue
            candidate_id = candidate["id"]
            if candidate_id in seen:
                errors.append(
                    f"candidate-registry.json: duplicate candidate {candidate_id}"
                )
            seen.add(candidate_id)
            if "#" not in candidate["catalog"]:
                errors.append(
                    f"candidate-registry.json: unresolved catalog for {candidate_id}"
                )
                continue
            filename, anchor = candidate["catalog"].split("#", 1)
            if anchor not in headings.get(filename, set()):
                errors.append(
                    f"candidate-registry.json: catalog anchor does not resolve "
                    f"for {candidate_id}"
                )
    return sorted(set(errors))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--references-dir", required=True)
    parser.add_argument("--forbid", default="", help="comma-separated source-stack terms")
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args(argv)
    errors = validate(
        Path(args.references_dir),
        [item for item in args.forbid.split(",") if item.strip()],
    )
    if args.as_json:
        print(json.dumps({"valid": not errors, "errors": errors}, indent=2))
    elif errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
    else:
        print("reference catalogs OK")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
