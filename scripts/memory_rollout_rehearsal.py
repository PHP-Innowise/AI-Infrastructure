#!/usr/bin/env python3
"""Rehearse bringing a project's installed accelerator up to this clone, on a copy.

`install_accelerator.py --sync` updates an installed project in place. A hook
that broke, a save the runtime now refuses or a capsule that stopped recalling
showed only in the first real session after it, in the client's project. This
tool copies the project first - its Git history and every file except
dependency and worktree trees - syncs the copy from this clone and runs the
installed hooks themselves, as Claude Code, Codex or Cursor would on one turn:

1. the session-start and prompt hooks, with a fixed prompt (the capsule);
2. the Stop hook, which asks the agent to save its result;
3. the save, made with the exact command the request names, carrying one
   learning that cites a file the project's Git tracks;
4. the closing Stop;
5. a new session's prompt, whose capsule must recall that learning.

No model runs and nothing is written to the project: every host and capsule
profile gets its own copy, HOME and TMPDIR point into the work directory, and
the copies are removed unless --keep. `config` rehearses the profile the
project's own `runtime.json` selects - `--sync` never changes that file, so
it is what the project will run - and any other profile is forced through
`CONTEXT_CAPSULE_PROFILE`.

The report names what the sync changed and kept, the profile the project
selects, each hook's exit and time, whether the save was requested, saved or
refused (with its code), the native journal's lines and whether the next
capsule recalled the save.

Maintainer-only and standard library only; never installed into a project.
The report names the project's paths; write it outside this clone.

    memory_rollout_rehearsal.py --target ~/Desktop/next --target ~/Desktop/scb
    memory_rollout_rehearsal.py --target ~/Desktop/next --hosts claude --profiles config,recall --json

Exit status: 0 when every rehearsal saved and recalled with every hook
exiting 0, 1 otherwise, 2 for a target this tool will not copy.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path
from typing import Any, Dict, List, Optional

ROOT = Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "scripts" / "install_accelerator.py"
# Dependency, build-state and worktree trees: large, never read by the
# memory hooks, and a nested worktree is another checkout's state. A bare
# name is left out at any depth; a path, from the project's root only.
DEFAULT_EXCLUDES = ("node_modules", "vendor", "var", ".worktrees", ".claude/worktrees")
WIRING = {"claude": ".claude/settings.json", "codex": ".codex/hooks.json", "cursor": ".cursor/hooks.json"}
# Each host's names for session start, prompt and Stop. Cursor reads its
# capsule from a rule file the prompt hook renders, and its Stop request comes
# back as `followup_message`.
EVENTS = {
    "claude": ("SessionStart", "UserPromptSubmit", "Stop"),
    "codex": ("SessionStart", "UserPromptSubmit", "Stop"),
    "cursor": ("sessionStart", "beforeSubmitPrompt", "stop"),
}
CURSOR_RULE = ".cursor/rules/working-memory.mdc"
CURSOR_VERSION = "1.7.36"
PROFILES = ("config", "full", "navigation", "recall")
# The command the Stop request names, verbatim: Claude Code's
# `cd ROOT && echo -E '<JSON>' | python3 ... --json` takes the JSON in
# place of <JSON>; Codex's and Cursor's `(cd ROOT && python3 ... --json)` on stdin.
SAVE_COMMAND = re.compile(r"exact command: (\(cd .*?--json\)|cd .*?--json)\n")
REFUSAL = re.compile(r"\b(memory-result-v1:[a-z_]+|native-result:[a-z-]+)")
# A learning cites a file of the project, not the accelerator's own.
ACCELERATOR_PREFIXES = (".agents/", ".claude/", ".codex/", ".cursor/", "memory-bank/", "project-brain/",
                        "specs/", "tasks/")
SOURCE_SUFFIXES = (".php", ".json", ".ts", ".js", ".py", ".yaml", ".yml", ".xml", ".twig", ".vue", ".go")
PREFERRED_SOURCES = ("composer.json", "package.json")
PROMPT = "Rehearsal: where does this project keep its main configuration?"


def inside(path: Path, parent: Path) -> bool:
    try:
        path.resolve().relative_to(parent.resolve())
    except ValueError:
        return False
    return True


def copy_tree(source: Path, destination: Path, excludes: List[str]) -> None:
    """Copy `source` without the excluded trees: a bare name at any depth, a
    path from the root only. Links are copied as links, never followed, and a
    link that leads out of the project is left out: through it a hook in the
    copy would write where the project's own link points."""
    anywhere = {item for item in excludes if "/" not in item.strip("/")}
    rooted = {item.strip("/") for item in excludes if "/" in item.strip("/")}
    real = source.resolve()

    def outside(path: Path) -> bool:
        if not path.is_symlink():
            return False
        try:
            path.resolve().relative_to(real)
        except (OSError, ValueError, RuntimeError):
            return True
        return False

    def ignore(directory: str, names: List[str]) -> set:
        relative = Path(directory).relative_to(source).as_posix()
        prefix = "" if relative == "." else relative + "/"
        return {name for name in names
                if name in anywhere or prefix + name in rooted or outside(Path(directory, name))}

    shutil.copytree(source, destination, symlinks=True, ignore=ignore)


