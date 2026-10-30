#!/usr/bin/env python3
"""Section merge of a team-edited bash-validator.sh for infra-update step 6.

hook-forge step 2 builds every edition's bash-validator.sh around blocks it
copies from the generator byte for byte, each running from a line
`# >>> bash-validator <name> >>>` through `# <<< bash-validator <name> <<<`
- the generic section first - and a team's own rules go below the generic
section, in BV_FRAMEWORK_RULES. A file edited there no longer matches its
manifest hash, so infra-update asks for a decision, and step 10 holds the
generic section to the current generator's: keeping a section an earlier
release wrote fails that gate and rolls the whole update back, and taking
the staged version drops the team's rules. The section merge keeps both:
the target version with each block the staged version marks replaced by
the staged one, everything else byte for byte. A block only the target
marks stays as it is.

Without --write it prints the choices to offer for one file and writes
nothing. With --write and the staged_sha256 it printed, it writes the merge
over the staged file, refusing if staging no longer holds that version. A
merge is offered only when step 10's own bash-validator check
(validate_generated.py) passes it, and only when the target holds each
staged block once: a block it lacks (an older generation, made before that
block existed) has no place in it, and a repeated or reversed marker leaves
the block undefined.

Usage:
    python3 merge_bash_validator.py --target <target> --staging <staging> --path .claude/hooks/bash-validator.sh [--write --staged-sha256 <hex>]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
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
    validate_bash_validators,
)

# Every validator path the generator writes, with its edition.
VALIDATORS = {
    f"{hooks}/{BASH_VALIDATOR}": edition
    for edition, (hooks, _wiring) in EDITION_HOOK_WIRING.items()
}
ORDINARY_CHOICES = ["keep", "take-staged", "manual-merge"]
# The line that opens a block; the gate's generic section markers are the
# block named "generic section".
BLOCK_OPENING = re.compile(rb"^# >>> bash-validator ([^\n]+?) >>>", re.M)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def label(name: bytes) -> str:
    return name.decode("utf-8", errors="replace")


def markers(name: bytes) -> tuple:
    """The opening and closing marker of the block called name."""
    return (
        b"# >>> bash-validator " + name + b" >>>",
        b"# <<< bash-validator " + name + b" <<<",
    )


def block_names(data: bytes) -> list:
    """The names of the blocks data opens, in order, each once."""
    names: list = []
    for match in BLOCK_OPENING.finditer(data):
        if match.group(1) not in names:
            names.append(match.group(1))
    return names


def block_span(data: bytes, name: bytes) -> tuple:
    """Where the block called name lies, cut as the gate cuts the section.

    Returns (start, end, "") with data[start:end] running from the opening
    marker through the closing one, or (-1, -1, why it cannot be cut out):
    each marker must occur exactly once, the opening one first.
    """
    opening, closing = markers(name)
    for marker in (opening, closing):
        count = data.count(marker)
        if count == 0:
            return -1, -1, f"has no `{label(marker)}` marker"
        if count > 1:
            return -1, -1, f"has the `{label(marker)}` marker {count} times"
    start, close = data.index(opening), data.index(closing)
    if close < start:
        return -1, -1, f"closes the {label(name)} before it opens it"
    return start, close + len(closing), ""


def overlap(spans: dict) -> str:
    """The first two blocks of spans that overlap, as a phrase, or ""."""
    ordered = sorted(spans.items(), key=lambda item: item[1][0])
    for (outer, (_, end)), (inner, (start, _)) in zip(ordered, ordered[1:]):
        if start < end:
            return f"opens its {label(inner)} inside its {label(outer)}"
    return ""


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

    The staged version must pass the check itself and hold each of its
    blocks once: otherwise staging is what needs repair.
    """
    staged_path = confined_target_path(staging, relative)
    if not staged_path.is_file():
        raise OwnershipError(f"no staged {relative} to merge with")
    staged = staged_path.read_bytes()
    problems = gate_problems(relative, staged)
    staged_spans: dict = {}
    for name in block_names(staged) if not problems else []:
        start, end, problem = block_span(staged, name)
        if problem:
            problems.append(f"{relative} {problem}")
        staged_spans[name] = (start, end)
    if not problems and overlap(staged_spans):
        problems.append(f"{relative} {overlap(staged_spans)}")
    if problems:
        raise OwnershipError(
            f"the staged {relative} is not as hook-forge step 2 builds it; "
            "repair staging through hook-forge first: " + "; ".join(problems)
        )
    report = {
        "path": relative,
        "blocks": {},
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
        report["problems"] = [f"the target has no {relative} to merge into"]
        return report, None
    if not target_path.is_file():
        raise OwnershipError(f"the target's {relative} is not a regular file")
    data = target_path.read_bytes()
    spans: dict = {}
    states: dict = {}
    for name, (staged_start, staged_end) in staged_spans.items():
        start, end, problem = block_span(data, name)
        if not problem:
            spans[name] = (start, end)
            same = data[start:end] == staged[staged_start:staged_end]
            states[name] = "current" if same else "differs"
        elif not any(marker in data for marker in markers(name)):
            states[name] = "absent"
            report["problems"].append(
                f"the target's {relative} has no {label(name)} - generated before "
                "that block existed, or its markers were removed - so the merge "
                "has no place for it"
            )
        else:
            states[name] = "malformed"
            report["problems"].append(
                f"the target's {relative} {problem}: no single {label(name)} to replace"
            )
    report["blocks"] = {label(name): state for name, state in states.items()}
    if not report["problems"] and overlap(spans):
        report["problems"].append(f"the target's {relative} {overlap(spans)}")
    if report["problems"]:
        return report, None
    if set(states.values()) == {"current"}:
        report["choices"] = list(ORDINARY_CHOICES)
        return report, None
    pieces, cursor = [], 0
    for name, (start, end) in sorted(spans.items(), key=lambda item: item[1][0]):
        staged_start, staged_end = staged_spans[name]
        pieces += [data[cursor:start], staged[staged_start:staged_end]]
        cursor = end
    merged = b"".join(pieces) + data[cursor:]
    problems = gate_problems(relative, merged)
    if problems:
        report["problems"] = problems
        return report, None
    first = min(spans, key=lambda name: spans[name][0])
    report.update(
        choices=["take-staged", "section-merge"],
        merged_sha256=sha256(merged),
        replaced_lines={
            label(name): [data.count(b"\n", 0, start) + 1, data.count(b"\n", 0, end) + 1]
            for name, (start, end) in spans.items()
            if states[name] == "differs"
        },
        above_section=(
            "same"
            if data[:spans[first][0]] == staged[:staged_spans[first][0]]
            else "differs"
        ),
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
                    "each block equals the staged one, an ordinary decision"
                    if report["blocks"] and set(report["blocks"].values()) == {"current"}
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
