#!/usr/bin/env python3
"""Reproduce the source-link pilot on isolated, synthetic prompt-time snapshots.

Creates only a new directory outside this clone, with a tiny Git fixture and
baseline/automatic results from the same memory_eval scorer. No models, service or network.
Run: python3 scripts/memory_graph_pilot.py --out-dir /tmp/graph-pilot-example
"""
from __future__ import annotations

import argparse
import json
import hashlib
import shutil
import os
from pathlib import Path, PurePosixPath
import subprocess
import sys

import memory_eval

ROOT = Path(__file__).resolve().parent.parent
SOURCE = "specs/rounding.md"
FIRST = "memory-bank/chunks/MEM-20260801-bbbbbbbb-minor-units.md"
SECOND = "memory-bank/chunks/MEM-20260801-cccccccc-refunds.md"
FUTURE = "memory-bank/chunks/MEM-20260810-dddddddd-reserve.md"
DECOY = "memory-bank/chunks/MEM-20260801-eeeeeeee-dashboard.md"
FIRST_ANSWER = "Floating point never touches a stored invoice total."
SECOND_ANSWER = "A refund can never exceed the stored invoice total."
FUTURE_ANSWER = "The reserve must be settled before the second transfer."


def write(root: Path, relative: str, value: str) -> None:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value, encoding="utf-8")