def child_env(home: Path, temporary: Path, extra: Dict[str, str]) -> Dict[str, str]:
    """A clean environment: no GIT_*, CONTEXT_* or ACCELERATOR_* from the caller,
    and no bytecode written beside this clone's scripts by the sync it runs."""
    env = {"HOME": str(home), "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
           "LANG": os.environ.get("LANG", "C.UTF-8"), "TMPDIR": str(temporary),
           "PYTHONDONTWRITEBYTECODE": "1", "GIT_OPTIONAL_LOCKS": "0"}
    if os.environ.get("SYSTEMROOT"):
        env["SYSTEMROOT"] = os.environ["SYSTEMROOT"]
    env.update(extra)
    return env


def prepare_home(home: Path) -> None:
    home.mkdir(parents=True, exist_ok=True)
    (home / ".gitconfig").write_text(
        "[user]\n\tname = Rollout rehearsal\n\temail = rehearsal@example.invalid\n"
        "[safe]\n\tdirectory = *\n", encoding="utf-8")


def sync(copy: Path, env: Dict[str, str], timeout: float) -> Dict[str, Any]:
    done = subprocess.run([sys.executable, str(INSTALLER), "--sync", "--target", str(copy)], cwd=ROOT, env=env,
                          capture_output=True, text=True, timeout=timeout)
    try:
        report = json.loads(done.stdout)
    except ValueError:
        return {"error": (done.stderr or done.stdout).strip()[-600:] or f"exit {done.returncode}"}
    return {"changed": len(report.get("changed") or []), "kept": len(report.get("kept") or []),
            "partial": report.get("partial"), "error": report.get("error"),
            "kept_paths": [item.get("path") for item in report.get("kept") or []]}


