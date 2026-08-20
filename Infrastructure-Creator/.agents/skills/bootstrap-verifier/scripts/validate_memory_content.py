#!/usr/bin/env python3
"""Validate that seeded memory chunks carry knowledge, not a template.

The bank's own runtime validator proves shape: IDs, dates, statuses, source
existence, index parity, secrets. It cannot ask whether a chunk's body says
anything. A measured publication showed why that matters: all fifteen seeded
chunks carried the same sentence with the title and paths substituted -
"X is a confirmed authority boundary recorded by TASK-001; re-open the cited
sources" - and the one contradiction the codebase actually contains (a
`finally` promoting FAILED to FINISHED) was named by a title and stated by
nothing.

This gate reads every seeded chunk and refuses the failure modes that
publication produced: a Durable Context that states no rule (no concrete
anchor, no behavioral modality), bodies that collapse to one shared skeleton
once identity is erased, the known boilerplate phrases, a cited source the
Verification section never explains, a line range the cited file cannot
contain, duplicate sources, and a contradiction chunk that records no
competing claims or presents itself as settled.

Invariants shared with the sibling gates: standard library only, no
execution, no network, byte-stable JSON output, fail-closed.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

REQUIRED_SECTIONS = ("Durable Context", "Consequences", "Verification")

# The exact prose the templated publication repeated fifteen times. A
# secondary net - the skeleton pass is the mechanism - but these exact
# phrases have already shipped once and must never ship again.
BOILERPLATE_PHRASES = (
    "re-open the cited sources",
    "apply the matching generated specialist",
    "authority boundary recorded by",
    "do not infer missing behavior",
    "instead of promoting an assumption",
    "recompute authority from:",
)

# A rule states behavior. Without one of these, a paragraph is a description.
MODALITY = re.compile(
    r"\b(must|must not|never|only|cannot|forbidden|required|requires|refuse[sd]?|"
    r"reject[sed]*|block[sed]*|remain[s]?|allowed|until|unless|when|no\b)",
    re.IGNORECASE,
)
# A rule names something concrete: a symbol, a constant, a number, a quoted
# value, a backticked span, or a path with a line range.
ANCHOR = re.compile(
    r"`[^`]+`|\b[A-Z][A-Z0-9_]{2,}\b|\b[a-z]+[A-Z]\w+\b|\b[A-Z]\w+[A-Z]\w*\b"
    r"|\b\d+\b|\"[^\"]+\"|'[^']+'|\S+:L\d+"
)
RANGE_REF = re.compile(r"([\w./\\-]+\.\w{1,12}):L(\d+)(?:\s*-\s*L?(\d+))?")
CONTRADICTION_MARK = re.compile(r"contradict|conflict|versus|disagree", re.IGNORECASE)
RESOLUTION_MARK = re.compile(
    r"current (?:safe )?(?:operating |operational )?rule|operate as|treat as|"
    r"until (?:it is )?resolved|owner|resolution|needs review",
    re.IGNORECASE,
)
WORD = re.compile(r"[a-zA-Z]{3,}")
SKELETON_MIN_CHARS = 40


@dataclass(frozen=True)
class Diagnostic:
    severity: str
    code: str
    message: str


def _diag(
    diagnostics: list[Diagnostic], code: str, message: str, severity: str = "error"
) -> None:
    diagnostics.append(Diagnostic(severity, code, message))


@dataclass
class Chunk:
    name: str
    meta: dict
    sections: dict[str, str]
    title_words: set[str]


def _parse_chunk(path: Path, diagnostics: list[Diagnostic]) -> Chunk | None:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        _diag(
            diagnostics, "MEMORY_CHUNK_UNREADABLE", f"{path.name}: unreadable: {error}"
        )
        return None
    match = re.match(r"^---\n(.*?)\n---\n", text, re.DOTALL)
    if not match:
        _diag(
            diagnostics,
            "MEMORY_CHUNK_UNREADABLE",
            f"{path.name}: no frontmatter block",
        )
        return None
    try:
        meta = json.loads(match.group(1))
    except json.JSONDecodeError as error:
        _diag(
            diagnostics,
            "MEMORY_CHUNK_UNREADABLE",
            f"{path.name}: frontmatter is not JSON: {error}",
        )
        return None
    body = text[match.end():]
    sections: dict[str, str] = {}
    for heading, content in re.findall(
        r"^## +(.+?)\s*\n(.*?)(?=^## |\Z)", body, re.DOTALL | re.MULTILINE
    ):
        sections[heading.strip()] = content.strip()
    missing = [name for name in REQUIRED_SECTIONS if not sections.get(name)]
    if missing:
        _diag(
            diagnostics,
            "MEMORY_CHUNK_UNREADABLE",
            f"{path.name}: missing or empty section(s): {', '.join(missing)}",
        )
        return None
    title = str(meta.get("title") or "")
    return Chunk(
        name=path.name,
        meta=meta if isinstance(meta, dict) else {},
        sections=sections,
        title_words={word.lower() for word in WORD.findall(title)},
    )


def _skeleton(text: str, title_words: set[str]) -> str:
    """Erase the chunk's identity; what remains is the reusable scaffolding."""
    text = text.lower()
    text = re.sub(r"`[^`]+`", " x ", text)
    text = re.sub(r"\S*[/\\]\S*", " p ", text)  # anything path-like
    text = re.sub(r"\bl?\d+(?:-l?\d+)?\b", " n ", text)
    words = [
        word
        for word in re.findall(r"[a-z]+", text)
        if word not in title_words
    ]
    return " ".join(words)


