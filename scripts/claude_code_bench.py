#!/usr/bin/env python3
"""Run the installed Claude Code through one answered turn of an edition, offline.

The native save after every answer depends on what Claude Code does with a Stop
hook's output and on its permission check for the command the agent runs, and
both change between Claude Code releases. This drives the real binary through
one turn without a model: Claude Code runs inside a network namespace with
loopback only (`unshare -rn`, so nothing can leave the machine and no tokens
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

With `--target DIR` it runs in a copy of an installed project instead of a
fresh install. The copy leaves out dependency trees as the rollout
rehearsal's does, and `--sync` first brings it up to this clone. The saved
learning cites one of the project's own tracked files, and the project itself
is never written.

  python3 scripts/claude_code_bench.py
  python3 scripts/claude_code_bench.py --shell /usr/bin/zsh --edition Laravel --keep
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
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Dict, List, Optional

ROOT = Path(__file__).resolve().parents[1]
EDITIONS = {"PHP Core": "PHP Core", "Laravel": "Laravel", "Symfony": "Symfony", "WordPress": "Cms/wordpress"}
GIT = ["git", "-c", "user.name=Bench", "-c", "user.email=bench@example.invalid", "-c", "commit.gpgsign=false",
       "-c", "core.hooksPath=/dev/null"]
KEY = "offline-bench-key-not-a-real-api-key"
MARKER = "INTERNAL_NATIVE_MEMORY_RESULT"


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

    def decide(self, body: Dict[str, Any]):
        messages = body.get("messages") or []
        if not body.get("tools"):
            return [{"type": "text", "text": "Invoice numbering"}], "end_turn", []
        start = max((n for n, item in enumerate(messages) if item.get("role") == "assistant"), default=-1)
        since = messages[start + 1:]
        texts = [text_of(item.get("content")) for item in since]
        blocks = [block for item in since if isinstance(item.get("content"), list) for block in item["content"]]
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


def prepare(work: Path, edition: str) -> Path:
    project = work / "project"
    (project / "src").mkdir(parents=True)
    (work / "home").mkdir()
    (project / "composer.json").write_text('{"name": "acme/shop"}\n', encoding="utf-8")
    (project / "src/Invoice.php").write_text("<?php\nfinal class Invoice {}\n", encoding="utf-8")
    env = clean_env()
    for args in (["init", "-q", "-b", "feature/SHOP-7-invoices"], ["add", "."], ["commit", "-qm", "Shop baseline"]):
        subprocess.run([*GIT, *args], cwd=project, env=env, check=True, capture_output=True)
    done = subprocess.run([sys.executable, str(ROOT / "scripts/install_accelerator.py"), "--edition", edition,
                           "--target", str(project), "--merge-existing"], cwd=ROOT, env=env,
                          capture_output=True, text=True)
    if done.returncode:
        raise SystemExit(f"install failed: {done.stdout[-600:]}{done.stderr[-600:]}")
    return project


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


def trust(work: Path, project: Path) -> None:
    # A person who opened the project once and accepted its trust dialog:
    # until then Claude Code ignores the project's allow rules.
    config = {"hasCompletedOnboarding": True, "projects": {str(project): {"hasTrustDialogAccepted": True}},
              "customApiKeyResponses": {"approved": [KEY[-20:]], "rejected": []}}
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
    done = subprocess.run([args.claude, "-p", "How are invoices numbered?", "--output-format", "stream-json",
                           "--verbose"], cwd=project, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                          text=True, timeout=args.timeout)
    server.shutdown()
    stream = [json.loads(line) for line in done.stdout.splitlines() if line.startswith("{")]
    steps = [request for request in Scripted.requests if any(MARKER in text for text in request["since"])]
    carried = sum(MARKER in text for text in steps[0]["since"]) if steps else 0
    save = next((request for request in Scripted.requests
                 if request["content"] and request["content"][0].get("text") == "Saved."), None)
    result = text_of(save["since"][0]) if save and save["since"] else ""
    journal_file = project / "memory-bank/local/native-results.ndjson"
    journal = [json.loads(line) for line in journal_file.read_text().splitlines()] if journal_file.exists() else []
    events = [f"{line['stage']}:{line['event']}" + (f":{line['code']}" if line.get("code") else "")
              for line in journal if line.get("stage") in ("stop", "save")]
    transcript = sorted((work / "home/.claude/projects").glob("*/*.jsonl"))
    records = [json.loads(line) for path in transcript for line in path.read_text().splitlines() if line.strip()]
    errors = [error for record in records if record.get("subtype") == "stop_hook_summary"
              for error in record.get("hookErrors") or []]
    report = {
        "claude": args.claude, "version": version(args.claude), "shell": args.shell, "exit": done.returncode,
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
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


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
            project = prepare_target(work, Path(args.target).expanduser().resolve(), args.sync, args.exclude,
                                     args.timeout)
        else:
            project = prepare(work, EDITIONS[args.edition])
        trust(work, project)
        done = subprocess.run(["unshare", "-rn", sys.executable, str(Path(__file__).resolve()), "--inside",
                               "--work", str(work), "--claude", args.claude, "--shell", args.shell,
                               "--source", source_in(project), "--timeout", str(args.timeout)],
                              timeout=args.timeout + 60)
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        return done.returncode
    finally:
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