def configured_profile(copy: Path) -> str:
    try:
        config = json.loads((copy / "project-brain/config/runtime.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return "full (no runtime.json)"
    value = config.get("capsule_profile") if isinstance(config, dict) else None
    return str(value) if value else "full (runtime default)"


def cited_source(copy: Path, env: Dict[str, str]) -> Optional[str]:
    """A tracked project file a learning may cite."""
    done = subprocess.run(["git", "-C", str(copy), "ls-files", "-z"], env=env, capture_output=True, timeout=120)
    names = [name for name in done.stdout.decode("utf-8", "replace").split("\0") if name]
    candidates = [name for name in names
                  if not name.startswith(ACCELERATOR_PREFIXES) and name.endswith(SOURCE_SUFFIXES)
                  and "/." not in "/" + name and (copy / name).is_file() and not (copy / name).is_symlink()]
    for preferred in PREFERRED_SOURCES:
        if preferred in candidates:
            return preferred
    return candidates[0] if candidates else None


def hooks(copy: Path, host: str, event: str) -> List[str]:
    wiring = json.loads((copy / WIRING[host]).read_text(encoding="utf-8"))
    entries = wiring.get("hooks", {}).get(event, [])
    if host == "cursor":
        # Cursor lists each command directly under its event.
        return [entry["command"] for entry in entries if isinstance(entry.get("command"), str)]
    return [hook["command"] for group in entries
            for hook in group.get("hooks", []) if isinstance(hook.get("command"), str)]


def run_event(copy: Path, host: str, event: str, payload: Dict[str, Any], env: Dict[str, str],
              timeout: float) -> List[Dict[str, Any]]:
    results = []
    for command in hooks(copy, host, event):
        started = time.monotonic()
        try:
            done = subprocess.run(["bash", "-c", command], input=json.dumps(payload).encode("utf-8"), cwd=copy,
                                  env=env, capture_output=True, timeout=timeout)
            code, stdout, stderr = done.returncode, done.stdout.decode("utf-8", "replace"), \
                done.stderr.decode("utf-8", "replace")
        except subprocess.TimeoutExpired:
            code, stdout, stderr = "timeout", "", ""
        name = re.findall(r"([\w.-]+\.sh)", command)
        results.append({"hook": name[-1] if name else command[:40], "exit": code,
                        "ms": round((time.monotonic() - started) * 1000), "stdout": stdout,
                        "stderr": stderr.strip()[-400:]})
    return results


def capsule_text(copy: Path, host: str, results: List[Dict[str, Any]]) -> str:
    """What the prompt hook gave the model: Cursor's rule file, else stdout."""
    if host == "cursor":
        try:
            return (copy / CURSOR_RULE).read_text(encoding="utf-8")
        except OSError:
            return ""
    text = "".join(item["stdout"] for item in results)
    try:
        return json.loads(text)["hookSpecificOutput"]["additionalContext"]
    except (ValueError, KeyError, TypeError):
        return text


def journal(copy: Path) -> List[str]:
    path = copy / "memory-bank/local/native-results.ndjson"
    try:
        return path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return []


def rehearse(copy: Path, host: str, profile: str, work: Path, timeout: float) -> Dict[str, Any]:
    home, temporary = work / "home", work / "tmp"
    extra = {"CLAUDE_PROJECT_DIR": str(copy)} if host == "claude" else {}
    if profile != "config":
        extra["CONTEXT_CAPSULE_PROFILE"] = profile
    env = child_env(home, temporary, extra)
    marker = "rehearsal" + secrets.token_hex(4)
    source = cited_source(copy, env)
    before = len(journal(copy))
    steps: Dict[str, List[Dict[str, Any]]] = {}

    started_event, prompt_event, stop_event = EVENTS[host]

    def call(session: str, event: str, **fields: Any) -> Dict[str, Any]:
        if host == "cursor":
            payload: Dict[str, Any] = {"conversation_id": session, "generation_id": str(uuid.uuid4()),
                                       "workspace_roots": [str(copy)], "cursor_version": CURSOR_VERSION}
        else:
            payload = {"session_id": session, "cwd": str(copy)}
            if host == "codex":
                payload["turn_id"] = str(uuid.uuid4())
        return {**payload, "hook_event_name": event, **fields}

    def stopping(session: str, again: bool) -> Dict[str, Any]:
        if host == "cursor":
            return call(session, stop_event, status="completed", loop_count=int(again))
        return call(session, stop_event, stop_hook_active=again,
                    last_assistant_message="Saved." if again else "Checked.")

    def asking(session: str, text: str) -> Dict[str, Any]:
        extra = {"attachments": []} if host == "cursor" else {}
        return call(session, prompt_event, prompt=text, **extra)

    first = str(uuid.uuid4())
    steps["start"] = run_event(copy, host, started_event, call(first, started_event, source="startup"), env, timeout)
    steps["prompt"] = run_event(copy, host, prompt_event, asking(first, PROMPT), env, timeout)
    capsule = capsule_text(copy, host, steps["prompt"])
    steps["stop"] = run_event(copy, host, stop_event, stopping(first, False), env, timeout)
    request = None
    for item in steps["stop"]:
        try:
            decision = json.loads(item["stdout"])
        except ValueError:
            continue
        if not isinstance(decision, dict):
            continue
        feedback = decision.get("hookSpecificOutput")
        if host == "cursor" and isinstance(decision.get("followup_message"), str):
            request = decision["followup_message"]
        elif (host == "claude" and isinstance(feedback, dict)
              and "native-result --request" in str(feedback.get("additionalContext"))):
            request = feedback["additionalContext"]
        elif decision.get("decision") == "block":
            request = str(decision.get("reason") or "")
    save: Dict[str, Any] = {"requested": request is not None, "saved": False, "refusal": None}
    if request is not None:
        match = SAVE_COMMAND.search(request)
        if match is None:
            save["refusal"] = "the request names no command"
        elif source is None:
            save["refusal"] = "the project tracks no file a learning may cite"
        else:
            result = {"progress": "Rehearsal turn: checked where the configuration lives.", "verified": True,
                      "learnings": [{"type": "finding", "title": f"Configuration entry point {marker}",
                                     "consequence": f"The project's configuration starts at {source} ({marker}).",
                                     "sources": [source]}]}
            command, data = match.group(1), json.dumps(result)
            stdin: Optional[bytes] = data.encode("utf-8")
            if "'<JSON>'" in command:
                command, stdin = command.replace("'<JSON>'", "'" + data.replace("'", "'\\''") + "'"), None
            started = time.monotonic()
            done = subprocess.run(["bash", "-c", command], input=stdin,
                                  cwd=copy, env=env, capture_output=True, timeout=timeout)
            save["save_ms"] = round((time.monotonic() - started) * 1000)
            output = done.stdout.decode("utf-8", "replace") + done.stderr.decode("utf-8", "replace")
            save["saved"] = done.returncode == 0
            if not save["saved"]:
                found = REFUSAL.search(output)
                save["refusal"] = found.group(1) if found else output.strip()[-300:] or f"exit {done.returncode}"
    steps["closing stop"] = run_event(copy, host, stop_event, stopping(first, True), env, timeout)
    second = str(uuid.uuid4())
    steps["next prompt"] = run_event(copy, host, prompt_event,
                                     asking(second, f"What did the last session find about {marker}?"), env, timeout)
    recalled = marker in capsule_text(copy, host, steps["next prompt"])
    failures = [f"{step}: {item['hook']} exit {item['exit']}" for step, items in steps.items() for item in items
                if item["exit"] != 0]
    lines = []
    for line in journal(copy)[before:]:
        try:
            item = json.loads(line)
        except ValueError:
            continue
        lines.append(":".join(str(item.get(key)) for key in ("stage", "event", "code") if item.get(key) is not None))
    return {
        "host": host, "profile": profile, "source": source,
        "capsule_chars": len(capsule),
        "ms": {step: sum(item["ms"] for item in items) for step, items in steps.items()},
        **save, "recalled": recalled, "hook_failures": failures,
        "stderr": [f"{item['hook']}: {item['stderr']}" for items in steps.values() for item in items if item["stderr"]],
        "journal": lines,
        "ok": save["saved"] and recalled and not failures,
    }


def rehearse_target(target: Path, hosts: List[str], profiles: List[str], work: Path, excludes: List[str],
                    timeout: float, keep: bool) -> Dict[str, Any]:
    name = re.sub(r"[^A-Za-z0-9._-]+", "-", target.name) or "project"
    base = work / name / "synced"
    prepare_home(work / "home")
    (work / "tmp").mkdir(parents=True, exist_ok=True)
    report: Dict[str, Any] = {"target": str(target), "runs": []}
    copy_tree(target, base, excludes)
    try:
        report["sync"] = sync(base, child_env(work / "home", work / "tmp", {}), timeout * 10)
        report["profile_in_config"] = configured_profile(base)
        if report["sync"].get("error"):
            return report
        for host in hosts:
            if not (base / WIRING[host]).is_file():
                report["runs"].append({"host": host, "ok": False, "skipped": f"{WIRING[host]} is not installed"})
                continue
            for profile in profiles:
                run = work / name / f"{host}-{profile}"
                shutil.copytree(base, run, symlinks=True)
                try:
                    report["runs"].append(rehearse(run, host, profile, work, timeout))
                finally:
                    if not keep:
                        shutil.rmtree(run, ignore_errors=True)
    finally:
        if not keep:
            shutil.rmtree(work / name, ignore_errors=True)
    return report


def print_report(reports: List[Dict[str, Any]]) -> None:
    for report in reports:
        sync_state = report.get("sync") or {}
        print(f"{report['target']}")
        if sync_state.get("error"):
            print(f"  sync failed: {sync_state['error']}")
            continue
        print(f"  sync: {sync_state.get('changed')} changed, {sync_state.get('kept')} kept"
              + (f"; {sync_state['partial']}" if sync_state.get("partial") else ""))
        print(f"  capsule profile in runtime.json: {report.get('profile_in_config')}")
        for run in report["runs"]:
            if run.get("skipped"):
                print(f"  {run['host']:6}  skipped: {run['skipped']}")
                continue
            outcome = "saved" if run["saved"] else (f"refused ({run['refusal']})" if run["requested"] else "no request")
            print(f"  {run['host']:6} {run['profile']:10} prompt {run['ms']['prompt']:5} ms  stop {run['ms']['stop']:5} ms"
                  f"  {outcome:28} recalled {'yes' if run['recalled'] else 'NO'}"
                  + (f"  hook failures: {', '.join(run['hook_failures'])}" if run["hook_failures"] else ""))


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--target", type=Path, action="append", required=True,
                        help="an installed project; repeatable")
    parser.add_argument("--hosts", default="claude,codex", help="some of claude, codex, cursor")
    parser.add_argument("--profiles", default="config,recall", help=f"some of {', '.join(PROFILES)}")
    parser.add_argument("--work", type=Path, help="where the copies go (default: /dev/shm, else the temp directory)")
    parser.add_argument("--exclude", action="append", default=[],
                        help="another tree to leave out of the copy: a name at any depth, or a root-relative path")
    parser.add_argument("--timeout", type=float, default=60.0, help="seconds per hook call")
    parser.add_argument("--keep", action="store_true", help="keep the copies for inspection")
    parser.add_argument("--json", action="store_true", help="print the report as JSON")
    arguments = parser.parse_args(argv)
    hosts = [item.strip() for item in arguments.hosts.split(",") if item.strip()]
    profiles = [item.strip() for item in arguments.profiles.split(",") if item.strip()]
    if not hosts or any(host not in WIRING for host in hosts):
        parser.error("--hosts must name some of claude, codex, cursor")
    if not profiles or any(profile not in PROFILES for profile in profiles):
        parser.error(f"--profiles must name some of {', '.join(PROFILES)}")
    base = arguments.work
    if base is None:
        shm = Path("/dev/shm")
        base = shm if shm.is_dir() and os.access(shm, os.W_OK) else Path(tempfile.gettempdir())
    reports = []
    for target in arguments.target:
        target = target.expanduser().resolve()
        problem = None
        if not (target / "memory-bank/scripts/context.py").is_file():
            problem = f"{target} has no installed accelerator (memory-bank/scripts/context.py)"
        elif (target / ".git").is_file():
            # A worktree or a submodule: its Git directory lives in another
            # checkout, which the copy would share and write to.
            problem = f"{target} is a Git worktree or submodule; rehearse its main checkout"
        elif inside(target, ROOT) or inside(ROOT, target):
            problem = f"{target} is this clone or holds it; rehearse an installed project"
        elif inside(base, target):
            problem = f"--work {base} is inside {target}"
        if problem:
            print(problem, file=sys.stderr)
            return 2
        work = Path(tempfile.mkdtemp(prefix="rollout-rehearsal-", dir=str(base)))
        try:
            reports.append(rehearse_target(target, hosts, profiles, work,
                                           [*DEFAULT_EXCLUDES, *arguments.exclude], arguments.timeout,
                                           arguments.keep))
        finally:
            if not arguments.keep:
                shutil.rmtree(work, ignore_errors=True)
            else:
                print(f"kept: {work}", file=sys.stderr)
    if arguments.json:
        print(json.dumps(reports, indent=1, ensure_ascii=False))
    else:
        print_report(reports)
    runs = [run for report in reports for run in report.get("runs", [])]
    synced = all(not (report.get("sync") or {}).get("error") for report in reports)
    return 0 if synced and runs and all(run.get("ok") for run in runs) else 1


if __name__ == "__main__":
    sys.exit(main())
