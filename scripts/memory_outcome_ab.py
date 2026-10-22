#!/usr/bin/env python3
"""Does the automatic capsule change what an agent does at the start of a task?

`memory_eval.py` scores what a capsule delivers against relevance labels.
This tool measures what an agent then does. For a task that really happened -
the first prompt of a recorded session that went on to change files - it
rebuilds the project as it was at that prompt (the reconstruction
`memory_eval.py` uses), runs a headless Codex session on the same prompt twice:

- `off`: no hooks at all;
- `capsule`: only the memory prompt hook (`working-memory-read.sh`), so the
  arms differ by the Task Capsule and nothing else.

It counts the tool calls before the agent first touches a file the original
session changed (the task's *gold* files). A run stops at that touch, at
`--cap` calls or at `--timeout`, so a pair costs the start of two sessions,
not two finished tasks. Recent-files history is seeded into the corpus from
the project's own sessions before the prompt, as the Stop hook would have
recorded it.

The `navigation` and `recall` arms run the same hook with that
`capsule_profile` in the corpus.

A *recall* set measures something else: whether an agent uses what an
earlier session saved instead of investigating again. A session whose first
turn read project files and answered with a conclusion is the source
(`recall-sources`); the conclusion, as a result save would record it, and a
follow-up question that needs it are written outside this tool and checked
by `recall-set`. `run` records the save on the corpus branch's task through
`record-result` before any arm runs, and counts the calls until the agent
touches a file the save cites (*needed*) or answers naming one.

Maintainer-only and standard library only; never installed into a project.
Sets and results name client paths and carry prompts: keep them outside this
repository (the tool refuses an output inside the clone). The agent runs in
Codex's read-only sandbox with `--ephemeral`, so neither the corpus nor
`~/.codex/sessions` keeps what it did.

    memory_outcome_ab.py sample --projects next,scb --before 2026-10-07 \\
        --n 30 --seed 1 --exclude-set tuning-set.json --out ~/ab/set.json
    memory_outcome_ab.py run --set ~/ab/set.json --out ~/ab/result.json
    memory_outcome_ab.py report ~/ab/result.json

    memory_outcome_ab.py recall-sources --projects next,scb --before 2026-10-07 \\
        --out ~/ab/recall-sources.json
    memory_outcome_ab.py recall-set --sources ~/ab/recall-sources.json \\
        --scenarios ~/ab/recall-authored.json --n 30 --seed 2028 --out ~/ab/recall-set.json
    memory_outcome_ab.py run --set ~/ab/recall-set.json --arms off,capsule,navigation,recall \\
        --out ~/ab/recall-result.json

See docs/MEMORY-EVAL.md ("Measuring task outcomes").
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
import queue
import random
import re
import shlex
import shutil
import signal
import sqlite3
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Any, Dict, Iterator, List, Optional, Sequence, Set, Tuple

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import memory_eval as stand  # noqa: E402
from accelerator_attach import _toml  # noqa: E402

ARMS = ("off", "capsule", "navigation", "recall")
DEFAULT_ARMS = ("off", "capsule")
# `navigation` and `recall` run the same prompt hook with that
# `capsule_profile` in the corpus. Navigation: working state and recent files,
# no documents and no manifest. Recall adds the task's own saves, with a
# manifest only on a turn one of them matched.
HOOKED_ARMS = ("capsule", "navigation", "recall")
PROFILE_ARMS = ("navigation", "recall")
# What a run counts: calls to the first gold touch, or for a recall set to the
# first needed touch or an answer naming a needed file.
METRICS = ("gold", "recall")
DEFAULT_CAP = 40
DEFAULT_TIMEOUT = 900.0
TOOL_ITEMS = ("command_execution", "file_change", "mcp_tool_call", "web_search")
SHELLS = {"bash", "sh", "zsh", "dash"}
# Files the memory system itself writes are not what a task is about.
NOT_GOLD = re.compile(
    r"^(?:memory-bank/|project-brain/|\.claude/|\.codex/|\.cursor/|\.agents/|\.git/|"
    r"\.context-handoff/|tasks/|AGENTS\.md$|CLAUDE\.md$|ACCELERATOR\.md$)"
)
PROMPT_HOOK = "working-memory-read.sh"
HOOK_ENTRY_POINTS = ("refresh", "hook-context")
SEEDED_TURNS = 400
# A recall source: a first turn that answered at this length or more.
RECALL_ANSWER_MINIMUM = 300
RECALL_ANSWER_LIMIT = 6000
# The question comes after the session that saved: a minute after its first
# turn's last record, so the corpus and the recent files are the ones then.
RECALL_GAP_SECONDS = 60
# The runtime's own limits for one learning (memory_results.normalize).
LEARNING_TITLE_LIMIT = 200
LEARNING_CONSEQUENCE_LIMIT = 1000
LEARNING_SOURCE_LIMIT = 10
_RUNTIME: Any = None


class OutcomeError(Exception):
    """A problem the user can fix; reported without a traceback."""


def runtime() -> Any:
    """The memory runtime's own transcript parsers, from this clone."""
    global _RUNTIME
    if _RUNTIME is None:
        scripts = ROOT / "PHP Core" / "memory-bank" / "scripts"
        sys.path.insert(0, str(scripts))
        spec = importlib.util.spec_from_file_location("outcome_ab_runtime", scripts / "context.py")
        if spec is None or spec.loader is None:
            raise OutcomeError("the memory runtime is missing from this clone")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _RUNTIME = module
    return _RUNTIME


# ---------------------------------------------------------------------------
# Paths a session names
# ---------------------------------------------------------------------------


def unwrap_shell(command: str) -> str:
    """The script of `/usr/bin/bash -lc "..."`; anything else unchanged."""
    try:
        words = shlex.split(command)
    except ValueError:
        return command
    if len(words) >= 3 and os.path.basename(words[0]) in SHELLS and words[1] in ("-c", "-lc", "-ic", "-lic"):
        return words[2]
    return command


def project_path(word: str, base: str, root: str) -> Optional[str]:
    """`word`, said in directory `base`, as a path relative to `root`.

    `path:12`, `path:12:3` and `path#L3` name their path. Nothing is checked
    on disk: a gold file may have been removed since the session.
    """
    word = word.strip().strip("'\"`,;()[]{}<>")
    if not word or word.startswith("-") or len(word) > 400 or "\0" in word:
        return None
    word = re.sub(r"(:\d+(:\d+)?|#L\d+(-L?\d+)?)$", "", word)
    candidate = word if os.path.isabs(word) else os.path.join(base, word)
    candidate = os.path.normpath(candidate)
    root = os.path.normpath(root)
    if candidate != root and candidate.startswith(root + os.sep):
        return PurePosixPath(*Path(os.path.relpath(candidate, root)).parts).as_posix()
    return None


def call_paths(name: str, data: Dict[str, Any], base: str, root: str) -> List[Tuple[str, bool]]:
    """(project path, changed) for what one recorded call names."""
    found: List[Tuple[str, bool]] = []
    for word, changed, directory in runtime()._call_file_words(name, data, base):
        relative = project_path(str(word), str(directory), root)
        if relative is not None:
            found.append((relative, bool(changed)))
    return found


