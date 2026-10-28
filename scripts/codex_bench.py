#!/usr/bin/env python3
"""Run the installed Codex through one answered turn of an edition, offline.

The Codex counterpart of `claude_code_bench.py`. Codex runs a project's hooks
only once their hashes are approved, and turns a Stop hook's block into a
`<hook_prompt>` item of the same turn; the save after every answer depends on
both and on Codex's sandbox letting the save write the project. This drives
the real binary through one turn without a model: Codex runs inside a network
namespace with loopback only (`unshare -rn`, so nothing can leave the machine
and no tokens are spent) against a scripted stand-in for the Responses API. The
stand-in reads a file, answers, and follows the save step as written, reading
XML entities as a model does.

Its isolated CODEX_HOME approves the project's hooks by their current hashes,
as a person does once in /hooks. `codex exec` runs with the workspace-write
sandbox.

It reports what the host did: whether the hooks were approved and ran, whether
the save ran, how the step reached the model and whether the second Stop saw
the recovery. Exit 0 when the save ran and the recovery was seen.

Linux only: user and network namespaces (`unshare`), and `ip` to raise the
loopback. `--target DIR [--sync]` runs in a copy of an installed project, as
`claude_code_bench.py` does. `--attached` runs a fresh project with nothing
installed and the edition lent from this clone, as the Harness launches Codex
(`accelerator_attach.py`: `-c` hooks and instructions, the state outside the
project as an added directory), with the hooks trusted the way
`accelerator_attach.py trust-codex-hooks` records them.

  python3 scripts/codex_bench.py
  python3 scripts/codex_bench.py --target ~/Desktop/next --sync
  python3 scripts/codex_bench.py --attached
  python3 scripts/codex_bench.py --queued 8
  python3 scripts/codex_bench.py --subfolder src

`--subfolder DIR` starts Codex in DIR below the project root (in its terminal
too, with `--queued`). Its
workspace-write sandbox then writes only below DIR, so the save cannot write
the project - nor its journal: it waits in the temporary directory, and the
session's next hook, outside the sandbox, writes it. The run passes when the
save went that way (journal `save` `saved` with code `spooled`) and the
recovery was seen. Its work folder is on disk under ~/.cache: Codex's
bubblewrap sandbox mounts its own /dev and writes /tmp, so a project in
/dev/shm or /tmp would be invisible or writable below its root.

`--queued SECONDS` runs Codex in its terminal instead, through a
pseudo-terminal, as claude_code_bench.py does: the stand-in takes SECONDS to
write the save, and a second question is typed while it does. Codex takes it
into the running turn and runs the prompt hook at once; the run passes when
the save in flight still lands. The terminal runs commands through the
app-server daemon installed under ~/.codex/packages, which the isolated
CODEX_HOME links to.

`--worktree [inside|beside]` runs the measured session in a Git worktree of
the project, on a branch of its own: inside the project where Claude Code
makes its worktrees (`.claude/worktrees/bench`), or beside it where Codex
makes its own (`CODEX_HOME/worktrees/<id>/project`), with `--subfolder DIR`
in DIR of it. The installation is not committed, so the worktree has none of
it. A first session in the project root sets up the dispatcher in the
isolated CODEX_HOME (memory-bank/scripts/codex_worktrees.py); a session in
the worktree then runs none of its entries, which the setup leaves for the
person's review; they are approved by their hashes, as Codex's own "Trust
all" records them, and the measured session follows. It passes when the
setup wrote no approval, the unapproved session ran no hook, and the
approved one got the capsule and every line of the project's AGENTS.md, its
save waited in the spool - Codex's sandbox writes only the worktree - and
landed in the root's Project Brain under the worktree's branch, the
worktree got an index of its own in the root, and nothing was written into
the worktree.

`--worktree codex` lets Codex make the worktree, as `codex --worktree` and
a new worktree in the Codex app do: `codex exec --worktree` checks HEAD out
in CODEX_HOME/worktrees/<id>/project on a detached HEAD - no branch to name
the task - and records the session's thread as its owner in
codex-thread.json of the worktree's Git directory. HEAD has a file the main
checkout's working tree has deleted, which the save's learning cites. The
unapproved session and the measured one each get a worktree of their own.
It passes as above, with the save under the task the owning thread names,
codex-thread/<thread>, the measured session's own.

`--cli checkpoint` has the stand-in make a checkpoint first, as the project's
`checkpoint` skill says (scripts/bench_checkpoint.py): the task, `turn
--flush` and `update`, with the memory CLI the session was told to run, else
the policy's. With `--worktree` the run passes only when that checkpoint
landed in the root's Project Brain under the worktree's task - from Codex's
sandbox it waits for the session's next hook (journal stage `queue`).

`--app-server` runs the measured turn through `codex app-server`, as the
Codex desktop app drives it: a thread started in the session's folder with
the workspace-write sandbox and on-request approvals, a turn, and every
approval Codex asks the client for refused and counted. It passes only when
none was asked. With `--worktree codex` the bench makes the worktree the
way the app does, the app-server having no request for one: `git worktree
add --detach` in CODEX_HOME/worktrees/<id>/project, and, once the thread has
started, its owner record one key a line.

  python3 scripts/codex_bench.py --worktree
  python3 scripts/codex_bench.py --worktree beside --subfolder src
  python3 scripts/codex_bench.py --worktree --app-server
  python3 scripts/codex_bench.py --worktree codex
  python3 scripts/codex_bench.py --worktree codex --app-server
  python3 scripts/codex_bench.py --worktree codex --cli checkpoint
"""
import argparse
import html
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import bench_checkpoint  # noqa: E402
import claude_code_bench as bench  # noqa: E402

