#!/usr/bin/env python3
"""Verify or regenerate per-tool mirrors inside every edition.

Each edition (Laravel, Symfony, "PHP Core", Infrastructure-Creator) mirrors
its skills, hooks, commands, agents and governance documents across the
.agents / .claude / .cursor / .codex directories. The rules for how each
mirror is derived from its canonical copy live INSIDE the edition - in
`memory-bank/scripts/context_retrieval.py` (MIRROR_RULES) for the PHP
editions, or in `mirror_rules.py` for Infrastructure-Creator - so a copied
edition keeps its own contract. This script is only the executor.

Usage:
    python3 scripts/build_mirrors.py --check [--edition NAME]
    python3 scripts/build_mirrors.py --write [--edition NAME]

--check lists every file whose mirror does not match what the rules derive
from the canonical copy (or is a stray file the rules do not account for,
or an "only" entry listed in the rules whose canonical file is gone) and
exits non-zero; it never writes. --write regenerates the mirrors from
the canonical copies and prints every file it changed.

Both modes also maintain `<edition>/.gitattributes`, which marks exactly the
generated mirror files as `-diff linguist-generated=true`. A generated file
that passes --check carries no information its canonical source does not, so
suppressing it from diffs keeps review payloads (and the tokens an agent
spends reading them) proportional to the change actually made. The list is
derived from the same MIRROR_RULES that generate the files, because no glob
is correct: the tool directories also hold canonical files that must keep
their diffs.

Python 3 stdlib only.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EDITION_PATHS = {
    "Laravel": Path("Laravel"),
    "Symfony": Path("Symfony"),
    "PHP Core": Path("PHP Core"),
    "WordPress": Path("Cms/wordpress"),
    "Infrastructure-Creator": Path("Infrastructure-Creator"),
}
EDITIONS = tuple(EDITION_PATHS)
RULE_LOCATIONS = (
    Path("memory-bank/scripts/context_retrieval.py"),
    Path("mirror_rules.py"),
)
IGNORED_NAMES = {"__pycache__", ".DS_Store"}
IGNORED_SUFFIXES = (".pyc",)
FRONTMATTER_DROP_KEYS = re.compile(r"^(model|invokes|phase):")


class BuildError(Exception):
    pass


def load_rules(edition_dir: Path) -> dict:
    for candidate in RULE_LOCATIONS:
        path = edition_dir / candidate
        if path.is_file():
            break
    else:
        raise BuildError(f"{edition_dir.name}: no rules module found")
    scripts_dir = str(path.parent)
    module_name = f"_mirror_rules_{abs(hash(str(path)))}"
    sys.path.insert(0, scripts_dir)
    # Each PHP edition ships its own byte-identical brain_runtime; make sure
    # the rules module resolves the one sitting next to it.
    sys.modules.pop("brain_runtime", None)
    try:
        spec = importlib.util.spec_from_file_location(module_name, path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(scripts_dir)
        sys.modules.pop(module_name, None)
        sys.modules.pop("brain_runtime", None)
    rules = getattr(module, "MIRROR_RULES", None)
    if not isinstance(rules, dict) or "classes" not in rules:
        raise BuildError(f"{path}: MIRROR_RULES missing or malformed")
    return rules


def edition_framework(edition_dir: Path) -> str | None:
    runtime = edition_dir / "project-brain" / "config" / "runtime.json"
    if not runtime.is_file():
        return None
    try:
        return json.loads(runtime.read_text(encoding="utf-8")).get("framework")
    except (OSError, ValueError):
        return None


def split_frontmatter(text: str) -> tuple[str | None, str]:
    if text.startswith("---\n"):
        end = text.find("\n---\n", 4)
        if end != -1:
            return text[4 : end + 1], text[end + 5 :]
    return None, text


def apply_replacements(text: str, spec: dict) -> str:
    for old, new in spec.get("replacements", []):
        text = text.replace(old, new)
    return text


def transform_copy(rel: str, text: str, spec: dict) -> str:
    return apply_replacements(text, spec)


def transform_cursor_command(rel: str, text: str, spec: dict) -> str:
    frontmatter, body = split_frontmatter(text)
    body = apply_replacements(body, spec)
    if frontmatter is None:
        return body
    if re.search(r"^name:", frontmatter, re.M):
        # Already Cursor-compatible (name/description); keep it as-is.
        return f"---\n{frontmatter}---\n{body}"
    name = rel.rsplit("/", 1)[-1]
    stem = name[:-3] if name.endswith(".md") else name
    overrides = spec.get("description_overrides", {})
    if name in overrides:
        description = overrides[name]
    else:
        lines = [
            line.strip()
            for line in body.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
        if not lines:
            raise BuildError(f"{rel}: cannot derive a description")
        description = lines[0]
    if spec.get("quote_description", True):
        description = f'"{description}"'
    return f"---\nname: {stem}\ndescription: {description}\n---\n{body}"


def transform_cursor_agent(rel: str, text: str, spec: dict) -> str:
    frontmatter, body = split_frontmatter(text)
    body = apply_replacements(body, spec)
    if frontmatter is None:
        return body
    kept = [
        line
        for line in frontmatter.splitlines()
        if not FRONTMATTER_DROP_KEYS.match(line)
    ]
    joined = "".join(f"{line}\n" for line in kept)
    return f"---\n{joined}---\n{body}"


TRANSFORMS = {
    "copy": transform_copy,
    "cursor-command": transform_cursor_command,
    "cursor-agent": transform_cursor_agent,
}


def is_ignored(rel_parts: tuple[str, ...]) -> bool:
    if any(part in IGNORED_NAMES for part in rel_parts):
        return True
    return rel_parts[-1].endswith(IGNORED_SUFFIXES)


def is_skipped(rel: str, skips: list[str]) -> bool:
    for entry in skips:
        if entry.endswith("/"):
            if rel.startswith(entry):
                return True
        elif rel == entry:
            return True
    return False


def class_skips(cls: dict, mirror_spec: dict, framework: str | None) -> list[str]:
    skips = list(cls.get("skip", [])) + list(mirror_spec.get("skip", []))
    if framework:
        skips += cls.get("skip_by_framework", {}).get(framework, [])
    return skips


def iter_canonical(canonical: Path, only: list[str] | None):
    if only is not None:
        for rel in only:
            path = canonical / rel
            if path.is_file():
                yield rel, path
        return
    for path in sorted(canonical.rglob("*")):
        if not path.is_file() or path.is_symlink():
            continue
        rel_parts = path.relative_to(canonical).parts
        if is_ignored(rel_parts):
            continue
        yield "/".join(rel_parts), path


def expected_mirror_files(
    edition_dir: Path, cls: dict, mirror_rel: str, framework: str | None
) -> dict[str, bytes]:
    canonical = edition_dir / cls["canonical"]
    mirror_spec = cls["mirrors"][mirror_rel]
    transform = TRANSFORMS[mirror_spec.get("transform", "copy")]
    skips = class_skips(cls, mirror_spec, framework)
    expected: dict[str, bytes] = {}
    for rel, path in iter_canonical(canonical, cls.get("only")):
        if is_skipped(rel, skips):
            continue
        raw = path.read_bytes()
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            if mirror_spec.get("transform", "copy") != "copy" or mirror_spec.get(
                "replacements"
            ):
                raise BuildError(f"{cls['canonical']}/{rel}: binary file in a transformed class")
            expected[rel] = raw
            continue
        expected[rel] = transform(rel, text, mirror_spec).encode("utf-8")
    return expected


GITATTRIBUTES_NAME = ".gitattributes"
GITATTRIBUTES_HEADER = """\
# Generated by scripts/build_mirrors.py -- do not edit by hand.
#
# Every path below is a mirror this repository regenerates from a canonical
# source; `build_mirrors.py --check` proves each one matches. Such a file
# carries no information its canon does not, so it is marked non-diffable:
# reviewers and agents read the canonical change instead of the same edit
# repeated across .claude/, .cursor/ and .codex/.
#
# Canonical files living inside those same directories are deliberately
# absent from this list and keep their diffs.
#
# A space in a path is written as `?` (gitattributes patterns are
# whitespace-separated and have no quoting); `?` matches any single
# character, which is exact enough for the paths involved.
"""
GITATTRIBUTES_SUFFIX = "-diff linguist-generated=true"


def expected_gitattributes(
    edition_dir: Path, rules: dict, framework: str | None
) -> bytes:
    """Render the .gitattributes marking every generated mirror file."""
    generated: set[str] = set()
    for cls in rules["classes"]:
        if not (edition_dir / cls["canonical"]).is_dir():
            continue
        for mirror_rel in cls["mirrors"]:
            for rel in expected_mirror_files(edition_dir, cls, mirror_rel, framework):
                generated.add(f"{mirror_rel}/{rel}")
    lines = [GITATTRIBUTES_HEADER]
    for path in sorted(generated):
        lines.append(f"{path.replace(' ', '?')} {GITATTRIBUTES_SUFFIX}\n")
    return "".join(lines).encode("utf-8")


def process_edition(
    edition_dir: Path, rules: dict, framework: str | None, write: bool
) -> tuple[list[str], list[str]]:
    problems: list[str] = []
    written: list[str] = []
    label = edition_dir.name
    for cls in rules["classes"]:
        canonical = edition_dir / cls["canonical"]
        if not canonical.is_dir():
            problems.append(f"{label}: canonical directory missing: {cls['canonical']}")
            continue
        if cls.get("only") is not None:
            # A listed "only" entry must exist in canon. iter_canonical
            # silently skips a listed path with no file behind it, so
            # without this report a typo in the list (or a canonical file
            # deleted or renamed after mirrors were generated) would mirror
            # nothing while --check stayed green.
            for rel in cls["only"]:
                if not (canonical / rel).is_file():
                    problems.append(
                        f"{label}: [{cls['name']}] {cls['canonical']}/{rel} "
                        f"is listed in 'only' but missing from canon"
                    )
        for mirror_rel, mirror_spec in cls["mirrors"].items():
            mirror = edition_dir / mirror_rel
            skips = class_skips(cls, mirror_spec, framework)
            expected = expected_mirror_files(edition_dir, cls, mirror_rel, framework)
            for rel, blob in expected.items():
                target = mirror / rel
                actual = target.read_bytes() if target.is_file() else None
                if actual == blob:
                    continue
                if write:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(blob)
                    written.append(f"{label}/{mirror_rel}/{rel}")
                else:
                    state = "missing from mirror" if actual is None else "differs from canon"
                    problems.append(
                        f"{label}: [{cls['name']}] {mirror_rel}/{rel} {state} "
                        f"({cls['canonical']}/{rel})"
                    )
            if cls.get("only") is None and mirror.is_dir():
                # Reverse pass: a file only the mirror carries is drift too.
                for path in sorted(mirror.rglob("*")):
                    if not path.is_file() or path.is_symlink():
                        continue
                    rel_parts = path.relative_to(mirror).parts
                    if is_ignored(rel_parts):
                        continue
                    rel = "/".join(rel_parts)
                    if rel in expected or is_skipped(rel, skips):
                        continue
                    problems.append(
                        f"{label}: [{cls['name']}] {mirror_rel}/{rel} "
                        f"has no source in {cls['canonical']}"
                    )
            elif cls.get("only") is not None:
                # Reverse pass for "only"-classes. The mirror directory also
                # holds files other classes own, so only the listed names are
                # examined: a listed mirror file whose canonical copy is gone
                # is drift, not a skip — otherwise deleting e.g. .claude/DOD.md
                # would leave .cursor/DOD.md and .codex/DOD.md orphaned with
                # no rule accounting for them.
                for rel in cls["only"]:
                    if rel in expected or is_skipped(rel, skips):
                        continue
                    if (mirror / rel).is_file():
                        problems.append(
                            f"{label}: [{cls['name']}] {mirror_rel}/{rel} "
                            f"has no source in {cls['canonical']}"
                        )

    # The generated-file manifest is derived from the same rules, so it stays
    # correct as skills come and go without anyone maintaining a glob.
    attributes = edition_dir / GITATTRIBUTES_NAME
    expected_attributes = expected_gitattributes(edition_dir, rules, framework)
    actual_attributes = attributes.read_bytes() if attributes.is_file() else None
    if actual_attributes != expected_attributes:
        if write:
            attributes.write_bytes(expected_attributes)
            written.append(f"{label}/{GITATTRIBUTES_NAME}")
        else:
            state = "missing" if actual_attributes is None else "stale"
            problems.append(
                f"{label}: {GITATTRIBUTES_NAME} is {state} "
                f"(regenerate with --write)"
            )
    return problems, written


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="report drift, write nothing")
    mode.add_argument("--write", action="store_true", help="regenerate mirrors from canon")
    parser.add_argument(
        "--edition",
        action="append",
        choices=EDITIONS,
        help="limit to one edition (repeatable); default: all",
    )
    args = parser.parse_args()

    selected = args.edition or list(EDITIONS)
    all_problems: list[str] = []
    all_written: list[str] = []
    for name in selected:
        edition_dir = ROOT / EDITION_PATHS[name]
        if not edition_dir.is_dir():
            all_problems.append(f"{name}: edition directory not found")
            continue
        try:
            rules = load_rules(edition_dir)
            problems, written = process_edition(
                edition_dir, rules, edition_framework(edition_dir), args.write
            )
        except BuildError as error:
            all_problems.append(str(error))
            continue
        all_problems.extend(problems)
        all_written.extend(written)

    for path in all_written:
        print(f"wrote {path}")
    if all_problems:
        print(f"Mirror drift ({len(all_problems)} finding(s)):", file=sys.stderr)
        for problem in all_problems:
            print(f"  {problem}", file=sys.stderr)
        return 1
    if args.check:
        print(f"Mirror check passed for: {', '.join(selected)}")
    else:
        print(f"Mirrors up to date for: {', '.join(selected)} ({len(all_written)} file(s) written)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
