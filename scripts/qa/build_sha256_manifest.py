#!/usr/bin/env python3
"""Create or verify a deterministic SHA-256 manifest for an evidence tree."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Iterable, List

from common import QaError, atomic_write_text, sha256_file


def artifact_paths(root: Path, output: Path) -> Iterable[Path]:
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise QaError("Evidence tree contains a symlink: {0}".format(path))
        if not path.is_file() or path.resolve() == output.resolve():
            continue
        if path.name.startswith("."):
            continue
        yield path


def render_manifest(root: Path, output: Path) -> str:
    lines: List[str] = []
    for path in artifact_paths(root, output):
        lines.append(
            "{0}  {1}".format(sha256_file(path), path.relative_to(root).as_posix())
        )
    return "\n".join(lines) + ("\n" if lines else "")


def verify_manifest(root: Path, manifest: Path) -> List[str]:
    errors: List[str] = []
    if manifest.is_symlink() or not manifest.is_file():
        return ["checksum manifest must be a regular file"]
    expected_lines = {}
    for number, raw_line in enumerate(
        manifest.read_text(encoding="utf-8").splitlines(), 1
    ):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 1)
        if (
            len(parts) != 2
            or len(parts[0]) != 64
            or any(character not in "0123456789abcdefABCDEF" for character in parts[0])
            or not parts[1].strip()
        ):
            raise QaError(
                "Malformed SHA-256 manifest line {0}: {1}".format(number, manifest)
            )
        relative = parts[1].strip()
        if relative in expected_lines:
            raise QaError("Duplicate checksum entry: {0}".format(relative))
        expected_lines[relative] = parts[0].lower()
    actual_paths = {
        path.relative_to(root).as_posix(): sha256_file(path)
        for path in artifact_paths(root, manifest)
    }
    for relative in sorted(set(expected_lines) | set(actual_paths)):
        if relative not in expected_lines:
            errors.append("unmanifested artifact: {0}".format(relative))
        elif relative not in actual_paths:
            errors.append("missing artifact: {0}".format(relative))
        elif expected_lines[relative] != actual_paths[relative]:
            errors.append("checksum mismatch: {0}".format(relative))
    return errors


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = args.root.resolve()
    output = (args.output or root / "SHA256SUMS.txt").resolve()
    try:
        if not root.is_dir():
            raise QaError("Evidence directory does not exist: {0}".format(root))
        if args.check:
            if not output.is_file():
                raise QaError("Checksum manifest does not exist: {0}".format(output))
            errors = verify_manifest(root, output)
            if errors:
                raise QaError("; ".join(errors))
            print("Verified {0}".format(output))
        else:
            atomic_write_text(output, render_manifest(root, output))
            print("Wrote {0}".format(output))
    except (QaError, OSError, ValueError) as exc:
        print("SHA-256 manifest operation failed: {0}".format(exc), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
