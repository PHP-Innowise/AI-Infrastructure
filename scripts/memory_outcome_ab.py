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

Maintainer-only and standard library only; never installed into a project.
Sets and results name client paths and carry prompts: keep them outside this
repository (the tool refuses an output inside the clone). The agent runs in
Codex's read-only sandbox with `--ephemeral`, so neither the corpus nor
`~/.codex/sessions` keeps what it did.

    memory_outcome_ab.py sample --projects next,scb --before 2026-10-07 \\
        --n 30 --seed 1 --exclude-set tuning-set.json --out ~/ab/set.json
    memory_outcome_ab.py run --set ~/ab/set.json --out ~/ab/result.json
    memory_outcome_ab.py report ~/ab/result.json

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

ARMS = ("off", "capsule")
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
    moments: Dict[str, List[datetime]] = {}
    for path in paths:
        try:
            items = stand.load_set(path)
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
# run: corpus, recent files, one arm
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
    command += hook_arguments(corpus) if arm == "capsule" else ["-c", "features.hooks=false"]
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

    def feed(self, event: Dict[str, Any]) -> Optional[str]:
        """'gold' or 'cap' when the run should stop now, else None."""
        kind = event.get("type")
        if kind == "turn.completed":
            self.usage = event.get("usage") if isinstance(event.get("usage"), dict) else None
            self.finished = True
            return None
        item = event.get("item") if isinstance(event.get("item"), dict) else {}
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


