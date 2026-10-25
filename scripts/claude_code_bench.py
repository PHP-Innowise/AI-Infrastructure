#!/usr/bin/env python3
"""Run the installed Claude Code through one answered turn of an edition, offline.

The native save after every answer depends on what Claude Code does with a Stop
hook's output and on its permission check for the command the agent runs, and
both change between Claude Code releases. This drives the real binary through
one turn without a model: Claude Code runs inside a network namespace with
loopback only (`unshare -n`, so nothing can leave the machine and no tokens
are spent) against a scripted stand-in for the Messages API. The stand-in reads
a file, answers, and then follows the Stop hook's save step exactly as written,
with a JSON that holds backticks, `$`, brackets, apostrophes, escapes and
Cyrillic.

It reports what the host did: whether the save ran or Claude Code's
permission check refused it, how many messages carried the step to the model,
whether the transcript labelled it a hook error, and the journal's events.
Exit 0 when the save ran, the step reached the model once, no hook error was
recorded and the second Stop saw the recovery.

Linux only: user and network namespaces (`unshare`), and `ip` to raise the
loopback. Claude Code is the installed one unless `--claude` names a binary.
The namespace keeps the person's own uid where `unshare` can map it
(util-linux 2.38 and later): Claude Code refuses `bypassPermissions` to root.
`--mode` runs one permission mode; in `plan` the save is not asked for, and
the run passes when the journal says so.

`--queued SECONDS` runs Claude Code in its terminal instead, through a
pseudo-terminal: the stand-in takes SECONDS to write the save, and a second
question is typed while it does. Claude Code queues it into the running turn
and runs the prompt hook at once; the run passes when the save in flight
still lands and the screen never says the result was not saved.

`--subfolder DIR` runs a first session in the project root, whose
session-start hook sets up the dispatcher in the bench's own Claude Code
settings (memory-bank/scripts/claude_subfolders.py), then the measured
session in DIR below it: the run passes when that session got the capsule
and every line of the project's AGENTS.md in the conversation itself - not
in a file Claude Code keeps an over-long hook output in - and its save lands.

`--permissions` (with `--subfolder`, `src` by default) has the stand-in try
the same reads, writes, commands and fetches in both sessions - secrets the
project denies, files it allows, commands it denies or allows, the project's
env, a domain it allows and one it does not - and passes when the session
below the root got the decision the root got for every one (deny, ask or
ran) and printed the same env.

With `--target DIR` it runs in a copy of an installed project instead of a
fresh install. The copy leaves out dependency trees as the rollout
rehearsal's does, and `--sync` first brings it up to this clone. The saved
learning cites one of the project's own tracked files, and the project itself
is never written.

  python3 scripts/claude_code_bench.py
  python3 scripts/claude_code_bench.py --shell /usr/bin/zsh --edition Laravel --keep
  python3 scripts/claude_code_bench.py --mode dontAsk
  python3 scripts/claude_code_bench.py --queued 8
  python3 scripts/claude_code_bench.py --subfolder src
  python3 scripts/claude_code_bench.py --permissions --mode bypassPermissions
  python3 scripts/claude_code_bench.py --attached
  python3 scripts/claude_code_bench.py --target ~/Desktop/next --sync
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

ROOT = Path(__file__).resolve().parents[1]
# The installer's edition names; WordPress lives in Cms/wordpress.
EDITIONS = ("PHP Core", "Laravel", "Symfony", "WordPress")
GIT = ["git", "-c", "user.name=Bench", "-c", "user.email=bench@example.invalid", "-c", "commit.gpgsign=false",
       "-c", "core.hooksPath=/dev/null"]
KEY = "offline-bench-key-not-a-real-api-key"
MARKER = "INTERNAL_NATIVE_MEMORY_RESULT"
MODES = ("default", "acceptEdits", "auto", "dontAsk", "bypassPermissions", "plan")
ANSI = re.compile(rb"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][0-9A-Za-z]|\x1b[=>78]")
UNSAVED = "not saved completely"


def result_for(source: str) -> Dict[str, Any]:
    """What a model writes: code in backticks, a shell-looking `$`, the
    arrays' brackets, an apostrophe, escapes and another script."""
    return {
        "progress": f"Read `{source}`: `Invoice::next($tenant)` restarts per tenant; fee is $5.\n"
                    "Tab\there; path C:\\shop\\src; it's \"quoted\"; счёт.",
        "verified": True,
        "learnings": [{"type": "finding", "title": "Invoice numbers are per tenant",
                       "consequence": "Call `Invoice::next($tenant)`; never share one counter between tenants.",
                       "sources": [source]}],
    }