MARKER = bench.MARKER
SAVE = bench.SAVE
# Where the bench's worktree goes: where Claude Code makes its own, inside the
# project, or where Codex makes its own, in CODEX_HOME beside it - by the
# bench, on a branch, or by Codex itself, detached (`codex`).
WORKTREES = {"inside": bench.WORKTREE, "beside": "codex-home/worktrees/b3c4/project", "codex": None}
# A file of the worktree's alone, which the save's learning cites.
WORKTREE_SOURCE = "src/Refund.php"
# The task a worktree Codex made gets: the thread that owns it names it.
THREAD_TASK = "codex-thread/"


class Scripted(BaseHTTPRequestHandler):
    """The stand-in for the Responses API: read, answer, save as told, done."""
    protocol_version = "HTTP/1.1"
    source = "composer.json"
    requests: List[Dict[str, Any]] = []
    lock = threading.Lock()
    save_delay = 0.0
    save_begun = threading.Event()
    read = ""
    # --cli checkpoint: the checkpoint the stand-in makes before it reads.
    checkpoint: Optional[bench_checkpoint.Checkpoint] = None

    def log_message(self, *args: Any) -> None:
        pass

    def reply(self, status: int, payload: Dict[str, Any]) -> None:
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:
        if self.path.split("?")[0].endswith("/models"):
            return self.reply(200, {"object": "list", "data": []})
        self.reply(404, {"error": {"message": "bench", "type": "not_found"}})

    def do_POST(self) -> None:
        body = json.loads(self.rfile.read(int(self.headers.get("content-length") or 0)) or b"{}")
        if not self.path.split("?")[0].endswith("/responses"):
            return self.reply(404, {"error": {"message": "bench", "type": "not_found"}})
        output, since = self.decide(body)
        if os.environ.get("CODEX_BENCH_DUMP"):
            with open(os.path.join(os.environ["CODEX_BENCH_DUMP"], f"request-{len(self.requests) + 1:02d}.json"),
                      "w", encoding="utf-8") as handle:
                json.dump(body, handle)
        tools = [tool.get("name") or tool.get("type") for tool in body.get("tools") or [] if isinstance(tool, dict)]
        with self.lock:
            self.requests.append({"since": since, "output": output, "tools": tools,
                                  "tool_shapes": [{key: tool.get(key) for key in ("type", "name", "format")}
                                                  for tool in body.get("tools") or [] if isinstance(tool, dict)]})
            number = len(self.requests)
        if number > 30:
            output = [message(number, "Loop guard.")]
        response = {"id": f"resp_{number}", "object": "response", "status": "completed",
                    "model": body.get("model", "bench"), "output": output,
                    "usage": {"input_tokens": 1000, "output_tokens": 20, "total_tokens": 1020,
                              "input_tokens_details": {"cached_tokens": 0},
                              "output_tokens_details": {"reasoning_tokens": 0}}}
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()

        def event(kind: str, payload: Dict[str, Any]) -> None:
            self.wfile.write(f"event: {kind}\ndata: {json.dumps(dict(payload, type=kind))}\n\n".encode())

        event("response.created", {"response": dict(response, status="in_progress", output=[])})
        # A model's stream opens before it writes: Codex 0.164's terminal,
        # after an earlier session in the same CODEX_HOME, cut a response that
        # stayed silent for 8 seconds and never ran the save it carried.
        self.wfile.flush()
        self.pause(output)
        for index, item in enumerate(output):
            event("response.output_item.added", {"output_index": index, "item": item})
            if item["type"] == "message":
                event("response.output_text.delta", {"item_id": item["id"], "output_index": index,
                                                     "content_index": 0, "delta": item["content"][0]["text"]})
            event("response.output_item.done", {"output_index": index, "item": item})
        event("response.completed", {"response": response})
        self.wfile.flush()
        self.close_connection = True

    def pause(self, output: List[Dict[str, Any]]) -> None:
        """--queued: the first save takes save_delay seconds to write."""
        if (self.save_delay and output and str(output[0].get("call_id", "")).startswith("call_save_")
                and not self.save_begun.is_set()):
            self.save_begun.set()
            time.sleep(self.save_delay)

    def decide(self, body: Dict[str, Any]):
        items = body.get("input") or []
        tools = body.get("tools") or []
        last = max((n for n, item in enumerate(items) if item.get("type") in ("function_call", "custom_tool_call")
                    or item.get("role") == "assistant"), default=-1)
        since = [text_of(item) for item in items[last + 1:]]
        number = len(self.requests) + 1
        if getattr(self, "checkpoint", None) is not None and MARKER not in html.unescape("\n".join(since)):
            planned = self.checkpoint_step(items, tools, last, number)
            if planned is not None:
                return [planned], since
        if any(item.get("type") in ("function_call_output", "custom_tool_call_output") for item in items[last + 1:]):
            asked = (items[last].get("arguments") or items[last].get("input") or "") if last >= 0 else ""
            text = "Saved." if "native-result" in asked else f"Invoices are numbered per tenant ({self.source})."
            return [message(number, text)], since
        # A model reads XML entities as the characters they stand for: Codex
        # escapes a hook prompt inside <hook_prompt>, & as &amp;.
        said = html.unescape("\n".join(since))
        if MARKER in said:
            found = SAVE.search(said)
            if found is None:
                return [message(number, "The step named no command.")], since
            command, data = found.group(1), json.dumps(bench.result_for(self.source), ensure_ascii=False)
            quoted = "'" + data.replace("'", "'\\''") + "'"
            command = (command.replace("'<JSON>'", quoted) if "'<JSON>'" in command
                       else f"printf '%s' {quoted} | {command}")
            return [shell_call(tools, command, f"call_save_{number}")], since
        return [shell_call(tools, f"cat {getattr(self, 'read', '') or self.source}", f"call_read_{number}")], since


    def checkpoint_step(self, items: List[Dict[str, Any]], tools: List[Any], last: int,
                        number: int) -> Optional[Dict[str, Any]]:
        """--cli checkpoint: its next command, the read once it is over, or
        None after that."""
        issued = [item for item in items if item.get("type") in ("function_call", "custom_tool_call")
                  and str(item.get("call_id", "")).startswith("call_ckpt_")]
        outputs = {item.get("call_id"): item for item in items
                   if item.get("type") in ("function_call_output", "custom_tool_call_output")}
        done = [(item["call_id"].split("_", 3)[3], *command_result(text_of(outputs.get(item["call_id"], {}))))
                for item in issued]
        for step in done[len(self.checkpoint.steps):]:
            self.checkpoint.record(*step)
        planned = self.checkpoint.next(html.unescape("\n".join(text_of(item) for item in items)), done)
        if planned is not None:
            name, command = planned
            return shell_call(tools, command, f"call_ckpt_{len(done)}_{name}")
        if last >= 0 and str(items[last].get("call_id", "")).startswith("call_ckpt_"):
            return shell_call(tools, f"cat {getattr(self, 'read', '') or self.source}", f"call_read_{number}")
        return None


