#!/usr/bin/env python3
"""Validate that discovery accounted for every surface of the target.

A scan that misses a module is indistinguishable from a scan that covered it,
because the only thing either produces is a list of what was found. The
generated accelerator then looks complete while carrying nothing about the part
nobody looked at, and the omission surfaces months later as a skill that does
not know a subsystem exists.

So each scanner writes, beside its report and its ledger, a coverage artifact
that gives every surface it saw one of four dispositions - `covered`,
`excluded`, `truncated`, `not-permitted` - and this validator holds the
artifacts against the target and against each other.

The second half of the same problem is what happens to a finding after it is
made. Seven scanners run in parallel and each states its claims in prose inside
its own ledger; nothing merges them, nothing detects that two of them contradict
each other, and nothing notices when a high-priority invariant that discovery
confirmed never reaches the plan. Reconciliation therefore promotes those prose
claims into one typed `project-claims.json`, and this validator holds the plan
against it: an invariant discovery found and the plan dropped is blocking, not a
judgement call made silently by whoever wrote the plan.

Invariants shared with the other gates in this directory: standard library
only, no execution, no network, byte-stable JSON output, and fail-closed - a
surface this validator cannot resolve is reported, not assumed clean. It never
opens a file it is checking the disposition of, so the secrets rule is enforced
without ever reading a secret.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

DISPOSITIONS = {"covered", "excluded", "truncated", "not-permitted"}
# Dispositions under which the scanner did look at some of the surface, so
# evidence from it is expected rather than contradictory. A truncated scan
# stopped partway; what it read before stopping is still evidence.
SCANNED_DISPOSITIONS = {"covered", "truncated"}
COVERAGE_FIELDS = {"surface", "kind", "disposition", "reason", "evidence_ids"}
SURFACE_KINDS = {"file", "tree"}

# VCS internals are the one thing exempt from an explicit disposition: they are
# not the target's own code, and the secrets rule forbids reading them anyway.
# Everything else - `vendor/`, `node_modules/`, `var/` - costs one honest line
# saying it was deliberately left out, which is the whole point of the artifact.
BUILTIN_EXEMPT = (".git",)

# Surfaces whose contents may never be read. A disposition of `covered` on any
# of these is a claim to have done what the contract forbids.
SECRET_SURFACE_PATTERNS = (
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.jks",
    "id_rsa",
    "id_ed25519",
    "*.keystore",
    "auth.json",
)

# Directories whose immediate children are separately meaningful surfaces: a
# module, a test family, an integration. Listing `src/` as covered without
# saying anything about `src/Legacy` is the omission this catches.
MODULE_CONTAINERS = (
    "src",
    "app",
    "apps",
    "lib",
    "libs",
    "modules",
    "packages",
    "bundles",
    "tests",
    "test",
    "config",
)
SURFACE_BUDGET = 4000

CLAIM_FIELDS = {
    "id",
    "statement",
    "claim_class",
    "priority",
    "evidence_ids",
    "scanners",
    "status",
}
CLAIM_CLASSES = {
    "invariant",
    "capability",
    "convention",
    "risk",
    "integration",
    "command",
}
CLAIM_PRIORITIES = {"high", "medium", "low"}
CLAIM_STATUSES = {"confirmed", "inferred", "unknown"}
CONTRADICTION_FIELDS = {"id", "claim_ids", "statement", "resolution"}
CONTRADICTION_RESOLUTIONS = {"unresolved", "resolved", "accepted"}
# Two statements are the same statement when they share this many meaningful
# words. The bar is the one the plan gate already uses for tracing a contract to
# its skill, so an invariant is not "carried" by a sentence that merely happens
# to mention the same file.
CLAIM_MATCH_TOKENS = 2
CLAIM_SERVICE_WORDS = frozenset(
    {
        "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has",
        "have", "in", "is", "it", "its", "must", "never", "no", "not", "of",
        "on", "once", "only", "or", "that", "the", "their", "then", "this",
        "to", "when", "which", "with",
    }
)


@dataclass(frozen=True)
class Diagnostic:
    severity: str
    code: str
    message: str


def _diag(
    diagnostics: list[Diagnostic], code: str, message: str, severity: str = "error"
) -> None:
    diagnostics.append(Diagnostic(severity, code, message))


def _is_nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _is_string_list(value: Any, *, allow_empty: bool = True) -> bool:
    if not isinstance(value, list) or (not allow_empty and not value):
        return False
    return all(_is_nonempty_string(item) for item in value)


def _matches(surface: str, kind: str, path: str) -> bool:
    """Whether one declared surface accounts for one target path."""
    surface = surface.strip().rstrip("/")
    if surface in {"**", "*"}:
        return True
    if surface.endswith("/**"):
        prefix = surface[:-3]
        return path == prefix or path.startswith(prefix + "/")
    if kind == "tree":
        return path == surface or path.startswith(surface + "/")
    if any(char in surface for char in "*?["):
        return fnmatch.fnmatch(path, surface)
    return path == surface


def _is_secret_surface(surface: str) -> bool:
    name = surface.strip().rstrip("/").split("/")[-1]
    return any(fnmatch.fnmatch(name, pattern) for pattern in SECRET_SURFACE_PATTERNS)


def target_surfaces(target: Path, diagnostics: list[Diagnostic]) -> list[str]:
    """Every surface of the target that requires a disposition.

    Depth one throughout, plus the immediate children of the directories that
    hold modules, test families, and configuration - the granularity at which
    "the scan missed a subsystem" is a statement someone can act on.
    """
    surfaces: list[str] = []
    try:
        entries = sorted(target.iterdir(), key=lambda item: item.name)
    except OSError as error:
        _diag(diagnostics, "SCAN_TARGET_UNREADABLE", f"target is unreadable: {error}")
        return []
    for entry in entries:
        if entry.name in BUILTIN_EXEMPT or entry.is_symlink():
            continue
        surfaces.append(entry.name)
        if entry.is_dir() and entry.name in MODULE_CONTAINERS:
            try:
                children = sorted(entry.iterdir(), key=lambda item: item.name)
            except OSError as error:
                _diag(
                    diagnostics,
                    "SCAN_TARGET_UNREADABLE",
                    f"{entry.name} is unreadable: {error}",
                )
                continue
            for child in children:
                if child.is_symlink():
                    continue
                surfaces.append(f"{entry.name}/{child.name}")
        if len(surfaces) > SURFACE_BUDGET:
            _diag(
                diagnostics,
                "SCAN_TARGET_TOO_LARGE",
                f"target exposes more than {SURFACE_BUDGET} surfaces; discovery "
                "coverage cannot be established",
            )
            return []
    return surfaces


def _load(path: Path, diagnostics: list[Diagnostic]) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        _diag(
            diagnostics,
            "SCAN_ARTIFACT_UNREADABLE",
            f"{path.name} is not readable JSON: {error}",
        )
        return None
    if not isinstance(value, dict):
        _diag(
            diagnostics,
            "SCAN_ARTIFACT_UNREADABLE",
            f"{path.name} root must be an object",
        )
        return None
    return value


def _read_coverage(
    path: Path, diagnostics: list[Diagnostic]
) -> tuple[str, list[dict[str, Any]]]:
    document = _load(path, diagnostics)
    if document is None:
        return "", []
    scanner = document.get("scanner")
    if not _is_nonempty_string(scanner):
        _diag(
            diagnostics,
            "SCAN_COVERAGE_INVALID",
            f"{path.name} must name the scanner that produced it",
        )
        return "", []
    scanner = scanner.strip()
    if set(document) != {"scanner", "target_root", "surfaces"}:
        _diag(
            diagnostics,
            "SCAN_COVERAGE_INVALID",
            f"{scanner} coverage must define exactly scanner, target_root, "
            "and surfaces",
        )
        return scanner, []
    entries = document.get("surfaces")
    if not isinstance(entries, list) or not entries:
        _diag(
            diagnostics,
            "SCAN_COVERAGE_INVALID",
            f"{scanner} coverage must declare a non-empty surfaces array",
        )
        return scanner, []
    valid: list[dict[str, Any]] = []
    for index, entry in enumerate(entries):
        if (
            not isinstance(entry, dict)
            or set(entry) != COVERAGE_FIELDS
            or not _is_nonempty_string(entry.get("surface"))
            or entry.get("kind") not in SURFACE_KINDS
            or entry.get("disposition") not in DISPOSITIONS
            or not _is_nonempty_string(entry.get("reason"))
            or not _is_string_list(entry.get("evidence_ids"))
        ):
            _diag(
                diagnostics,
                "SCAN_COVERAGE_INVALID",
                f"{scanner} surfaces[{index}] must declare surface, kind, "
                "disposition, reason, and evidence_ids",
            )
            continue
        surface = str(entry["surface"]).strip()
        if surface.startswith("/") or ".." in Path(surface).parts:
            _diag(
                diagnostics,
                "SCAN_SURFACE_UNSAFE",
                f"{scanner} declares a surface outside the target: {surface}",
            )
            continue
        valid.append(entry)
    return scanner, valid


def _ledger_ids(path: Path, diagnostics: list[Diagnostic]) -> set[str]:
    """Every evidence id one scanner's ledger declares, absence entries too."""
    document = _load(path, diagnostics)
    if document is None:
        return set()
    return {
        entry["id"].strip()
        for entry in document.get("evidence") or []
        if isinstance(entry, dict) and _is_nonempty_string(entry.get("id"))
    }