def _check_rule(chunk: Chunk, diagnostics: list[Diagnostic]) -> None:
    durable = chunk.sections["Durable Context"]
    if not (MODALITY.search(durable) and ANCHOR.search(durable)):
        _diag(
            diagnostics,
            "MEMORY_RULE_MISSING",
            f"{chunk.name}: Durable Context states no rule - it needs a "
            "behavioral modality (must/never/only/...) AND a concrete anchor "
            "(symbol, constant, number, quoted value, or path:Lrange); a "
            "paragraph that only describes its own title preserves nothing",
        )


def _check_boilerplate(chunk: Chunk, diagnostics: list[Diagnostic]) -> None:
    body = " ".join(chunk.sections.values()).lower()
    for phrase in BOILERPLATE_PHRASES:
        if phrase in body:
            _diag(
                diagnostics,
                "MEMORY_GENERIC_PHRASE",
                f"{chunk.name}: carries the known template phrase "
                f"{phrase!r} - serialize the actual rule instead",
            )


def _check_sources(
    chunk: Chunk, target: Path | None, diagnostics: list[Diagnostic]
) -> None:
    sources = chunk.meta.get("sources")
    if not isinstance(sources, list):
        return
    seen: set[str] = set()
    verification = chunk.sections["Verification"]
    for source in sources:
        source = str(source)
        if source in seen:
            _diag(
                diagnostics,
                "MEMORY_SOURCE_DUPLICATE",
                f"{chunk.name}: cites {source} more than once",
            )
            continue
        seen.add(source)
        if source.startswith(("http://", "https://")):
            continue
        if target is not None and not (target / source).exists():
            _diag(
                diagnostics,
                "MEMORY_SOURCE_MISSING",
                f"{chunk.name}: cited source does not exist in the target: "
                f"{source}",
            )
        lines = [
            line for line in verification.splitlines() if source in line
        ]
        if not lines:
            _diag(
                diagnostics,
                "MEMORY_SOURCE_UNEXPLAINED",
                f"{chunk.name}: Verification never says what {source} proves",
            )
            continue
        residue: list[str] = []
        for line in lines:
            stripped = RANGE_REF.sub(" ", line)
            stripped = re.sub(r"\S*[/\\]\S*", " ", stripped)
            residue.extend(WORD.findall(stripped))
        if len(residue) < 4:
            _diag(
                diagnostics,
                "MEMORY_SOURCE_UNEXPLAINED",
                f"{chunk.name}: {source} is listed but never explained - "
                "state what the cited range proves and what change triggers "
                "re-review",
            )