def command_result(text: str) -> Tuple[str, bool]:
    """A shell call's own output and whether it failed, from what Codex hands
    the model: exec_command's "Process exited with code N ... Output:" text,
    or the shell tool's JSON with metadata.exit_code."""
    code: Optional[int] = None
    found = re.search(r"exited with code (-?\d+)", text)
    if found:
        code = int(found.group(1))
    body = text.split("Output:\n", 1)[1] if "Output:\n" in text else text
    try:
        value = json.loads(text)
    except ValueError:
        value = None
    if isinstance(value, dict) and isinstance(value.get("output"), str):
        body = value["output"]
        metadata = value.get("metadata") if isinstance(value.get("metadata"), dict) else {}
        code = metadata.get("exit_code", code)
    return body, code not in (None, 0)


def message(number: int, text: str) -> Dict[str, Any]:
    return {"type": "message", "id": f"msg_{number}", "role": "assistant", "status": "completed",
            "content": [{"type": "output_text", "text": text, "annotations": []}]}


def shell_call(tools: List[Any], command: str, call_id: str) -> Dict[str, Any]:
    names = {tool.get("name") for tool in tools if isinstance(tool, dict)}
    if not tools:
        # Code mode: models Codex gives no tool list write a script for its
        # `exec` tool, as gpt-5.6 does in the transcripts.
        return {"type": "custom_tool_call", "id": "ctc_" + call_id, "call_id": call_id, "name": "exec",
                "input": f"text(await tools.exec_command({{cmd:{json.dumps(command)}}}));", "status": "completed"}
    if "exec_command" in names:
        name, arguments = "exec_command", {"cmd": command}
    elif "shell_command" in names:
        name, arguments = "shell_command", {"command": command}
    else:
        name, arguments = "shell", {"command": ["bash", "-lc", command]}
    return {"type": "function_call", "id": "fc_" + call_id, "call_id": call_id, "name": name,
            "arguments": json.dumps(arguments), "status": "completed"}


def text_of(item: Dict[str, Any]) -> str:
    if isinstance(item.get("content"), str):
        return item["content"]
    parts = [part["text"] for part in item.get("content") or []
             if isinstance(part, dict) and isinstance(part.get("text"), str)]
    if isinstance(item.get("output"), str):
        parts.append(item["output"])
    elif isinstance(item.get("output"), list):
        parts.extend(part["text"] for part in item["output"]
                     if isinstance(part, dict) and isinstance(part.get("text"), str))
    return "\n".join(parts)


def stop_helpers(home: Path) -> int:
    """Codex starts helpers that outlive it - the terminal's app-server daemon,
    code mode's host - and they keep writing to CODEX_HOME after the run is
    gone. Stop every process whose CODEX_HOME is this run's; returns how many."""
    marker = b"CODEX_HOME=" + os.fsencode(str(home))
    stopped = 0
    for _ in range(2):  # a supervisor may restart one between passes
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit() or int(entry.name) in (os.getpid(), os.getppid()):
                continue
            try:
                if marker not in (entry / "environ").read_bytes().split(b"\0"):
                    continue
                os.kill(int(entry.name), signal.SIGKILL)
                stopped += 1
            except (OSError, ValueError):
                continue
        time.sleep(0.5)
    return stopped


def codex_binary(name: str) -> str:
    """The Codex a person runs: the command on PATH, the npm launcher included."""
    found = shutil.which(name)
    return os.path.realpath(found) if found else name


