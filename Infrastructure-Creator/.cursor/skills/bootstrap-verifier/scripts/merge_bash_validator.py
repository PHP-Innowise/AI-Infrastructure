#!/usr/bin/env python3
"""Section merge of a team-edited bash-validator.sh for infra-update step 6.

hook-forge step 2 builds every edition's bash-validator.sh around the
generator's generic section, copied byte for byte, and a team's own rules
go below the section, in BV_FRAMEWORK_RULES. A file edited there no longer
matches its manifest hash, so infra-update asks for a decision, and step 10
holds the section to the current generator's: keeping a section an earlier
release wrote fails that gate and rolls the whole update back, and taking
the staged version drops the team's rules. The section merge keeps both:
the target version with only the bytes from the opening marker through the
closing one replaced by the staged ones, everything above and below byte
for byte.

Without --write it prints the choices to offer for one file and writes
nothing. With --write and the staged_sha256 it printed, it writes the merge
over the staged file, refusing if staging no longer holds that version. A
merge is offered only when step 10's own bash-validator check
(validate_generated.py) passes it.

Usage:
    python3 merge_bash_validator.py --target <target> --staging <staging> --path .claude/hooks/bash-validator.sh [--write --staged-sha256 <hex>]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
import sys
import tempfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from infra_ownership import (  # noqa: E402
    OwnershipError,
    confined_target_path,
    normalize_relative_path,
    resolve_target,
)
from validate_generated import (  # noqa: E402
    BASH_VALIDATOR,
    EDITION_HOOK_WIRING,
    bash_validator_section,
    validate_bash_validators,
)

# Every validator path the generator writes, with its edition.
VALIDATORS = {
    f"{hooks}/{BASH_VALIDATOR}": edition
    for edition, (hooks, _wiring) in EDITION_HOOK_WIRING.items()
}
ORDINARY_CHOICES = ["keep", "take-staged", "manual-merge"]


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def gate_problems(relative: str, data: bytes) -> list:
    """What step 10's bash-validator check says about data at relative."""
    with tempfile.TemporaryDirectory(prefix="bash-validator-merge-") as root:
        path = Path(root) / relative
        path.parent.mkdir(parents=True)
        path.write_bytes(data)
        errors: list = []
        validate_bash_validators(Path(root), [VALIDATORS[relative]], {relative: ""}, errors)
    return errors


def section_merge(target: Path, staging: Path, relative: str) -> tuple:
    """The report for one validator and the merged bytes, or None.

    The staged version must pass the check itself: when it does not, no
    choice can pass step 10, and staging is what needs repair.
    """
    staged_path = confined_target_path(staging, relative)
    if not staged_path.is_file():
        raise OwnershipError(f"no staged {relative} to merge with")
    staged = staged_path.read_bytes()
    problems = gate_problems(relative, staged)
    if problems:
        raise OwnershipError(
            f"the staged {relative} fails step 10's check, so no choice would "
            "pass it; repair staging through hook-forge first: " + "; ".join(problems)
        )
    staged_start, staged_end, _ = bash_validator_section(staged)
    report = {
        "path": relative,
        "section": "",
        "choices": ["take-staged"],
        "staged_sha256": sha256(staged),
        "merged_sha256": None,
        "replaced_lines": None,
        "above_section": None,
        "problems": [],
        "written": False,
    }
    target_path = confined_target_path(target, relative)
    if not target_path.exists():
        report["section"] = "missing"
        report["problems"] = [f"the target has no {relative} to merge into"]
        return report, None
    if not target_path.is_file():
        raise OwnershipError(f"the target's {relative} is not a regular file")
    data = target_path.read_bytes()
    start, end, problem = bash_validator_section(data)
    if problem:
        report["section"] = "unmarked"
        report["problems"] = [f"the target's {relative} {problem}: no section to replace"]
        return report, None
    if data[start:end] == staged[staged_start:staged_end]:
        report["section"] = "current"
        report["choices"] = list(ORDINARY_CHOICES)
        return report, None
    report["section"] = "differs"
    merged = data[:start] + staged[staged_start:staged_end] + data[end:]
    problems = gate_problems(relative, merged)
    if problems:
        report["problems"] = problems
        return report, None
    report.update(
        choices=["take-staged", "section-merge"],
        merged_sha256=sha256(merged),
        replaced_lines=[data.count(b"\n", 0, start) + 1, data.count(b"\n", 0, end) + 1],
        above_section="same" if data[:start] == staged[:staged_start] else "differs",
    )
    return report, merged


def replace_staged(path: Path, data: bytes) -> None:
    """Write data over the staged file in one step, keeping its mode (+x)."""
    mode = stat.S_IMODE(path.stat().st_mode)
    temporary = path.with_name(f".{path.name}.section-merge-{os.getpid()}")
    try:
        temporary.write_bytes(data)
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def main(argv: list | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--target", required=True)
    parser.add_argument("--staging", required=True)
    parser.add_argument(
        "--path",
        required=True,
        help=f"the requires-decision validator: one of {', '.join(VALIDATORS)}",
    )
    parser.add_argument(
        "--write",
        action="store_true",
        help="write the section merge over the staged file",
    )
    parser.add_argument(
        "--staged-sha256",
        help="the staged_sha256 the check printed; required with --write",
    )
    args = parser.parse_args(argv)
    if args.write and not args.staged_sha256:
        parser.error("--write needs --staged-sha256: the staged version the decision was about")
    try:
        relative = normalize_relative_path(args.path)
        if relative not in VALIDATORS:
            raise OwnershipError(
                f"{relative} is no edition's bash-validator.sh ({', '.join(VALIDATORS)})"
            )
        target = resolve_target(args.target)
        staging = resolve_target(args.staging)
        for label, root in (("target", target), ("staging directory", staging)):
            if not root.is_dir():
                raise OwnershipError(f"{label} not found: {root}")
        report, merged = section_merge(target, staging, relative)
        if args.staged_sha256 and args.staged_sha256 != report["staged_sha256"]:
            raise OwnershipError(
                f"staging no longer holds the {relative} that was checked "
                f"(sha256 {args.staged_sha256}, now {report['staged_sha256']}): "
                "it was merged or regenerated since; check it again"
            )
        if args.write:
            if merged is None:
                reason = (
                    "its section equals the staged one, an ordinary decision"
                    if report["section"] == "current"
                    else "; ".join(report["problems"])
                )
                raise OwnershipError(f"no section merge to write for {relative}: {reason}")
            replace_staged(confined_target_path(staging, relative), merged)
            report["written"] = True
        print(json.dumps(report, indent=2))
    except (OSError, OwnershipError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