SAVE = re.compile(r"exact command: (cd .*?--json|\(cd .*?--json\))\n")


class Scripted(BaseHTTPRequestHandler):
    """The stand-in for the Messages API: Read, answer, save as told, done."""
    protocol_version = "HTTP/1.1"
    project = ""
    source = "src/Invoice.php"
    requests: List[Dict[str, Any]] = []
    lock = threading.Lock()
    # --queued: how long the save takes to write, and when it began.
    save_delay = 0.0
    save_begun = threading.Event()
    # --permissions: the tool calls to try, one a turn, before answering.
    probes: List[Dict[str, Any]] = []

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
        self.reply(404, {"type": "error", "error": {"type": "not_found_error", "message": "bench"}})

    def do_POST(self) -> None:
        body = json.loads(self.rfile.read(int(self.headers.get("content-length") or 0)) or b"{}")
        path = self.path.split("?")[0]
        if path.endswith("/count_tokens"):
            return self.reply(200, {"input_tokens": 1000})
        if not path.endswith("/v1/messages"):
            return self.reply(404, {"type": "error", "error": {"type": "not_found_error", "message": "bench"}})
        content, stop, since = self.decide(body)
        self.pause(content)
        with self.lock:
            self.requests.append({"since": since, "content": content})
            number = len(self.requests)
        if number > 30:
            content, stop = [{"type": "text", "text": "Loop guard."}], "end_turn"
        message = {"id": f"msg_bench_{number}", "type": "message", "role": "assistant",
                   "model": body.get("model", "bench"), "content": [], "stop_reason": None, "stop_sequence": None,
                   "usage": {"input_tokens": 1000, "output_tokens": 20, "cache_creation_input_tokens": 0,
                             "cache_read_input_tokens": 0}}
        if not body.get("stream"):
            return self.reply(200, dict(message, content=content, stop_reason=stop))
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()

        def event(kind: str, payload: Dict[str, Any]) -> None:
            self.wfile.write(f"event: {kind}\ndata: {json.dumps(payload)}\n\n".encode())

        event("message_start", {"type": "message_start", "message": message})
        for index, block in enumerate(content):
            if block["type"] == "text":
                start = {"type": "text", "text": ""}
                delta = {"type": "text_delta", "text": block["text"]}
            else:
                start = {"type": "tool_use", "id": block["id"], "name": block["name"], "input": {}}
                delta = {"type": "input_json_delta", "partial_json": json.dumps(block["input"])}
            event("content_block_start", {"type": "content_block_start", "index": index, "content_block": start})
            event("content_block_delta", {"type": "content_block_delta", "index": index, "delta": delta})
            event("content_block_stop", {"type": "content_block_stop", "index": index})
        event("message_delta", {"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": None},
                                "usage": {"output_tokens": 20}})
        event("message_stop", {"type": "message_stop"})
        self.wfile.flush()
        self.close_connection = True

    def pause(self, content: List[Dict[str, Any]]) -> None:
        """--queued: the first save takes save_delay seconds to write, as a
        model's JSON does; the time a person types the next question in."""
        if self.save_delay and content and content[0].get("id") == "toolu_save" and not self.save_begun.is_set():
            self.save_begun.set()
            time.sleep(self.save_delay)

    def decide(self, body: Dict[str, Any]):
        messages = body.get("messages") or []
        if not body.get("tools"):
            return [{"type": "text", "text": "Invoice numbering"}], "end_turn", []
        start = max((n for n, item in enumerate(messages) if item.get("role") == "assistant"), default=-1)
        since = messages[start + 1:]
        texts = [text_of(item.get("content")) for item in since]
        blocks = [block for item in since if isinstance(item.get("content"), list) for block in item["content"]]
        probes = getattr(self, "probes", None)
        if probes and MARKER not in "\n".join(texts):
            issued = sum(1 for item in messages if item.get("role") == "assistant"
                         and isinstance(item.get("content"), list) for block in item["content"]
                         if isinstance(block, dict) and str(block.get("id", "")).startswith("toolu_probe_"))
            if issued < len(probes):
                probe = probes[issued]
                return [{"type": "tool_use", "id": f"toolu_probe_{issued}", "name": probe["name"],
                         "input": probe["input"]}], "tool_use", texts
        if start >= 0 and any(block.get("type") == "tool_result" for block in blocks):
            used = [block for block in messages[start].get("content", [])
                    if isinstance(block, dict) and block.get("type") == "tool_use"]
            if used and used[-1]["name"] == "Bash":
                return [{"type": "text", "text": "Saved."}], "end_turn", texts
            return [{"type": "text", "text": f"Invoices are numbered per tenant ({self.source})."}], "end_turn", texts
        said = "\n".join(texts)
        if MARKER in said:
            found = SAVE.search(said)
            if found is None:
                return [{"type": "text", "text": "The step named no command."}], "end_turn", texts
            command, data = found.group(1), json.dumps(result_for(self.source), ensure_ascii=False)
            quoted = "'" + data.replace("'", "'\\''") + "'"
            command = (command.replace("'<JSON>'", quoted) if "'<JSON>'" in command
                       else f"echo -E {quoted} | {command}")
            return [{"type": "tool_use", "id": "toolu_save", "name": "Bash",
                     "input": {"command": command, "description": "Save the turn's memory result"}}], "tool_use", texts
        return [{"type": "tool_use", "id": "toolu_read", "name": "Read",
                 "input": {"file_path": os.path.join(self.project, self.source)}}], "tool_use", texts