def prompt_text(record: Dict[str, Any]) -> str:
    message = record.get("message") if isinstance(record.get("message"), dict) else {}
    content = message.get("content")
    if isinstance(content, list):
        content = "".join(
            part["text"] for part in content
            if isinstance(part, dict) and isinstance(part.get("text"), str)
        )
    return content if isinstance(content, str) else ""


def read_records(path: Path) -> Iterator[Dict[str, Any]]:
    try:
        handle = path.open("rb")
    except OSError:
        return
    with handle:
        for raw in handle:
            try:
                record = json.loads(raw)
            except (UnicodeDecodeError, ValueError):
                continue
            if isinstance(record, dict):
                yield record


def session_turns(path: Path, root: str) -> Iterator[Dict[str, Any]]:
    """Each human turn of a Claude Code session: its prompt, when it was
    asked, and the project files its calls changed and read, in call order."""
    rt = runtime()
    turn: Optional[Dict[str, Any]] = None
    for record in read_records(path):
        if rt._transcript_prompt(record):
            if turn is not None:
                yield turn
            turn = {"prompt": prompt_text(record), "ts": record.get("timestamp"),
                    "changed": [], "read": []}
            continue
        if turn is None:
            continue
        for name, data, base in rt._transcript_calls(record, root):
            for relative, changed in call_paths(name, data, base, root):
                (turn["changed"] if changed else turn["read"]).append(relative)
    if turn is not None:
        yield turn


def transcript_directory(transcripts: Path, project: Path) -> Path:
    """Claude Code keeps a project's sessions under its path with every
    character other than a letter or digit replaced by a dash."""
    return transcripts / re.sub(r"[^A-Za-z0-9]", "-", str(project))


def session_files(transcripts: Path, project: Path) -> List[Path]:
    """Main sessions only: side-agent transcripts sit in subdirectories."""
    folder = transcript_directory(transcripts, project)
    try:
        return sorted(path for path in folder.glob("*.jsonl") if path.is_file())
    except OSError:
        return []


# ---------------------------------------------------------------------------
# sample
# ---------------------------------------------------------------------------


def named_in(prompt: str, path: str) -> bool:
    folded = prompt.casefold()
    return path.casefold() in folded or PurePosixPath(path).name.casefold() in folded


def excluded_moments(paths: Sequence[Path]) -> Dict[str, List[datetime]]:
    """When the prompts of earlier sets were asked, per project: a memory_eval
    set, or a sample this tool made."""
    moments: Dict[str, List[datetime]] = {}
    for path in paths:
        try:
            document = json.loads(path.read_text(encoding="utf-8"))
            items = (document["tasks"] if isinstance(document, dict) and isinstance(document.get("tasks"), list)
                     else stand.load_set(path))
        except (OSError, ValueError, stand.EvalError) as error:
            raise OutcomeError(f"--exclude-set {path}: {error}") from error
        for item in items:
            moment = stand.parse_instant(item.get("ts"))
            if moment is not None:
                moments.setdefault(PurePosixPath(str(item.get("project"))).name, []).append(moment)
    return moments


def instant(value: str) -> Optional[datetime]:
    """A timestamp, or a bare date as its UTC midnight."""
    moment = stand.parse_instant(value)
    if moment is None and re.fullmatch(r"\d{4}-\d{2}-\d{2}", value or ""):
        moment = stand.parse_instant(value + "T00:00:00Z")
    return moment


def task_id(project: str, ts: str) -> str:
    return "T" + hashlib.sha256(f"{project}\0{ts}".encode("utf-8")).hexdigest()[:10]


SHELL_SYNTAX = re.compile(r"[$`*?\\\s\"'|;&<>(){}]|^s/|^\+")


def git_paths(project: Path) -> Set[str]:
    """Every path the project's Git has tracked, on any branch, or tracks now.

    A word a shell command wrote to is a gold file only if it names one:
    `sed -i 's/a/b/'`, `chmod +x` and `$DIR/x` are words, not files.
    """
    known: Set[str] = set()
    for arguments in (("log", "--all", "--no-renames", "--name-only", "--format="), ("ls-files",)):
        listing = stand.read_git(project, *arguments)
        if listing:
            known.update(line for line in listing.splitlines() if line)
    return known


def cold_starts(project: Path, transcripts: Path, after: Optional[datetime],
                before: datetime) -> Iterator[Dict[str, Any]]:
    """The first human prompt of each session that went on to change a file
    the prompt does not name."""
    root = os.path.realpath(str(project))
    known = git_paths(project)
    for path in session_files(transcripts, project):
        first = next(session_turns(path, root), None)
        if first is None:
            continue
        moment = stand.parse_instant(first["ts"])
        if moment is None or moment >= before or (after is not None and moment < after):
            continue
        gold = [
            item for item in dict.fromkeys(first["changed"])
            if item in known and not SHELL_SYNTAX.search(item) and not NOT_GOLD.match(item)
            and runtime()._recent_file_kept(item) and not named_in(first["prompt"], item)
        ]
        if not gold or not first["prompt"].strip():
            continue
        yield {
            "id": task_id(project.name, str(first["ts"])),
            "project": project.name,
            "ts": str(first["ts"]),
            "prompt": first["prompt"],
            "gold": gold,
            "session": path.stem,
        }


def command_sample(arguments: argparse.Namespace) -> int:
    out = arguments.out.expanduser()
    if stand.inside_clone(out):
        raise OutcomeError(f"--out {out} is inside this clone: a set carries client prompts")
    before = instant(arguments.before)
    after = instant(arguments.after) if arguments.after else None
    if before is None or (arguments.after and after is None):
        raise OutcomeError("--before/--after must be ISO dates or timestamps")
    exclude = excluded_moments([Path(path).expanduser() for path in arguments.exclude_set or []])
    projects_root = arguments.projects_root.expanduser()
    transcripts = arguments.transcripts.expanduser()
    candidates: List[Dict[str, Any]] = []
    for name in [value.strip() for value in arguments.projects.split(",") if value.strip()]:
        project = stand.resolve_project(name, projects_root)
        if project is None:
            raise OutcomeError(f"no project {name} under {projects_root}")
        for task in cold_starts(project, transcripts, after, before):
            moment = stand.parse_instant(task["ts"])
            if any(abs((moment - other).total_seconds()) <= 5 for other in exclude.get(task["project"], [])):
                continue
            candidates.append(task)
    candidates.sort(key=lambda task: (task["project"], task["ts"]))
    chosen = candidates if arguments.n >= len(candidates) else random.Random(arguments.seed).sample(candidates, arguments.n)
    chosen.sort(key=lambda task: (task["project"], task["ts"]))
    stand.write_json(out, {
        "version": 1,
        "created": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "before": arguments.before, "after": arguments.after, "seed": arguments.seed,
        "candidates": len(candidates), "tasks": chosen,
    })
    os.chmod(out, 0o600)
    print(f"{len(chosen)} of {len(candidates)} cold starts with changed files -> {out}")
    return 0


# ---------------------------------------------------------------------------
# recall-sources and recall-set
# ---------------------------------------------------------------------------


