#!/usr/bin/env python3
"""Relative-link checker for tracked Markdown files.

Walks every .md file tracked by git (git ls-files), extracts relative
markdown links [text](path) — ignoring http(s)/mailto/tel schemes and
anchor-only links — URL-decodes the path (%20 etc.), resolves it against
the containing file, strips any #anchor, and verifies the target exists
(file or directory).

Fenced code blocks (```/~~~) and inline code spans are skipped so that
markdown examples inside code are not treated as real links.

An optional allowlist file scripts/check_links_ignore.txt may contain one
fnmatch-style path pattern per line (comments with '#', blank lines
ignored). A broken link is suppressed when the pattern matches either the
raw link target or the repo-relative resolved path. Aim to keep it empty.

Output: one line per broken link (file:line: [text](target)); exit 1 if
any broken links remain, 0 otherwise.
"""

from __future__ import annotations

import fnmatch
import re
import subprocess
import sys
import urllib.parse
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
IGNORE_FILE = REPO_ROOT / "scripts" / "check_links_ignore.txt"

# [text](target) with optional <...> wrapping and optional "title".
LINK_RE = re.compile(
    r"\[[^\]]*\]\(\s*(?:<(?P<angle>[^>]*)>|(?P<plain>[^()\s]+))"
    r"(?:\s+\"[^\"]*\")?\s*\)"
)
INLINE_CODE_RE = re.compile(r"`[^`]*`")
FENCE_RE = re.compile(r"^\s*(```|~~~)")
SCHEME_RE = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")


def tracked_markdown_files() -> list[Path]:
    out = subprocess.run(
        ["git", "ls-files", "-z", "--", "*.md"],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
    ).stdout
    return [
        REPO_ROOT / p.decode("utf-8")
        for p in out.split(b"\0")
        if p
    ]


def load_ignore_patterns() -> list[str]:
    if not IGNORE_FILE.is_file():
        return []
    patterns = []
    for raw in IGNORE_FILE.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip()
        if line:
            patterns.append(line)
    return patterns


def is_ignored(patterns: list[str], raw_target: str, resolved_rel: str) -> bool:
    return any(
        fnmatch.fnmatch(raw_target, pat) or fnmatch.fnmatch(resolved_rel, pat)
        for pat in patterns
    )


def iter_links(text: str):
    """Yield (line_number, raw_target) for candidate links outside code."""
    in_fence = False
    fence_marker = ""
    for lineno, line in enumerate(text.splitlines(), start=1):
        fence_match = FENCE_RE.match(line)
        if fence_match:
            marker = fence_match.group(1)
            if not in_fence:
                in_fence, fence_marker = True, marker
            elif marker == fence_marker:
                in_fence = False
            continue
        if in_fence:
            continue
        scannable = INLINE_CODE_RE.sub("", line)
        for m in LINK_RE.finditer(scannable):
            target = m.group("angle")
            if target is None:
                target = m.group("plain")
            target = target.strip()
            if target:
                yield lineno, target


def check_file(md_file: Path, patterns: list[str]) -> list[str]:
    broken = []
    text = md_file.read_text(encoding="utf-8", errors="replace")
    rel_file = md_file.relative_to(REPO_ROOT)
    for lineno, raw_target in iter_links(text):
        if SCHEME_RE.match(raw_target):
            continue  # http(s), mailto, tel, etc.
        path_part = raw_target.split("#", 1)[0]
        if not path_part:
            continue  # anchor-only link within the same file
        decoded = urllib.parse.unquote(path_part)
        if decoded.startswith("/"):
            candidate = (REPO_ROOT / decoded.lstrip("/")).resolve()
        else:
            candidate = (md_file.parent / decoded).resolve()
        if candidate.exists():
            continue
        try:
            resolved_rel = str(candidate.relative_to(REPO_ROOT))
        except ValueError:
            resolved_rel = str(candidate)
        if is_ignored(patterns, raw_target, resolved_rel):
            continue
        broken.append(
            f"{rel_file}:{lineno}: broken link ({raw_target}) -> {resolved_rel}"
        )
    return broken


def main() -> int:
    patterns = load_ignore_patterns()
    broken: list[str] = []
    for md_file in tracked_markdown_files():
        if not md_file.is_file():
            continue  # tracked but deleted in working tree
        broken.extend(check_file(md_file, patterns))
    if broken:
        print(f"Found {len(broken)} broken relative link(s):\n")
        for entry in broken:
            print(entry)
        return 1
    print("All relative markdown links resolve. No broken links found.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