def probe_calls(project: Path) -> List[Dict[str, Any]]:
    """--permissions: what the stand-in tries, against the editions' rules."""
    root = str(project)

    def read(relative: str) -> Dict[str, Any]:
        return {"name": "Read", "input": {"file_path": f"{root}/{relative}"}}

    def bash(command: str) -> Dict[str, Any]:
        return {"name": "Bash", "input": {"command": command, "description": "Probe"}}

    return [read(".env"), read("src/.env"), read(".env.local"), read("secrets/key.txt"),
            read("src/secrets/key.txt"), read("composer.json"), read("src/Invoice.php"),
            {"name": "Read", "input": {"file_path": "/etc/hostname"}},
            {"name": "Write", "input": {"file_path": f"{root}/notes.txt", "content": "Notes.\n"}},
            # A new file: Claude Code asks to read an existing one before
            # writing it, in some sessions before the permission check.
            {"name": "Write", "input": {"file_path": f"{root}/.env.probe", "content": "PROBE=1\n"}},
            bash("rm -rf build"), bash("sudo -n true"), bash("chmod 777 notes.txt"), bash("ls"),
            bash("git status --short"), bash("echo probe && rm -rf build"),
            # The project's env: where both sessions run it, both print the same.
            dict(bash("printenv APP_ENV FORCE_COLOR"), same_output=True),
            {"name": "WebFetch", "input": {"url": "https://github.com/", "prompt": "Title?"}},
            {"name": "WebFetch", "input": {"url": "https://example.com/", "prompt": "Title?"}}]


def verdict(text: str) -> str:
    """deny, ask or ran, from what Claude Code answered a probe with."""
    if re.search(r"denied by your permission|has been denied|project's settings deny|covered by a Read deny", text):
        return "deny"
    if re.search(r"haven't granted|requires approval", text):
        return "ask"
    return "ran"


def probe_session(args: argparse.Namespace, cwd: Path, env: Dict[str, str], mode: List[str]) -> List[str]:
    """Each probe's verdict; a probe marked same_output carries its output."""
    Scripted.requests.clear()  # each session has the stand-in's loop guard to itself
    done = subprocess.run([args.claude, "-p", "Try the probes.", "--output-format", "stream-json", "--verbose", *mode],
                          cwd=cwd, env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True,
                          timeout=args.timeout)
    found: Dict[int, str] = {}
    for line in done.stdout.splitlines():
        try:
            item = json.loads(line)
        except ValueError:
            continue
        message = item.get("message") if isinstance(item, dict) else None
        for block in message.get("content") if isinstance(message, dict) and isinstance(message.get("content"), list) else []:
            if isinstance(block, dict) and block.get("type") == "tool_result" \
                    and str(block.get("tool_use_id", "")).startswith("toolu_probe_"):
                index = int(block["tool_use_id"].rsplit("_", 1)[1])
                found[index] = verdict(text_of([block]))
                if found[index] == "ran" and index < len(Scripted.probes) and Scripted.probes[index].get("same_output"):
                    found[index] += ": " + " ".join(text_of([block]).split())
    return [found.get(index, "missing") for index in range(len(Scripted.probes))]