def first_turn(path: Path, root: str) -> Optional[Dict[str, Any]]:
    """A session's first human turn: its prompt, the files its calls changed
    and read, its last answer and the time of its last record."""
    rt = runtime()
    turn: Optional[Dict[str, Any]] = None
    for record in read_records(path):
        if rt._transcript_prompt(record):
            if turn is not None:
                break
            turn = {"prompt": prompt_text(record), "ts": record.get("timestamp"),
                    "end": record.get("timestamp"), "changed": [], "read": [], "answer": ""}
            continue
        if turn is None:
            continue
        if isinstance(record.get("timestamp"), str):
            turn["end"] = record["timestamp"]
        message = record.get("message") if isinstance(record.get("message"), dict) else {}
        if record.get("type") == "assistant" and isinstance(message.get("content"), list):
            texts = [part["text"] for part in message["content"]
                     if isinstance(part, dict) and part.get("type") == "text"
                     and isinstance(part.get("text"), str) and part["text"].strip()]
            if texts:
                turn["answer"] = texts[-1]
        for name, data, base in rt._transcript_calls(record, root):
            for relative, changed in call_paths(name, data, base, root):
                (turn["changed"] if changed else turn["read"]).append(relative)
    return turn


def later_moment(project: Path, repository: Any, history: List[Tuple[datetime, List[str]]],
                 starts: List[datetime], moment: datetime, named: Sequence[str],
                 before: datetime) -> Optional[str]:
    """The first later session start whose recent files show none of `named`.

    The recent files are replayed from every human turn before that start,
    as the Stop hook kept them (record_recent_files), and shown as the capsule
    shows them: the first RECENT_FILES_LIMIT kept that exist in the corpus,
    the project's tree in the commit before that start. Asked then, a question
    about the save finds no pointer to its subject in the navigation capsule.
    Every named file must still exist in that tree.
    """
    rt = runtime()
    wanted = set(named)
    recent: List[str] = []
    position = 0
    for start in starts:
        while position < len(history) and history[position][0] < start:
            recent = list(dict.fromkeys([*history[position][1], *recent]))[:rt.RECENT_FILES_KEPT]
            position += 1
        if start <= moment or start >= before:
            continue
        commit = repository.commit_before(start)
        tree = repository.tree(commit) if commit else None
        listing = stand.read_git(project, "ls-tree", "-r", "--name-only", tree) if tree else None
        if listing is None:
            continue
        present = set(listing.splitlines())
        if not wanted <= present:
            continue
        shown = [item for item in recent if rt._recent_file_kept(item) and item in present][:rt.RECENT_FILES_LIMIT]
        if not wanted & set(shown):
            return start.isoformat().replace("+00:00", "Z")
    return None


def recall_sources(project: Path, transcripts: Path, after: Optional[datetime], before: datetime,
                   exclude: Optional[Dict[str, List[datetime]]] = None) -> Iterator[Dict[str, Any]]:
    """First turns that read project files and answered with a conclusion
    naming one of them: what a session would save as a learning.

    A turn that changed a file is left out - its conclusion describes code
    the corpus a minute later may not hold yet - and so is a sampled cold
    start. The question comes RECALL_GAP_SECONDS after the turn's last record;
    every file the answer names must exist in the commit the corpus is built
    from then. `later` is the start of the first later session at which the
    recent files no longer show any of them (later_moment), or None.
    """
    root = os.path.realpath(str(project))
    known = git_paths(project)
    repository = stand.Repository(project)
    history: List[Tuple[datetime, List[str]]] = []
    starts: List[datetime] = []
    for path in session_files(transcripts, project):
        for number, turn in enumerate(session_turns(path, root)):
            when = stand.parse_instant(turn["ts"])
            if when is None:
                continue
            if number == 0:
                starts.append(when)
            files = [item for item in dict.fromkeys(turn["changed"] + turn["read"]) if item in known]
            if files:
                history.append((when, files))
    history.sort(key=lambda entry: entry[0])
    starts.sort()
    for path in session_files(transcripts, project):
        turn = first_turn(path, root)
        if turn is None or not turn["prompt"].strip():
            continue
        asked, ended = stand.parse_instant(turn["ts"]), stand.parse_instant(turn["end"])
        if asked is None or ended is None or asked >= before or (after is not None and asked < after):
            continue
        if any(abs((asked - other).total_seconds()) <= 5 for other in (exclude or {}).get(project.name, [])):
            continue

        def kept(item: str) -> bool:
            return item in known and not SHELL_SYNTAX.search(item) and not NOT_GOLD.match(item)

        if any(kept(item) for item in turn["changed"]):
            continue
        answer = turn["answer"].strip()
        read = [item for item in dict.fromkeys(turn["read"]) if kept(item) and runtime()._recent_file_kept(item)]
        named = [item for item in read if named_in(answer, item)]
        if len(answer) < RECALL_ANSWER_MINIMUM or not named:
            continue
        moment = ended.timestamp() + RECALL_GAP_SECONDS
        stamp = datetime.fromtimestamp(moment, timezone.utc).isoformat().replace("+00:00", "Z")
        commit = repository.commit_before(stand.parse_instant(stamp)) if repository.prefix is not None else None
        if commit is None:
            continue
        present = [item for item in named if stand.read_git(project, "cat-file", "-e", f"{commit}:{item}") is not None]
        if present != named:
            continue
        yield {
            "id": task_id(project.name, stamp),
            "project": project.name,
            "ts": stamp,
            "asked": str(turn["ts"]),
            "session": path.stem,
            "prompt": turn["prompt"],
            "answer": answer[:RECALL_ANSWER_LIMIT],
            "read": read[:20],
            "named": named,
            "later": later_moment(project, repository, history, starts,
                                  stand.parse_instant(stamp), named, before),
        }


def command_recall_sources(arguments: argparse.Namespace) -> int:
    out = arguments.out.expanduser()
    if stand.inside_clone(out):
        raise OutcomeError(f"--out {out} is inside this clone: sources carry client prompts and answers")
    before = instant(arguments.before)
    after = instant(arguments.after) if arguments.after else None
    if before is None or (arguments.after and after is None):
        raise OutcomeError("--before/--after must be ISO dates or timestamps")
    exclude = excluded_moments([Path(path).expanduser() for path in arguments.exclude_set or []])
    projects_root = arguments.projects_root.expanduser()
    sources: List[Dict[str, Any]] = []
    for name in [value.strip() for value in arguments.projects.split(",") if value.strip()]:
        project = stand.resolve_project(name, projects_root)
        if project is None:
            raise OutcomeError(f"no project {name} under {projects_root}")
        sources.extend(recall_sources(project, arguments.transcripts.expanduser(), after, before, exclude))
    sources.sort(key=lambda source: (source["project"], source["ts"]))
    stand.write_json(out, {
        "version": 1, "kind": "recall-sources",
        "created": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "before": arguments.before, "after": arguments.after, "sources": sources,
    })
    os.chmod(out, 0o600)
    print(f"{len(sources)} first turns that answered from files they read -> {out}")
    return 0


