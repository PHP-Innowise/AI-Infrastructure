#!/usr/bin/env python3
"""Validate the record of the content review a staged bundle must survive.

The deterministic gates prove that generated skills are well formed, that
their evidence resolves, and that no two of them share a skeleton. They
cannot read a policy document and ask whether it would survive against a
different repository, whether a procedure is complete enough to execute
without the author in the room, or whether the generation report's claims
match what was actually staged. Those need a reader — `infra-validate`'s
parallel content reviewers.

What a gate *can* do is refuse to accept the reader's word for having looked.
This validator holds the review record against the publication plan: every
published file dispositioned exactly once, every reviewed file answered on
every dimension, every verbatim exemption limited to the sanctioned runtime
surface, every blocking finding repaired through its owning forge within the
bounded rounds — never accepted, never left open — and the mechanical gates
re-run after any repair.

Invariants shared with the other gates here: standard library only, no
execution, no network, byte-stable JSON output, fail-closed.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

# The questions a reviewer must answer for every published text file. Each is
# a question the deterministic gates provably cannot ask.
DIMENSIONS = (
    ("uniqueness", "whether the file would read identically against a different PHP repository"),
    ("completeness", "whether an agent could execute it end to end with nothing missing"),
    ("accuracy", "whether every claim still matches the current target state"),
    ("coherence", "whether it agrees with its sibling files and the plans"),
)
VERDICTS = {"pass", "fail"}
DIMENSION_FIELDS = {"verdict", "note"}

# Repairs route through the forge that owns the file class; a reviewer never
# edits in place, and a lane cannot be repaired by a forge that does not own it.
LANE_FORGES = {
    "skills": {"skill-forge"},
    "wrappers": {"agent-forge", "command-forge", "skill-flow-composer"},
    "policy": {"policy-forge"},
    "hooks": {"hook-forge"},
    "memory": {"memory-seed"},
    "reports": {"profile-synthesizer"},
}
LANES = set(LANE_FORGES) | {"runtime-verbatim"}

# The stack-agnostic runtime ships verbatim by design (see specs/MANIFEST.md);
# holding it to a project-uniqueness bar would be judging a carve-out by the
# rule it is exempt from. Nothing else may claim the exemption.
VERBATIM_PREFIXES = (
    "memory-bank/scripts/",
    "memory-bank/templates/",
    "project-brain/PROTOCOL.md",
    "project-brain/schemas/",
    "project-brain/templates/",
)

# The generator-side artifacts the review must also have walked: the profile
# and plan whose claims the staged content is supposed to realize.
REQUIRED_REPORT_BASENAMES = (
    "infra-scan-project-profile.md",
    "skill-generation-plan.json",
)

FINDING_ACTIONS = {"reforged", "escalated", "accepted"}
SEVERITIES = {"blocking", "advisory"}
MAX_REPAIR_ROUNDS = 2
REQUIRED_RECORD_FIELDS = {
    "task",
    "reviewer",
    "generation_root",
    "rounds",
    "files",
    "blockers",
}
REQUIRED_POST_REPAIR_GATES = {"validate_skill_quality", "validate_generated"}


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


def _load_review(path: Path, diagnostics: list[Diagnostic]) -> dict | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        _diag(
            diagnostics,
            "CONTENT_REVIEW_UNREADABLE",
            f"{path.name} is not readable JSON: {error}",
        )
        return None
    if not isinstance(value, dict):
        _diag(
            diagnostics,
            "CONTENT_REVIEW_UNREADABLE",
            f"{path.name} root must be an object",
        )
        return None
    return value


def _load_plan_paths(path: Path, diagnostics: list[Diagnostic]) -> list[str] | None:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        _diag(
            diagnostics,
            "CONTENT_REVIEW_PLAN_UNREADABLE",
            f"{path.name} is not readable: {error}",
        )
        return None
    return [line.strip() for line in text.splitlines() if line.strip()]


def _is_sanctioned_verbatim(path: str) -> bool:
    return any(
        path == prefix.rstrip("/") or path.startswith(prefix)
        for prefix in VERBATIM_PREFIXES
    )


def _check_finding(
    name: str,
    index: int,
    finding: Any,
    lane: str,
    rounds: int,
    diagnostics: list[Diagnostic],
) -> None:
    label = f"{name}.findings[{index}]"
    if (
        not isinstance(finding, dict)
        or finding.get("dimension") not in dict(DIMENSIONS)
        or finding.get("severity") not in SEVERITIES
        or not _is_nonempty_string(finding.get("statement"))
        or not isinstance(finding.get("resolution"), dict)
    ):
        _diag(
            diagnostics,
            "CONTENT_REVIEW_FINDING_INVALID",
            f"{label} must record a dimension, severity, statement, and resolution",
        )
        return
    resolution = finding["resolution"]
    action = resolution.get("action")
    if action not in FINDING_ACTIONS or not _is_nonempty_string(
        resolution.get("note")
    ):
        _diag(
            diagnostics,
            "CONTENT_REVIEW_FINDING_INVALID",
            f"{label} resolution must carry a sanctioned action and a note",
        )
        return
    if action == "accepted" and finding["severity"] == "blocking":
        _diag(
            diagnostics,
            "CONTENT_REVIEW_BLOCKER_ACCEPTED",
            f"{label} accepts a blocking finding; a blocker is repaired or "
            "escalated, never waved through",
        )
    if action == "escalated":
        _diag(
            diagnostics,
            "CONTENT_REVIEW_ESCALATION_OPEN",
            f"{label} is escalated to a human; publication waits for that "
            f"decision: {finding['statement']}",
        )
    if action == "reforged":
        forge = resolution.get("forge")
        allowed = LANE_FORGES.get(lane, set())
        if forge not in allowed:
            _diag(
                diagnostics,
                "CONTENT_REVIEW_FORGE_MISMATCH",
                f"{label} was repaired by {forge!r}, which does not own the "
                f"{lane} lane",
            )
        repair_round = resolution.get("round")
        if not isinstance(repair_round, int) or not 1 <= repair_round <= rounds:
            _diag(
                diagnostics,
                "CONTENT_REVIEW_FINDING_INVALID",
                f"{label} must name the repair round it was resolved in "
                f"(1..{rounds})",
            )


def validate(plan_path: Path, review_path: Path) -> list[Diagnostic]:
    diagnostics: list[Diagnostic] = []
    plan_paths = _load_plan_paths(plan_path, diagnostics)
    review = _load_review(review_path, diagnostics)
    if plan_paths is None or review is None:
        return diagnostics

    rounds = review.get("rounds")
    if not isinstance(rounds, int) or rounds < 0:
        _diag(diagnostics, "CONTENT_REVIEW_INVALID", "rounds must be a count >= 0")
        rounds = 0
    allowed_fields = REQUIRED_RECORD_FIELDS | (
        {"post_repair_gates"} if rounds > 0 else set()
    )
    if not REQUIRED_RECORD_FIELDS <= set(review) or not set(review) <= allowed_fields:
        _diag(
            diagnostics,
            "CONTENT_REVIEW_INVALID",
            "the record must define exactly task, reviewer, generation_root, "
            "rounds, files, and blockers (plus post_repair_gates after repairs)",
        )
        return diagnostics
    if not _is_nonempty_string(review.get("task")) or not _is_nonempty_string(
        review.get("generation_root")
    ):
        _diag(
            diagnostics,
            "CONTENT_REVIEW_INVALID",
            "task and generation_root must name this run",
        )
    if review.get("reviewer") != "independent":
        _diag(
            diagnostics,
            "CONTENT_REVIEW_NOT_INDEPENDENT",
            "the review must be carried out by a reader that did not author "
            "the content; a forge is the worst judge of whether its own file "
            "is distinguishable from a template",
        )
    if rounds > MAX_REPAIR_ROUNDS:
        _diag(
            diagnostics,
            "CONTENT_REVIEW_ROUNDS_EXCEEDED",
            f"{rounds} repair rounds exceed the bound of {MAX_REPAIR_ROUNDS}; "
            "an oscillating forge is an escalation, not a loop",
        )
    if rounds > 0:
        gates = review.get("post_repair_gates")
        ran = (
            {str(item).strip() for item in gates}
            if isinstance(gates, list)
            and all(_is_nonempty_string(item) for item in gates)
            else set()
        )
        if not REQUIRED_POST_REPAIR_GATES <= ran:
            _diag(
                diagnostics,
                "CONTENT_REVIEW_GATES_NOT_RERUN",
                "a repair changed staged content, so the mechanical gates "
                "must have been re-run: "
                + ", ".join(sorted(REQUIRED_POST_REPAIR_GATES - ran)),
            )

    blockers = review.get("blockers")
    if not isinstance(blockers, list) or any(
        not _is_nonempty_string(item) for item in blockers
    ):
        _diag(
            diagnostics, "CONTENT_REVIEW_INVALID", "blockers must be an array of statements"
        )
        blockers = []
    for blocker in blockers:
        _diag(
            diagnostics,
            "CONTENT_REVIEW_BLOCKER_OPEN",
            f"the review left a blocker open: {blocker}",
        )

    entries = review.get("files")
    if not isinstance(entries, list):
        _diag(diagnostics, "CONTENT_REVIEW_INVALID", "files must be an array")
        entries = []
    planned = set(plan_paths)
    seen: set[str] = set()
    report_basenames: set[str] = set()
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict) or not _is_nonempty_string(entry.get("path")):
            _diag(
                diagnostics,
                "CONTENT_REVIEW_INVALID",
                f"files[{index}] must name the path it reviews",
            )
            continue
        path = str(entry["path"]).strip()
        lane = entry.get("lane")
        if lane not in LANES:
            _diag(
                diagnostics,
                "CONTENT_REVIEW_LANE_UNKNOWN",
                f"{path} claims lane {lane!r}; lanes are "
                + ", ".join(sorted(LANES)),
            )
            continue
        if lane == "reports":
            report_basenames.add(path.rsplit("/", 1)[-1])
        elif path not in planned:
            _diag(
                diagnostics,
                "CONTENT_REVIEW_FILE_UNKNOWN",
                f"the review covers {path}, which the publication plan does "
                "not list",
            )
            continue
        if path in seen:
            _diag(
                diagnostics,
                "CONTENT_REVIEW_FILE_DUPLICATE",
                f"the review covers {path} more than once",
            )
            continue
        seen.add(path)
        if lane == "runtime-verbatim":
            if not _is_sanctioned_verbatim(path):
                _diag(
                    diagnostics,
                    "CONTENT_REVIEW_VERBATIM_UNSANCTIONED",
                    f"{path} claims the verbatim-runtime exemption outside "
                    "the sanctioned runtime surface",
                )
            if not _is_nonempty_string(entry.get("basis")):
                _diag(
                    diagnostics,
                    "CONTENT_REVIEW_INVALID",
                    f"{path} must state the basis for shipping verbatim",
                )
            continue
        expected = {"path", "lane", "findings"} | {key for key, _ in DIMENSIONS}
        if set(entry) != expected:
            _diag(
                diagnostics,
                "CONTENT_REVIEW_DIMENSION_MISSING",
                f"{path} was not answered on every dimension: "
                f"{', '.join(sorted(expected - set(entry))) or 'unexpected members'}",
            )
            continue
        for key, question in DIMENSIONS:
            answer = entry[key]
            if (
                not isinstance(answer, dict)
                or set(answer) != DIMENSION_FIELDS
                or answer.get("verdict") not in VERDICTS
                or not _is_nonempty_string(answer.get("note"))
            ):
                _diag(
                    diagnostics,
                    "CONTENT_REVIEW_ANSWER_INVALID",
                    f"{path}.{key} must record a verdict and a note on {question}",
                )
                continue
            if answer["verdict"] == "fail":
                _diag(
                    diagnostics,
                    "CONTENT_REVIEW_VERDICT_FAILED",
                    f"{path} failed review on {key}: {answer['note']}",
                )
        findings = entry["findings"]
        if not isinstance(findings, list):
            _diag(
                diagnostics,
                "CONTENT_REVIEW_INVALID",
                f"{path}.findings must be an array",
            )
            continue
        for finding_index, finding in enumerate(findings):
            _check_finding(path, finding_index, finding, lane, rounds, diagnostics)

    for path in sorted(planned - seen):
        _diag(
            diagnostics,
            "CONTENT_REVIEW_FILE_MISSING",
            f"{path} was published but never reviewed",
        )
    for basename in REQUIRED_REPORT_BASENAMES:
        if basename not in report_basenames:
            _diag(
                diagnostics,
                "CONTENT_REVIEW_REPORTS_MISSING",
                f"the review never walked {basename}; staged content is only "
                "as honest as the plan it claims to realize",
            )
    return diagnostics


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--publication-plan", required=True)
    parser.add_argument("--review", required=True)
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args(argv)
    diagnostics = validate(Path(args.publication_plan), Path(args.review))
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
            print("content review OK")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