def permission_run(args: argparse.Namespace, work: Path, project: Path, env: Dict[str, str], mode: List[str]) -> int:
    """--permissions: the same probes in the root and below it."""
    for relative, text in ((".env", "A=1\n"), ("src/.env", "B=2\n"), (".env.local", "C=3\n"),
                           ("secrets/key.txt", "key\n"), ("src/secrets/key.txt", "key\n")):
        (project / relative).parent.mkdir(parents=True, exist_ok=True)
        (project / relative).write_text(text, encoding="utf-8")
    Scripted.probes = probe_calls(project)
    in_root = probe_session(args, project, env, mode)
    listed = work / "home/.claude/ai-accelerator/projects"
    deadline = time.time() + 15
    while time.time() < deadline and str(project) not in (listed.read_text().splitlines() if listed.exists() else []):
        time.sleep(0.2)
    (project / "notes.txt").unlink(missing_ok=True)
    below_cwd = project / args.subfolder
    below_cwd.mkdir(parents=True, exist_ok=True)
    below = probe_session(args, below_cwd, env, mode)
    labels = [f"{probe['name']} {probe['input'].get('file_path') or probe['input'].get('command') or probe['input'].get('url')}"
              .replace(str(project), "<root>") for probe in Scripted.probes]
    differ = [f"{label}: root {first}, {args.subfolder} {second}"
              for label, first, second in zip(labels, in_root, below) if first != second]
    report = {"claude": args.claude, "version": version(args.claude), "mode": args.mode, "started_in": args.subfolder,
              "probes": dict(zip(labels, in_root)), "differ": differ,
              "ok": not differ and "missing" not in in_root + below}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


def text_of(content: Any) -> str:
    if isinstance(content, str):
        return content
    parts = []
    for block in content or []:
        if block.get("type") == "text":
            parts.append(block.get("text", ""))
        elif block.get("type") == "tool_result":
            parts.append(text_of(block.get("content")) if not isinstance(block.get("content"), str)
                         else block["content"])
    return "\n".join(parts)


def shop(work: Path) -> Path:
    """A small PHP project in its own Git repository, with nothing installed."""
    project = work / "project"
    (project / "src").mkdir(parents=True)
    (work / "home").mkdir()
    (project / "composer.json").write_text('{"name": "acme/shop"}\n', encoding="utf-8")
    (project / "src/Invoice.php").write_text("<?php\nfinal class Invoice {}\n", encoding="utf-8")
    env = clean_env()
    for args in (["init", "-q", "-b", "feature/SHOP-7-invoices"], ["add", "."], ["commit", "-qm", "Shop baseline"]):
        subprocess.run([*GIT, *args], cwd=project, env=env, check=True, capture_output=True)
    return project


def prepare(work: Path, edition: str) -> Path:
    project = shop(work)
    env = clean_env()
    done = subprocess.run([sys.executable, str(ROOT / "scripts/install_accelerator.py"), "--edition", edition,
                           "--target", str(project), "--merge-existing"], cwd=ROOT, env=env,
                          capture_output=True, text=True)
    if done.returncode:
        raise SystemExit(f"install failed: {done.stdout[-600:]}{done.stderr[-600:]}")
    return project


def prepare_attached(work: Path, edition: str) -> Tuple[Path, Path]:
    """A project with nothing installed and the edition lent from this clone,
    as the Harness launches it: the overlay goes to `work/overlay.json`, the
    accelerator's state to a directory outside the project."""
    sys.path.insert(0, str(ROOT / "scripts"))
    import accelerator_attach as attach
    project = shop(work)
    state = attach.state_directory(project, work / "state-base")
    overlay = attach.claude_overlay(edition, project, state)
    (work / "overlay.json").write_text(json.dumps({"settings": overlay.settings, "arguments": overlay.arguments,
                                                   "environment": overlay.environment}), encoding="utf-8")
    return project, state