def recall_problem(source: Dict[str, Any], scenario: Dict[str, Any]) -> Optional[str]:
    """Why an authored scenario cannot run on its source, or None.

    The learning must be one `record-result` accepts and cite only files the
    source turn read; the needed files are among those it cites; the question
    names none of them, so an agent either recalls the save or searches.
    """
    learning = scenario.get("learning")
    if not isinstance(learning, dict) or set(learning) != {"type", "title", "consequence", "sources"}:
        return "learning needs exactly type, title, consequence and sources"
    if learning["type"] not in ("finding", "decision"):
        return "learning type is finding or decision"
    for key, limit in (("title", LEARNING_TITLE_LIMIT), ("consequence", LEARNING_CONSEQUENCE_LIMIT)):
        if not isinstance(learning[key], str) or not learning[key].strip() or len(learning[key]) > limit:
            return f"learning {key} is text within {limit} characters"
    cited = learning["sources"]
    if (not isinstance(cited, list) or not 1 <= len(cited) <= LEARNING_SOURCE_LIMIT
            or any(item not in source["read"] for item in cited)):
        return "learning sources are files the source turn read"
    needed = scenario.get("needed")
    if not isinstance(needed, list) or not needed or any(item not in cited for item in needed):
        return "needed files are among the learning's sources"
    question = scenario.get("prompt")
    if not isinstance(question, str) or not question.strip():
        return "the question is missing"
    if any(named_in(question, item) for item in needed):
        return "the question names a needed file"
    try:
        runtime()
        import memory_results  # noqa: E402 - the runtime put its folder on the path
        memory_results.normalize(ROOT, {"learnings": [learning], "verified": True}, check_files=False)
    except Exception as error:  # noqa: BLE001 - any refusal is the reason
        return f"record-result refuses the learning: {error}"
    return None


def command_recall_set(arguments: argparse.Namespace) -> int:
    out = arguments.out.expanduser()
    if stand.inside_clone(out):
        raise OutcomeError(f"--out {out} is inside this clone: a set carries client prompts")
    document = json.loads(arguments.sources.expanduser().read_text(encoding="utf-8"))
    sources = {item["id"]: item for item in document.get("sources") or []} if isinstance(document, dict) else {}
    authored = json.loads(arguments.scenarios.expanduser().read_text(encoding="utf-8"))
    scenarios = authored.get("scenarios") if isinstance(authored, dict) else authored
    if not sources or not isinstance(scenarios, list):
        raise OutcomeError("--sources and --scenarios must be made for this tool")
    valid: List[Dict[str, Any]] = []
    refused: Dict[str, str] = {}
    left_out = set(arguments.exclude_id.split(",")) if arguments.exclude_id else set()
    for scenario in scenarios:
        identity = str(scenario.get("id")) if isinstance(scenario, dict) else ""
        if identity in left_out:
            refused[identity] = "left out (--exclude-id)"
            continue
        if identity not in sources:
            refused[identity or "?"] = "no such source"
            continue
        if scenario.get("skip"):
            refused[identity] = f"skipped: {scenario['skip']}"
            continue
        problem = recall_problem(sources[identity], scenario)
        if problem:
            refused[identity] = problem
            continue
        source = sources[identity]
        moment = source["ts"]
        if arguments.regime == "later":
            # The same save and question, asked once other work has pushed
            # the subject out of the recent files.
            if not source.get("later"):
                refused[identity] = "no later session without the named files in its recent files"
                continue
            moment = source["later"]
        valid.append({
            "id": identity if arguments.regime == "next" else task_id(source["project"], moment),
            "source": identity, "regime": arguments.regime,
            "project": source["project"], "ts": moment, "session": source["session"],
            "prompt": scenario["prompt"].strip(), "gold": list(scenario["needed"]),
            "needed": list(scenario["needed"]),
            "saves": [{"learnings": [scenario["learning"]], "verified": True}],
        })
    valid.sort(key=lambda task: (task["project"], task["ts"]))
    chosen = valid if arguments.n >= len(valid) else random.Random(arguments.seed).sample(valid, arguments.n)
    chosen.sort(key=lambda task: (task["project"], task["ts"]))
    stand.write_json(out, {
        "version": 1, "kind": "recall", "regime": arguments.regime,
        "created": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "seed": arguments.seed, "candidates": len(valid), "refused": refused, "tasks": chosen,
    })
    os.chmod(out, 0o600)
    print(f"{len(chosen)} of {len(valid)} valid scenarios ({len(refused)} refused) -> {out}")
    return 0


# ---------------------------------------------------------------------------
# run: corpus, recent files, saves, one arm
# ---------------------------------------------------------------------------


def build_corpus(task: Dict[str, Any], corpus: Path, cache: Path, projects_root: Path,
                 editions: Dict[str, Any], timeout: float) -> Dict[str, Any]:
    """The project as it was at the prompt, with the edition under test."""
    moment = stand.parse_instant(task["ts"])
    project = stand.resolve_project(task["project"], projects_root)
    if moment is None or project is None:
        raise stand.Skip("no-project")
    name, _source = stand.detect_edition(project)
    edition = editions.setdefault(name, stand.Edition(name))
    repository = stand.Repository(project)
    commit = repository.commit_before(moment) if repository.prefix is not None else None
    tree = repository.tree(commit) if commit else None
    if tree is None:
        raise stand.Skip("no-history")
    shutil.copytree(stand.materialize(project, tree, cache), corpus)
    stand.reconstruct_memory(project, corpus, moment, "drop")
    config = PurePosixPath("project-brain/config/runtime.json")
    if not (corpus / config).is_file() and (project / config).is_file():
        stand.prepare_parent(corpus, corpus / config)
        shutil.copy2(project / config, corpus / config)
    stand.overlay(corpus, edition)
    env = stand.runtime_env(cache, moment, None)
    objects = repository.history(cache, commit)
    stand.commit_corpus(corpus, env, commit if objects else None, objects, f"ab/{stand.safe_name(task['id'])}")
    warm = stand.run_runtime(corpus, ["index"], env, timeout)
    if warm["exit"] != 0:
        raise stand.Skip("index-failed")
    return {"commit": commit, "edition": name, "project": project}


def seed_recent_files(task: Dict[str, Any], corpus: Path, project: Path, transcripts: Path) -> int:
    """Write the recent files the Stop hook would have kept before the prompt.

    Every human turn of the project's sessions before the prompt, oldest
    first, moves its changed and read files to the front, as each turn's
    flush does. A path that does not exist in the corpus is dropped - the
    capsule would not show it.
    """
    moment = stand.parse_instant(task["ts"])
    root = os.path.realpath(str(project))
    turns: List[Tuple[datetime, List[str]]] = []
    for path in session_files(transcripts, project):
        for turn in session_turns(path, root):
            when = stand.parse_instant(turn["ts"])
            if when is None or moment is None or when >= moment:
                continue
            files = [
                item for item in dict.fromkeys(turn["changed"] + turn["read"])
                if (corpus / item).is_file() and runtime()._recent_file_kept(item)
            ]
            if files:
                turns.append((when, files))
    turns.sort(key=lambda entry: entry[0])
    rt = runtime()
    connection = sqlite3.connect(str(corpus / "memory-bank" / "local" / "context.db"))
    try:
        for _when, files in turns[-SEEDED_TURNS:]:
            rt.record_recent_files(connection, files)
        shown = rt.load_recent_files(connection)
    finally:
        connection.close()
    return len(shown)