def inside(args: argparse.Namespace) -> int:
    """In the namespace: raise the loopback, approve the hooks, serve, run one turn, report."""
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    work = Path(args.work)
    project, home = work / "project", work / "codex-home"
    home.mkdir(exist_ok=True)
    Scripted.source = args.source
    # The file the stand-in reads, from the folder Codex starts in.
    Scripted.read = os.path.relpath(project / args.source, project / args.subfolder) if args.subfolder else args.source
    server = ThreadingHTTPServer(("127.0.0.1", 0), Scripted)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    (home / "config.toml").write_text(
        'model = "gpt-5.1-codex"\nmodel_provider = "bench"\napproval_policy = "never"\n'
        f'sandbox_mode = "{args.sandbox}"\n\n'
        '[model_providers.bench]\nname = "bench"\n'
        f'base_url = "http://127.0.0.1:{server.server_address[1]}/v1"\n'
        'wire_api = "responses"\nenv_key = "BENCH_API_KEY"\n\n'
        f'[projects.{json.dumps(str(project))}]\ntrust_level = "trusted"\n', encoding="utf-8")
    env = bench.clean_env()
    env.update(HOME=str(work / "home"), CODEX_HOME=str(home), TMPDIR=str(work / "tmp"), BENCH_API_KEY="offline")
    (work / "tmp").mkdir(exist_ok=True)
    os.environ.update(HOME=str(work / "home"), CODEX_HOME=str(home))
    overlay: Dict[str, Any] = {}
    if args.attached:
        import accelerator_attach as attach
        overlay = json.loads((work / "overlay.json").read_text(encoding="utf-8"))
    try:
        if args.attached:
            entries = attach.trust_codex_hooks(args.codex, args.edition, project)
            trust = {"hooks": len(entries), "trusted": sum(entry["status"] == "trusted" for entry in entries)}
        else:
            trust = approve_hooks(args.codex, project)
    except Exception as error:  # the run below shows whether hooks ran without it
        trust = {"error": f"{type(error).__name__}: {error}"}
    start = project / args.subfolder if args.subfolder else project
    journal_file = (Path(args.attached) if args.attached else project) / "memory-bank/local/native-results.ndjson"
    first_lines = 0
    setup: Dict[str, Any] = {}
    place = project
    if args.worktree:
        place, setup = worktree_setup(args, work, project, home, env, journal_file)
        start = place / args.subfolder if args.subfolder else place
        first_lines = len(journal_file.read_text().splitlines()) if journal_file.exists() else 0
        Scripted.requests.clear()
    start.mkdir(parents=True, exist_ok=True)
    if args.cli == "checkpoint":
        # The measured session's checkpoint, as the project's skill says.
        Scripted.checkpoint = bench_checkpoint.Checkpoint(
            (project / ".agents/skills/checkpoint/SKILL.md").read_text(encoding="utf-8"))
    made = managed_worktrees(home)
    # `--worktree codex` in a terminal: Codex makes the worktree and works there.
    managed = ["--worktree"] if args.worktree == "codex" and not args.app_server else []
    command = [args.codex, "exec", "-C", str(start), *managed, "--skip-git-repo-check", "--sandbox", args.sandbox,
               "How are invoices numbered?"]
    threads: List[str] = []
    if args.attached:
        command = attach.apply_overlay("codex", command, overlay)
        env.update(overlay["environment"])
    screen = ""
    asked: List[Dict[str, Any]] = []
    if args.queued is not None:
        if args.packages and not (home / "packages").exists():
            (home / "packages").symlink_to(args.packages)
        Scripted.save_delay = args.queued
        # The terminal reads the sandbox from config.toml and starts in the project.
        screen = bench.terminal([args.codex], start, dict(env, TERM="xterm-256color"), journal_file,
                                args.timeout, Scripted.save_begun, Scripted.requests, settle=8.0)
        (work / "screen.txt").write_text(screen, encoding="utf-8")
        done = subprocess.CompletedProcess([], 0, "", "")
    elif args.app_server:
        def started(thread: str) -> None:
            threads.append(thread)
            if args.worktree == "codex":
                own(place, thread)

        done, asked = app_server_turn(args.codex, start, env, "How are invoices numbered?", args.sandbox,
                                      args.timeout, journal_file, on_thread=started)
    else:
        done = subprocess.run(command, cwd=start, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                              text=True, timeout=args.timeout)
        threads.extend(re.findall(r"^session id: (\S+)$", done.stderr, re.M))
    server.shutdown()
    helpers = stop_helpers(home)
    if managed:
        # The worktree Codex made for this session.
        place = next((path for path in managed_worktrees(home) if path not in made), place)
        start = place / args.subfolder if args.subfolder else place
    steps = [request for request in Scripted.requests if any(MARKER in text for text in request["since"])]
    carried = [text for text in steps[0]["since"] if MARKER in text] if steps else []
    lines = journal_file.read_text().splitlines()[first_lines:] if journal_file.exists() else []
    journal = [json.loads(line) for line in lines]
    events = [f"{line['stage']}:{line['event']}" + (f":{line['code']}" if line.get("code") else "")
              for line in journal if line.get("stage") in ("prompt", "stop", "save")]
    report = {
        "codex": args.codex, "version": version(args.codex), "exit": done.returncode,
        "tools": Scripted.requests[0]["tools"] if Scripted.requests else [],
        "hook_trust": trust,
        "step_messages": len(carried),
        "step_as_hook_prompt": bool(carried) and carried[0].lstrip().startswith("<hook_prompt"),
        "step_escaped": bool(carried) and "&amp;" in carried[0],
        "saved": "stop:skipped:recovery-saved" in events,
        "events": events,
        "stderr_tail": done.stderr[-300:] if done.returncode else "",
        "helpers_stopped": helpers,
    }
    report["ok"] = report["saved"] and len(carried) == 1
    if args.sandbox == "read-only":
        # The turn can write nothing, so the Stop asks for no save: the
        # stand-in got no save step, and the journal says why. A save asked
        # for there would be the failure.
        report["ok"] = not carried and "stop:skipped:read-only-sandbox" in events
    if args.queued is not None:
        report["unsaved_on_screen"] = bench.UNSAVED in screen
        report["ok"] = bench.queued_ok(events, screen)
    if args.app_server:
        # An approval the app would have asked the person for: the save's above all.
        report["app_server"] = True
        report["approval_requests"] = asked
        report["ok"] = report["ok"] and not asked
    if args.subfolder or args.worktree:
        # The sandbox writes only below the folder - not the journal either -
        # so the line is the next hook's: saved, through the spool.
        report["started_in"] = (start.relative_to(work)).as_posix()
        report["spooled"] = "save:saved:spooled" in events
        report["ok"] = report["ok"] and report["spooled"]
    if args.worktree:
        # The root's memory, the worktree's own task and index, and nothing
        # written into the worktree - once the person approved the entries,
        # which the setup did not do for them.
        heard = "\n".join(text for request in Scripted.requests for text in request["since"])
        report["setup"] = setup
        report["capsule"] = "working:" in heard
        policy = (project / "AGENTS.md").read_text(encoding="utf-8").splitlines()
        report["policy"] = (all(line in heard for line in policy if line.strip())
                            and "Warning: truncated output" not in heard)
        tasks = [json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
                 for path in (project / "project-brain/dynamic/tasks").glob("*.md")]
        # A worktree on a branch: the branch names the task. One Codex made
        # is detached, and the thread that owns it - this session's - does.
        owner = owner_of(place) if args.worktree == "codex" else ""
        report["task"] = THREAD_TASK + owner if args.worktree == "codex" else bench.WORKTREE_BRANCH
        report["worktree_task"] = any(task.get("external_id") == report["task"] and task.get("progress")
                                      for task in tasks)
        if args.worktree == "codex":
            report["worktree"] = place.relative_to(work).as_posix()
            report["detached"] = subprocess.run(["git", "-C", str(place), "symbolic-ref", "-q", "HEAD"],
                                                capture_output=True).returncode == 1
            report["owner_is_session"] = bool(owner) and threads == [owner]
            report["ok"] = report["ok"] and report["detached"] and report["owner_is_session"]
        # The learning cites the worktree's own file, which the main checkout lacks.
        findings = [json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
                    for path in (project / "project-brain/dynamic/findings").glob("*.md")]
        report["learning_from_worktree"] = any(source.get("path") == WORKTREE_SOURCE for finding in findings
                                               for source in finding.get("source_fingerprints") or [])
        report["own_index"] = any((project / "memory-bank/local/worktrees").glob("*/context.db"))
        report["worktree_clean"] = subprocess.run(["git", "status", "--porcelain", "--ignored"], cwd=place,
                                                  capture_output=True, text=True).stdout == ""
        unapproved = setup.get("unapproved") or {}
        report["ok"] = (report["ok"] and report["capsule"] and report["policy"] and report["worktree_task"]
                        and report["learning_from_worktree"] and report["own_index"] and report["worktree_clean"] and setup.get("listed") is True
                        and setup.get("approvals_before") == [] and (setup.get("approval") or {}).get("approved", 0) > 0
                        and unapproved.get("hook_lines") == 0 and not unapproved.get("capsule")
                        and not unapproved.get("save_asked"))
    if args.cli == "checkpoint":
        # The checkpoint landed in the root's Project Brain, under the task
        # of the checkout the session worked in.
        task = report.get("task") or subprocess.run(["git", "-C", str(project), "branch", "--show-current"],
                                                    capture_output=True, text=True).stdout.strip()
        report["checkpoint"] = Scripted.checkpoint.report(project, task)
        report["checkpoint"]["queue"] = [f"{line['event']}" + (f":{line['code']}" if line.get("code") else "")
                                         for line in journal if line.get("stage") == "queue"]
        report["ok"] = report["ok"] and report["checkpoint"]["ok"]
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


