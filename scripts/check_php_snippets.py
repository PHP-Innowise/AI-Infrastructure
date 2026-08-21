#!/usr/bin/env python3
"""Syntax-check the complete PHP snippets the accelerator ships in Markdown.

The repository tracks no `.php` files, but its skills, examples and DOD carry
hundreds of fenced PHP blocks, and those blocks are what an agent copies when
it writes code. A snippet with a syntax error therefore propagates into
generated applications while every existing CI job stays green.

Only blocks that begin with `<?php` are checked: those claim to be whole
files. Fragments (a method body, a class with `...` elisions) are counted and
reported, not linted - `php -l` cannot judge them, and pretending otherwise
would turn a real guard into noise. The skipped count is printed so the
coverage this job provides is never overstated.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BLOCK = re.compile(r"^```php[ \t]*\n(.*?)^```", re.S | re.M)


def tracked_markdown(root: Path) -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z", "--", "*.md"],
        cwd=root,
        capture_output=True,
        check=True,
    )
    return [
        root / raw.decode("utf-8")
        for raw in result.stdout.split(b"\0")
        if raw
    ]


def php_available() -> bool:
    try:
        return subprocess.run(
            ["php", "--version"], capture_output=True, check=False
        ).returncode == 0
    except OSError:
        return False


def check(root: Path) -> tuple[int, int, list[str]]:
    checked = 0
    skipped = 0
    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="php-snippets-") as raw:
        target = Path(raw) / "snippet.php"
        for path in tracked_markdown(root):
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError):
                continue
            for index, block in enumerate(BLOCK.findall(text), start=1):
                if not block.lstrip().startswith("<?php"):
                    skipped += 1
                    continue
                checked += 1
                target.write_text(block, encoding="utf-8")
                result = subprocess.run(
                    ["php", "-l", str(target)], capture_output=True, text=True
                )
                if result.returncode:
                    # php -l splits its report: the parse error itself may go
                    # to either stream, with "Errors parsing <file>" as the
                    # summary. Prefer the line that names the syntax problem,
                    # because the summary alone says nothing actionable.
                    lines = [
                        line.strip()
                        for line in (result.stdout + "\n" + result.stderr).splitlines()
                        if line.strip()
                    ]
                    message = next(
                        (line for line in lines if "rror" in line and "Errors parsing" not in line),
                        lines[0] if lines else "php -l failed",
                    )
                    relative = path.relative_to(root).as_posix()
                    failures.append(
                        f"{relative} block #{index}: "
                        + message.replace(str(target), "<snippet>")
                    )
    return checked, skipped, failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument(
        "--require-php",
        action="store_true",
        help="fail when php is unavailable instead of reporting the check as skipped",
    )
    args = parser.parse_args()
    root = args.source_root.resolve()

    if not php_available():
        # A silently skipped check reads as a pass. CI passes --require-php so
        # a runner that loses php fails loudly; a contributor without php gets
        # a statement of what was not checked.
        if args.require_php:
            print("php-snippets: php is not available", file=sys.stderr)
            return 1
        print("SKIPPED\tphp not available; no snippet was checked")
        return 0

    checked, skipped, failures = check(root)
    for failure in failures:
        print(f"INVALID\t{failure}", file=sys.stderr)
    print(f"CHECKED\t{checked} complete snippet(s)\tSKIPPED\t{skipped} fragment(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