def corpus_branch(task: Dict[str, Any]) -> str:
    """The branch commit_corpus leaves the corpus on, which the prompt hook
    takes for the task ID."""
    return f"ab/{stand.safe_name(str(task['id']))}"


def saved_documents(corpus: Path) -> Set[str]:
    """What a save can add that retrieval delivers: chunks and dynamic records."""
    found: Set[str] = set()
    for folder in (corpus / "memory-bank" / "chunks", corpus / "project-brain" / "dynamic"):
        if folder.is_dir():
            found.update(path.relative_to(corpus).as_posix() for path in folder.rglob("*.md"))
    return found


def plant_saves(task: Dict[str, Any], corpus: Path, env: Dict[str, str], timeout: float,
                scratch: Path) -> Dict[str, Any]:
    """Record the task's saved results in the corpus, as the session before
    the question would have: on the branch's task, through `record-result`,
    with the project's own promotion settings, then committed and indexed so
    every arm starts from the same tree.

    The task gets the goal the runtime gives a task it provisions for a
    branch, so its working state says nothing the save does not.
    """
    saves = task.get("saves") or []
    if not saves:
        return {}
    branch = corpus_branch(task)
    before = saved_documents(corpus)
    started = stand.run_runtime(corpus, ["start", "--task-id", branch, "--goal",
                                         runtime().derive_goal(branch), "--json"], env, timeout)
    if started["exit"] != 0:
        raise stand.Skip("plant-start-failed")
    planted = {"records": 0, "promoted": 0}
    scratch.mkdir(parents=True, exist_ok=True)
    for number, save in enumerate(saves, start=1):
        request = scratch / f"{stand.safe_name(str(task['id']))}-save-{number}.json"
        request.write_text(json.dumps(save), encoding="utf-8")
        saved = stand.run_runtime(corpus, ["record-result", "--task-id", branch, "--result-id", f"save-{number}",
                                           "--revision", "auto", "--input", str(request), "--json"], env, timeout)
        request.unlink()
        if saved["exit"] != 0:
            raise stand.Skip("plant-save-failed")
        report = json.loads(saved["stdout"])
        planted["records"] += sum(1 for record in report.get("records") or [] if record.get("state") == "created")
        planted["promoted"] += len(((report.get("promotion") or {}).get("promoted")) or [])
    for arguments in (["add", "-A"], ["commit", "-q", "--no-verify", "-m", "memory: saved results"]):
        committed = subprocess.run(["git", "-c", "commit.gpgsign=false", "-c", "core.fsmonitor=false", *arguments],
                                   cwd=str(corpus), env=env, capture_output=True)
        if committed.returncode != 0:
            raise stand.Skip("plant-commit-failed")
    if stand.run_runtime(corpus, ["index"], env, timeout)["exit"] != 0:
        raise stand.Skip("index-failed")
    planted["documents"] = sorted(saved_documents(corpus) - before)
    return planted


def hook_arguments(corpus: Path) -> List[str]:
    """Codex session flags that run the corpus's memory prompt hook alone."""
    hooks = json.loads((corpus / ".codex" / "hooks.json").read_text(encoding="utf-8")).get("hooks") or {}
    groups = []
    for group in hooks.get("UserPromptSubmit") or []:
        handlers = [
            handler for handler in group.get("hooks") or []
            if isinstance(handler, dict) and str(handler.get("command") or "").rstrip().endswith(PROMPT_HOOK)
        ]
        if handlers:
            groups.append({**group, "hooks": handlers})
    if not groups:
        raise OutcomeError(f"{corpus}/.codex/hooks.json has no {PROMPT_HOOK} prompt hook")
    return ["--dangerously-bypass-hook-trust", "-c", "features.hooks=true",
            "-c", f"hooks.UserPromptSubmit={_toml(groups)}"]


def codex_command(codex: str, corpus: Path, arm: str, prompt: str, model: Optional[str]) -> List[str]:
    # The corpus is a disposable copy: the agent may write in it as it wrote
    # in the project, so it works the task instead of stopping at a sandbox
    # that refuses edits. Codex's workspace-write sandbox keeps the network off.
    command = [codex, "exec", "--json", "--ephemeral", "--ignore-user-config", "--skip-git-repo-check",
               "-s", "workspace-write", "-C", str(corpus)]
    if model:
        command += ["-m", model]
    command += hook_arguments(corpus) if arm in HOOKED_ARMS else ["-c", "features.hooks=false"]
    return [*command, prompt]


def event_paths(item: Dict[str, Any], corpus: str) -> List[str]:
    """The corpus files one Codex exec item names."""
    found: List[str] = []
    if item.get("type") == "command_execution" and isinstance(item.get("command"), str):
        script = unwrap_shell(item["command"])
        for word, _changed, directory in runtime()._shell_file_words(script, corpus, dialect="sh"):
            relative = project_path(str(word), str(directory), corpus)
            if relative is not None:
                found.append(relative)
    elif item.get("type") == "file_change":
        for change in item.get("changes") or []:
            if isinstance(change, dict) and isinstance(change.get("path"), str):
                relative = project_path(change["path"], corpus, corpus)
                if relative is not None:
                    found.append(relative)
    return found


class Counter:
    """Calls and the first gold touch, fed one exec event at a time.

    A tool item is one call, counted where it first appears - its start, or
    its completion when no start was emitted. Its paths are read from both
    events: a file change may name its path only once it completed.
    """

    def __init__(self, corpus: Path, gold: Sequence[str], cap: int) -> None:
        self.corpus = os.path.realpath(str(corpus))
        self.gold = set(gold)
        self.cap = cap
        self.calls = 0
        self.first_gold: Optional[int] = None
        self.numbers: Dict[str, int] = {}
        self.usage: Optional[Dict[str, Any]] = None
        self.finished = False
        self.message = ""

    def feed(self, event: Dict[str, Any]) -> Optional[str]:
        """'gold' or 'cap' when the run should stop now, else None."""
        kind = event.get("type")
        if kind == "turn.completed":
            self.usage = event.get("usage") if isinstance(event.get("usage"), dict) else None
            self.finished = True
            return None
        item = event.get("item") if isinstance(event.get("item"), dict) else {}
        if kind == "item.completed" and item.get("type") == "agent_message" and isinstance(item.get("text"), str):
            # The last one is the answer a finished turn gave.
            self.message = item["text"]
        if kind not in ("item.started", "item.completed") or item.get("type") not in TOOL_ITEMS:
            return None
        identity = str(item.get("id") or "")
        number = self.numbers.get(identity) if identity else None
        if number is None:
            self.calls += 1
            number = self.calls
            if identity:
                self.numbers[identity] = number
        if self.first_gold is None and self.gold & set(event_paths(item, self.corpus)):
            self.first_gold = number
            return "gold"
        if self.calls >= self.cap:
            return "cap"
        return None

    def answered(self) -> bool:
        """Whether the turn finished, without a gold touch, on an answer that
        names a gold file: what a recall run counts as reaching it."""
        return self.finished and self.first_gold is None and any(named_in(self.message, path) for path in self.gold)


