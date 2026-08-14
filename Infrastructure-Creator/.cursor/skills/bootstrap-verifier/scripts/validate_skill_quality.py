#!/usr/bin/env python3
"""Deterministically validate generated skills against a semantic JSON plan."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable
from urllib.parse import urlparse

SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_REGISTRY = (
    SCRIPT_DIR.parent.parent
    / "skill-forge"
    / "references"
    / "candidate-registry.json"
)

REQUIRED_PLAN_FIELDS = (
    "schema_version",
    "catalog_version",
    "target_root",
    "profile",
    "evidence",
    "skills",
    "rejected_candidates",
)
REQUIRED_SKILL_FIELDS = (
    "name",
    "category",
    "kind",
    "phase",
    "necessity_rationale",
    "selection_gate",
    "triggers",
    "evidence_ids",
    "source_paths",
    "owned_scope",
    "excluded_scope",
    "required_procedure_roles",
    "decision_points",
    "verification",
    "output_contract",
    "failure_handling",
    "related_skills",
    "nearest_siblings",
    "writes",
)
SECTION_ALIASES = {
    "purpose": ("purpose",),
    "inputs": ("project evidence", "inputs", "project evidence / inputs"),
    "procedure": ("procedure", "process", "procedure / process"),
    "verification": ("verification",),
    "outputs": ("outputs", "output contract"),
    "failure": (
        "guardrails",
        "failure handling",
        "guardrails / failure handling",
    ),
}
GENERIC_PHRASES = (
    "follow best practices",
    "ensure quality",
    "handle errors appropriately",
    "review the relevant files",
    "perform the task",
    "use this skill to use this skill",
    "this skill handles this skill",
    "as needed",
)
STOPWORDS = {
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in",
    "into", "is", "it", "of", "on", "or", "that", "the", "their", "this",
    "to", "when", "with", "without", "must", "should", "will",
}
TARGET_DERIVED_KINDS = {
    "adapted", "project-adapted", "project_adapted", "project-derived",
    "project_derived", "integration", "specialty", "domain", "domain-review",
}
INTEGRATION_CATEGORIES = {"integration", "integrations"}
DOMAIN_CATEGORIES = {"domain", "domain-review", "domain_review", "domain review"}
SPECIALIST_CATEGORIES = (
    INTEGRATION_CATEGORIES
    | DOMAIN_CATEGORIES
    | {"specialty", "framework-specialty", "framework_specialty"}
)
IDENTITY_STOPWORDS = {
    "adapter", "contract", "domain", "integration", "review", "service", "services",
    "skill", "workflow",
}
LINE_FAIL_THRESHOLD = 0.70
TOKEN_FAIL_THRESHOLD = 0.80
TOKEN_WARN_THRESHOLD = 0.65


@dataclass(frozen=True, order=True)
class Diagnostic:
    severity: str
    code: str
    message: str

    def as_dict(self) -> dict[str, str]:
        return {"severity": self.severity, "code": self.code, "message": self.message}


def _diag(items: list[Diagnostic], code: str, message: str, severity: str = "error") -> None:
    items.append(Diagnostic(severity, code, message))


def _is_nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _is_string_list(value: Any, *, allow_empty: bool = False) -> bool:
    return (
        isinstance(value, list)
        and (allow_empty or bool(value))
        and all(_is_nonempty_string(item) for item in value)
    )


def _flatten_strings(value: Any) -> list[str]:
    if isinstance(value, str):
        return [value]
    if isinstance(value, list):
        flattened: list[str] = []
        for item in value:
            flattened.extend(_flatten_strings(item))
        return flattened
    if isinstance(value, dict):
        flattened = []
        for key in sorted(value):
            flattened.extend(_flatten_strings(value[key]))
        return flattened
    return []


def _is_substantive_contract(value: Any, *, allow_empty: bool = False) -> bool:
    if isinstance(value, str):
        return bool(value.strip())
    if isinstance(value, list):
        if not value:
            return allow_empty
        return all(
            isinstance(item, (str, dict))
            and bool(_flatten_strings(item))
            and all(part.strip() for part in _flatten_strings(item))
            for item in value
        )
    return False


def _fixed_block_contents(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [
        item["content"]
        for item in value
        if isinstance(item, dict)
        and _is_nonempty_string(item.get("id"))
        and _is_nonempty_string(item.get("version"))
        and _is_nonempty_string(item.get("content"))
    ]


def _safe_relative(value: Any) -> bool:
    if not _is_nonempty_string(value):
        return False
    path = Path(value)
    return not path.is_absolute() and ".." not in path.parts and value not in (".", "./")


def _confined(root: Path, relative: str) -> Path | None:
    if not _safe_relative(relative):
        return None
    candidate = root
    for part in Path(relative).parts:
        candidate = candidate / part
        if candidate.is_symlink():
            return None
    try:
        candidate.resolve().relative_to(root.resolve())
    except (OSError, ValueError):
        return None
    return candidate


def _approved_url(value: str) -> bool:
    parsed = urlparse(value)
    return (
        parsed.scheme in {"http", "https"}
        and bool(parsed.hostname)
        and parsed.username is None
        and parsed.password is None
        and not parsed.fragment
    )


def _evidence_location(entry: dict[str, Any]) -> tuple[str, str] | None:
    candidates = [
        ("path", entry.get("path")),
        ("url", entry.get("url")),
        ("source", entry.get("source")),
    ]
    populated = [(key, value.strip()) for key, value in candidates if _is_nonempty_string(value)]
    if len(populated) != 1:
        return None
    key, value = populated[0]
    if key == "source":
        key = "url" if value.startswith(("http://", "https://")) else "path"
    return key, value


def _load_plan(path: Path, diagnostics: list[Diagnostic]) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        _diag(diagnostics, "PLAN_UNREADABLE", f"plan is not readable JSON: {error}")
        return {}
    if not isinstance(value, dict):
        _diag(diagnostics, "PLAN_TYPE", "plan root must be an object")
        return {}
    return value


def _heading_slug(value: str) -> str:
    value = re.sub(r"[`*_]", "", value.strip().lower())
    value = re.sub(r"[^a-z0-9 -]", "", value)
    return re.sub(r"[- ]+", "-", value).strip("-")


def _load_registry(
    path: Path, catalog_version: Any, diagnostics: list[Diagnostic]
) -> dict[str, dict[str, str]]:
    try:
        registry = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        _diag(
            diagnostics,
            "REGISTRY_INVALID",
            f"candidate registry unreadable or invalid: {error}",
        )
        return {}
    if not isinstance(registry, dict) or set(registry) != {
        "schema_version",
        "catalog_version",
        "candidates",
    }:
        _diag(diagnostics, "REGISTRY_INVALID", "candidate registry fields are invalid")
        return {}
    if (
        registry.get("schema_version") != "1.0"
        or registry.get("catalog_version") != catalog_version
        or not isinstance(registry.get("candidates"), list)
    ):
        _diag(
            diagnostics,
            "REGISTRY_VERSION",
            "candidate registry schema/catalog version does not match plan",
        )
        return {}
    candidates: dict[str, dict[str, str]] = {}
    for index, item in enumerate(registry["candidates"]):
        if (
            not isinstance(item, dict)
            or set(item) != {"id", "catalog", "category", "mode"}
            or not all(_is_nonempty_string(item.get(key)) for key in item)
            or item.get("mode") not in {"static", "runtime-fixed", "family"}
        ):
            _diag(
                diagnostics,
                "REGISTRY_CANDIDATE_INVALID",
                f"candidate registry entry {index} is invalid",
            )
            continue
        candidate_id = item["id"]
        if candidate_id in candidates:
            _diag(
                diagnostics,
                "REGISTRY_CANDIDATE_DUPLICATE",
                f"duplicate candidate registry id: {candidate_id}",
            )
            continue
        catalog_parts = item["catalog"].split("#", 1)
        catalog_file = path.parent / catalog_parts[0]
        if (
            len(catalog_parts) != 2
            or not catalog_file.is_file()
            or not catalog_parts[1]
        ):
            _diag(
                diagnostics,
                "REGISTRY_CATALOG_INVALID",
                f"{candidate_id}: catalog reference does not resolve",
            )
        else:
            headings = {
                _heading_slug(match.group(1))
                for match in re.finditer(
                    r"^#{1,6}\s+(.+)$",
                    catalog_file.read_text(encoding="utf-8"),
                    re.M,
                )
            }
            if catalog_parts[1] not in headings:
                _diag(
                    diagnostics,
                    "REGISTRY_CATALOG_ANCHOR",
                    f"{candidate_id}: catalog anchor does not resolve: {item['catalog']}",
                )
        candidates[candidate_id] = item
    return candidates


def _parse_frontmatter(text: str) -> tuple[dict[str, str], str]:
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return {}, text
    try:
        end = next(index for index in range(1, len(lines)) if lines[index].strip() == "---")
    except StopIteration:
        return {}, text
    fields: dict[str, str] = {}
    for line in lines[1:end]:
        match = re.match(r"^([A-Za-z][A-Za-z0-9_-]*):\s*(.*?)\s*$", line)
        if match:
            fields[match.group(1)] = match.group(2).strip().strip("\"'")
    return fields, "\n".join(lines[end + 1 :])


def _sections(body: str) -> dict[str, str]:
    found: dict[str, list[str]] = {}
    current = ""
    for line in body.splitlines():
        heading = re.match(r"^#{2,6}\s+(.+?)\s*$", line)
        if heading:
            current = re.sub(r"[*_`]", "", heading.group(1)).strip().lower()
            found.setdefault(current, [])
        elif current:
            found[current].append(line)
    return {name: "\n".join(lines).strip() for name, lines in found.items()}


def _section(sections: dict[str, str], aliases: Iterable[str]) -> str:
    for alias in aliases:
        if alias in sections:
            return sections[alias]
    return ""


def _tokens(value: str) -> list[str]:
    return [
        token
        for token in re.findall(r"[a-z0-9][a-z0-9_.:/-]*", value.lower())
        if token not in STOPWORDS and len(token) > 1
    ]


def _meaningful_tokens(value: str) -> set[str]:
    return {token for token in _tokens(value) if len(token) >= 3 or any(char.isdigit() for char in token)}


def _contract_matches(contract: Any, text: str) -> bool:
    values = _flatten_strings(contract)
    expected = set().union(*(_meaningful_tokens(item) for item in values))
    if not expected:
        return False
    actual = _meaningful_tokens(text)
    required = 1 if len(expected) < 4 else 2
    return len(expected & actual) >= required


def _normalize_line(line: str) -> str:
    line = re.sub(r"TASK-\d+", "task-id", line, flags=re.IGNORECASE)
    line = re.sub(r"\bprofile[-_ ]?[a-z0-9_-]*\b", "profile-id", line, flags=re.IGNORECASE)
    line = re.sub(r"\s+", " ", line.strip().lower())
    return line


def _normalized_body(body: str, fixed_blocks: list[str]) -> tuple[list[str], list[str]]:
    cleaned = body
    for block in sorted(fixed_blocks, key=lambda item: (-len(item), item)):
        cleaned = cleaned.replace(block, "")
    lines: list[str] = []
    for raw in cleaned.splitlines():
        if re.match(r"^#{1,6}\s+", raw) or not raw.strip():
            continue
        normalized = _normalize_line(raw)
        if normalized:
            lines.append(normalized)
    return lines, _tokens("\n".join(lines))


def _line_similarity(left: list[str], right: list[str]) -> float:
    if not left or not right:
        return 0.0
    overlap = sum((Counter(left) & Counter(right)).values())
    return overlap / min(len(left), len(right))


def _ngrams(tokens: list[str], size: int = 3) -> set[tuple[str, ...]]:
    if len(tokens) < size:
        return {(token,) for token in tokens}
    return {tuple(tokens[index : index + size]) for index in range(len(tokens) - size + 1)}


def _token_similarity(left: list[str], right: list[str]) -> float:
    left_set, right_set = _ngrams(left), _ngrams(right)
    union = left_set | right_set
    return len(left_set & right_set) / len(union) if union else 0.0


def _scope_parts(values: list[str]) -> set[str]:
    return {re.sub(r"/+$", "", value.strip().lower()) for value in values}


def _scopes_collide(left: set[str], right: set[str]) -> set[str]:
    collisions: set[str] = set()
    for one in left:
        for two in right:
            if one == two or one.startswith(two + "/") or two.startswith(one + "/"):
                collisions.add(one if len(one) <= len(two) else two)
    return collisions


def _routing_tokens(skill: dict[str, Any]) -> set[str]:
    triggers = skill.get("triggers", {})
    return _meaningful_tokens(" ".join(triggers.get("positive", [])))


def _has_explicit_routing_precedence(
    left: dict[str, Any], right_name: str
) -> bool:
    for sibling in left.get("nearest_siblings", []):
        if not isinstance(sibling, dict) or sibling.get("name") != right_name:
            continue
        boundary = " ".join(_flatten_strings(sibling.get("boundary")))
        if re.search(r"\b(primary|defer|deferred|fallback|owns|owner)\b", boundary, re.I):
            return True
    return False


def _validate_plan(
    plan: dict[str, Any],
    plan_path: Path,
    target: Path,
    registry_path: Path,
    diagnostics: list[Diagnostic],
) -> tuple[dict[str, dict[str, Any]], dict[str, tuple[str, str]]]:
    for field in REQUIRED_PLAN_FIELDS:
        if field not in plan:
            _diag(diagnostics, "PLAN_FIELD_MISSING", f"plan missing top-level field: {field}")
    if diagnostics:
        return {}, {}
    extra_fields = set(plan) - set(REQUIRED_PLAN_FIELDS)
    if extra_fields:
        _diag(
            diagnostics,
            "PLAN_FIELD_UNKNOWN",
            f"plan contains unknown top-level fields: {sorted(extra_fields)}",
        )
    if plan["schema_version"] != "1.0":
        _diag(diagnostics, "PLAN_SCHEMA_VERSION", "schema_version must equal '1.0'")
    if not _is_nonempty_string(plan["catalog_version"]):
        _diag(
            diagnostics,
            "PLAN_CATALOG_VERSION",
            "catalog_version must be a non-empty reference-corpus version",
        )
    registry = _load_registry(
        registry_path.expanduser().resolve(), plan["catalog_version"], diagnostics
    )
    if not _is_nonempty_string(plan["target_root"]):
        _diag(diagnostics, "PLAN_TARGET_ROOT", "target_root must be a non-empty string")
    else:
        root_value = Path(plan["target_root"]).expanduser()
        declared = (target / root_value).resolve() if not root_value.is_absolute() else root_value.resolve()
        if declared != target.resolve():
            _diag(diagnostics, "PLAN_TARGET_MISMATCH", f"target_root does not match --target: {plan['target_root']}")
    profile = plan["profile"]
    profile_rel = Path(profile) if _safe_relative(profile) else None
    profile_candidate = plan_path.parent / profile_rel.name if profile_rel else None
    profile_task_matches = bool(
        profile_rel
        and len(profile_rel.parts) == 3
        and profile_rel.parts[0] == "tasks"
        and profile_rel.parts[1] == plan_path.parent.name
        and profile_rel.parts[2] == "infra-scan-project-profile.md"
    )
    if (
        profile_candidate is None
        or not profile_task_matches
        or not profile_candidate.is_file()
    ):
        _diag(
            diagnostics,
            "PROFILE_INVALID",
            f"profile must resolve beside the generation plan: {profile!r}",
        )

    evidence_map: dict[str, tuple[str, str]] = {}
    evidence_claims: dict[str, list[str]] = {}
    evidence = plan["evidence"]
    if not isinstance(evidence, list) or not evidence:
        _diag(diagnostics, "EVIDENCE_EMPTY", "evidence must be a non-empty array")
    else:
        for index, entry in enumerate(evidence):
            if not isinstance(entry, dict) or not _is_nonempty_string(entry.get("id")):
                _diag(diagnostics, "EVIDENCE_INVALID", f"evidence[{index}] requires a non-empty id")
                continue
            evidence_id = entry["id"].strip()
            if evidence_id in evidence_map:
                _diag(diagnostics, "EVIDENCE_ID_DUPLICATE", f"duplicate evidence id: {evidence_id}")
                continue
            location = _evidence_location(entry)
            if location is None:
                _diag(diagnostics, "EVIDENCE_LOCATION", f"evidence {evidence_id} must define exactly one path or URL")
                continue
            kind, value = location
            allowed_evidence_fields = {
                "id",
                "path",
                "url",
                "source_type",
                "authority",
                "confidence",
                "line_range",
                "fingerprint",
                "supported_claims",
            }
            unknown_evidence_fields = set(entry) - allowed_evidence_fields
            if unknown_evidence_fields:
                _diag(
                    diagnostics,
                    "EVIDENCE_FIELD_UNKNOWN",
                    f"evidence {evidence_id} contains unknown fields: "
                    f"{sorted(unknown_evidence_fields)}",
                )
            for field in ("source_type", "authority", "confidence"):
                if not _is_nonempty_string(entry.get(field)):
                    _diag(
                        diagnostics,
                        "EVIDENCE_METADATA",
                        f"evidence {evidence_id} requires {field}",
                    )
            if entry.get("confidence") not in {"confirmed", "inferred", "unknown"}:
                _diag(
                    diagnostics,
                    "EVIDENCE_CONFIDENCE",
                    f"evidence {evidence_id} has invalid confidence",
                )
            if kind == "url":
                if not _approved_url(value):
                    _diag(diagnostics, "EVIDENCE_URL_INVALID", f"evidence {evidence_id} has an unapproved URL: {value}")
                    continue
            else:
                resolved = _confined(target, value)
                if resolved is None or not resolved.is_file():
                    _diag(diagnostics, "EVIDENCE_PATH_INVALID", f"evidence {evidence_id} path is unsafe or missing: {value}")
                    continue
                if re.search(r"(^|/)tasks/TASK-[^/]+/", value, re.I):
                    _diag(
                        diagnostics,
                        "EVIDENCE_TASK_PATH",
                        f"evidence {evidence_id} points to generator runtime: {value}",
                    )
                fingerprint = entry.get("fingerprint")
                actual = "sha256:" + hashlib.sha256(resolved.read_bytes()).hexdigest()
                if not _is_nonempty_string(fingerprint):
                    _diag(
                        diagnostics,
                        "EVIDENCE_FINGERPRINT_MISSING",
                        f"evidence {evidence_id} requires a sha256 fingerprint",
                    )
                elif fingerprint != actual:
                    _diag(
                        diagnostics,
                        "EVIDENCE_FINGERPRINT",
                        f"evidence {evidence_id} fingerprint drifted",
                    )
                line_range = entry.get("line_range")
                if line_range is not None and (
                    not isinstance(line_range, dict)
                    or set(line_range) != {"start", "end"}
                    or type(line_range.get("start")) is not int
                    or type(line_range.get("end")) is not int
                    or line_range["start"] < 1
                    or line_range["end"] < line_range["start"]
                ):
                    _diag(
                        diagnostics,
                        "EVIDENCE_LINE_RANGE",
                        f"evidence {evidence_id} has an invalid line_range",
                    )
                elif line_range is not None:
                    line_count = len(
                        resolved.read_text(
                            encoding="utf-8", errors="replace"
                        ).splitlines()
                    )
                    if line_range["end"] > line_count:
                        _diag(
                            diagnostics,
                            "EVIDENCE_LINE_RANGE",
                            f"evidence {evidence_id} line_range exceeds file length",
                        )
            if not _is_string_list(entry.get("supported_claims")):
                _diag(
                    diagnostics,
                    "EVIDENCE_CLAIMS",
                    f"evidence {evidence_id} requires supported_claims",
                )
            else:
                evidence_claims[evidence_id] = entry["supported_claims"]
                if kind == "path" and resolved is not None and resolved.is_file():
                    lines = resolved.read_text(
                        encoding="utf-8", errors="replace"
                    ).splitlines()
                    cited_text = "\n".join(lines)
                    if isinstance(entry.get("line_range"), dict):
                        start = max(0, entry["line_range"].get("start", 1) - 1)
                        end = min(len(lines), entry["line_range"].get("end", 0))
                        cited_text = "\n".join(lines[start:end])
                    cited_tokens = _meaningful_tokens(cited_text)
                    for claim in entry["supported_claims"]:
                        claim_tokens = _meaningful_tokens(claim) - {
                            "project", "confirmed", "supports", "uses", "runtime"
                        }
                        required = 1 if len(claim_tokens) <= 3 else 2
                        if len(claim_tokens & cited_tokens) < required:
                            _diag(
                                diagnostics,
                                "EVIDENCE_CLAIM_UNSUPPORTED",
                                f"evidence {evidence_id} claim is not grounded "
                                f"in the cited source/range: {claim}",
                            )
            evidence_map[evidence_id] = location

    plan_skills: dict[str, dict[str, Any]] = {}
    rejected = plan["rejected_candidates"]
    rejected_names: set[str] = set()
    rejected_candidate_ids: list[str] = []
    if not isinstance(rejected, list):
        _diag(
            diagnostics,
            "REJECTED_CANDIDATES_INVALID",
            "rejected_candidates must be an array",
        )
    else:
        for index, item in enumerate(rejected):
            if not isinstance(item, dict) or set(item) != {
                "candidate_id", "name", "category", "reason", "missing_evidence"
            }:
                _diag(
                    diagnostics,
                    "REJECTED_CANDIDATE_INVALID",
                    f"rejected_candidates[{index}] has invalid fields",
                )
            elif (
                not all(
                    _is_nonempty_string(item.get(field))
                    for field in ("candidate_id", "name", "category", "reason")
                )
                or not _is_string_list(item.get("missing_evidence"), allow_empty=True)
            ):
                _diag(
                    diagnostics,
                    "REJECTED_CANDIDATE_INVALID",
                    f"rejected_candidates[{index}] is incomplete",
                )
            elif item["name"] in rejected_names:
                _diag(
                    diagnostics,
                    "REJECTED_CANDIDATE_DUPLICATE",
                    f"duplicate rejected candidate: {item['name']}",
                )
            else:
                rejected_names.add(item["name"])
                rejected_candidate_ids.append(item["candidate_id"])
                registry_candidate = registry.get(item["candidate_id"])
                if registry_candidate is None:
                    _diag(
                        diagnostics,
                        "REJECTED_CANDIDATE_UNKNOWN",
                        f"rejected candidate is absent from registry: "
                        f"{item['candidate_id']}",
                    )
                elif (
                    item["category"].lower()
                    != registry_candidate["category"].lower()
                    or (
                        registry_candidate["mode"] == "static"
                        and item["name"] != item["candidate_id"]
                    )
                    or registry_candidate["mode"] == "runtime-fixed"
                    or not item["missing_evidence"]
                ):
                    _diag(
                        diagnostics,
                        "REJECTED_CANDIDATE_CONTRACT",
                        f"rejected candidate does not satisfy registry disposition "
                        f"rules: {item['candidate_id']}",
                    )
    skills = plan["skills"]
    if not isinstance(skills, list) or not skills:
        _diag(diagnostics, "SKILL_PLAN_EMPTY", "skills must be a non-empty array")
        return {}, evidence_map
    for index, skill in enumerate(skills):
        if not isinstance(skill, dict):
            _diag(diagnostics, "SKILL_PLAN_INVALID", f"skills[{index}] must be an object")
            continue
        missing = [field for field in REQUIRED_SKILL_FIELDS if field not in skill]
        if missing:
            _diag(diagnostics, "SKILL_PLAN_INCOMPLETE", f"skills[{index}] missing: {', '.join(missing)}")
            continue
        name = skill.get("name")
        if not _is_nonempty_string(name):
            _diag(diagnostics, "SKILL_NAME_INVALID", f"skills[{index}] has an invalid name")
            continue
        name = name.strip()
        if name in plan_skills:
            _diag(diagnostics, "SKILL_NAME_DUPLICATE", f"duplicate skill name: {name}")
            continue
        scalar_fields = ("category", "kind", "phase", "necessity_rationale")
        for field in scalar_fields:
            if not _is_nonempty_string(skill[field]):
                _diag(diagnostics, "SKILL_PLAN_VALUE", f"{name}.{field} must be substantive")
        triggers = skill["triggers"]
        if (
            not isinstance(triggers, dict)
            or not _is_string_list(triggers.get("positive"))
            or not _is_string_list(triggers.get("negative"))
        ):
            _diag(diagnostics, "SKILL_TRIGGERS_INVALID", f"{name}.triggers requires positive and negative string arrays")
        runtime_fixed = str(skill.get("kind", "")).lower() == "runtime-fixed"
        selection_gate = skill.get("selection_gate")
        if not isinstance(selection_gate, dict) or set(selection_gate) != {
            "catalog",
            "candidate_id",
            "candidate",
            "conditions",
            "distinct_value_from",
        }:
            _diag(
                diagnostics,
                "SKILL_SELECTION_GATE",
                f"{name}.selection_gate has invalid fields",
            )
        else:
            registry_candidate = registry.get(selection_gate.get("candidate_id"))
            if registry_candidate is None:
                _diag(
                    diagnostics,
                    "SKILL_SELECTION_CANDIDATE_UNKNOWN",
                    f"{name} references unknown registry candidate",
                )
            elif (
                selection_gate.get("catalog") != registry_candidate["catalog"]
                or str(skill.get("category", "")).lower()
                != registry_candidate["category"].lower()
                or (
                    registry_candidate["mode"] in {"static", "runtime-fixed"}
                    and selection_gate.get("candidate_id") != name
                )
                or (
                    registry_candidate["mode"] == "runtime-fixed"
                    and not runtime_fixed
                )
            ):
                _diag(
                    diagnostics,
                    "SKILL_SELECTION_REGISTRY_MISMATCH",
                    f"{name} selection gate does not match candidate registry",
                )
            if (
                not _is_nonempty_string(selection_gate.get("catalog"))
                or not _is_nonempty_string(selection_gate.get("candidate_id"))
                or selection_gate.get("candidate") != name
                or not _is_string_list(
                    selection_gate.get("distinct_value_from"), allow_empty=True
                )
                or not isinstance(selection_gate.get("conditions"), list)
                or not selection_gate["conditions"]
            ):
                _diag(
                    diagnostics,
                    "SKILL_SELECTION_GATE",
                    f"{name}.selection_gate is incomplete",
                )
            else:
                for condition in selection_gate["conditions"]:
                    if (
                        not isinstance(condition, dict)
                        or set(condition)
                        != {"requirement", "evidence_ids", "status", "explanation"}
                        or not _is_nonempty_string(condition.get("requirement"))
                        or condition.get("status") != "satisfied"
                        or not _is_nonempty_string(condition.get("explanation"))
                        or not _is_string_list(
                            condition.get("evidence_ids"),
                            allow_empty=runtime_fixed,
                        )
                    ):
                        _diag(
                            diagnostics,
                            "SKILL_SELECTION_CONDITION",
                            f"{name} has an unsatisfied/incomplete selection condition",
                        )
                        continue
                    for condition_evidence in condition.get("evidence_ids", []):
                        if condition_evidence not in skill.get("evidence_ids", []):
                            _diag(
                                diagnostics,
                                "SKILL_SELECTION_EVIDENCE",
                                f"{name} selection condition uses undeclared evidence: "
                                f"{condition_evidence}",
                            )
                    claims = [
                        claim
                        for condition_evidence in condition.get("evidence_ids", [])
                        for claim in evidence_claims.get(condition_evidence, [])
                    ]
                    if claims and not _contract_matches(
                        claims,
                        condition.get("requirement", "")
                        + " "
                        + condition.get("explanation", "")
                        + " "
                        + str(skill.get("necessity_rationale", "")),
                    ):
                        _diag(
                            diagnostics,
                            "SKILL_SELECTION_CLAIM_TRACE",
                            f"{name} selection condition is not supported by cited claims",
                        )
        for field in ("evidence_ids", "source_paths", "writes", "related_skills"):
            allow_empty = field == "writes" or (runtime_fixed and field in {"evidence_ids", "source_paths"})
            if not _is_string_list(skill[field], allow_empty=allow_empty):
                _diag(diagnostics, "SKILL_PLAN_VALUE", f"{name}.{field} must be a string array")
        for field in (
            "owned_scope", "excluded_scope", "required_procedure_roles",
            "decision_points", "verification", "output_contract",
            "failure_handling", "nearest_siblings",
        ):
            if not _is_substantive_contract(skill[field]):
                _diag(diagnostics, "SKILL_PLAN_VALUE", f"{name}.{field} must be a substantive array")
        fixed_blocks = skill.get("fixed_blocks", [])
        if not isinstance(fixed_blocks, list) or len(
            _fixed_block_contents(fixed_blocks)
        ) != len(fixed_blocks):
            _diag(
                diagnostics,
                "SKILL_FIXED_BLOCKS_INVALID",
                f"{name}.fixed_blocks must contain versioned id/version/content objects",
            )
        for evidence_id in skill.get("evidence_ids", []):
            if evidence_id not in evidence_map:
                _diag(diagnostics, "SKILL_EVIDENCE_UNKNOWN", f"{name} references unknown evidence: {evidence_id}")
        for source in skill.get("source_paths", []):
            resolved = _confined(target, source)
            if resolved is None or not resolved.exists():
                _diag(diagnostics, "SKILL_SOURCE_INVALID", f"{name} source path is unsafe or missing: {source}")
            if re.search(r"(^|/)tasks/TASK-[^/]+/", source, re.I):
                _diag(
                    diagnostics,
                    "SKILL_SOURCE_TASK_PATH",
                    f"{name} source path points to generator runtime: {source}",
                )
        for write in skill.get("writes", []):
            if not _safe_relative(write):
                _diag(diagnostics, "SKILL_WRITE_INVALID", f"{name} write path must be target-relative: {write}")
        plan_skills[name] = skill

    names = set(plan_skills)
    selected_candidate_ids = [
        skill.get("selection_gate", {}).get("candidate_id")
        for skill in plan_skills.values()
        if _is_nonempty_string(
            skill.get("selection_gate", {}).get("candidate_id")
        )
    ]
    coverage_counts = Counter(selected_candidate_ids + rejected_candidate_ids)
    dual_disposition = set(selected_candidate_ids) & set(rejected_candidate_ids)
    if dual_disposition:
        _diag(
            diagnostics,
            "INVENTORY_CANDIDATE_DUAL_DISPOSITION",
            f"candidate families cannot be both selected and rejected: "
            f"{sorted(dual_disposition)}",
        )
    for candidate_id, candidate in sorted(registry.items()):
        count = coverage_counts.get(candidate_id, 0)
        if count == 0:
            _diag(
                diagnostics,
                "INVENTORY_CANDIDATE_UNACCOUNTED",
                f"registry candidate is neither selected nor rejected: {candidate_id}",
            )
        elif candidate["mode"] != "family" and count != 1:
            _diag(
                diagnostics,
                "INVENTORY_CANDIDATE_DUPLICATE",
                f"non-family candidate appears {count} times: {candidate_id}",
            )
    for candidate_id in sorted(set(coverage_counts) - set(registry)):
        _diag(
            diagnostics,
            "INVENTORY_CANDIDATE_UNKNOWN",
            f"plan accounts for unknown registry candidate: {candidate_id}",
        )
    overlap = names & rejected_names
    if overlap:
        _diag(
            diagnostics,
            "INVENTORY_SELECTED_AND_REJECTED",
            f"candidates cannot be selected and rejected: {sorted(overlap)}",
        )
    for name, skill in sorted(plan_skills.items()):
        siblings = list(skill.get("related_skills", [])) + [
            sibling.get("name")
            for sibling in skill.get("nearest_siblings", [])
            if isinstance(sibling, dict)
        ]
        for sibling in siblings:
            if sibling == name or sibling not in names:
                _diag(diagnostics, "SKILL_SIBLING_UNKNOWN", f"{name} has unresolved sibling reference: {sibling}")
        for candidate in skill.get("selection_gate", {}).get(
            "distinct_value_from", []
        ):
            if candidate not in names | rejected_names:
                _diag(
                    diagnostics,
                    "SKILL_SELECTION_BOUNDARY_UNKNOWN",
                    f"{name} distinguishes unknown candidate: {candidate}",
                )
    return plan_skills, evidence_map


def _validate_skill_file(
    name: str,
    plan: dict[str, Any],
    path: Path,
    evidence_map: dict[str, tuple[str, str]],
    target: Path,
    diagnostics: list[Diagnostic],
) -> tuple[str, list[str], list[str]]:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        _diag(diagnostics, "SKILL_UNREADABLE", f"{name}: cannot read SKILL.md: {error}")
        return "", [], []
    frontmatter, body = _parse_frontmatter(text)
    if frontmatter.get("name") != name:
        _diag(diagnostics, "SKILL_FRONTMATTER_NAME", f"{name}: directory and frontmatter name differ")
    description = frontmatter.get("description", "")
    if not _is_nonempty_string(description) or not re.search(r"\b(when|use|trigger|for)\b", description, re.I):
        _diag(diagnostics, "SKILL_DESCRIPTION", f"{name}: description must identify when the skill is selected")

    sections = _sections(body)
    resolved: dict[str, str] = {}
    insubstantial_sections = 0
    for role, aliases in SECTION_ALIASES.items():
        content = _section(sections, aliases)
        resolved[role] = content
        if len(_meaningful_tokens(content)) < 3:
            insubstantial_sections += 1
            _diag(diagnostics, "SKILL_SECTION_MISSING", f"{name}: missing or insubstantial {role} section")

    lower = body.lower()
    triggers = plan.get("triggers", {})
    if not _contract_matches(triggers.get("positive"), description + "\n" + body):
        _diag(
            diagnostics,
            "SKILL_POSITIVE_TRIGGER_TRACE",
            f"{name}: positive routing trigger is not traceable",
        )
    if not _contract_matches(triggers.get("negative"), body):
        _diag(
            diagnostics,
            "SKILL_NEGATIVE_TRIGGER_TRACE",
            f"{name}: negative routing/deferral is not traceable",
        )
    for phrase in GENERIC_PHRASES:
        if phrase in lower:
            _diag(diagnostics, "SKILL_GENERIC_PHRASE", f"{name}: generic phrase is not operational: {phrase!r}")
    if re.search(rf"\b{name.replace('-', r'[- ]')}\b.*\b{name.replace('-', r'[- ]')}\b", resolved["purpose"], re.I):
        _diag(diagnostics, "SKILL_CIRCULAR_PURPOSE", f"{name}: purpose is circular")

    checks = (
        ("necessity_rationale", body, "SKILL_RATIONALE_TRACE"),
        ("required_procedure_roles", resolved["procedure"], "SKILL_PROCEDURE_TRACE"),
        ("decision_points", resolved["procedure"], "SKILL_DECISION_TRACE"),
        ("verification", resolved["verification"], "SKILL_VERIFICATION_TRACE"),
        ("output_contract", resolved["outputs"], "SKILL_OUTPUT_TRACE"),
        ("failure_handling", resolved["failure"], "SKILL_FAILURE_TRACE"),
        ("owned_scope", body, "SKILL_OWNED_SCOPE_TRACE"),
        ("excluded_scope", body, "SKILL_EXCLUDED_SCOPE_TRACE"),
    )
    for field, haystack, code in checks:
        if not _contract_matches(plan[field], haystack):
            _diag(diagnostics, code, f"{name}: {field} is not traceable to skill content")

    expected_refs: list[str] = list(plan.get("source_paths", []))
    for evidence_id in plan.get("evidence_ids", []):
        location = evidence_map.get(evidence_id)
        if location and location[0] == "path":
            expected_refs.append(location[1])
    kind = str(plan.get("kind", "")).lower()
    if kind in TARGET_DERIVED_KINDS or expected_refs:
        if not any(reference.lower() in lower for reference in expected_refs):
            _diag(diagnostics, "SKILL_TARGET_REFERENCE", f"{name}: no concrete target evidence path appears in the skill")
    if kind == "runtime-fixed" and not re.search(
        r"\b(memory-bank|project-brain)/", body
    ):
        _diag(
            diagnostics,
            "SKILL_RUNTIME_REFERENCE",
            f"{name}: runtime-fixed skill must cite its generated target-relative runtime",
        )

    category = str(plan.get("category", "")).lower()
    if category in INTEGRATION_CATEGORIES or category in DOMAIN_CATEGORIES:
        evidence_chunks: list[str] = []
        for evidence_id in plan.get("evidence_ids", []):
            if evidence_id not in evidence_map:
                continue
            evidence_kind, evidence_value = evidence_map[evidence_id]
            evidence_chunks.append(evidence_value)
            evidence_path = _confined(target, evidence_value) if evidence_kind == "path" else None
            if evidence_path and evidence_path.is_file():
                try:
                    evidence_chunks.append(evidence_path.read_text(encoding="utf-8")[:200_000])
                except (OSError, UnicodeError):
                    pass
        evidence_text = " ".join(evidence_chunks)
        identifying = _meaningful_tokens(name.replace("-", " ")) - IDENTITY_STOPWORDS
        if identifying and not identifying.intersection(_meaningful_tokens(evidence_text)):
            _diag(diagnostics, "SKILL_EVIDENCE_IRRELEVANT", f"{name}: evidence does not identify the planned concern")
    if category in INTEGRATION_CATEGORIES:
        identity = set(_meaningful_tokens(name.replace("-", " ")) | _meaningful_tokens(evidence_text))
        if not identity.intersection(_meaningful_tokens(body)):
            _diag(diagnostics, "INTEGRATION_IDENTITY", f"{name}: integration identity is not concrete")
        if not re.search(r"\b(runtime|boundary|client|adapter|service|transport|sdk|api)\b", body, re.I):
            _diag(diagnostics, "INTEGRATION_BOUNDARY", f"{name}: integration runtime boundary is missing")
    if category in DOMAIN_CATEGORIES:
        if not re.search(r"\b(invariant|transition|permission|role|failure|forbidden|state)\b", body, re.I):
            _diag(diagnostics, "DOMAIN_CONTRACT", f"{name}: domain invariants, transitions, permissions, or failures are missing")

    siblings = list(plan.get("related_skills", [])) + [
        sibling.get("name")
        for sibling in plan.get("nearest_siblings", [])
        if isinstance(sibling, dict)
    ]
    for sibling in siblings:
        if not isinstance(sibling, str) or sibling.lower() not in lower:
            _diag(diagnostics, "SKILL_SIBLING_TRACE", f"{name}: sibling {sibling} is not referenced in skill content")
    for sibling in plan.get("nearest_siblings", []):
        if isinstance(sibling, dict) and not _contract_matches(
            sibling.get("boundary"), body
        ):
            _diag(
                diagnostics,
                "SKILL_SIBLING_BOUNDARY_TRACE",
                f"{name}: nearest-sibling boundary is not traceable",
            )

    fixed_blocks = _fixed_block_contents(plan.get("fixed_blocks", []))
    for block in fixed_blocks:
        if block not in body:
            _diag(diagnostics, "FIXED_BLOCK_MISSING", f"{name}: approved fixed block is not an exact body block")
        elif block not in resolved["failure"]:
            _diag(diagnostics, "FIXED_BLOCK_SCOPE", f"{name}: fixed block must be confined to failure handling")
        if len(_tokens(block)) > 120:
            _diag(diagnostics, "FIXED_BLOCK_OVERSIZED", f"{name}: fixed block is too large for a shared exemption")
    normalized_lines, normalized_tokens = _normalized_body(body, fixed_blocks)
    if (
        insubstantial_sections >= 3
        or len(set(normalized_lines)) < 5
        or len(set(normalized_tokens)) < 18
    ):
        _diag(diagnostics, "SKILL_CONTENT_EMPTY", f"{name}: substantive content is empty or heavily paraphrased filler")
    return body, normalized_lines, normalized_tokens


def validate(
    skills_dir: Path,
    plan_path: Path,
    target: Path,
    registry_path: Path = DEFAULT_REGISTRY,
) -> list[Diagnostic]:
    diagnostics: list[Diagnostic] = []
    target = target.expanduser().resolve()
    skills_dir = skills_dir.expanduser().resolve()
    plan_path = plan_path.expanduser().resolve()
    if not target.is_dir():
        _diag(diagnostics, "TARGET_INVALID", f"target is not a directory: {target}")
        return sorted(set(diagnostics))
    if not skills_dir.is_dir():
        _diag(diagnostics, "SKILLS_DIR_INVALID", f"skills directory is not a directory: {skills_dir}")
        return sorted(set(diagnostics))
    if skills_dir.is_symlink():
        _diag(diagnostics, "SKILLS_DIR_UNSAFE", f"skills directory must not be a symlink: {skills_dir}")
        return sorted(set(diagnostics))

    plan = _load_plan(plan_path, diagnostics)
    plan_skills, evidence_map = (
        _validate_plan(plan, plan_path, target, registry_path, diagnostics)
        if plan
        else ({}, {})
    )
    actual: dict[str, Path] = {}
    for child in sorted(skills_dir.iterdir(), key=lambda item: item.name):
        if child.is_symlink():
            _diag(diagnostics, "SKILL_PATH_UNSAFE", f"skill path must not be a symlink: {child.name}")
        elif child.is_dir() and (child / "SKILL.md").is_file():
            actual[child.name] = child / "SKILL.md"
    for name in sorted(set(plan_skills) - set(actual)):
        _diag(diagnostics, "SKILL_FILE_MISSING", f"planned skill has no SKILL.md: {name}")
    for name in sorted(set(actual) - set(plan_skills)):
        _diag(diagnostics, "SKILL_UNPLANNED", f"skill has no plan entry: {name}")

    normalized: dict[str, tuple[list[str], list[str]]] = {}
    for name in sorted(set(plan_skills) & set(actual)):
        _, lines, tokens = _validate_skill_file(
            name, plan_skills[name], actual[name], evidence_map, target, diagnostics
        )
        normalized[name] = (lines, tokens)

    names = sorted(normalized)
    for index, left_name in enumerate(names):
        left = plan_skills[left_name]
        left_scope = _scope_parts(left.get("owned_scope", []))
        left_writes = _scope_parts(left.get("writes", []))
        for right_name in names[index + 1 :]:
            right = plan_skills[right_name]
            collisions = _scopes_collide(left_scope, _scope_parts(right.get("owned_scope", [])))
            write_collisions = _scopes_collide(left_writes, _scope_parts(right.get("writes", [])))
            if collisions or write_collisions:
                values = sorted(collisions | write_collisions)
                _diag(diagnostics, "SCOPE_COLLISION", f"{left_name} and {right_name} overlap ownership: {', '.join(values)}")
            left_routing = _routing_tokens(left)
            right_routing = _routing_tokens(right)
            routing_union = left_routing | right_routing
            routing_score = (
                len(left_routing & right_routing) / len(routing_union)
                if routing_union
                else 0.0
            )
            if routing_score >= 0.75 and not (
                _has_explicit_routing_precedence(left, right_name)
                or _has_explicit_routing_precedence(right, left_name)
            ):
                _diag(
                    diagnostics,
                    "ROUTING_AMBIGUITY",
                    f"{left_name} and {right_name} have overlapping positive "
                    f"routing ({routing_score:.3f}) without explicit precedence",
                )
            line_score = _line_similarity(normalized[left_name][0], normalized[right_name][0])
            token_score = _token_similarity(normalized[left_name][1], normalized[right_name][1])
            if line_score >= LINE_FAIL_THRESHOLD or token_score >= TOKEN_FAIL_THRESHOLD:
                repeated = sorted(set(normalized[left_name][0]) & set(normalized[right_name][0]))[:3]
                detail = "; repeated: " + " | ".join(repeated) if repeated else ""
                _diag(
                    diagnostics,
                    "SKILL_SIMILARITY",
                    f"{left_name} and {right_name} duplicate substantive content "
                    f"(line={line_score:.3f}, token={token_score:.3f}){detail}",
                )
            elif token_score >= TOKEN_WARN_THRESHOLD:
                _diag(
                    diagnostics,
                    "SKILL_SIMILARITY_WARN",
                    f"{left_name} and {right_name} have high token similarity "
                    f"(line={line_score:.3f}, token={token_score:.3f})",
                    "warning",
                )

    block_owners: defaultdict[tuple[str, ...], set[str]] = defaultdict(set)
    for name, (lines, _) in normalized.items():
        for size in range(3, min(7, len(lines) + 1)):
            for index in range(len(lines) - size + 1):
                block = tuple(lines[index : index + size])
                if sum(len(_tokens(line)) for line in block) >= 20:
                    block_owners[block].add(name)
    for block, owners in sorted(block_owners.items()):
        if len(owners) > 1 and len(block) == max(
            len(candidate)
            for candidate, candidate_owners in block_owners.items()
            if candidate_owners == owners
        ):
            _diag(
                diagnostics,
                "REPEATED_BLOCK",
                f"repeated substantive block across {', '.join(sorted(owners))}: {' | '.join(block[:2])}",
            )

    return sorted(set(diagnostics))


def validate_plan(
    plan_path: Path,
    target: Path,
    registry_path: Path = DEFAULT_REGISTRY,
) -> list[Diagnostic]:
    """Validate evidence and per-skill contracts before authoring starts."""
    diagnostics: list[Diagnostic] = []
    plan_path = plan_path.expanduser().resolve()
    target = target.expanduser().resolve()
    if not target.is_dir():
        _diag(diagnostics, "TARGET_INVALID", f"target is not a directory: {target}")
        return sorted(set(diagnostics))
    plan = _load_plan(plan_path, diagnostics)
    if plan:
        _validate_plan(plan, plan_path, target, registry_path, diagnostics)
    return sorted(set(diagnostics))


def validate_agent_routing(
    agents_dir: Path,
    plan_path: Path,
    target: Path,
    *,
    require_invokes: bool = False,
    registry_path: Path = DEFAULT_REGISTRY,
) -> list[Diagnostic]:
    """Validate generated wrapper selection against skill routing contracts."""
    diagnostics = validate_plan(plan_path, target, registry_path)
    if any(item.severity == "error" for item in diagnostics):
        return diagnostics
    plan = _load_plan(plan_path.expanduser().resolve(), diagnostics)
    contracts = {item["name"]: item for item in plan.get("skills", [])}
    agents_dir = agents_dir.expanduser().resolve()
    if not agents_dir.is_dir():
        _diag(
            diagnostics,
            "AGENTS_DIR_INVALID",
            f"agents directory is not a directory: {agents_dir}",
        )
        return sorted(set(diagnostics))
    actual_agents = {
        path.name.removesuffix("-agent.md")
        for path in agents_dir.glob("*-agent.md")
        if path.is_file() and not path.is_symlink()
    }
    for path in sorted(agents_dir.glob("*.md")):
        if path.name == "README.md":
            continue
        if not path.name.endswith("-agent.md") or path.is_symlink():
            _diag(
                diagnostics,
                "AGENT_ROUTING_NONCONFORMING_FILE",
                f"agent roster contains a nonconforming file: {path.name}",
            )
    for unplanned in sorted(actual_agents - set(contracts)):
        _diag(
            diagnostics,
            "AGENT_ROUTING_UNPLANNED",
            f"{unplanned}: wrapper has no skill contract",
        )
    for name, contract in sorted(contracts.items()):
        path = agents_dir / f"{name}-agent.md"
        if not path.is_file() or path.is_symlink():
            _diag(
                diagnostics,
                "AGENT_ROUTING_MISSING",
                f"{name}: matching agent wrapper is missing",
            )
            continue
        text = path.read_text(encoding="utf-8")
        frontmatter, body = _parse_frontmatter(text)
        if frontmatter.get("name") != f"{name}-agent":
            _diag(
                diagnostics,
                "AGENT_FRONTMATTER_NAME",
                f"{name}: wrapper name must be {name}-agent",
            )
        invokes = frontmatter.get("invokes")
        if (require_invokes and invokes != name) or (
            invokes is not None and invokes != name
        ):
            _diag(
                diagnostics,
                "AGENT_INVOKES_MISMATCH",
                f"{name}: wrapper must invoke exactly {name}",
            )
        description = frontmatter.get("description", "")
        if not _contract_matches(
            contract.get("triggers", {}).get("positive"),
            description + "\n" + body,
        ):
            _diag(
                diagnostics,
                "AGENT_POSITIVE_ROUTING",
                f"{name}: positive trigger is not traceable to wrapper",
            )
        if not _contract_matches(
            contract.get("triggers", {}).get("negative"), body
        ):
            _diag(
                diagnostics,
                "AGENT_NEGATIVE_ROUTING",
                f"{name}: negative deferral is not traceable to wrapper",
            )
        if not _contract_matches(contract.get("output_contract"), body):
            _diag(
                diagnostics,
                "AGENT_OUTPUT_ROUTING",
                f"{name}: expected result is not traceable to wrapper",
            )
        for sibling in contract.get("nearest_siblings", []):
            if isinstance(sibling, dict) and (
                str(sibling.get("name", "")).lower() not in body.lower()
                or not _contract_matches(sibling.get("boundary"), body)
            ):
                _diag(
                    diagnostics,
                    "AGENT_SIBLING_ROUTING",
                    f"{name}: sibling deferral/boundary is not traceable",
                )
        writes_expected = bool(contract.get("writes"))
        writes_actual = frontmatter.get("writes", "").strip().lower() == "true"
        if writes_actual != writes_expected:
            _diag(
                diagnostics,
                "AGENT_WRITES_MISMATCH",
                f"{name}: wrapper writes flag does not match contract",
            )
        if re.search(
            rf"\b(use|select)\b.*\b{name.replace('-', r'[- ]')}\b.*"
            rf"\b(governed|handled|skill)\b",
            description,
            re.I,
        ):
            _diag(
                diagnostics,
                "AGENT_CIRCULAR_ROUTING",
                f"{name}: wrapper description is circular",
            )
    return sorted(set(diagnostics))


def validate_flow_routing(
    commands_dir: Path,
    plan_path: Path,
    target: Path,
    registry_path: Path = DEFAULT_REGISTRY,
) -> list[Diagnostic]:
    """Validate that generated flows route specialists conditionally."""
    diagnostics = validate_plan(plan_path, target, registry_path)
    if any(item.severity == "error" for item in diagnostics):
        return diagnostics
    plan = _load_plan(plan_path.expanduser().resolve(), diagnostics)
    specialists = {
        item["name"]: item
        for item in plan.get("skills", [])
        if str(item.get("category", "")).lower() in SPECIALIST_CATEGORIES
    }
    commands_dir = commands_dir.expanduser().resolve()
    if not commands_dir.is_dir():
        _diag(
            diagnostics,
            "COMMANDS_DIR_INVALID",
            f"commands directory is not a directory: {commands_dir}",
        )
        return sorted(set(diagnostics))
    for flow_name in ("flow-feature", "flow-review"):
        path = commands_dir / f"{flow_name}.md"
        if not path.is_file() or path.is_symlink():
            _diag(
                diagnostics,
                "FLOW_ROUTING_MISSING",
                f"{flow_name}: generated flow command is missing",
            )
            continue
        text = path.read_text(encoding="utf-8")
        _, body = _parse_frontmatter(text)
        if specialists and "specialist routing" not in body.lower():
            _diag(
                diagnostics,
                "FLOW_SPECIALIST_SECTION",
                f"{flow_name}: Specialist Routing section is missing",
            )
        frontmatter_text = text.split("---", 2)[1] if text.startswith("---") else ""
        for name, contract in sorted(specialists.items()):
            agent_name = f"{name}-agent"
            if agent_name in frontmatter_text:
                _diag(
                    diagnostics,
                    "FLOW_SPECIALIST_UNCONDITIONAL",
                    f"{flow_name}: specialist {agent_name} is in a static stage",
                )
            matching_lines = [
                line
                for line in body.splitlines()
                if agent_name.lower() in line.lower()
            ]
            routing_text = "\n".join(matching_lines)
            if not matching_lines:
                _diag(
                    diagnostics,
                    "FLOW_SPECIALIST_UNROUTED",
                    f"{flow_name}: specialist {agent_name} is absent from routing",
                )
            if not _contract_matches(
                contract.get("triggers", {}).get("positive"), routing_text
            ) or not _contract_matches(
                contract.get("triggers", {}).get("negative"), routing_text
            ):
                _diag(
                    diagnostics,
                    "FLOW_SPECIALIST_TRIGGER",
                    f"{flow_name}: {agent_name} lacks positive/negative scope routing",
                )
            if flow_name == "flow-review" and contract.get("writes"):
                pattern = rf"{re.escape(agent_name)}[^\n]*(skip|not selected|write-capable)"
                if not re.search(pattern, routing_text, re.I):
                    _diag(
                        diagnostics,
                        "FLOW_REVIEW_WRITER",
                        f"{flow_name}: write-capable {agent_name} must be explicitly skipped",
                    )
    return sorted(set(diagnostics))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skills-dir")
    parser.add_argument("--agents-dir")
    parser.add_argument("--plan", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument(
        "--registry",
        default=str(DEFAULT_REGISTRY),
        help="machine-readable candidate registry",
    )
    parser.add_argument(
        "--plan-only",
        action="store_true",
        help="validate evidence and contracts before any SKILL.md files exist",
    )
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args(argv)
    if args.plan_only:
        diagnostics = validate_plan(
            Path(args.plan), Path(args.target), Path(args.registry)
        )
    elif not args.skills_dir:
        parser.error("--skills-dir is required unless --plan-only is used")
    else:
        diagnostics = validate(
            Path(args.skills_dir),
            Path(args.plan),
            Path(args.target),
            Path(args.registry),
        )
        if args.agents_dir:
            diagnostics = sorted(
                set(
                    diagnostics
                    + validate_agent_routing(
                        Path(args.agents_dir),
                        Path(args.plan),
                        Path(args.target),
                        registry_path=Path(args.registry),
                    )
                )
            )
    errors = [item for item in diagnostics if item.severity == "error"]
    if args.as_json:
        print(
            json.dumps(
                {
                    "valid": not errors,
                    "error_count": len(errors),
                    "warning_count": len(diagnostics) - len(errors),
                    "diagnostics": [item.as_dict() for item in diagnostics],
                },
                indent=2,
                sort_keys=True,
            )
        )
    else:
        for item in diagnostics:
            stream = sys.stderr if item.severity == "error" else sys.stdout
            print(f"{item.severity.upper()} [{item.code}] {item.message}", file=stream)
        print(
            f"skill quality: {'PASS' if not errors else 'FAIL'} "
            f"({len(errors)} errors, {len(diagnostics) - len(errors)} warnings)"
        )
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