def prepare_target(work: Path, target: Path, sync: bool, excludes: List[str], timeout: float) -> Path:
    """A copy of an installed project, synced to this clone when asked."""
    sys.path.insert(0, str(ROOT / "scripts"))
    import memory_rollout_rehearsal as rehearsal
    project = work / "project"
    rehearsal.copy_tree(target, project, [*rehearsal.DEFAULT_EXCLUDES, *excludes])
    (work / "home").mkdir()
    if sync:
        report = rehearsal.sync(project, clean_env(), timeout * 10)
        if report.get("error"):
            raise SystemExit(f"sync of the copy failed: {report['error']}")
    return project


def target_problem(target: Path) -> Optional[str]:
    """Why a target is not run, as the rollout rehearsal refuses it."""
    if not (target / "memory-bank/scripts/context.py").is_file():
        return f"{target} has no installed accelerator (memory-bank/scripts/context.py)"
    if (target / ".git").is_file():
        # A worktree or a submodule: the copy would share its Git directory.
        return f"{target} is a Git worktree or submodule; run its main checkout"
    for one, other in ((target, ROOT), (ROOT, target)):
        try:
            one.relative_to(other)
            return f"{target} is this clone or holds it; run an installed project"
        except ValueError:
            pass
    return None


def trust(work: Path, project: Path, accepted: bool = True, terminal: bool = False) -> None:
    # A person who opened the project once and accepted its trust dialog:
    # until then Claude Code ignores the project's allow rules.
    config: Dict[str, Any] = {"hasCompletedOnboarding": True,
                              "projects": {str(project): {"hasTrustDialogAccepted": accepted}},
                              "customApiKeyResponses": {"approved": [KEY[-20:]], "rejected": []}}
    if terminal:
        # The terminal asks for a theme and a project's onboarding once.
        config["theme"] = "dark"
        config["projects"][str(project)]["hasCompletedProjectOnboarding"] = True
    (work / "home/.claude.json").write_text(json.dumps(config), encoding="utf-8")


def source_in(project: Path) -> str:
    """The tracked project file the saved learning cites and the stand-in reads."""
    sys.path.insert(0, str(ROOT / "scripts"))
    import memory_rollout_rehearsal as rehearsal
    return rehearsal.cited_source(project, clean_env()) or "composer.json"


def clean_env() -> Dict[str, str]:
    # A run from a Claude Code or Codex session inherits that session's marks.
    return {key: value for key, value in os.environ.items()
            if not key.startswith(("GIT_", "CONTEXT_", "ACCELERATOR_", "CLAUDE", "CODEX", "ANTHROPIC"))}


def terminal(argv: List[str], project: Path, env: Dict[str, str], journal: Path, wait: float,
             begun: threading.Event, requests: List[Any], settle: float = 6.0) -> str:
    """A host in its terminal: a question, then a second one once the
    stand-in has begun writing the save (`begun`); what the screen showed,
    as text. A question the stand-in never hears of (`requests` stays as it
    was) - the terminal was not ready, or a notice took the keys - is typed
    again. Also run by codex_bench.py."""
    import fcntl
    import pty
    import select
    import signal
    import struct
    import termios
    env = dict(env, TERM="xterm-256color", COLUMNS="160", LINES="50")
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(project)
        os.execve(argv[0], argv, env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 160, 0, 0))
    screen = bytearray()

    def pump(seconds: float) -> bool:
        """Read the screen for `seconds`; False once the terminal has closed."""
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([fd], [], [], 0.2)
            if not ready:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return False
            if not chunk:
                return False
            screen.extend(chunk)
        return True

    def type_line(text: str) -> None:
        for character in text:
            os.write(fd, character.encode())
            time.sleep(0.01)
        time.sleep(0.3)
        os.write(fd, b"\r")

    try:
        pump(settle)
        for _ in range(3):
            before = len(requests)
            type_line("How are invoices numbered?")
            heard = time.time() + 15
            while len(requests) == before and time.time() < heard and pump(0.5):
                pass
            if len(requests) > before:
                break
        deadline = time.time() + wait
        while not begun.is_set() and time.time() < deadline and pump(0.5):
            pass
        pump(1)
        type_line("And how are credit notes numbered?")
        # The turn ends with the continuation's Stop, after the save.
        while time.time() < deadline and pump(1):
            lines = journal.read_text().splitlines() if journal.exists() else []
            if any('"stage":"stop"' in line and '"recovery-' in line for line in lines):
                break
        if pump(2):
            for _ in range(2):
                os.write(fd, b"\x03")
                if not pump(1):
                    break
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
            os.waitpid(pid, 0)
        except (ProcessLookupError, ChildProcessError):
            pass
    return ANSI.sub(b"", bytes(screen)).decode("utf-8", "replace")