def _check_ranges(
    chunk: Chunk, target: Path | None, diagnostics: list[Diagnostic]
) -> None:
    if target is None:
        return
    body = " ".join(chunk.sections.values())
    for path_text, start_text, end_text in RANGE_REF.findall(body):
        source = target / path_text
        if not source.is_file():
            continue  # a missing source is already reported once, from sources
        try:
            total = len(source.read_text(encoding="utf-8", errors="replace").splitlines())
        except OSError:
            continue
        start = int(start_text)
        end = int(end_text) if end_text else start
        if start < 1 or end < start or end > total:
            _diag(
                diagnostics,
                "MEMORY_VERIFICATION_RANGE",
                f"{chunk.name}: {path_text}:L{start}"
                + (f"-L{end}" if end != start else "")
                + f" lies outside the file's {total} line(s) or runs backwards",
            )


def _check_contradiction(chunk: Chunk, diagnostics: list[Diagnostic]) -> None:
    haystack = str(chunk.meta.get("title", "")) + " " + " ".join(
        str(tag) for tag in chunk.meta.get("tags") or []
    )
    if not CONTRADICTION_MARK.search(haystack):
        return
    if chunk.meta.get("status") != "needs-review":
        _diag(
            diagnostics,
            "MEMORY_CONTRADICTION_SETTLED",
            f"{chunk.name}: records a contradiction but is not "
            "status needs-review - an unresolved conflict must not read as a "
            "settled decision",
        )
    durable = chunk.sections["Durable Context"]
    cited_sides = len(RANGE_REF.findall(durable))
    if (
        cited_sides < 2
        or not CONTRADICTION_MARK.search(durable)
        or not RESOLUTION_MARK.search(durable)
    ):
        _diag(
            diagnostics,
            "MEMORY_CONTRADICTION_IMPLICIT",
            f"{chunk.name}: a contradiction chunk must state both competing "
            "claims with their cited ranges, the current safe operating "
            "rule, and the owner or condition for resolution - a title is "
            "not a record",
        )


def validate(bank: Path, target: Path | None) -> list[Diagnostic]:
    diagnostics: list[Diagnostic] = []
    chunks_dir = bank / "chunks"
    if not chunks_dir.is_dir():
        _diag(
            diagnostics,
            "MEMORY_BANK_UNREADABLE",
            f"{chunks_dir} is not a directory",
        )
        return diagnostics
    chunks: list[Chunk] = []
    for path in sorted(chunks_dir.glob("MEM-*.md")):
        chunk = _parse_chunk(path, diagnostics)
        if chunk is not None:
            chunks.append(chunk)
    if not chunks:
        _diag(
            diagnostics,
            "MEMORY_BANK_EMPTY",
            "the bank seeds no readable chunks",
        )
        return diagnostics
    for chunk in chunks:
        _check_rule(chunk, diagnostics)
        _check_boilerplate(chunk, diagnostics)
        _check_sources(chunk, target, diagnostics)
        _check_ranges(chunk, target, diagnostics)
        _check_contradiction(chunk, diagnostics)
    for section in ("Durable Context", "Consequences"):
        skeletons: dict[str, str] = {}
        for chunk in chunks:
            skeleton = _skeleton(chunk.sections[section], chunk.title_words)
            if len(skeleton) < SKELETON_MIN_CHARS:
                continue
            if skeleton in skeletons:
                _diag(
                    diagnostics,
                    "MEMORY_BODY_TEMPLATED",
                    f"{chunk.name}: its {section} collapses to the same "
                    f"skeleton as {skeletons[skeleton]} once identity is "
                    "erased - one template with substituted nouns preserves "
                    "nothing",
                )
            else:
                skeletons[skeleton] = chunk.name
    return diagnostics


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bank", required=True, help="path to memory-bank/")
    parser.add_argument(
        "--target",
        default=None,
        help="real target root the chunks cite; enables source and range checks",
    )
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args(argv)
    diagnostics = validate(
        Path(args.bank), Path(args.target).resolve() if args.target else None
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
            print("memory content OK")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