def run_arm(command: Sequence[str], corpus: Path, gold: Sequence[str], cap: int,
            timeout: float, events: Optional[Path] = None) -> Dict[str, Any]:
    """One headless session, stopped at the first gold touch, the cap or the
    deadline. Only counts leave this function."""
    manifests = corpus / "memory-bank" / "local" / "retrieval-manifests"
    before = set(manifests.glob("*.json")) if manifests.is_dir() else set()
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
    for path in after - before:
        try:
            point = str(json.loads(path.read_text(encoding="utf-8")).get("entry_point"))
        except (OSError, ValueError, AttributeError):
            point = "unreadable"
        entry_points[point] = entry_points.get(point, 0) + 1
    delivered = sum(count for point, count in entry_points.items() if point in HOOK_ENTRY_POINTS)
    return {
        "stopped": reason,
        "calls": counter.calls,
        "calls_to_gold": counter.first_gold,
        "seconds": round(time.monotonic() - started, 1),
        "usage": counter.usage,
        "capsule_delivered": delivered > 0,
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
            # Alternate which arm goes first, so a time-of-day effect on the
            # model cannot favour one arm.
            for replicate in range(arguments.repeat):
                order = arms if (index + replicate) % 2 == 0 else list(reversed(arms))
                for arm in order:
                    corpus = work / f"{stand.safe_name(str(task.get('id')))}-{arm}-{replicate}"
                    shutil.copytree(base, corpus, symlinks=True)
                    try:
                        outcome = run_arm(
                            codex_command(arguments.codex, corpus, arm, task["prompt"], arguments.model),
                            corpus, task.get("gold") or [], arguments.cap, arguments.timeout,
                            events=(arguments.events.expanduser() / f"{task.get('id')}-{arm}-{replicate}.jsonl"
                                    if arguments.events else None),
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
                str(run['calls_to_gold']) if run['calls_to_gold'] is not None else run['stopped']
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


def censored(arm: Dict[str, Any], cap: int) -> int:
    """Calls to the first gold touch; a run that never touched one counts
    one past the cap, whatever stopped it."""
    value = arm.get("calls_to_gold")
    return int(value) if isinstance(value, int) else cap + 1


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


def summarize(result: Dict[str, Any]) -> Dict[str, Any]:
    cap = int(result.get("meta", {}).get("cap") or DEFAULT_CAP)
    pairs = []
    for task_key, item in sorted((result.get("items") or {}).items()):
        arms = {arm: runs if isinstance(runs, list) else [runs] for arm, runs in (item.get("arms") or {}).items()}
        if item.get("status") != "ok" or not all(arms.get(arm) for arm in ARMS):
            continue
        if any(run.get("stopped") == "error" for arm in ARMS for run in arms[arm]):
            continue
        if not all(run.get("capsule_delivered") for run in arms["capsule"]):
            # The capsule arm is only a capsule arm when the hook delivered.
            continue
        if any(run.get("capsule_delivered") for run in arms["off"]):
            continue
        pairs.append({
            "id": task_key,
            "off": sum(censored(run, cap) for run in arms["off"]) / len(arms["off"]),
            "capsule": sum(censored(run, cap) for run in arms["capsule"]) / len(arms["capsule"]),
            "off_touched": all(run.get("calls_to_gold") is not None for run in arms["off"]),
            "capsule_touched": all(run.get("calls_to_gold") is not None for run in arms["capsule"]),
        })
    differences = [pair["capsule"] - pair["off"] for pair in pairs]
    summary: Dict[str, Any] = {
        "tasks": len(result.get("items") or {}),
        "pairs": len(pairs),
        "cap": cap,
        "mean_off": round(sum(p["off"] for p in pairs) / len(pairs), 2) if pairs else None,
        "mean_capsule": round(sum(p["capsule"] for p in pairs) / len(pairs), 2) if pairs else None,
        "mean_difference": round(sum(differences) / len(differences), 2) if differences else None,
        "ci95": [round(value, 2) for value in bootstrap(differences)] if len(differences) >= 2 else None,
        "sign_test_p": round(sign_test(differences), 4) if differences else None,
        "capsule_fewer": sum(1 for value in differences if value < 0),
        "capsule_more": sum(1 for value in differences if value > 0),
        "touched_off": sum(1 for p in pairs if p["off_touched"]),
        "touched_capsule": sum(1 for p in pairs if p["capsule_touched"]),
    }
    summary["verdict"] = verdict(summary)
    return summary


def verdict(summary: Dict[str, Any]) -> str:
    """The rule written down before the runs (docs/MEMORY-EVAL.md)."""
    interval = summary.get("ci95")
    if not interval or summary["pairs"] < 20:
        return "insufficient"
    if interval[1] < 0 and summary["touched_capsule"] >= summary["touched_off"] - 1:
        return "capsule-helps"
    if interval[0] > 0:
        return "capsule-hurts"
    return "no-difference-shown"


def command_report(arguments: argparse.Namespace) -> int:
    result = json.loads(arguments.result.expanduser().read_text(encoding="utf-8"))
    summary = summarize(result)
    if arguments.json:
        print(json.dumps(summary, indent=1))
        return 0
    print(f"pairs {summary['pairs']} of {summary['tasks']} tasks; a run without a gold touch counts {summary['cap'] + 1}")
    if summary["pairs"]:
        print(f"calls to the first gold file: off {summary['mean_off']}, capsule {summary['mean_capsule']}")
        print(f"capsule - off: {summary['mean_difference']} (95% CI {summary['ci95']}), "
              f"fewer {summary['capsule_fewer']} / more {summary['capsule_more']}, sign test p {summary['sign_test_p']}")
        print(f"gold touched within the cap: off {summary['touched_off']}, capsule {summary['touched_capsule']}")
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
    run = commands.add_parser("run", help="each task with and without the capsule")
    run.add_argument("--set", type=Path, required=True)
    run.add_argument("--out", type=Path, required=True)
    run.add_argument("--projects-root", type=Path, default=Path.home() / "Desktop")
    run.add_argument("--transcripts", type=Path, default=Path.home() / ".claude" / "projects")
    run.add_argument("--cache", type=Path)
    run.add_argument("--arms", default=",".join(ARMS))
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
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    arguments = build_parser().parse_args(argv)
    try:
        if arguments.command == "sample":
            return command_sample(arguments)
        if arguments.command == "run":
            return command_run(arguments)
        return command_report(arguments)
    except (OutcomeError, stand.EvalError) as error:
        print(f"memory_outcome_ab: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
