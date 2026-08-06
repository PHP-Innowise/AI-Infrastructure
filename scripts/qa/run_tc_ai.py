#!/usr/bin/env python3
"""Run deterministic TC-AI checks and emit a complete per-run evidence contract."""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Dict, Iterable, List, Mapping, Sequence, Set

from build_sha256_manifest import render_manifest
from common import (
    CODEX_CASE_IDS,
    CODEX_RUN_IDS,
    ROOT,
    QaError,
    atomic_write_json,
    atomic_write_text,
    git_output,
    load_json,
    repository_commit,
    utc_now,
)


DEFAULT_CASES = ROOT / "qa" / "tc-ai" / "cases.json"
DEFAULT_OUTPUT = ROOT / "docs" / "qa" / "evidence"


def validate_catalog(catalog: Mapping[str, Any]) -> List[Dict[str, Any]]:
    if catalog.get("schema_version") != "1.0.0":
        raise QaError("Unsupported TC-AI catalog schema_version")
    cases = catalog.get("cases")
    if not isinstance(cases, list) or len(cases) != 18:
        raise QaError("TC-AI catalog must contain exactly 18 cases")
    run_ids = [case.get("run_id") for case in cases]
    case_ids = [case.get("test_case_id") for case in cases]
    if set(run_ids) != set(CODEX_RUN_IDS) or len(set(run_ids)) != 18:
        raise QaError("TC-AI catalog run IDs must be RUN-20260803-001..018")
    if set(case_ids) != set(CODEX_CASE_IDS) or len(set(case_ids)) != 18:
        raise QaError("TC-AI catalog case IDs must be TC-AI-001..018")
    for case in cases:
        if case.get("classification") not in {
            "deterministic",
            "hybrid",
            "native",
        }:
            raise QaError("Invalid classification for {0}".format(case.get("run_id")))
        if not isinstance(case.get("commands"), list):
            raise QaError("commands must be an array for {0}".format(case["run_id"]))
        if (
            case["classification"] == "deterministic"
            and not case["commands"]
            and not case.get("blocker")
        ):
            raise QaError(
                "Deterministic case lacks commands or blocker: {0}".format(
                    case["run_id"]
                )
            )
    return list(cases)


def missing_test_patterns(
    patterns: Iterable[Mapping[str, str]], root: Path
) -> List[str]:
    missing: List[str] = []
    for pattern in patterns:
        candidates = sorted(root.glob(pattern["glob"]))
        expression = re.compile(pattern["regex"])
        if not any(
            expression.search(path.read_text(encoding="utf-8", errors="replace"))
            for path in candidates
            if path.is_file()
        ):
            missing.append(
                "{0} / {1}".format(pattern["glob"], pattern["regex"])
            )
    return missing


def expand_argv(argv: Sequence[str], root: Path) -> List[str]:
    exact_replacements = {
        "{python}": sys.executable,
        "{root}": str(root),
    }
    embedded_replacements = {
        "{python_shell}": shlex.quote(sys.executable),
        "{root_shell}": shlex.quote(str(root)),
    }
    expanded: List[str] = []
    for value in argv:
        if value in exact_replacements:
            expanded.append(exact_replacements[value])
            continue
        rendered = value
        for placeholder, replacement in embedded_replacements.items():
            rendered = rendered.replace(placeholder, replacement)
        # Legacy shell snippets used a bare `python3`; execute them with the
        # runner's pinned interpreter so QA dependencies stay available.
        rendered = re.sub(
            r"(?<![\w/.-])python3(?=\s)",
            shlex.quote(sys.executable),
            rendered,
        )
        expanded.append(rendered)
    return expanded


def sanitized_argv(argv: Sequence[str], root: Path) -> List[str]:
    """Remove machine-local interpreter and repository paths from evidence."""
    replacements = (
        (str(root.resolve()), "{root}"),
        (sys.executable, "{python}"),
        (str(Path.home()), "~"),
    )
    sanitized: List[str] = []
    for value in argv:
        rendered = value
        for private_value, placeholder in replacements:
            rendered = rendered.replace(private_value, placeholder)
        sanitized.append(rendered)
    return sanitized