def _ledger_paths(path: Path, diagnostics: list[Diagnostic]) -> dict[str, str]:
    """Evidence id to cited path, for the ledger beside a coverage artifact."""
    document = _load(path, diagnostics)
    if document is None:
        return {}
    cited: dict[str, str] = {}
    for entry in document.get("evidence") or []:
        if not isinstance(entry, dict) or not _is_nonempty_string(entry.get("id")):
            continue
        # A URL entry cites external authority and an absence entry cites a
        # search; neither occupies a surface of this target.
        if _is_nonempty_string(entry.get("path")):
            cited[entry["id"].strip()] = str(entry["path"]).strip()
    return cited


def _claim_tokens(text: str) -> set[str]:
    words = "".join(
        character if character.isalnum() else " " for character in text.lower()
    ).split()
    return {word for word in words if len(word) > 2} - CLAIM_SERVICE_WORDS


def _same_statement(left: str, right: str) -> bool:
    return len(_claim_tokens(left) & _claim_tokens(right)) >= CLAIM_MATCH_TOKENS


def _validate_claims(
    task_dir: Path,
    known_evidence: dict[str, set[str]],
    diagnostics: list[Diagnostic],
) -> list[dict[str, Any]]:
    """Grade the reconciled claim set, and report what it contradicts.

    The claims are a promotion of what the scanners already said in prose, so
    nothing here invents a finding; it makes the finding addressable. A claim
    with no resolvable evidence is the one shape that must not survive: it is
    the point at which a scan's conclusion stops being traceable to the target.
    """
    claims_path = task_dir / "project-claims.json"
    if not claims_path.is_file():
        _diag(
            diagnostics,
            "CLAIMS_MISSING",
            "reconciliation wrote no project-claims.json, so nothing merges "
            "seven parallel scanners' findings or detects that two contradict",
        )
        return []
    document = _load(claims_path, diagnostics)
    if document is None:
        return []
    if set(document) != {"target_root", "claims", "contradictions"}:
        _diag(
            diagnostics,
            "CLAIMS_INVALID",
            "project-claims.json must define exactly target_root, claims, "
            "and contradictions",
        )
        return []
    all_evidence = {
        identifier for ids in known_evidence.values() for identifier in ids
    }
    claims: list[dict[str, Any]] = []
    seen_ids: set[str] = set()
    for index, claim in enumerate(document.get("claims") or []):
        if (
            not isinstance(claim, dict)
            or set(claim) != CLAIM_FIELDS
            or not _is_nonempty_string(claim.get("id"))
            or not _is_nonempty_string(claim.get("statement"))
            or claim.get("claim_class") not in CLAIM_CLASSES
            or claim.get("priority") not in CLAIM_PRIORITIES
            or claim.get("status") not in CLAIM_STATUSES
            or not _is_string_list(claim.get("evidence_ids"), allow_empty=False)
            or not _is_string_list(claim.get("scanners"), allow_empty=False)
        ):
            _diag(
                diagnostics,
                "CLAIMS_INVALID",
                f"claims[{index}] must declare id, statement, claim_class, "
                "priority, evidence_ids, scanners, and status",
            )
            continue
        identifier = claim["id"].strip()
        if identifier in seen_ids:
            _diag(
                diagnostics,
                "CLAIM_ID_DUPLICATE",
                f"project-claims.json repeats claim id {identifier}",
            )
            continue
        seen_ids.add(identifier)
        unknown = sorted(
            item for item in claim["evidence_ids"] if item.strip() not in all_evidence
        )
        if unknown:
            _diag(
                diagnostics,
                "CLAIM_EVIDENCE_UNKNOWN",
                f"{identifier} rests on evidence no scanner ledger carries: "
                f"{', '.join(unknown)}",
            )
            continue
        absent = sorted(
            item for item in claim["scanners"] if item.strip() not in known_evidence
        )
        if absent:
            _diag(
                diagnostics,
                "CLAIM_SCANNER_UNKNOWN",
                f"{identifier} is attributed to a scanner that wrote no ledger: "
                f"{', '.join(absent)}",
            )
            continue
        claims.append(claim)

    by_id = {claim["id"].strip(): claim for claim in claims}
    for index, entry in enumerate(document.get("contradictions") or []):
        if (
            not isinstance(entry, dict)
            or set(entry) != CONTRADICTION_FIELDS
            or not _is_nonempty_string(entry.get("id"))
            or not _is_nonempty_string(entry.get("statement"))
            or entry.get("resolution") not in CONTRADICTION_RESOLUTIONS
            or not _is_string_list(entry.get("claim_ids"), allow_empty=False)
            or len(entry["claim_ids"]) < 2
        ):
            _diag(
                diagnostics,
                "CLAIMS_INVALID",
                f"contradictions[{index}] must name at least two claim ids, "
                "a statement, and a resolution",
            )
            continue
        missing = sorted(
            item for item in entry["claim_ids"] if item.strip() not in by_id
        )
        if missing:
            _diag(
                diagnostics,
                "CONTRADICTION_CLAIM_UNKNOWN",
                f"{entry['id']} contradicts claims that do not exist: "
                f"{', '.join(missing)}",
            )
            continue
        if entry["resolution"] != "unresolved":
            continue
        involved = [by_id[item.strip()] for item in entry["claim_ids"]]
        blocking = [
            claim
            for claim in involved
            if claim["claim_class"] == "invariant" and claim["priority"] == "high"
        ]
        if blocking:
            _diag(
                diagnostics,
                "CONTRADICTION_UNRESOLVED_INVARIANT",
                f"{entry['id']} leaves a high-priority invariant in unresolved "
                "conflict; a skill cannot be told to honour something discovery "
                "is still arguing about",
            )
        else:
            _diag(
                diagnostics,
                "CONTRADICTION_UNRESOLVED",
                f"{entry['id']} is unresolved: {entry['statement']}",
                "warning",
            )
    return claims


