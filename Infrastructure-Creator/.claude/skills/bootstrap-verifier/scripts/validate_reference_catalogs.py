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


def validate(directory: Path, forbidden: list[str] | None = None) -> list[str]:
    errors: list[str] = []
    directory = directory.expanduser().resolve()
    files = sorted(directory.glob("*.md")) if directory.is_dir() else []
    if len(files) < 6:
        errors.append(f"expected at least six reference catalogs, found {len(files)}")
    headings: dict[str, set[str]] = {}
    for path in files:
        text = path.read_text(encoding="utf-8")
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
        or registry.get("schema_version") != "1.0"
        or not isinstance(candidates, list)
    ):
        errors.append("candidate-registry.json: invalid schema")
    else:
        seen: set[str] = set()
        for index, candidate in enumerate(candidates):
            if (
                not isinstance(candidate, dict)
                or set(candidate) != {"id", "catalog", "category", "mode"}
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