def worktree_setup(args: argparse.Namespace, work: Path, project: Path, home: Path, env: Dict[str, str],
                   journal_file: Path) -> Tuple[Path, Dict[str, Any]]:
    """--worktree: a session in the project root, whose session-start hook sets
    up the dispatcher (in the background); the worktree; a session there
    before the person approved the entries; then their approval. Returns the
    worktree and what each step left."""
    subprocess.run([args.codex, "exec", "-C", str(project), "--skip-git-repo-check", "--sandbox", args.sandbox,
                    "How are invoices numbered?"], cwd=project, env=env, stdin=subprocess.DEVNULL,
                   capture_output=True, text=True, timeout=args.timeout)
    listed = home / "ai-accelerator" / "projects"
    deadline = time.time() + 15
    while time.time() < deadline and str(project) not in (listed.read_text().splitlines() if listed.exists() else []):
        time.sleep(0.2)
    if args.worktree == "codex":
        # Codex checks out HEAD and leaves the working tree's changes behind:
        # a file committed, then deleted in the main checkout, is the
        # worktree's alone. The stand-in reads it and the learning cites it -
        # checked where the session works, or the save is refused.
        (project / WORKTREE_SOURCE).parent.mkdir(parents=True, exist_ok=True)
        (project / WORKTREE_SOURCE).write_text("<?php\nfinal class Refund {}\n", encoding="utf-8")
        for step in (["add", WORKTREE_SOURCE], ["commit", "-qm", "Refunds"]):
            subprocess.run([*bench.GIT, *step], cwd=project, check=True, capture_output=True)
        (project / WORKTREE_SOURCE).unlink()
        # A terminal session starts in the project and Codex makes the
        # worktree; the app makes it before it starts the thread.
        place = app_worktree(home, project) if args.app_server else project
    else:
        place = (project if args.worktree == "inside" else work) / WORKTREES[args.worktree]
        place.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([*bench.GIT, "worktree", "add", "-q", str(place), "-b", bench.WORKTREE_BRANCH], cwd=project,
                       check=True, capture_output=True)
        # The branch's own work: a file the main checkout does not have, which
        # the stand-in reads and its learning cites - checked where the session
        # works, or the save is refused.
        (place / WORKTREE_SOURCE).parent.mkdir(parents=True, exist_ok=True)
        (place / WORKTREE_SOURCE).write_text("<?php\nfinal class Refund {}\n", encoding="utf-8")
        for step in (["add", WORKTREE_SOURCE], ["commit", "-qm", "Refunds"]):
            subprocess.run([*bench.GIT, *step], cwd=place, check=True, capture_output=True)
    start = place / args.subfolder if args.subfolder else place
    start.mkdir(parents=True, exist_ok=True)
    Scripted.source = WORKTREE_SOURCE
    Scripted.read = os.path.relpath(place / WORKTREE_SOURCE, start)
    # Codex makes the unapproved session a worktree of its own, from the project.
    unapproved = ([args.codex, "exec", "-C", str(project / args.subfolder if args.subfolder else project),
                   "--worktree"] if args.worktree == "codex" else [args.codex, "exec", "-C", str(start)])
    setup: Dict[str, Any] = {
        "listed": listed.exists() and str(project) in listed.read_text().splitlines(),
        "entries": dispatcher_entries(home),
        # The setup leaves the approval to the person: no record of its own.
        "approvals_before": user_approvals(home),
    }
    # Before the person approved them, the entries run nowhere.
    before = len(journal_file.read_text().splitlines()) if journal_file.exists() else 0
    Scripted.requests.clear()
    subprocess.run([*unapproved, "--skip-git-repo-check", "--sandbox", args.sandbox, "How are invoices numbered?"],
                   cwd=start, env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=args.timeout)
    heard = "\n".join(text for request in Scripted.requests for text in request["since"])
    setup["unapproved"] = {
        "hook_lines": len(journal_file.read_text().splitlines()) - before if journal_file.exists() else 0,
        "capsule": "working:" in heard,
        "save_asked": MARKER in heard,
    }
    try:
        setup["approval"] = approve_user_hooks(args.codex, start, home)
    except Exception as error:  # the run shows the entries did not run
        setup["approval"] = {"error": f"{type(error).__name__}: {error}"}
    return place, setup


