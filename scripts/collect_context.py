#!/usr/bin/env python3
"""Collect a context bundle from this monorepo for an external model.

A wrapper around the `code2prompt` CLI that knows this repository's shape.
Calling `code2prompt` by hand here is not a shortcut, it is a trap: this
checkout carries three machine-generated mirror trees, a client-owned
specification directory, and a `.git` directory whose blobs the tool will
happily read. The bare invocation in the upstream docs picks up all three.
The scopes below encode what a context bundle from this repository should
actually contain.

    ./collect                                   # list the scopes
    ./collect skills --edition Laravel --dry-run
    ./collect edition --edition Symfony
    ./collect diff --base origin/main --stdout

`./collect` is a one-line wrapper at the repository root; this file is
executable too, so `scripts/collect_context.py` works the same way. Inside
Claude Code, `.claude/commands/collect.md` exposes the same tool as
`/collect <scope> [options]`. Both entry points sit at the root, outside every
edition, so they travel with the monorepo and never with an installed
accelerator.

The tool is optional and developer-local: nothing in the editions, the
installer inventories, the hooks, the skills or a blocking CI gate depends on
it, and `tests/test_collect_context.py::test_containment` keeps it that way.
It saves the project zero runtime tokens — it spends the maintainer's, to hand
a model a bundle that is scoped on purpose.

Four invariants are enforced on every run, each because the unguarded
behaviour was observed on code2prompt 4.3.0:

  .git is always excluded         The repo root with `--hidden` and no `-i`
                                  reads 2,213 files, `.git/config` (remote
                                  URLs) and `.git/COMMIT_EDITMSG` among them.
  Task/ is excluded by default    `Laravel/Task/Epics/Epic-0*_*_SPEC.md` is
                                  named-client product specification —
                                  payments, CRM, player management. Including
                                  `Laravel/**/*.md` without the exclude adds
                                  10 files and ~87k tokens of it. `--with-task`
                                  opts in, loudly.
  the run is config-isolated      A `.c2pconfig` in the working directory is
                                  picked up automatically and silently injects
                                  settings such as `line_numbers`, even when
                                  the analysed path is elsewhere; there is no
                                  flag to disable it. Every run therefore
                                  executes in an empty temporary directory
                                  with `XDG_CONFIG_HOME` pointed at it.
  an empty match is an error      A pattern that matches nothing exits 0 with
                                  an empty bundle and no warning.

Mirrors (`.claude/`, `.cursor/`, `.codex/`) are excluded by default: they are
generated from canon by `scripts/build_mirrors.py`, carry no information their
canon does not, and triple the bundle. `--with-mirrors` includes them for the
rare question that is actually about the generated form.

Token counts come from cl100k (OpenAI BPE, code2prompt's tokenizer). That is
NOT a count of Claude tokens; it is a calibrated relative unit, useful for
comparing two bundles, not for predicting a bill.

Python 3 stdlib only; the `code2prompt` binary is the sole external
requirement and its absence is reported, not tolerated silently.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT_DIR = ROOT / ".c2p"

BIN_ENV = "CODE2PROMPT_BIN"
DEFAULT_BIN = "code2prompt"
# 4.2.0 is the oldest release whose flag set matches the one used here; the
# major bound is a tripwire, not a prediction — a 5.x that keeps the contract
# only needs this constant raised.
MIN_VERSION = (4, 2, 0)
MAX_VERSION = (5, 0, 0)
RUN_TIMEOUT = 120
PROBE_TIMEOUT = 15

EDITIONS = ("Laravel", "Symfony", "PHP Core", "Infrastructure-Creator")

# Applied to every scope, unconditionally. Each entry is either something that
# must not reach a model prompt, or something whose bytes the prompt already
# has in another form.
ALWAYS_EXCLUDE = (
    ".git/**",
    "**/.git/**",
    "**/__pycache__/**",
    "**/*.pyc",
    ".venv/**",
    "**/.venv/**",
    ".worktrees/**",
    ".superpowers/**",
    # Per-checkout runtime state: an indexing database and the last turn's
    # report. Gitignored, machine-local, and meaningless to a reader.
    "**/memory-bank/local/**",
    "**/project-brain/local/**",
    "**/*.db",
    "**/*.sqlite",
    "**/*.sqlite3",
    # Credential shapes. None are tracked here today; the cost of listing them
    # is nil and the cost of pasting one into a prompt is not.
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/id_rsa*",
)

# Client-owned specification material. Excluded unless --with-task.
CLIENT_DATA_EXCLUDE = ("**/Task/**",)

# Generated tool mirrors. Excluded unless --with-mirrors.
MIRROR_EXCLUDE = ("**/.claude/**", "**/.cursor/**", "**/.codex/**")

# code2prompt's glob dialect is not the shell's: `*` and `**` both cross
# directory separators. `-i "*.md"` at the repository root therefore reads
# every markdown file in the tree — 419 files, ~527k tokens — not the four at
# the top level, and `-i "CHANGELOG.md"` matches five files, one per edition.
# The one form that anchors is a `./`-prefixed literal: `./CHANGELOG.md`
# matches exactly one. So a scope states its top-level wants in
# `include_local`, expanded here by Python's own (non-recursive) glob into
# anchored literals; `include` keeps the recursive dialect, on purpose.
#
# root is relative to the repository root; "{edition}" is substituted when the
# scope takes one. Both pattern lists are relative to that root.
SCOPES = {
    "edition": {
        "summary": "one edition's whole canonical tree",
        "needs_edition": True,
        "root": "{edition}",
        "include": [
            ".agents/skills/**",
            "memory-bank/**",
            "project-brain/**",
            "specs/**",
            "examples/**",
        ],
        "include_local": ["*.md", "VERSION", "*.py"],
        "mirror_include": [".claude/**", ".cursor/**", ".codex/**"],
    },
    "skills": {
        "summary": "one edition's canonical skills only (.agents/skills)",
        "needs_edition": True,
        "root": "{edition}",
        "include": [".agents/skills/**"],
        "include_local": ["AGENTS.md"],
        "mirror_include": [".claude/skills/**", ".cursor/skills/**"],
    },
    "core": {
        "summary": "the shared Python memory/context core of one edition — the "
                   "part cross-edition parity keeps byte-identical",
        "needs_edition": True,
        "root": "{edition}",
        "include": [
            "memory-bank/scripts/**",
            "memory-bank/tests/**",
            "memory-bank/templates/**",
            "project-brain/PROTOCOL.md",
            "project-brain/schemas/**",
            "project-brain/scripts/**",
            "project-brain/tests/**",
        ],
        # Infrastructure-Creator keeps the mirror rules at its top level
        # instead of inside a memory-bank runtime it does not have.
        "include_local": ["mirror_rules.py"],
    },
    "hooks": {
        "summary": "one edition's canonical hooks (.claude/hooks) plus their "
                   "generated Cursor and Codex mirrors",
        "needs_edition": True,
        "root": "{edition}",
        "include": [".claude/hooks/**", ".cursor/hooks/**", ".codex/hooks/**"],
        # This scope is *about* the mirrors, so it opts itself in.
        "force_mirrors": True,
    },
    "tooling": {
        "summary": "repository-level tooling: scripts/, tests/, CI workflows",
        "needs_edition": False,
        "root": ".",
        "include": ["scripts/**", "tests/**", ".github/**", "install/**"],
    },
    "docs": {
        "summary": "repository-level documentation and changelog",
        "needs_edition": False,
        "root": ".",
        "include": ["docs/**"],
        "include_local": ["*.md"],
    },
    "harness": {
        "summary": "the external batch harness (harness/)",
        "needs_edition": False,
        "root": ".",
        "include": ["harness/**"],
    },
    "diff": {
        "summary": "every file changed against --base (default origin/main)",
        "needs_edition": False,
        "root": ".",
        "include": [],  # computed from git
    },
    "custom": {
        "summary": "your own --include patterns, with the safety excludes still on",
        "needs_edition": False,
        "root": ".",
        "include": [],  # requires --include
    },
}


class CollectError(Exception):
    """A condition the operator has to resolve; reported without a traceback."""


# --------------------------------------------------------------------------
# the code2prompt binary
# --------------------------------------------------------------------------

def resolve_binary() -> str:
    name = os.environ.get(BIN_ENV) or DEFAULT_BIN
    path = shutil.which(name)
    if path is None and os.path.sep in name:
        candidate = Path(name)
        path = str(candidate) if candidate.is_file() and os.access(candidate, os.X_OK) else None
    if path is None:
        raise CollectError(
            f"code2prompt not found (looked for {name!r}).\n"
            f"  Install it:  cargo install code2prompt\n"
            f"  or point at an existing binary:  {BIN_ENV}=/path/to/code2prompt"
        )
    return path


def parse_version(text: str) -> tuple[int, int, int]:
    """Parse `code2prompt 4.3.0` into (4, 3, 0)."""
    for token in text.split():
        parts = token.split(".")
        if len(parts) == 3 and all(p.isdigit() for p in parts):
            return tuple(int(p) for p in parts)  # type: ignore[return-value]
    raise CollectError(f"cannot parse a version out of {text.strip()!r}")


def check_version(binary: str) -> tuple[int, int, int]:
    try:
        proc = subprocess.run([binary, "--version"], capture_output=True,
                              text=True, timeout=PROBE_TIMEOUT)
    except (OSError, subprocess.SubprocessError) as exc:
        raise CollectError(f"cannot run {binary}: {exc}") from exc
    if proc.returncode != 0:
        raise CollectError(f"{binary} --version exited {proc.returncode}")
    version = parse_version(proc.stdout or proc.stderr)
    if not MIN_VERSION <= version < MAX_VERSION:
        want = ".".join(map(str, MIN_VERSION))
        cap = ".".join(map(str, MAX_VERSION))
        got = ".".join(map(str, version))
        raise CollectError(
            f"code2prompt {got} is outside the tested range [{want}, {cap}). "
            f"The flag set this script uses was verified on 4.3.0; re-verify "
            f"before raising MAX_VERSION."
        )
    return version


def build_argv(binary: str, target: Path, includes: list[str],
               excludes: list[str], fmt: str, encoding: str) -> list[str]:
    argv = [binary, str(target), "--hidden", "--quiet",
            "--output-format", fmt, "--output-file", "-",
            "--encoding", encoding]
    for pattern in includes:
        argv += ["--include", pattern]
    for pattern in excludes:
        argv += ["--exclude", pattern]
    return argv


def run_code2prompt(argv: list[str], sandbox: Path) -> str:
    """Run code2prompt in a directory and config home we control.

    Both matter: the tool auto-loads `.c2pconfig` from the working directory
    with no way to opt out, so any repository or home directory that happens
    to carry one would silently rewrite the bundle.
    """
    env = dict(os.environ)
    env["XDG_CONFIG_HOME"] = str(sandbox)
    env.pop("NO_COLOR", None)
    try:
        proc = subprocess.run(argv, capture_output=True, text=True,
                              timeout=RUN_TIMEOUT, cwd=sandbox, env=env)
    except subprocess.TimeoutExpired as exc:
        raise CollectError(f"code2prompt timed out after {RUN_TIMEOUT}s") from exc
    except OSError as exc:
        raise CollectError(f"cannot run code2prompt: {exc}") from exc
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or "").strip()
        raise CollectError(f"code2prompt exited {proc.returncode}: {detail}")
    return proc.stdout


# --------------------------------------------------------------------------
# scope resolution
# --------------------------------------------------------------------------

def git_changed_files(base: str) -> list[str]:
    """Repo-relative paths that differ from the merge base with `base`."""
    def git(*args: str) -> subprocess.CompletedProcess:
        # quotepath=false keeps non-ASCII names literal rather than escaped,
        # so a path can be handed straight back as an include pattern.
        return subprocess.run(["git", "-c", "core.quotepath=false", *args],
                              cwd=ROOT, capture_output=True, text=True,
                              timeout=PROBE_TIMEOUT)

    merge_base = git("merge-base", base, "HEAD")
    ref = merge_base.stdout.strip()
    if merge_base.returncode != 0 or not ref:
        if base == "origin/main":
            fallback = git("merge-base", "main", "HEAD")
            ref = fallback.stdout.strip()
            if fallback.returncode == 0 and ref:
                base = "main"
        if not ref:
            raise CollectError(
                f"cannot resolve a merge base with {base!r} — pass an existing "
                f"ref with --base"
            )

    names = git("diff", "--name-only", "--diff-filter=d", ref, "--")
    if names.returncode != 0:
        raise CollectError(f"git diff against {base} failed: {names.stderr.strip()}")
    paths = [line for line in names.stdout.splitlines() if line.strip()]
    if not paths:
        raise CollectError(f"no files changed against {base} ({ref[:12]}).")
    return paths


def resolve_scope(args: argparse.Namespace) -> tuple[Path, list[str], list[str], str]:
    """Return (target dir, include patterns, exclude patterns, bundle label)."""
    spec = SCOPES[args.scope]

    if spec["needs_edition"]:
        if not args.edition:
            raise CollectError(
                f"scope {args.scope!r} needs --edition "
                f"({', '.join(repr(e) for e in EDITIONS)})"
            )
        target = ROOT / args.edition
        if not target.is_dir():
            raise CollectError(f"no such edition directory: {target}")
        label = f"{args.scope}-{args.edition.replace(' ', '-').lower()}"
    else:
        if args.edition:
            raise CollectError(f"scope {args.scope!r} does not take --edition")
        target = ROOT
        label = args.scope

    if args.scope == "diff":
        includes = git_changed_files(args.base)
    else:
        includes = list(spec["include"])
        for pattern in spec.get("include_local", ()):
            # Python's glob does not cross separators, which is exactly the
            # top-level-only semantics code2prompt cannot express; the `./`
            # prefix then keeps the resulting literal anchored on its side.
            includes += [f"./{match.name}" for match in sorted(target.glob(pattern))
                         if match.is_file()]
        if args.with_mirrors:
            # For a scope anchored on canon, lifting the exclusion is not
            # enough — the mirror trees were never in its include list.
            includes += list(spec.get("mirror_include", ()))
    includes += args.include
    if not includes:
        raise CollectError(
            f"scope {args.scope!r} defines no include patterns; pass --include. "
            f"Running code2prompt without one reads the whole tree."
        )

    excludes = list(ALWAYS_EXCLUDE)
    if not args.with_task:
        excludes += list(CLIENT_DATA_EXCLUDE)
    if not (args.with_mirrors or spec.get("force_mirrors")):
        excludes += list(MIRROR_EXCLUDE)
    excludes += args.exclude

    return target, includes, excludes, label


# --------------------------------------------------------------------------
# reporting
# --------------------------------------------------------------------------

def list_scopes() -> None:
    width = max(len(name) for name in SCOPES)
    print("Scopes — /collect <scope> [options] in Claude Code,")
    print("         ./collect <scope> [options] in a shell\n")
    for name, spec in SCOPES.items():
        marker = " *" if spec["needs_edition"] else "  "
        print(f"  {name.ljust(width)}{marker}  {spec['summary']}")
    print("\n  * needs --edition {" + ", ".join(EDITIONS) + "}")
    print("\nAlways excluded: .git, __pycache__, virtualenvs, memory-bank/local,")
    print("project-brain/local, databases, .env and key files.")
    print("Excluded by default: Task/ (client-owned specs; --with-task),")
    print(".claude|.cursor|.codex mirrors (generated; --with-mirrors).")


def print_summary(label: str, target: Path, files: list[str], tokens: int,
                  nbytes: int, encoding: str, destination: str) -> None:
    rel = target.relative_to(ROOT) if target != ROOT else Path(".")
    print(f"bundle      {label}", file=sys.stderr)
    print(f"root        {rel}", file=sys.stderr)
    print(f"files       {len(files)}", file=sys.stderr)
    print(f"size        {nbytes} B", file=sys.stderr)
    print(f"tokens      {tokens} ({encoding}; OpenAI BPE, not a Claude count)",
          file=sys.stderr)
    print(f"output      {destination}", file=sys.stderr)


def largest(files: list[str], target: Path, top: int) -> list[tuple[int, str]]:
    sized = []
    for name in files:
        try:
            sized.append(((target / name).stat().st_size, name))
        except OSError:
            continue
    return sorted(sized, reverse=True)[:top]


# --------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__.splitlines()[0],
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="python3 scripts/collect_context.py --list  shows every scope.",
    )
    parser.add_argument("scope", nargs="?", choices=sorted(SCOPES),
                        help="what to collect; --list describes each one")
    parser.add_argument("--list", action="store_true",
                        help="describe every scope and exit")
    parser.add_argument("--edition", choices=EDITIONS,
                        help="edition for the scopes that need one")
    parser.add_argument("--include", action="append", default=[], metavar="GLOB",
                        help="extra include pattern, relative to the scope root "
                             "(repeatable)")
    parser.add_argument("--exclude", action="append", default=[], metavar="GLOB",
                        help="extra exclude pattern (repeatable)")
    parser.add_argument("--base", default="origin/main", metavar="REF",
                        help="base ref for the 'diff' scope (default: origin/main, "
                             "falling back to main)")
    parser.add_argument("--format", choices=("markdown", "xml", "json"),
                        default="markdown", help="bundle format (default: markdown)")
    parser.add_argument("--encoding", choices=("cl100k", "p50k", "p50k_edit", "r50k"),
                        default="cl100k", help="tokenizer for the count (default: cl100k)")
    parser.add_argument("-o", "--output", metavar="PATH",
                        help="write the bundle here (default: .c2p/<scope>.<ext>)")
    parser.add_argument("--stdout", action="store_true",
                        help="write the bundle to stdout instead of a file")
    parser.add_argument("--dry-run", action="store_true",
                        help="report file count, size and tokens; write nothing")
    parser.add_argument("--top", type=int, default=0, metavar="N",
                        help="also list the N largest files in the bundle")
    parser.add_argument("--with-task", action="store_true",
                        help="include Task/ — client-owned specification material")
    parser.add_argument("--with-mirrors", action="store_true",
                        help="include the generated .claude/.cursor/.codex trees")
    args = parser.parse_args()

    # A bare invocation lists the scopes rather than erroring: it is what
    # `/collect` with no arguments runs, and there is nothing sensible to
    # collect by default.
    if args.list or not args.scope:
        list_scopes()
        return 0

    try:
        binary = resolve_binary()
        version = ".".join(map(str, check_version(binary)))
        target, includes, excludes, label = resolve_scope(args)

        if args.with_task:
            print("WARNING: --with-task includes Task/, which holds client-owned "
                  "product specifications. Do not send this bundle anywhere the "
                  "client's material is not already allowed.", file=sys.stderr)

        with tempfile.TemporaryDirectory(prefix="collect-context-") as sandbox_name:
            sandbox = Path(sandbox_name)
            # One JSON call carries the file list, the token count and — because
            # the `prompt` field is byte-identical to `--output-format xml` — the
            # xml rendering too. Markdown is the only format needing a second run.
            raw = run_code2prompt(
                build_argv(binary, target, includes, excludes, "json", args.encoding),
                sandbox)
            try:
                payload = json.loads(raw)
                files = list(payload["files"])
                tokens = int(payload["token_count"])
                prompt = payload["prompt"]
            except (ValueError, KeyError, TypeError) as exc:
                raise CollectError(f"unexpected code2prompt JSON: {exc}") from exc

            if not files:
                raise CollectError(
                    "the include patterns matched no files. code2prompt reports "
                    "this as success with an empty bundle, so it is caught here.\n"
                    f"  patterns: {', '.join(includes)}\n"
                    f"  root:     {target}"
                )

            if args.format == "json":
                bundle = raw
            elif args.format == "xml":
                bundle = prompt
            else:
                bundle = run_code2prompt(
                    build_argv(binary, target, includes, excludes, "markdown",
                               args.encoding),
                    sandbox)

        nbytes = len(bundle.encode("utf-8"))

        if args.dry_run:
            print_summary(label, target, files, tokens, nbytes, args.encoding,
                          "(dry run — nothing written)")
        elif args.stdout:
            sys.stdout.write(bundle)
            print_summary(label, target, files, tokens, nbytes, args.encoding,
                          "(stdout)")
        else:
            extension = {"markdown": "md", "xml": "xml", "json": "json"}[args.format]
            out_path = (Path(args.output).expanduser().resolve() if args.output
                        else DEFAULT_OUT_DIR / f"{label}.{extension}")
            out_path.parent.mkdir(parents=True, exist_ok=True)
            out_path.write_text(bundle, encoding="utf-8")
            manifest = out_path.with_suffix(out_path.suffix + ".manifest.json")
            manifest.write_text(json.dumps({
                "scope": args.scope,
                "edition": args.edition,
                "root": str(target.relative_to(ROOT)) if target != ROOT else ".",
                "include": includes,
                "exclude": excludes,
                "format": args.format,
                "encoding": args.encoding,
                "files": len(files),
                "bytes": nbytes,
                "tokens": tokens,
                "with_task": args.with_task,
                "with_mirrors": args.with_mirrors,
                "code2prompt": version,
            }, indent=2) + "\n", encoding="utf-8")
            try:
                shown = out_path.relative_to(ROOT)
            except ValueError:
                shown = out_path
            print_summary(label, target, files, tokens, nbytes, args.encoding,
                          str(shown))

        if args.top:
            print(f"\nlargest {args.top}:", file=sys.stderr)
            for size, name in largest(files, target, args.top):
                print(f"  {size:>9} B  {name}", file=sys.stderr)

    except CollectError as exc:
        print(f"collect-context: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