def git(root: Path, when: str, *args: str) -> None:
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update({
        "GIT_AUTHOR_NAME": "Graph fixture", "GIT_COMMITTER_NAME": "Graph fixture",
        "GIT_AUTHOR_EMAIL": "fixture@example.invalid", "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
        "GIT_AUTHOR_DATE": when, "GIT_COMMITTER_DATE": when,
        "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
    })
    subprocess.run(
        ["git", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main", *args],
        cwd=root, env=env, check=True, capture_output=True,
    )


def chunk(path: str, title: str, created: str, source: str, body: str) -> str:
    metadata = {
        "id": "-".join(Path(path).stem.split("-")[:3]),
        "title": title, "type": "convention", "status": "active",
        "scope": ["application"], "tags": ["billing"], "created": created,
        "last_verified": created, "review_after": "2027-08-01",
        "sources": [source], "supersedes": [], "superseded_by": None,
    }
    return "---\n" + json.dumps(metadata, indent=2) + "\n---\n\n# " + title + "\n\n" + body + "\n"


def fixture(base: Path) -> tuple[Path, Path, Path, Path]:
    project = base / "projects" / "billing"
    project.mkdir(parents=True)
    write(project, SOURCE, "# Rounding\n\nAmounts use half-to-even banker tie breaking.\n")
    write(project, "specs/delivery.md", "# Delivery\n\nQuartz shipment customs boundary: check customs before releasing the shipment.\n")
    write(project, "specs/reserve.md", "# Reserve\n\nThe heliotrope reserve escrow has a separate settlement policy.\n")
    write(project, "specs/format.md", "# Format\n\nAmber representation encoding serialization requires UTF-8.\n")
    write(project, FIRST, chunk(FIRST, "Minor units", "2026-08-01", SOURCE, FIRST_ANSWER))
    write(project, SECOND, chunk(SECOND, "Refund validation", "2026-08-01", SOURCE, SECOND_ANSWER))
    write(project, DECOY, chunk(DECOY, "Export dashboard", "2026-08-01", "specs/format.md",
                               "Use blue headings in the export dashboard."))
    git(project, "2026-08-01T10:00:00Z", "init", "--quiet")
    git(project, "2026-08-01T10:00:00Z", "add", "specs", "memory-bank")
    git(project, "2026-08-01T10:00:00Z", "commit", "--quiet", "-m", "Initial source-backed knowledge")
    write(project, FUTURE, chunk(FUTURE, "Reserve consequence", "2026-08-10", "specs/reserve.md", FUTURE_ANSWER))
    git(project, "2026-08-10T10:00:00Z", "add", "memory-bank")
    git(project, "2026-08-10T10:00:00Z", "commit", "--quiet", "-m", "Future knowledge")
    prompts = [
        {"id": "source-link", "prompt": "Explain half-to-even banker tie breaking."},
        {"id": "direct-memory", "prompt": "Can a refund exceed the stored invoice total?"},
        {"id": "unrelated", "prompt": "Explain the quartz shipment customs boundary."},
        {"id": "no-answer", "prompt": "Explain vermilion semaphore calibration."},
        {"id": "future", "prompt": "Explain heliotrope reserve escrow."},
        {"id": "source-decoy", "prompt": "Explain amber representation encoding serialization."},
    ]
    for prompt in prompts:
        prompt.update(project="billing", ts="2026-08-05T12:00:00Z")
    grades = {
        "source-link": {SOURCE: 1, FIRST: 2, SECOND: 2},
        "direct-memory": {FIRST: 1, SECOND: 2},
        "unrelated": {"specs/delivery.md": 2},
        "no-answer": {},
        "future": {"specs/reserve.md": 1, FUTURE: 2},
        "source-decoy": {"specs/format.md": 2, DECOY: 0},
    }
    passages = {
        "source-link": {
            FIRST: {"useful": True, "passages": [FIRST_ANSWER]},
            SECOND: {"useful": True, "passages": [SECOND_ANSWER]},
        },
        "direct-memory": {SECOND: {"useful": True, "passages": [SECOND_ANSWER]}},
        "unrelated": {"specs/delivery.md": {"useful": True, "passages": ["check customs before releasing the shipment"]}},
        "future": {FUTURE: {"useful": True, "passages": [FUTURE_ANSWER]}},
        "source-decoy": {"specs/format.md": {"useful": True, "passages": ["requires UTF-8"]}},
    }
    # These questions ask for fixture facts, not development procedures.
    # Grade all shipped skill pointers as noise rather than hide them under
    # "unjudged"; an unrelated source-linked memory is an explicit decoy too.
    edition = ROOT / "PHP Core"
    skill_paths = [path.relative_to(edition).as_posix()
                   for path in (edition / ".agents/skills").rglob("*.md")]
    for entries in grades.values():
        for path in skill_paths:
            entries.setdefault(path, 0)
    inputs = base / "inputs"
    inputs.mkdir()
    for name, data in (("set", prompts), ("judgments", grades), ("passages", passages)):
        write(inputs, name + ".json", json.dumps(data, indent=2))
    return project, inputs / "set.json", inputs / "judgments.json", inputs / "passages.json"



def materialize_baseline(base: Path, reference: str) -> tuple[Path, str]:
    """Read exact Git blobs; archive export attributes must not change runtime."""
    commit = memory_eval.read_git(ROOT, "rev-parse", "--verify", "--end-of-options", reference + "^{commit}")
    if not commit:
        raise ValueError("Baseline reference does not identify a local commit")
    target = base / "baseline-runtime"
    target.mkdir()
    tree = subprocess.run(
        ["git", "-C", str(ROOT), "ls-tree", "-rz", "--full-tree", commit,
         "--", *[str(path) for path in memory_eval.installer.EDITION_PATHS.values()],
         "scripts", "install/inventories"],
        capture_output=True, check=True, env=memory_eval.read_env(),
    ).stdout
    with memory_eval.BlobReader(ROOT) as blobs:
        for entry in tree.split(b"\0"):
            if not entry:
                continue
            header, raw_path = entry.split(b"\t", 1)
            mode, kind, identifier = header.split()
            relative = PurePosixPath(raw_path.decode("utf-8"))
            if relative.is_absolute() or ".." in relative.parts or kind != b"blob" or mode not in (b"100644", b"100755"):
                raise ValueError("Baseline contains an unsupported file entry")
            data = blobs.read(identifier)
            if data is None:
                raise ValueError("Baseline file is unavailable")
            path = target.joinpath(*relative.parts)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            path.chmod(0o755 if mode == b"100755" else 0o644)
    if not (target / "PHP Core/memory-bank/scripts/context.py").is_file():
        raise ValueError("Baseline does not contain the PHP Core runtime")
    # Both versions use the same current scoring code, not their old metrics.
    shutil.copyfile(ROOT / "scripts/memory_eval.py", target / "scripts/memory_eval.py")
    return target, commit


def evaluate_runtime(root: Path, arguments: list[str]) -> int:
    return subprocess.run(
        [sys.executable, str(root / "scripts/memory_eval.py"), *arguments],
        cwd=root, env=memory_eval.read_env(), check=False,
    ).returncode


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", required=True, type=Path,
                        help="new directory outside this clone; existing directories are refused")
    parser.add_argument("--baseline-ref", default="HEAD",
                        help="local committed runtime to compare with the working tree (default HEAD)")
    args = parser.parse_args(argv)
    base = args.out_dir.expanduser().resolve()
    if base == ROOT or ROOT in base.parents or base.exists():
        parser.error("--out-dir must be a new directory outside the clone")
    base.mkdir(parents=True, mode=0o700)
    try:
        baseline_root, commit = materialize_baseline(base, args.baseline_ref)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.error(str(error))
    project, prompts, grades, passages = fixture(base)
    # Skills present only in the committed runtime are graded too.
    judgments = json.loads(grades.read_text())
    for path in (baseline_root / "PHP Core/.agents/skills").rglob("*.md"):
        relative = path.relative_to(baseline_root / "PHP Core").as_posix()
        for entries in judgments.values():
            entries.setdefault(relative, 0)
    grades.write_text(json.dumps(judgments, indent=2))
    outputs = []
    scorer = hashlib.sha256((ROOT / "scripts/memory_eval.py").read_bytes()).hexdigest()
    for label, root in (("baseline", baseline_root), ("automatic", ROOT)):
        output = base / "results" / (label + ".json")
        status = evaluate_runtime(root, [
            "run", "--set", str(prompts), "--judgments", str(grades),
            "--passages", str(passages), "--projects-root", str(project.parent),
            "--edition", "PHP Core", "--host", "codex",
            "--cache", str(base / "cache" / label), "--out", str(output),
        ])
        if status != 0:
            return status
        if output.is_file():
            result = json.loads(output.read_text())
            result["meta"]["runtime_under_test"] = {
                "label": label, "commit": commit if label == "baseline" else memory_eval.read_git(ROOT, "rev-parse", "HEAD"),
                "materialization": "git-blobs" if label == "baseline" else "working-tree",
                "scorer_sha256": scorer,
            }
            output.write_text(json.dumps(result, indent=2))
        outputs.append(output)
    return memory_eval.main(["report", str(outputs[0]), "--compare", str(outputs[1])])


if __name__ == "__main__":
    raise SystemExit(main())