def managed_worktrees(home: Path) -> List[Path]:
    """The checkouts in CODEX_HOME/worktrees/<id>/, where Codex makes them."""
    return sorted(path for path in (home / "worktrees").glob("*/*") if (path / ".git").is_file())


def git_directory(checkout: Path) -> Path:
    return Path(subprocess.run(["git", "-C", str(checkout), "rev-parse", "--absolute-git-dir"], check=True,
                               capture_output=True, text=True).stdout.strip())


def owner_of(checkout: Path) -> str:
    """The thread codex-thread.json of the checkout's Git directory names."""
    try:
        record = json.loads((git_directory(checkout) / "codex-thread.json").read_text(encoding="utf-8"))
    except (OSError, ValueError, subprocess.CalledProcessError):
        return ""
    owner = record.get("ownerThreadId") if isinstance(record, dict) else None
    return owner if isinstance(owner, str) else ""


def app_worktree(home: Path, project: Path) -> Path:
    """A worktree as the Codex app makes one before it starts the thread:
    `git worktree add --detach` in CODEX_HOME/worktrees/<4 hex>/<the
    project's folder name>, that folder marked with .codex-worktree-name."""
    place = home / "worktrees" / "a7e1" / project.name
    place.parent.mkdir(parents=True)
    (place.parent / ".codex-worktree-name").write_text("", encoding="utf-8")
    subprocess.run([*bench.GIT, "worktree", "add", "-q", "--detach", str(place), "HEAD"], cwd=project,
                   check=True, capture_output=True)
    return place


def own(checkout: Path, thread: str) -> None:
    """The owner record the app writes once the thread has started, as
    JSON.stringify(record, null, 2) writes it."""
    (git_directory(checkout) / "codex-thread.json").write_text(
        json.dumps({"version": 1, "ownerThreadId": thread}, indent=2) + "\n", encoding="utf-8")


def dispatcher_entries(home: Path) -> int:
    """How many of the person's hooks run the dispatcher."""
    marker = shlex.quote(str(home / "ai-accelerator" / "ai-accelerator-dispatch.sh")) + " "
    try:
        hooks = json.loads((home / "hooks.json").read_text(encoding="utf-8")).get("hooks") or {}
    except (OSError, ValueError):
        return 0
    return sum(str(hook.get("command", "")).startswith(marker)
               for groups in hooks.values() for group in groups for hook in group.get("hooks") or [])


def user_approvals(home: Path) -> List[str]:
    """The approvals config.toml holds for hooks of the person's hooks.json."""
    text = (home / "config.toml").read_text(encoding="utf-8")
    prefix = str(home / "hooks.json") + ":"
    try:
        import tomllib
    except ImportError:
        return sorted(set(re.findall(r'\[hooks\.state\."(' + re.escape(prefix) + r'[^"]*)"\]', text)))
    state = (tomllib.loads(text).get("hooks") or {}).get("state") or {}
    return sorted(key for key in state if key.startswith(prefix))


def approve_user_hooks(codex: str, cwd: Path, home: Path) -> Dict[str, Any]:
    """Approve the dispatcher's entries by their current hashes, as a person
    does once with Codex's own "Trust all" - the same records through the
    same request; nothing else of the person's."""
    import accelerator_attach
    marker = shlex.quote(str(home / "ai-accelerator" / "ai-accelerator-dispatch.sh")) + " "
    [listing] = accelerator_attach._app_server(codex, [], cwd, [
        {"method": "hooks/list", "params": {"cwds": [str(cwd)]}}])
    hooks = [hook for scope in (listing.get("result") or {}).get("data") or []
             for hook in scope.get("hooks") or []
             if hook.get("source") == "user" and str(hook.get("command", "")).startswith(marker)]
    pending = {hook["key"]: {"trusted_hash": hook["currentHash"]} for hook in hooks
               if hook.get("trustStatus") != "trusted"}
    if pending:
        [written] = accelerator_attach._app_server(codex, [], cwd, [{"method": "config/batchWrite", "params": {
            "edits": [{"keyPath": "hooks.state", "mergeStrategy": "upsert", "value": pending}]}}])
        if "error" in written:
            return {"error": str(written["error"])[:200], "entries": len(hooks)}
    return {"entries": len(hooks), "approved": len(pending),
            "statuses": sorted({hook.get("trustStatus") for hook in hooks})}