def environment_record(root: Path) -> Dict[str, Any]:
    return {
        "captured_at": utc_now(),
        "platform": platform.platform(),
        "python": sys.version,
        "executable": "{python}",
        "repository_root": "{root}",
        "repository_commit": repository_commit(root),
        "branch": git_output(["branch", "--show-current"], root=root).strip(),
    }


def assertion_summary(items: List[Dict[str, str]]) -> Dict[str, Any]:
    counts = {
        status: sum(item["status"] == status for item in items)
        for status in ("passed", "failed", "skipped")
    }
    return {
        "executed": len(items),
        "passed": counts["passed"],
        "failed": counts["failed"],
        "skipped": counts["skipped"],
        "items": items,
    }


def final_status(assertions: Mapping[str, int]) -> str:
    if assertions["failed"]:
        return "failed"
    if assertions["skipped"]:
        return "skipped"
    return "passed"


def native_record(
    case: Mapping[str, Any], available_hosts: Set[str]
) -> Dict[str, Any]:
    requirement = case.get("native_requirement")
    if not requirement:
        return {
            "required": False,
            "status": "not-applicable",
            "reason": None,
        }
    required_hosts = {part.strip() for part in str(requirement).split(",")}
    missing = sorted(required_hosts - available_hosts)
    if missing:
        return {
            "required": True,
            "status": "skipped",
            "reason": "Native host unavailable: {0}".format(", ".join(missing)),
        }
    return {
        "required": True,
        "status": "skipped",
        "reason": (
            "Native host availability was declared, but this deterministic runner "
            "cannot manufacture native-client observations; attach reviewed native "
            "evidence separately."
        ),
    }


