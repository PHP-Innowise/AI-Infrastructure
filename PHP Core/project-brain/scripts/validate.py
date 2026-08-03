#!/usr/bin/env python3
"""Validate active and archived Project Brain records."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


EDITION_ROOT = Path(__file__).resolve().parents[2]
RUNTIME = EDITION_ROOT / "memory-bank" / "scripts"
sys.path.insert(0, str(RUNTIME))
try:
    from brain_runtime import validate_repository
finally:
    sys.path.pop(0)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root", type=Path, default=EDITION_ROOT, help="edition or consuming-project root"
    )
    parser.add_argument("--json", action="store_true")
    arguments = parser.parse_args()
    errors = validate_repository(arguments.root.resolve())
    if arguments.json:
        print(json.dumps({"valid": not errors, "errors": errors}, ensure_ascii=False))
    elif errors:
        print(f"Project Brain validation failed ({len(errors)} error(s)):", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
    else:
        print("Project Brain validation passed.")
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