def _evidence_locator(entry: dict[str, Any]) -> str:
    """What an evidence entry points at, independent of how it is numbered.

    `profile-synthesizer` renumbers scanner evidence when it merges seven
    ledgers, so an id is not a stable join key between discovery and the plan -
    matching on one would report every honest plan as having lost every
    invariant. What does survive the merge is the thing cited: a target path, a
    URL, or the subject of an absence.
    """
    for field in ("path", "url"):
        if _is_nonempty_string(entry.get(field)):
            return str(entry[field]).strip()
    absence = entry.get("absence")
    if isinstance(absence, dict) and _is_nonempty_string(absence.get("subject")):
        return str(absence["subject"]).strip()
    return ""


def _locators(document: dict[str, Any]) -> dict[str, str]:
    return {
        entry["id"].strip(): _evidence_locator(entry)
        for entry in document.get("evidence") or []
        if isinstance(entry, dict) and _is_nonempty_string(entry.get("id"))
    }


def _validate_invariant_carry_through(
    claims: list[dict[str, Any]],
    ledger_locators: dict[str, str],
    plan_path: Path,
    diagnostics: list[Diagnostic],
) -> None:
    """Report a high-priority invariant discovery found and the plan dropped.

    Dropping one may well be right - the invariant may not intersect any
    selected skill - but it is a decision, and an undocumented decision is
    indistinguishable from an oversight. The plan already carries
    `critical_invariants` with their evidence, so nothing new has to be stored:
    an invariant is carried when the plan states it against at least one of the
    same cited sources, in words that share the statement.
    """
    plan = _load(plan_path, diagnostics)
    if plan is None:
        return
    plan_locators = _locators(plan)
    # From schema 1.4 a skill names the claims it rests on outright, so the
    # carry-through question stops being a guess about wording. The statement
    # match below stays for plans written before that.
    known = {claim["id"].strip() for claim in claims}
    referenced: set[str] = set()
    for skill in plan.get("skills") or []:
        if not isinstance(skill, dict):
            continue
        unknown = sorted(
            str(item).strip()
            for item in skill.get("claim_ids") or []
            if _is_nonempty_string(item) and str(item).strip() not in known
        )
        if unknown:
            _diag(
                diagnostics,
                "CLAIM_ID_UNKNOWN",
                f"{skill.get('name', '?')} rests on claims reconciliation did "
                f"not make: {', '.join(unknown)}",
            )
        referenced.update(
            str(item).strip()
            for item in skill.get("claim_ids") or []
            if _is_nonempty_string(item)
        )
    carried = [
        (
            str(entry["statement"]),
            {
                plan_locators.get(str(item).strip(), "")
                for item in entry.get("evidence_ids") or []
            }
            - {""},
        )
        for entry in plan.get("critical_invariants") or []
        if isinstance(entry, dict) and _is_nonempty_string(entry.get("statement"))
    ]
    for claim in claims:
        if claim["claim_class"] != "invariant" or claim["priority"] != "high":
            continue
        sources = {
            ledger_locators.get(item.strip(), "") for item in claim["evidence_ids"]
        } - {""}
        if claim["id"].strip() in referenced:
            continue
        if any(
            sources & entry_sources and _same_statement(statement, claim["statement"])
            for statement, entry_sources in carried
        ):
            continue
        _diag(
            diagnostics,
            "CLAIM_INVARIANT_LOST",
            f"{claim['id']} is a high-priority invariant discovery confirmed "
            f"and the plan does not carry: {claim['statement']}",
        )