def stop(process: subprocess.Popen) -> None:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except (OSError, AttributeError):
        process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except (OSError, AttributeError):
            process.kill()
        process.wait(timeout=10)


def health_lines(path: Path) -> int:
    try:
        with path.open("rb") as handle:
            return sum(1 for _ in handle)
    except OSError:
        return 0


def set_capsule_profile(corpus: Path, profile: str) -> None:
    """Write the capsule profile into a corpus's runtime.json."""
    config = corpus / "project-brain" / "config" / "runtime.json"
    try:
        values = json.loads(config.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        values = {}
    if not isinstance(values, dict):
        values = {}
    values["capsule_profile"] = profile
    config.parent.mkdir(parents=True, exist_ok=True)
    config.write_text(json.dumps(values, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run_arm(command: Sequence[str], corpus: Path, gold: Sequence[str], cap: int,
            timeout: float, events: Optional[Path] = None,
            saved: Optional[Sequence[str]] = None) -> Dict[str, Any]:
    """One headless session, stopped at the first gold touch, the cap or the
    deadline. Only counts leave this function.

    ``saved`` names the documents a recall set's save added; the outcome says
    whether a hook capsule delivered one."""
    manifests = corpus / "memory-bank" / "local" / "retrieval-manifests"
    before = set(manifests.glob("*.json")) if manifests.is_dir() else set()
    health = corpus / "memory-bank" / "local" / "refresh-health.ndjson"
    health_before = health_lines(health)
    counter = Counter(corpus, gold, cap)
    started = time.monotonic()
    reason = "finished"
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    process = subprocess.Popen(
        list(command), cwd=str(corpus), env=environment, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True,
    )
    assert process.stdout is not None
    # A session can go quiet for minutes while the model thinks; a reader
    # thread keeps the deadline enforceable without a blocking readline.
    lines: "queue.Queue[Optional[bytes]]" = queue.Queue()

    def pump() -> None:
        assert process.stdout is not None
        for raw in iter(process.stdout.readline, b""):
            lines.put(raw)
        lines.put(None)

    reader = threading.Thread(target=pump, daemon=True)
    reader.start()
    log = None
    if events is not None:
        events.parent.mkdir(parents=True, exist_ok=True)
        log = os.fdopen(os.open(str(events), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "wb")
    try:
        while True:
            remaining = timeout - (time.monotonic() - started)
            if remaining <= 0:
                reason = "timeout"
                break
            try:
                line = lines.get(timeout=min(remaining, 5.0))
            except queue.Empty:
                continue
            if line is None:
                break
            if log is not None:
                log.write(line)
            try:
                event = json.loads(line)
            except (UnicodeDecodeError, ValueError):
                continue
            if isinstance(event, dict):
                verdict = counter.feed(event)
                if verdict:
                    reason = verdict
                    break
    finally:
        if process.poll() is None:
            stop(process)
        reader.join(timeout=10)
        process.stdout.close()
        if log is not None:
            log.close()
    after = set(manifests.glob("*.json")) if manifests.is_dir() else set()
    if reason == "finished" and process.returncode not in (0, None) and not counter.finished:
        reason = "error"
    # The policy has agents run `context.py retrieve` themselves, in either
    # arm. The Codex prompt hook runs `refresh`; Claude's runs `hook-context`.
    entry_points: Dict[str, int] = {}
    capsule_tokens = 0
    saved_delivered = False
    for path in after - before:
        try:
            manifest = json.loads(path.read_text(encoding="utf-8"))
            point = str(manifest.get("entry_point"))
        except (OSError, ValueError, AttributeError):
            manifest, point = {}, "unreadable"
        entry_points[point] = entry_points.get(point, 0) + 1
        if point in HOOK_ENTRY_POINTS:
            estimates = manifest.get("token_estimates") if isinstance(manifest, dict) else None
            if isinstance(estimates, dict) and isinstance(estimates.get("total"), (int, float)):
                capsule_tokens += int(estimates["total"])
            selected = manifest.get("selected") if isinstance(manifest, dict) else None
            if saved and isinstance(selected, list):
                saved_delivered = saved_delivered or any(
                    isinstance(entry, dict) and entry.get("path") in saved for entry in selected)
    delivered = sum(count for point, count in entry_points.items() if point in HOOK_ENTRY_POINTS)
    return {
        "stopped": reason,
        "calls": counter.calls,
        "calls_to_gold": counter.first_gold,
        # A recall run reaches the save's subject by touching a needed file or
        # by answering with one named; calls before that answer.
        "calls_to_answer": counter.calls if counter.answered() else None,
        "saved_delivered": saved_delivered,
        "seconds": round(time.monotonic() - started, 1),
        "usage": counter.usage,
        "capsule_delivered": delivered > 0,
        # Every prompt-hook refresh appends one health line, with or without
        # documents: the navigation profile writes no manifest but this.
        "hook_ran": health_lines(health) > health_before,
        "capsule_tokens": capsule_tokens,
        "manifests": dict(sorted(entry_points.items())),
        "exit": process.returncode,
    }


def command_run(arguments: argparse.Namespace) -> int:
    out = arguments.out.expanduser()
    if stand.inside_clone(out):
        raise OutcomeError(f"--out {out} is inside this clone: results name client paths")
    if arguments.events and stand.inside_clone(arguments.events):
        raise OutcomeError(f"--events {arguments.events} is inside this clone: events carry client prompts")
    if shutil.which(arguments.codex) is None:
        raise OutcomeError(f"{arguments.codex} is not on PATH; the runs need a signed-in Codex CLI")
    document = json.loads(arguments.set.expanduser().read_text(encoding="utf-8"))
    tasks = document.get("tasks") if isinstance(document, dict) else None
    if not isinstance(tasks, list):
        raise OutcomeError("--set is not a sample made by this tool")
    kind = "recall" if document.get("kind") == "recall" else "cold-start"
    wanted = set(arguments.ids.split(",")) if arguments.ids else None
    arms = [arm.strip() for arm in arguments.arms.split(",") if arm.strip()]
    if not arms or any(arm not in ARMS for arm in arms):
        raise OutcomeError(f"--arms must name some of {', '.join(ARMS)}")
    cache = (arguments.cache or (stand.default_cache().parent / f"{stand.default_cache().name}-outcome")).expanduser().resolve()
    stand.prepare_cache(cache)
    work = cache / "runs" / f"{time.strftime('%Y%m%dT%H%M%S')}-{os.getpid()}"
    work.mkdir(parents=True)
    projects_root = arguments.projects_root.expanduser()
    transcripts = arguments.transcripts.expanduser()
    editions: Dict[str, Any] = {}
    result: Dict[str, Any] = {
        "version": 1,
        "meta": {
            "started": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "set": str(arguments.set), "arms": arms, "cap": arguments.cap,
            "timeout": arguments.timeout, "model": arguments.model, "repeat": arguments.repeat,
            "codex": subprocess.run([arguments.codex, "--version"], capture_output=True, text=True).stdout.strip(),
            "runtime": stand.read_git(ROOT, "rev-parse", "HEAD"),
            "kind": kind, "metric": "recall" if kind == "recall" else "gold",
        },
        "items": {},
    }
    for index, task in enumerate(tasks):
        if wanted is not None and task.get("id") not in wanted:
            continue
        item: Dict[str, Any] = {"project": task.get("project"), "ts": task.get("ts"),
                                "gold": len(task.get("gold") or []), "arms": {}}
        base = work / f"{stand.safe_name(str(task.get('id')))}-base"
        try:
            built = build_corpus(task, base, cache, projects_root, editions, arguments.timeout)
            item.update(commit=built["commit"], edition=built["edition"])
            item["recent_files"] = seed_recent_files(task, base, built["project"], transcripts)
            planted = plant_saves(task, base, stand.runtime_env(cache, stand.parse_instant(task["ts"]), None),
                                  arguments.timeout, work / "saves")
            if planted:
                item["planted"] = {key: len(value) if isinstance(value, list) else value
                                   for key, value in planted.items()}
            # Alternate which arm goes first, so a time-of-day effect on the
            # model cannot favour one arm.
            for replicate in range(arguments.repeat):
                order = arms if (index + replicate) % 2 == 0 else list(reversed(arms))
                for arm in order:
                    corpus = work / f"{stand.safe_name(str(task.get('id')))}-{arm}-{replicate}"
                    shutil.copytree(base, corpus, symlinks=True)
                    if arm in PROFILE_ARMS:
                        set_capsule_profile(corpus, arm)
                    try:
                        outcome = run_arm(
                            codex_command(arguments.codex, corpus, arm, task["prompt"], arguments.model),
                            corpus, task.get("gold") or [], arguments.cap, arguments.timeout,
                            events=(arguments.events.expanduser() / f"{task.get('id')}-{arm}-{replicate}.jsonl"
                                    if arguments.events else None),
                            saved=planted.get("documents") if planted else None,
                        )
                    finally:
                        if not arguments.keep:
                            stand.remove_tree(corpus)
                    item["arms"].setdefault(arm, []).append(outcome)
            item["status"] = "ok"
        except stand.Skip as skip:
            item["status"] = f"skipped:{skip}"
        finally:
            if not arguments.keep and base.exists():
                stand.remove_tree(base)
        result["items"][str(task.get("id"))] = item
        stand.write_json(out, result)
        os.chmod(out, 0o600)
        arms_done = ", ".join(
            f"{arm} " + "/".join(
                str(run['calls_to_gold']) if run['calls_to_gold'] is not None
                else f"answer@{run['calls_to_answer']}" if run.get('calls_to_answer') is not None
                else run['stopped']
                for run in runs
            )
            for arm, runs in item["arms"].items()
        )
        print(f"{task.get('id')}: {item['status']} {arms_done}", flush=True)
    result["meta"]["finished"] = datetime.now(timezone.utc).isoformat(timespec="seconds")
    stand.write_json(out, result)
    os.chmod(out, 0o600)
    return 0


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------


def reached(arm: Dict[str, Any], metric: str = "gold") -> Optional[int]:
    """Calls before the run reached what its metric counts, or None.

    `gold`: the first gold touch. `recall`: the first touch of a needed file,
    or - a run that finished without one - an answer naming one.
    """
    value = arm.get("calls_to_gold")
    if not isinstance(value, int) and metric == "recall":
        value = arm.get("calls_to_answer")
    return value if isinstance(value, int) else None


def censored(arm: Dict[str, Any], cap: int, metric: str = "gold") -> int:
    """Calls to what the metric counts; a run that never reached it counts
    one past the cap, whatever stopped it."""
    value = reached(arm, metric)
    return int(value) if value is not None else cap + 1


def bootstrap(values: Sequence[float], rounds: int = 10000, seed: int = 20261021) -> Tuple[float, float]:
    generator = random.Random(seed)
    count = len(values)
    means = sorted(
        sum(values[generator.randrange(count)] for _ in range(count)) / count
        for _ in range(rounds)
    )
    return means[int(0.025 * rounds)], means[int(0.975 * rounds) - 1]


def sign_test(differences: Sequence[float]) -> float:
    """Exact two-sided sign test on the non-zero differences."""
    positive = sum(1 for value in differences if value > 0)
    negative = sum(1 for value in differences if value < 0)
    total = positive + negative
    if not total:
        return 1.0
    tail = sum(math.comb(total, k) for k in range(0, min(positive, negative) + 1)) / 2 ** total
    return min(1.0, 2 * tail)


def valid_arm(arm: str, runs: Sequence[Dict[str, Any]]) -> bool:
    """Whether an arm got the treatment it names, in every repeat.

    A result written before `hook_ran` existed only knows deliveries.
    """
    delivered = [bool(run.get("capsule_delivered")) for run in runs]
    ran = [run.get("hook_ran") for run in runs]
    if arm == "capsule":
        return all(delivered)
    if arm == "navigation":
        return all(value is True for value in ran) and not any(delivered)
    if arm == "recall":
        # A manifest only on a turn one of the task's saves matched.
        return all(value is True for value in ran)
    return not any(delivered) and not any(value is True for value in ran)


def summarize(result: Dict[str, Any], compare: Tuple[str, str] = DEFAULT_ARMS,
              metric: Optional[str] = None) -> Dict[str, Any]:
    """Paired calls to the first gold file - or, for the recall metric, to the
    save's subject - treatment `compare[1]` against baseline `compare[0]`."""
    baseline, treatment = compare
    cap = int(result.get("meta", {}).get("cap") or DEFAULT_CAP)
    metric = metric or str(result.get("meta", {}).get("metric") or "gold")
    pairs = []
    for task_key, item in sorted((result.get("items") or {}).items()):
        arms = {arm: runs if isinstance(runs, list) else [runs] for arm, runs in (item.get("arms") or {}).items()}
        if item.get("status") != "ok" or not all(arms.get(arm) for arm in compare):
            continue
        if any(run.get("stopped") == "error" for arm in compare for run in arms[arm]):
            continue
        if not all(valid_arm(arm, arms[arm]) for arm in compare):
            continue
        pairs.append({
            "id": task_key,
            baseline: sum(censored(run, cap, metric) for run in arms[baseline]) / len(arms[baseline]),
            treatment: sum(censored(run, cap, metric) for run in arms[treatment]) / len(arms[treatment]),
            f"{baseline}_touched": all(reached(run, metric) is not None for run in arms[baseline]),
            f"{treatment}_touched": all(reached(run, metric) is not None for run in arms[treatment]),
            f"{baseline}_saved": any(run.get("saved_delivered") for run in arms[baseline]),
            f"{treatment}_saved": any(run.get("saved_delivered") for run in arms[treatment]),
        })
    differences = [pair[treatment] - pair[baseline] for pair in pairs]
    summary: Dict[str, Any] = {
        "compare": [baseline, treatment],
        "metric": metric,
        "tasks": len(result.get("items") or {}),
        "pairs": len(pairs),
        "cap": cap,
        f"mean_{baseline}": round(sum(p[baseline] for p in pairs) / len(pairs), 2) if pairs else None,
        f"mean_{treatment}": round(sum(p[treatment] for p in pairs) / len(pairs), 2) if pairs else None,
        "mean_difference": round(sum(differences) / len(differences), 2) if differences else None,
        "ci95": [round(value, 2) for value in bootstrap(differences)] if len(differences) >= 2 else None,
        "sign_test_p": round(sign_test(differences), 4) if differences else None,
        f"{treatment}_fewer": sum(1 for value in differences if value < 0),
        f"{treatment}_more": sum(1 for value in differences if value > 0),
        f"touched_{baseline}": sum(1 for p in pairs if p[f"{baseline}_touched"]),
        f"touched_{treatment}": sum(1 for p in pairs if p[f"{treatment}_touched"]),
    }
    if metric == "recall":
        # How often a hook capsule handed the save over, in any repeat: the
        # mechanism the arms differ by. Not part of the verdict.
        summary[f"saved_delivered_{baseline}"] = sum(1 for p in pairs if p[f"{baseline}_saved"])
        summary[f"saved_delivered_{treatment}"] = sum(1 for p in pairs if p[f"{treatment}_saved"])
    summary["verdict"] = verdict(summary)
    return summary


def verdict(summary: Dict[str, Any]) -> str:
    """The rule written down before the runs (docs/MEMORY-EVAL.md)."""
    baseline, treatment = summary.get("compare") or DEFAULT_ARMS
    interval = summary.get("ci95")
    if not interval or summary["pairs"] < 20:
        return "insufficient"
    if interval[1] < 0 and summary[f"touched_{treatment}"] >= summary[f"touched_{baseline}"] - 1:
        return f"{treatment}-helps"
    if interval[0] > 0:
        return f"{treatment}-hurts"
    return "no-difference-shown"


def command_report(arguments: argparse.Namespace) -> int:
    result = json.loads(arguments.result.expanduser().read_text(encoding="utf-8"))
    compare = tuple(value.strip() for value in arguments.compare.split(",") if value.strip())
    if len(compare) != 2 or any(arm not in ARMS for arm in compare) or compare[0] == compare[1]:
        raise OutcomeError(f"--compare takes two different arms of {', '.join(ARMS)}")
    baseline, treatment = compare
    if arguments.metric is not None and arguments.metric not in METRICS:
        raise OutcomeError(f"--metric is one of {', '.join(METRICS)}")
    summary = summarize(result, compare, arguments.metric)
    if arguments.json:
        print(json.dumps(summary, indent=1))
        return 0
    target = "the save's subject (needed file or an answer naming it)" if summary["metric"] == "recall" else "the first gold file"
    print(f"pairs {summary['pairs']} of {summary['tasks']} tasks; a run that never reached it counts {summary['cap'] + 1}")
    if summary["pairs"]:
        print(f"calls to {target}: {baseline} {summary[f'mean_{baseline}']}, "
              f"{treatment} {summary[f'mean_{treatment}']}")
        print(f"{treatment} - {baseline}: {summary['mean_difference']} (95% CI {summary['ci95']}), "
              f"fewer {summary[f'{treatment}_fewer']} / more {summary[f'{treatment}_more']}, "
              f"sign test p {summary['sign_test_p']}")
        print(f"reached within the cap: {baseline} {summary[f'touched_{baseline}']}, "
              f"{treatment} {summary[f'touched_{treatment}']}")
        if summary["metric"] == "recall":
            print(f"save delivered by the hook: {baseline} {summary[f'saved_delivered_{baseline}']}, "
                  f"{treatment} {summary[f'saved_delivered_{treatment}']}")
    print(f"verdict: {summary['verdict']}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    sample = commands.add_parser("sample", help="cold starts that changed files, from Claude Code transcripts")
    sample.add_argument("--projects", required=True, help="comma-separated project folder names")
    sample.add_argument("--projects-root", type=Path, default=Path.home() / "Desktop")
    sample.add_argument("--transcripts", type=Path, default=Path.home() / ".claude" / "projects")
    sample.add_argument("--before", required=True)
    sample.add_argument("--after")
    sample.add_argument("--exclude-set", action="append", help="a memory_eval set whose prompts are left out")
    sample.add_argument("--n", type=int, required=True)
    sample.add_argument("--seed", type=int, required=True)
    sample.add_argument("--out", type=Path, required=True)
    sources = commands.add_parser("recall-sources",
                                  help="first turns that answered from files they read: what a session would save")
    sources.add_argument("--projects", required=True, help="comma-separated project folder names")
    sources.add_argument("--projects-root", type=Path, default=Path.home() / "Desktop")
    sources.add_argument("--transcripts", type=Path, default=Path.home() / ".claude" / "projects")
    sources.add_argument("--before", required=True)
    sources.add_argument("--after")
    sources.add_argument("--exclude-set", action="append", help="a set whose sessions are left out")
    sources.add_argument("--out", type=Path, required=True)
    recall = commands.add_parser("recall-set", help="check authored recall scenarios against their sources and sample them")
    recall.add_argument("--sources", type=Path, required=True)
    recall.add_argument("--scenarios", type=Path, required=True,
                        help="JSON list of {id, learning, needed, prompt} or {id, skip}")
    recall.add_argument("--regime", choices=("next", "later"), default="next",
                        help="ask a minute after the source turn, or at the first later session start "
                             "whose recent files no longer show the files the answer named")
    recall.add_argument("--exclude-id", help="comma-separated source IDs to leave out (a pilot)")
    recall.add_argument("--n", type=int, required=True)
    recall.add_argument("--seed", type=int, required=True)
    recall.add_argument("--out", type=Path, required=True)
    run = commands.add_parser("run", help="each task with and without the capsule")
    run.add_argument("--set", type=Path, required=True)
    run.add_argument("--out", type=Path, required=True)
    run.add_argument("--projects-root", type=Path, default=Path.home() / "Desktop")
    run.add_argument("--transcripts", type=Path, default=Path.home() / ".claude" / "projects")
    run.add_argument("--cache", type=Path)
    run.add_argument("--arms", default=",".join(DEFAULT_ARMS), help=f"some of {', '.join(ARMS)}")
    run.add_argument("--ids")
    run.add_argument("--cap", type=int, default=DEFAULT_CAP)
    run.add_argument("--repeat", type=int, default=1, help="runs of each arm per task")
    run.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT)
    run.add_argument("--model")
    run.add_argument("--codex", default="codex")
    run.add_argument("--keep", action="store_true", help="keep the corpora for inspection")
    run.add_argument("--events", type=Path,
                     help="pilot only: keep each run's raw Codex events (prompts and commands) in this private folder")
    report = commands.add_parser("report", help="paired calls to the first gold file")
    report.add_argument("result", type=Path)
    report.add_argument("--json", action="store_true")
    report.add_argument("--compare", default=",".join(DEFAULT_ARMS),
                        help="baseline,treatment: two arms of the result")
    report.add_argument("--metric", help=f"one of {', '.join(METRICS)}; the result's own by default")
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    arguments = build_parser().parse_args(argv)
    try:
        if arguments.command == "sample":
            return command_sample(arguments)
        if arguments.command == "recall-sources":
            return command_recall_sources(arguments)
        if arguments.command == "recall-set":
            return command_recall_set(arguments)
        if arguments.command == "run":
            return command_run(arguments)
        return command_report(arguments)
    except (OutcomeError, stand.EvalError) as error:
        print(f"memory_outcome_ab: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