def app_server_turn(codex: str, cwd: Path, env: Dict[str, str], prompt: str, sandbox: str, timeout: float,
                    journal_file: Path, on_thread: Optional[Callable[[str], None]] = None
                    ) -> Tuple[subprocess.CompletedProcess, List[Dict[str, Any]]]:
    """One turn through `codex app-server`, as the Codex desktop app drives one:
    a thread in `cwd` with the sandbox and on-request approvals, a turn, and
    every approval Codex asks for refused. `on_thread` gets the thread's ID
    before its turn starts. Returns how it ended and what Codex asked the
    client."""
    import queue
    process = subprocess.Popen([codex, "app-server"], cwd=env["HOME"], env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    incoming: "queue.Queue[str]" = queue.Queue()
    threading.Thread(target=lambda: [incoming.put(line) for line in process.stdout], daemon=True).start()
    asked: List[Dict[str, Any]] = []
    seen: List[str] = []
    deadline = time.monotonic() + timeout

    def send(message: Dict[str, Any]) -> None:
        process.stdin.write(json.dumps(message) + "\n")
        process.stdin.flush()

    def until(done: Any) -> Dict[str, Any]:
        while time.monotonic() < deadline:
            try:
                line = incoming.get(timeout=0.5)
            except queue.Empty:
                if process.poll() is not None:
                    break
                continue
            seen.append(line)
            try:
                message = json.loads(line)
            except ValueError:
                continue
            if "method" in message and "id" in message:
                # A request to the client: an approval the app would show.
                asked.append({"method": message["method"], "params": json.dumps(message.get("params"))[:200]})
                send({"jsonrpc": "2.0", "id": message["id"], "result": {"decision": "decline"}})
                continue
            if done(message):
                return message
        return {}

    status = "incomplete"
    try:
        send({"jsonrpc": "2.0", "id": 0, "method": "initialize",
              "params": {"clientInfo": {"name": "codex-bench-desktop", "version": "1"}}})
        until(lambda message: message.get("id") == 0)
        send({"jsonrpc": "2.0", "method": "initialized"})
        send({"jsonrpc": "2.0", "id": 1, "method": "thread/start",
              "params": {"cwd": str(cwd), "approvalPolicy": "on-request", "sandbox": sandbox}})
        started = until(lambda message: message.get("id") == 1)
        thread = ((started.get("result") or {}).get("thread") or {}).get("id")
        if thread and on_thread is not None:
            on_thread(thread)
        if thread:
            send({"jsonrpc": "2.0", "id": 2, "method": "turn/start",
                  "params": {"threadId": thread, "input": [{"type": "text", "text": prompt}]}})
            finished = until(lambda message: message.get("method") == "turn/completed")
            status = ((finished.get("params") or {}).get("turn") or {}).get("status") or status
            # The recovery Stop's line lands as the turn ends.
            settle = time.monotonic() + 10
            while time.monotonic() < settle and not any(
                    '"stage":"stop"' in line and '"recovery-' in line
                    for line in (journal_file.read_text().splitlines() if journal_file.exists() else [])):
                time.sleep(0.2)
    finally:
        try:
            process.stdin.close()
            process.wait(timeout=10)
        except (OSError, subprocess.TimeoutExpired):
            process.kill()
    error = process.stderr.read() if process.stderr else ""
    return subprocess.CompletedProcess([codex, "app-server"], 0 if status == "completed" else 1,
                                       "".join(seen), f"turn {status}\n{error[-400:]}"), asked


def prepare_attached(work: Path, edition: str) -> Tuple[Path, Path]:
    """A project with nothing installed and the edition lent from this clone,
    as the Harness launches Codex: the overlay goes to `work/overlay.json`."""
    import accelerator_attach as attach
    project = bench.shop(work)
    state = attach.state_directory(project, work / "state-base")
    overlay = attach.codex_overlay(edition, project, state)
    (work / "overlay.json").write_text(json.dumps({"arguments": overlay.arguments,
                                                   "environment": overlay.environment}), encoding="utf-8")
    return project, state


def approve_hooks(codex: str, project: Path) -> Dict[str, Any]:
    """Approve every hook the project's .codex/hooks.json defines, by its current
    hash, as a person does once in /hooks; returns how many were approved."""
    import accelerator_attach
    [listing] = accelerator_attach._app_server(codex, [], project, [
        {"method": "hooks/list", "params": {"cwds": [str(project)]}}])
    hooks = [hook for scope in (listing.get("result") or {}).get("data") or []
             for hook in scope.get("hooks") or [] if hook.get("source") == "project"]
    pending = {hook["key"]: {"trusted_hash": hook["currentHash"]} for hook in hooks
               if hook.get("trustStatus") != "trusted"}
    if pending:
        [written] = accelerator_attach._app_server(codex, [], project, [{"method": "config/batchWrite", "params": {
            "edits": [{"keyPath": "hooks.state", "mergeStrategy": "upsert", "value": pending}]}}])
        if "error" in written:
            return {"error": str(written["error"])[:200], "hooks": len(hooks)}
    return {"hooks": len(hooks), "approved": len(pending)}


def version(codex: str) -> str:
    try:
        return subprocess.run([codex, "--version"], capture_output=True, text=True, timeout=30).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--codex", default="codex", help="the Codex command (default: codex on PATH)")
    parser.add_argument("--edition", choices=sorted(bench.EDITIONS), default="PHP Core")
    parser.add_argument("--timeout", type=float, default=240.0)
    parser.add_argument("--sandbox", default="workspace-write", choices=("workspace-write", "read-only"),
                        help="Codex's sandbox; read-only shows the save request skipped")
    parser.add_argument("--keep", action="store_true", help="keep the work directory and print its path")
    parser.add_argument("--target", help="run in a copy of this installed project instead of a fresh install")
    parser.add_argument("--sync", action="store_true", help="sync the copy from this clone first (with --target)")
    parser.add_argument("--exclude", action="append", default=[], help="another directory name to leave out of the copy")
    parser.add_argument("--attached", nargs="?", const="", default=None, metavar="",
                        help="lend the edition from this clone as the Harness does; install nothing")
    parser.add_argument("--queued", type=float, default=None, metavar="SECONDS",
                        help="in Codex's terminal, type a second question while the save, which the stand-in "
                             "takes SECONDS to write, is in flight")
    parser.add_argument("--subfolder", metavar="DIR", default=None,
                        help="start Codex in DIR below the project root; the save goes through the spool")
    parser.add_argument("--worktree", nargs="?", const="inside", choices=sorted(WORKTREES), default=None,
                        help="after a session in the project root, run the measured one in a Git worktree of the "
                             "project, inside it (the default) or beside it, or in one Codex makes (codex)")
    parser.add_argument("--app-server", action="store_true",
                        help="run the measured turn through codex app-server, as the Codex desktop app does")
    parser.add_argument("--cli", choices=("checkpoint",), default=None,
                        help="the stand-in first runs the memory CLI as the session was told to: checkpoint makes "
                             "a checkpoint as the project's checkpoint skill says")
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--work", help=argparse.SUPPRESS)
    parser.add_argument("--source", help=argparse.SUPPRESS)
    parser.add_argument("--packages", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inside:
        return inside(args)
    if sys.platform != "linux" or not shutil.which("unshare") or not shutil.which("ip"):
        print("codex_bench: needs Linux with unshare and ip (user and network namespaces)", file=sys.stderr)
        return 2
    args.codex = codex_binary(args.codex)
    if args.attached is not None and args.target:
        print("codex_bench: --attached runs a fresh project; it takes no --target", file=sys.stderr)
        return 2
    if args.queued is not None and args.attached is not None:
        print("codex_bench: --queued runs an installed edition in the terminal; it takes no --attached",
              file=sys.stderr)
        return 2
    if args.subfolder is not None and (args.attached is not None or args.sandbox != "workspace-write"):
        print("codex_bench: --subfolder runs Codex with the workspace-write sandbox in an installed project; "
              "it takes no --attached or --sandbox read-only", file=sys.stderr)
        return 2
    if args.subfolder is not None and (Path(args.subfolder).is_absolute() or ".." in Path(args.subfolder).parts):
        print("codex_bench: --subfolder names a folder inside the project", file=sys.stderr)
        return 2
    if args.worktree and (args.attached is not None or args.sandbox != "workspace-write"):
        print("codex_bench: --worktree runs Codex with the workspace-write sandbox in a worktree of an installed "
              "project; it takes no --attached or --sandbox read-only", file=sys.stderr)
        return 2
    if args.worktree == "codex" and args.queued is not None:
        print("codex_bench: --worktree codex lets `codex exec --worktree` or, with --app-server, the bench as the "
              "app make the worktree; it takes no --queued", file=sys.stderr)
        return 2
    if args.cli and (args.queued is not None or args.attached is not None or args.sandbox != "workspace-write"):
        print("codex_bench: --cli checkpoint makes a checkpoint before one answered turn of an installed edition; it "
              "takes no --queued, --attached or --sandbox read-only", file=sys.stderr)
        return 2
    if args.app_server and (args.queued is not None or args.attached is not None):
        print("codex_bench: --app-server runs one turn of an installed edition; it takes no --queued or "
              "--attached", file=sys.stderr)
        return 2
    if args.target:
        problem = bench.target_problem(Path(args.target).expanduser().resolve())
        if problem:
            print(f"codex_bench: {problem}", file=sys.stderr)
            return 2
    shm = Path("/dev/shm")
    base = shm if shm.is_dir() and os.access(shm, os.W_OK) else None
    if args.subfolder or args.worktree:
        # Codex's bubblewrap sandbox mounts its own /dev, and /tmp and $TMPDIR
        # are among its writable roots: below a project kept in /dev/shm or
        # /tmp a command would see none of it or could write all of it. On
        # disk, the project is read-only to a command started below its root
        # or in a worktree, as in a person's checkout.
        base = Path.home() / ".cache" / "ai-infrastructure-benches"
        base.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="codex-bench-", dir=str(base) if base else None))
    try:
        if args.target:
            project = bench.prepare_target(work, Path(args.target).expanduser().resolve(), args.sync,
                                           args.exclude, args.timeout)
        elif args.attached is not None:
            project, state = prepare_attached(work, args.edition)
            args.attached = str(state)
        else:
            project = bench.prepare(work, args.edition)
        done = subprocess.run(["unshare", "-rn", sys.executable, str(Path(__file__).resolve()), "--inside",
                               "--work", str(work), "--codex", args.codex, "--sandbox", args.sandbox,
                               "--source", bench.source_in(project), "--timeout", str(args.timeout),
                               "--edition", args.edition,
                               *(["--attached", args.attached] if args.attached else []),
                               *(["--subfolder", args.subfolder] if args.subfolder else []),
                               *(["--worktree", args.worktree] if args.worktree else []),
                               *(["--app-server"] if args.app_server else []),
                               *(["--cli", args.cli] if args.cli else []),
                               *(["--queued", str(args.queued), "--packages", str(Path.home() / ".codex/packages")]
                                 if args.queued is not None else [])],
                              timeout=args.timeout + 120)
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        return done.returncode
    finally:
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