def run_case(
    case: Mapping[str, Any],
    root: Path,
    output_root: Path,
    available_hosts: Set[str],
) -> Path:
    destination = output_root / str(case["run_id"])
    if destination.exists():
        raise QaError(
            "Evidence directory already exists; refusing to overwrite: {0}".format(
                destination
            )
        )
    git_before = git_output(["status", "--porcelain=v1"], root=root)
    started_at = utc_now()

    with tempfile.TemporaryDirectory(prefix="tc-ai-evidence-") as temporary_name:
        temporary = Path(temporary_name)
        (temporary / "stdout").mkdir()
        (temporary / "stderr").mkdir()
        command_records: List[Dict[str, Any]] = []
        assertions: List[Dict[str, str]] = []

        missing = missing_test_patterns(
            case.get("required_test_patterns", []), root
        )
        blocker = case.get("blocker")
        if missing:
            assertions.append(
                {
                    "id": "required-regression-discovery",
                    "status": "skipped",
                    "message": (
                        "Required focused regression tests are not discoverable: "
                        + "; ".join(missing)
                    ),
                }
            )
        elif blocker and not case.get("commands"):
            assertions.append(
                {
                    "id": "declared-blocker",
                    "status": "skipped",
                    "message": str(blocker),
                }
            )
        else:
            for index, command in enumerate(case.get("commands", []), 1):
                command_id = str(command["id"])
                cwd = (root / str(command["cwd"])).resolve()
                try:
                    cwd.relative_to(root.resolve())
                except ValueError as exc:
                    raise QaError(
                        "Command cwd escapes repository: {0}".format(cwd)
                    ) from exc
                if not cwd.is_dir():
                    assertions.append(
                        {
                            "id": command_id,
                            "status": "skipped",
                            "message": "Command cwd does not exist: {0}".format(
                                command["cwd"]
                            ),
                        }
                    )
                    continue
                required_executable = command.get("requires_executable")
                if required_executable and shutil.which(
                    str(required_executable), path=os.environ.get("PATH")
                ) is None:
                    assertions.append(
                        {
                            "id": command_id,
                            "status": "skipped",
                            "message": (
                                "Required executable is not configured: "
                                + str(required_executable)
                            ),
                        }
                    )
                    continue
                argv = expand_argv(command["argv"], root)
                command_started = utc_now()
                completed = subprocess.run(
                    argv,
                    cwd=str(cwd),
                    text=True,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                )
                command_ended = utc_now()
                stdout_path = Path("stdout") / "{0:02d}-{1}.txt".format(
                    index, command_id
                )
                stderr_path = Path("stderr") / "{0:02d}-{1}.txt".format(
                    index, command_id
                )
                atomic_write_text(temporary / stdout_path, completed.stdout)
                atomic_write_text(temporary / stderr_path, completed.stderr)
                command_records.append(
                    {
                        "id": command_id,
                        "argv": sanitized_argv(argv, root),
                        "cwd": cwd.relative_to(root).as_posix() or ".",
                        "started_at": command_started,
                        "ended_at": command_ended,
                        "exit_code": completed.returncode,
                        "stdout_path": stdout_path.as_posix(),
                        "stderr_path": stderr_path.as_posix(),
                    }
                )
                assertions.append(
                    {
                        "id": command_id,
                        "status": (
                            "passed" if completed.returncode == 0 else "failed"
                        ),
                        "message": "exit code {0}".format(completed.returncode),
                    }
                )

        native = native_record(case, available_hosts)
        if native["required"]:
            assertions.append(
                {
                    "id": "native-client-observation",
                    "status": native["status"],
                    "message": str(native["reason"]),
                }
            )
        if not assertions:
            assertions.append(
                {
                    "id": "empty-case-guard",
                    "status": "failed",
                    "message": "Case executed no commands and declared no blocker",
                }
            )

        git_after = git_output(["status", "--porcelain=v1"], root=root)
        summary = assertion_summary(assertions)
        evidence = {
            "schema_version": "1.0.0",
            "run_id": case["run_id"],
            "test_case_id": case["test_case_id"],
            "classification": case["classification"],
            "started_at": started_at,
            "ended_at": utc_now(),
            "status": final_status(summary),
            "commands": command_records,
            "assertions": summary,
            "native": native,
        }
        atomic_write_json(temporary / "case.json", case)
        atomic_write_json(temporary / "environment.json", environment_record(root))
        atomic_write_json(temporary / "commands.json", command_records)
        atomic_write_json(temporary / "exit-codes.json", {
            command["id"]: command["exit_code"] for command in command_records
        })
        atomic_write_json(temporary / "assertions.json", summary)
        atomic_write_json(temporary / "run.json", evidence)
        atomic_write_text(temporary / "git-before.txt", git_before)
        atomic_write_text(temporary / "git-after.txt", git_after)
        atomic_write_json(
            temporary / "inventory.json",
            {
                "repository_status_changed": git_before != git_after,
                "files": sorted(
                    path.relative_to(temporary).as_posix()
                    for path in temporary.rglob("*")
                    if path.is_file()
                ),
            },
        )
        atomic_write_json(
            temporary / "provenance.json",
            {
                "catalog": str(DEFAULT_CASES.relative_to(ROOT)),
                "runner": str(Path(__file__).resolve().relative_to(ROOT)),
                "repository_commit": repository_commit(root),
                "generated_at": utc_now(),
                "historical_claim_used_as_pass_evidence": False,
            },
        )
        atomic_write_text(
            temporary / "SHA256SUMS.txt",
            render_manifest(temporary, temporary / "SHA256SUMS.txt"),
        )
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(temporary, destination)
    return destination


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, default=DEFAULT_CASES)
    parser.add_argument("--output-root", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument(
        "--run-id",
        action="append",
        help="Run only this ID; repeat for multiple IDs. Default: all 18.",
    )
    parser.add_argument(
        "--native-host",
        action="append",
        choices=("claude-code", "cursor", "codex"),
        default=[],
        help="Declare an installed host. Native observations still require review.",
    )
    parser.add_argument("--list", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        cases = validate_catalog(load_json(args.cases.resolve()))
        selected = set(args.run_id or CODEX_RUN_IDS)
        unknown = selected - set(CODEX_RUN_IDS)
        if unknown:
            raise QaError("Unknown Run IDs: {0}".format(", ".join(sorted(unknown))))
        cases = [case for case in cases if case["run_id"] in selected]
        if args.list:
            for case in cases:
                print(
                    "{0} {1} {2}".format(
                        case["run_id"],
                        case["classification"],
                        case["title"],
                    )
                )
            return 0
        destinations = [
            run_case(
                case,
                ROOT,
                args.output_root.resolve(),
                set(args.native_host),
            )
            for case in cases
        ]
    except (QaError, OSError, ValueError) as exc:
        print("TC-AI runner failed: {0}".format(exc), file=sys.stderr)
        return 2
    print("Wrote evidence for {0} case(s)".format(len(destinations)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
