#!/usr/bin/env python3
"""Syntax-check the complete PHP snippets the accelerator ships in Markdown.

The repository tracks no `.php` files, but its skills, examples and DOD carry
hundreds of fenced PHP blocks, and those blocks are what an agent copies when
it writes code. A snippet with a syntax error therefore propagates into
generated applications while every existing CI job stays green.

A block that begins with `<?php` claims to be a whole file and is linted as
it is. Most blocks are fragments - statements, or the members of a class -
and those are linted in the first shape that makes them a file: behind an
added `<?php` tag, then as the body of a class. A fragment is accepted raw
only when it carries its own `<?php`/`<?=` tag (a template): `php -l` treats
untagged text as inline HTML and accepts anything, so raw mode for untagged
text would pass every broken fragment. A fragment that is deliberately not
valid PHP on its own (pieces of two files, a config excerpt) is fenced
```` ```php fragment ```` and only counted. Counts per mode are printed, so the
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
BLOCK = re.compile(r"^```php(?P<info>[^\n]*)\n(?P<body>.*?)^```", re.S | re.M)
# The shapes a fragment is tried in, in order: statements, then class members.
FRAGMENT_SHAPES = (
    ("statements", "<?php\n{}\n"),
    ("class members", "<?php\nclass __Snippet\n{{\n{}\n}}\n"),
)


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


def lint(target: Path, source: str) -> str | None:
    """None when `source` parses, else php's message naming the problem."""
    target.write_text(source, encoding="utf-8")
    result = subprocess.run(["php", "-l", str(target)], capture_output=True, text=True)
    if not result.returncode:
        return None
    # php -l splits its report: the parse error itself may go to either
    # stream, with "Errors parsing <file>" as the summary. Prefer the line that
    # names the syntax problem, because the summary alone says nothing
    # actionable.
    lines = [
        line.strip()
        for line in (result.stdout + "\n" + result.stderr).splitlines()
        if line.strip()
    ]
    message = next(
        (line for line in lines if "rror" in line and "Errors parsing" not in line),
        lines[0] if lines else "php -l failed",
    )
    return message.replace(str(target), "<snippet>")


def check(root: Path) -> tuple[dict[str, int], list[str]]:
    """Counts per mode, and one failure line per block no mode accepts."""
    counts = {"complete": 0, "statements": 0, "class members": 0, "template": 0, "declared fragment": 0}
    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="php-snippets-") as raw:
        target = Path(raw) / "snippet.php"
        for path in tracked_markdown(root):
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError):
                continue
            relative = path.relative_to(root).as_posix()
            for index, match in enumerate(BLOCK.finditer(text), start=1):
                block = match.group("body")
                if "fragment" in match.group("info").split():
                    counts["declared fragment"] += 1
                    continue
                if block.lstrip().startswith("<?php"):
                    problem = lint(target, block)
                    if problem is None:
                        counts["complete"] += 1
                    else:
                        failures.append(f"{relative} block #{index}: {problem}")
                    continue
                first_problem = None
                for mode, shape in FRAGMENT_SHAPES:
                    problem = lint(target, shape.format(block))
                    if problem is None:
                        counts[mode] += 1
                        break
                    first_problem = first_problem or problem
                else:
                    if ("<?php" in block or "<?=" in block) and lint(target, block) is None:
                        counts["template"] += 1
                        continue
                    failures.append(
                        f"{relative} block #{index}: no shape parses (statements: "
                        f"{first_problem}); fix it, or fence it ```php fragment"
                    )
    return counts, failures


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

    counts, failures = check(root)
    for failure in failures:
        print(f"INVALID\t{failure}", file=sys.stderr)
    linted = sum(value for key, value in counts.items() if key != "declared fragment")
    detail = ", ".join(f"{value} {key}" for key, value in counts.items() if key != "declared fragment")
    print(
        f"CHECKED\t{linted} snippet(s) ({detail})\t"
        f"SKIPPED\t{counts['declared fragment']} declared fragment(s)\t"
        f"INVALID\t{len(failures)}"
    )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