def validate(
    target: Path, task_dir: Path, plan_path: Path | None = None
) -> list[Diagnostic]:
    diagnostics: list[Diagnostic] = []
    coverage_files = sorted(task_dir.glob("*-coverage.json"))
    if not coverage_files:
        _diag(
            diagnostics,
            "SCAN_COVERAGE_MISSING",
            f"no scanner coverage artifact in {task_dir.name}; discovery "
            "completeness cannot be established",
        )
        return diagnostics

    surfaces = target_surfaces(target, diagnostics)
    accounted: dict[str, list[tuple[str, str]]] = {name: [] for name in surfaces}
    seen_scanners: set[str] = set()
    known_evidence: dict[str, set[str]] = {}
    ledger_locators: dict[str, str] = {}

    for coverage_path in coverage_files:
        scanner, entries = _read_coverage(coverage_path, diagnostics)
        if not scanner:
            continue
        if scanner in seen_scanners:
            _diag(
                diagnostics,
                "SCAN_COVERAGE_DUPLICATE",
                f"more than one coverage artifact claims to be {scanner}",
            )
            continue
        seen_scanners.add(scanner)
        ledger_path = task_dir / f"{scanner}-evidence.json"
        if not ledger_path.is_file():
            _diag(
                diagnostics,
                "SCAN_LEDGER_MISSING",
                f"{scanner} declares coverage but wrote no evidence ledger",
            )
            cited: dict[str, str] = {}
        else:
            cited = _ledger_paths(ledger_path, diagnostics)
            known_evidence[scanner] = _ledger_ids(ledger_path, diagnostics)
            ledger_document = _load(ledger_path, diagnostics)
            if ledger_document is not None:
                ledger_locators.update(_locators(ledger_document))

        scanned_surfaces = [
            entry
            for entry in entries
            if entry["disposition"] in SCANNED_DISPOSITIONS
        ]
        for entry in entries:
            surface = str(entry["surface"]).strip()
            disposition = str(entry["disposition"])
            if disposition == "covered" and _is_secret_surface(surface):
                _diag(
                    diagnostics,
                    "SCAN_SECRET_COVERED",
                    f"{scanner} claims to have covered a secret-bearing "
                    f"surface: {surface}. Existence may be recorded; contents "
                    "may not.",
                )
            if disposition == "covered":
                supporting = [
                    identifier
                    for identifier in entry["evidence_ids"]
                    if identifier in cited
                    and _matches(surface, str(entry["kind"]), cited[identifier])
                ]
                if not supporting:
                    _diag(
                        diagnostics,
                        "SCAN_COVERED_WITHOUT_EVIDENCE",
                        f"{scanner} reports {surface} covered but its ledger "
                        "carries no evidence inside it",
                    )
            elif disposition not in SCANNED_DISPOSITIONS and entry["evidence_ids"]:
                _diag(
                    diagnostics,
                    "SCAN_DISPOSITION_EVIDENCE_CONFLICT",
                    f"{scanner} marks {surface} {disposition} yet cites "
                    "evidence from it",
                )
            if disposition == "truncated":
                _diag(
                    diagnostics,
                    "SCAN_SURFACE_TRUNCATED",
                    f"{scanner} stopped short of {surface}: {entry['reason']}",
                    "warning",
                )
            for name in surfaces:
                if _matches(surface, str(entry["kind"]), name):
                    accounted[name].append((scanner, disposition))

        for identifier, cited_path in sorted(cited.items()):
            if not any(
                _matches(str(entry["surface"]).strip(), str(entry["kind"]), cited_path)
                for entry in scanned_surfaces
            ):
                _diag(
                    diagnostics,
                    "SCAN_EVIDENCE_OUTSIDE_COVERAGE",
                    f"{scanner} cites {identifier} from {cited_path}, which "
                    "none of the surfaces it says it read contains",
                )

    for name in surfaces:
        dispositions = accounted[name]
        if not dispositions:
            _diag(
                diagnostics,
                "SCAN_SURFACE_UNACCOUNTED",
                f"no scanner gave {name} a disposition; a surface nobody "
                "looked at is not the same as one nobody needed",
            )
            continue
        kinds = {disposition for _, disposition in dispositions}
        if "covered" in kinds and "not-permitted" in kinds:
            owners = ", ".join(
                f"{scanner}:{disposition}" for scanner, disposition in dispositions
            )
            _diag(
                diagnostics,
                "SCAN_DISPOSITION_CONFLICT",
                f"{name} is both covered and forbidden: {owners}",
            )

    claims = _validate_claims(task_dir, known_evidence, diagnostics)
    if plan_path is not None:
        if not plan_path.is_file():
            _diag(
                diagnostics,
                "CLAIMS_PLAN_UNREADABLE",
                f"{plan_path.name} does not exist, so no invariant can be "
                "shown to have survived into it",
            )
        else:
            _validate_invariant_carry_through(
                claims, ledger_locators, plan_path, diagnostics
            )
    return diagnostics


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--task-dir", required=True)
    parser.add_argument(
        "--plan",
        help="skill-generation-plan.json; when given, every high-priority "
        "invariant discovery confirmed must survive into it",
    )
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args(argv)
    diagnostics = validate(
        Path(args.target).resolve(),
        Path(args.task_dir),
        Path(args.plan) if args.plan else None,
    )
    errors = [item for item in diagnostics if item.severity == "error"]
    if args.as_json:
        print(
            json.dumps(
                {
                    "valid": not errors,
                    "diagnostics": [
                        {
                            "severity": item.severity,
                            "code": item.code,
                            "message": item.message,
                        }
                        for item in diagnostics
                    ],
                },
                indent=2,
                sort_keys=True,
            )
        )
    else:
        for item in diagnostics:
            stream = sys.stderr if item.severity == "error" else sys.stdout
            print(f"{item.severity.upper()}: {item.code}: {item.message}", file=stream)
        if not diagnostics:
            print("discovery coverage OK")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
