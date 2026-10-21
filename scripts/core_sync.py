#!/usr/bin/env python3
"""One source for the memory core: `PHP Core/`, written out to its copies.

The memory core - the runtime scripts, their tests, the Project Brain
protocol, schemas and engine hooks - is byte-identical in the four editions
(`context.py parity --cross-edition`), in the generator's memory-seed asset
(`asset_parity.py`) and in each edition's tool mirrors (`build_mirrors.py`).
Those gates only compare. A core change used to be made in one edition and
copied to the others by hand: every change touched dozens of files, and a
reviewer had to tell the one real diff from its copies.

`PHP Core/` is now the one place a core change is made:

    python3 scripts/core_sync.py --check   # report what differs; exit 1 if anything does
    python3 scripts/core_sync.py --write   # copy, then asset parity and mirrors --write

The file set is the runtime's own: `CROSS_EDITION_CORE_MANIFEST` minus
`CROSS_EDITION_ALLOWED_DRIFT`, read from `PHP Core`'s `context_retrieval.py`,
so the writer and the parity gate cannot disagree about what the core is. A
core file removed from `PHP Core/` is removed from the copies. The executable
bit follows the canonical file. The copies are marked `linguist-generated` in
`.gitattributes`, so a pull request shows the canonical change once.

Standard library only; writes only with --write.
"""
from __future__ import annotations

import argparse
import importlib.util
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

REPO_ROOT = Path(__file__).resolve().parent.parent
CANONICAL = "PHP Core"


def manifest(repo_root: Path = REPO_ROOT) -> Any:
    """The canonical runtime's parity definitions."""
    scripts = repo_root / CANONICAL / "memory-bank" / "scripts"
    sys.path.insert(0, str(scripts))
    try:
        spec = importlib.util.spec_from_file_location("core_sync_runtime", scripts / "context_retrieval.py")
        if spec is None or spec.loader is None:
            raise SystemExit(f"core_sync: {scripts / 'context_retrieval.py'} is missing")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(scripts))
    return module


def core_files(base: Path, runtime: Any) -> Dict[str, Path]:
    """Every core file under one edition root, by its edition-relative path."""
    found: Dict[str, Path] = {}
    for pattern in runtime.CROSS_EDITION_CORE_MANIFEST:
        for path in sorted(base.glob(pattern)):
            if not path.is_file() or path.is_symlink():
                continue
            parts = path.relative_to(base).parts
            if runtime._mirror_is_ignored(parts):
                continue
            relative = "/".join(parts)
            if runtime._cross_edition_allowed(relative):
                continue
            found[relative] = path
    return found


def executable(path: Path) -> bool:
    return bool(path.stat().st_mode & 0o111)


def plan(repo_root: Path, runtime: Any) -> List[Dict[str, str]]:
    """What --write would do, edition by edition: write, remove or chmod."""
    canonical = core_files(repo_root / CANONICAL, runtime)
    actions: List[Dict[str, str]] = []
    for name, relative_root in runtime.CROSS_EDITION_PATHS.items():
        if name == CANONICAL:
            continue
        base = repo_root / relative_root
        if not base.is_dir():
            continue
        copies = core_files(base, runtime)
        for relative, source in canonical.items():
            target = base / relative
            if relative not in copies:
                actions.append({"edition": name, "path": relative, "action": "add"})
            elif target.read_bytes() != source.read_bytes():
                actions.append({"edition": name, "path": relative, "action": "update"})
            elif executable(target) != executable(source):
                actions.append({"edition": name, "path": relative, "action": "mode"})
        for relative in sorted(set(copies) - set(canonical)):
            actions.append({"edition": name, "path": relative, "action": "remove"})
    return actions


def write_bytes(target: Path, payload: bytes, mode: int) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{target.name}.", dir=str(target.parent))
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(payload)
        os.chmod(temporary, mode)
        os.replace(temporary, target)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def apply(repo_root: Path, runtime: Any, actions: Sequence[Dict[str, str]]) -> None:
    canonical_root = repo_root / CANONICAL
    for action in actions:
        base = repo_root / runtime.CROSS_EDITION_PATHS[action["edition"]]
        target = base / action["path"]
        if action["action"] == "remove":
            target.unlink()
            continue
        source = canonical_root / action["path"]
        mode = 0o755 if executable(source) else 0o644
        if action["action"] == "mode":
            os.chmod(target, mode)
        else:
            write_bytes(target, source.read_bytes(), mode)


def chained(repo_root: Path) -> int:
    """The generator asset and the tool mirrors follow the editions."""
    status = 0
    for command in (["scripts/asset_parity.py", "--write"], ["scripts/build_mirrors.py", "--write"]):
        completed = subprocess.run([sys.executable, *command], cwd=str(repo_root))
        status = status or completed.returncode
    return status


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="report the copies that differ from PHP Core")
    mode.add_argument("--write", action="store_true", help="write PHP Core's core to every copy")
    parser.add_argument("--root", type=Path, default=REPO_ROOT, help=argparse.SUPPRESS)
    parser.add_argument("--no-chain", action="store_true",
                        help="with --write, skip asset_parity.py --write and build_mirrors.py --write")
    arguments = parser.parse_args(argv)
    repo_root = arguments.root.resolve()
    runtime = manifest(REPO_ROOT)
    actions = plan(repo_root, runtime)
    for action in actions:
        print(f"{action['action'].upper()}\t{action['edition']}\t{action['path']}")
    if arguments.check:
        if actions:
            print(f"core_sync: {len(actions)} copy difference(s); run scripts/core_sync.py --write", file=sys.stderr)
            return 1
        print("core_sync: every copy matches PHP Core")
        return 0
    apply(repo_root, runtime, actions)
    print(f"core_sync: {len(actions)} change(s) written")
    if arguments.no_chain:
        return 0
    return chained(repo_root)


if __name__ == "__main__":
    raise SystemExit(main())