def queued_ok(events: List[str], screen: str) -> bool:
    """The save in flight when the second question came still landed, and
    nothing told the agent or the person it was not saved."""
    landed = any(event in ("save:saved", "save:saved:in-flight", "save:saved:spooled") for event in events)
    return landed and not any("request-unavailable" in event for event in events) and UNSAVED not in screen


def inside(args: argparse.Namespace) -> int:
    """In the namespace: raise the loopback, serve, run one turn, report."""
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    work = Path(args.work)
    project = work / "project"
    Scripted.project, Scripted.source = str(project), args.source
    server = ThreadingHTTPServer(("127.0.0.1", 0), Scripted)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    env = clean_env()
    env.update(HOME=str(work / "home"), TMPDIR=str(work / "tmp"), SHELL=args.shell,
               ANTHROPIC_BASE_URL=f"http://127.0.0.1:{server.server_address[1]}", ANTHROPIC_API_KEY=KEY,
               CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1", DISABLE_AUTOUPDATER="1", DISABLE_TELEMETRY="1")
    (work / "tmp").mkdir(exist_ok=True)
    mode = ["--permission-mode", args.mode] if args.mode != "default" else []
    lent: List[str] = []
    if args.attached:
        # The Harness's launch: one --settings with the edition's permissions
        # and hooks, the edition as an added directory, its roots in the env.
        overlay = json.loads((work / "overlay.json").read_text(encoding="utf-8"))
        env.update(overlay["environment"])
        lent = ["--settings", json.dumps(overlay["settings"]), *overlay["arguments"]]
    if args.permissions:
        return permission_run(args, work, project, env, mode)
    journal_file = (Path(args.attached) if args.attached else project) / "memory-bank/local/native-results.ndjson"
    screen = ""
    first_lines = 0
    if args.subfolder:
        # A session in the project root first: its session-start hook sets up
        # the dispatcher (in the background), which the session below needs.
        subprocess.run([args.claude, "-p", "How are invoices numbered?", "--output-format", "stream-json", "--verbose",
                        *mode, *lent], cwd=project, env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True,
                       timeout=args.timeout)
        listed = work / "home/.claude/ai-accelerator/projects"
        deadline = time.time() + 15
        while time.time() < deadline and str(project) not in (listed.read_text().splitlines() if listed.exists() else []):
            time.sleep(0.2)
        first_lines = len(journal_file.read_text().splitlines()) if journal_file.exists() else 0
        Scripted.requests.clear()
        project_cwd = project / args.subfolder
        project_cwd.mkdir(parents=True, exist_ok=True)
    else:
        project_cwd = project
    if args.queued is not None:
        Scripted.save_delay = args.queued
        screen = terminal([args.claude, *mode, *lent], project, env, journal_file, args.timeout, Scripted.save_begun,
                          Scripted.requests)
        (work / "screen.txt").write_text(screen, encoding="utf-8")
        done = subprocess.CompletedProcess([], 0, "", "")
    else:
        done = subprocess.run([args.claude, "-p", "How are invoices numbered?", "--output-format", "stream-json",
                               "--verbose", *mode, *lent], cwd=project_cwd, env=env, stdin=subprocess.DEVNULL,
                              capture_output=True, text=True, timeout=args.timeout)
    server.shutdown()
    stream = [json.loads(line) for line in done.stdout.splitlines() if line.startswith("{")]
    steps = [request for request in Scripted.requests if any(MARKER in text for text in request["since"])]
    carried = sum(MARKER in text for text in steps[0]["since"]) if steps else 0
    save = next((request for request in Scripted.requests
                 if request["content"] and request["content"][0].get("text") == "Saved."), None)
    result = text_of(save["since"][0]) if save and save["since"] else ""
    journal = [json.loads(line) for line in journal_file.read_text().splitlines()[first_lines:]] if journal_file.exists() else []
    events = [f"{line['stage']}:{line['event']}" + (f":{line['code']}" if line.get("code") else "")
              for line in journal if line.get("stage") in ("stop", "save")
              or (args.queued is not None and line.get("stage") == "prompt")]
    transcript = sorted((work / "home/.claude/projects").glob("*/*.jsonl"))
    records = [json.loads(line) for path in transcript for line in path.read_text().splitlines() if line.strip()]
    errors = [error for record in records if record.get("subtype") == "stop_hook_summary"
              for error in record.get("hookErrors") or []]
    report = {
        "claude": args.claude, "version": version(args.claude), "shell": args.shell, "mode": args.mode,
        "exit": done.returncode,
        "step_messages": carried,
        "step_chars": sum(len(text) for text in steps[0]["since"]) if steps else 0,
        "save_result": result[:200],
        "saved": "stop:skipped:recovery-saved" in events,
        "events": events,
        "hook_errors": len(errors),
        "error_notifications": sum(item.get("subtype") == "notification"
                                   and "error" in str(item.get("key", "")) for item in stream),
    }
    report["ok"] = (report["saved"] and carried == 1 and not errors and not report["error_notifications"])
    if args.mode == "plan":
        # Plan mode tells the agent to change nothing: no save is asked for.
        report["ok"] = (not steps and "stop:skipped:plan-mode" in events and not errors
                        and not report["error_notifications"])
    if args.queued is not None:
        report["unsaved_on_screen"] = UNSAVED in screen
        report["ok"] = queued_ok(events, screen) and not errors
    if args.subfolder:
        # The session below the root got the capsule and the policy, and saved.
        report["started_in"] = args.subfolder
        heard = "\n".join(text for request in Scripted.requests for text in request["since"])
        report["capsule"] = "working:" in heard
        policy = (project / "AGENTS.md").read_text(encoding="utf-8").splitlines()
        report["policy"] = (all(line in heard for line in policy if line.strip())
                            and "Output too large" not in heard)
        report["ok"] = report["saved"] and report["capsule"] and report["policy"] and not errors
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


def person_trust(target: Path) -> Optional[bool]:
    """Whether your own Claude Code accepted the trust dialog for this project.

    The bench runs in a HOME of its own that trusts the copy, so it cannot see
    this. Without that trust, `claude -p` in the project runs the hooks but
    ignores the project's allow rules and refuses the save. None when the
    configuration cannot be read.
    """
    folder = os.environ.get("CLAUDE_CONFIG_DIR")
    config = Path(folder) / ".claude.json" if folder else Path.home() / ".claude.json"
    try:
        data = json.loads(config.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    projects = data.get("projects") if isinstance(data, dict) else None
    entry = projects.get(str(target)) if isinstance(projects, dict) else None
    return isinstance(entry, dict) and entry.get("hasTrustDialogAccepted") is True


def namespace(unshare_help: str) -> List[str]:
    """The namespace the bench runs in: loopback only, and the person's own uid
    where `unshare` can map it, since Claude Code refuses bypassPermissions to root."""
    if "--map-user" in unshare_help:
        return ["unshare", "-n", "--user", f"--map-user={os.getuid()}", f"--map-group={os.getgid()}",
                "--keep-caps"]
    return ["unshare", "-rn"]


def version(claude: str) -> str:
    try:
        return subprocess.run([claude, "--version"], capture_output=True, text=True, timeout=30).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--claude", default=shutil.which("claude") or "claude")
    parser.add_argument("--edition", choices=sorted(EDITIONS), default="PHP Core")
    parser.add_argument("--shell", default=shutil.which("bash") or "/bin/bash",
                        help="the shell Claude Code's Bash tool runs (bash or zsh)")
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument("--mode", choices=MODES, default="default", help="the permission mode to run")
    parser.add_argument("--subfolder", metavar="DIR", default=None,
                        help="after a session in the project root, run the measured one in DIR below it")
    parser.add_argument("--permissions", action="store_true",
                        help="try the same tool calls in the root and below it, and compare the decisions")
    parser.add_argument("--queued", type=float, default=None, metavar="SECONDS",
                        help="in Claude Code's terminal, type a second question while the save, which the "
                             "stand-in takes SECONDS to write, is in flight")
    parser.add_argument("--attached", nargs="?", const="", default=None, metavar="",
                        help="lend the edition from this clone as the Harness does; install nothing")
    parser.add_argument("--target", help="run in a copy of this installed project instead of a fresh install")
    parser.add_argument("--sync", action="store_true", help="sync the copy from this clone first (with --target)")
    parser.add_argument("--exclude", action="append", default=[], help="another directory name to leave out of the copy")
    parser.add_argument("--source", help=argparse.SUPPRESS)
    parser.add_argument("--keep", action="store_true", help="keep the work directory and print its path")
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--work", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inside:
        return inside(args)
    if sys.platform != "linux" or not shutil.which("unshare") or not shutil.which("ip"):
        print("claude_code_bench: needs Linux with unshare and ip (user and network namespaces)", file=sys.stderr)
        return 2
    args.claude = os.path.realpath(args.claude)
    if args.attached is not None and args.target:
        print("claude_code_bench: --attached runs a fresh project; it takes no --target", file=sys.stderr)
        return 2
    if args.queued is not None and args.mode == "plan":
        print("claude_code_bench: plan mode asks for no save, so there is none to type a question during; "
              "--queued takes no --mode plan", file=sys.stderr)
        return 2
    if args.permissions and args.subfolder is None:
        args.subfolder = "src"
    if args.permissions and args.target:
        print("claude_code_bench: --permissions writes probe files into the project; it takes no --target",
              file=sys.stderr)
        return 2
    if args.subfolder is not None and (args.queued is not None or args.attached is not None or args.mode == "plan"):
        print("claude_code_bench: --subfolder runs two sessions with -p in an installed project and expects a "
              "save; it takes no --queued, --attached or --mode plan", file=sys.stderr)
        return 2
    if args.subfolder is not None and (Path(args.subfolder).is_absolute() or ".." in Path(args.subfolder).parts):
        print("claude_code_bench: --subfolder names a folder inside the project", file=sys.stderr)
        return 2
    if args.target:
        problem = target_problem(Path(args.target).expanduser().resolve())
        if problem:
            print(f"claude_code_bench: {problem}", file=sys.stderr)
            return 2
    shm = Path("/dev/shm")
    base = shm if shm.is_dir() and os.access(shm, os.W_OK) else None
    work = Path(tempfile.mkdtemp(prefix="claude-code-bench-", dir=str(base) if base else None))
    try:
        if args.target:
            target = Path(args.target).expanduser().resolve()
            if person_trust(target) is False:
                print(f"claude_code_bench: your Claude Code has not accepted the trust dialog for {target}; "
                      "there `claude -p` ignores the project's allow rules and refuses the save. Open the "
                      "project once in interactive Claude Code and accept it.", file=sys.stderr)
            project = prepare_target(work, target, args.sync, args.exclude, args.timeout)
        elif args.attached is not None:
            project, state = prepare_attached(work, args.edition)
            args.attached = str(state)
        else:
            project = prepare(work, args.edition)
        # Attached, the project is left untrusted: the Harness's --settings
        # permissions count without the trust dialog, project ones would not.
        trust(work, project, accepted=args.attached is None, terminal=args.queued is not None)
        unshare_help = subprocess.run(["unshare", "--help"], capture_output=True, text=True).stdout
        done = subprocess.run([*namespace(unshare_help), sys.executable, str(Path(__file__).resolve()),
                               "--inside", "--work", str(work), "--claude", args.claude, "--shell", args.shell,
                               "--mode", args.mode, "--source", source_in(project),
                               *(["--attached", args.attached] if args.attached else []),
                               *(["--queued", str(args.queued)] if args.queued is not None else []),
                               *(["--subfolder", args.subfolder] if args.subfolder else []),
                               *(["--permissions"] if args.permissions else []),
                               "--timeout", str(args.timeout)],
                              timeout=args.timeout + 60)
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        return done.returncode
    finally:
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
