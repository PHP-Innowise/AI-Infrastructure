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
`claude_code_bench.py` does.

  python3 scripts/codex_bench.py
  python3 scripts/codex_bench.py --target ~/Desktop/next --sync
"""
import argparse
import html
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
from typing import Any, Dict, List

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import claude_code_bench as bench  # noqa: E402

MARKER = bench.MARKER
SAVE = bench.SAVE


class Scripted(BaseHTTPRequestHandler):
    """The stand-in for the Responses API: read, answer, save as told, done."""
    protocol_version = "HTTP/1.1"
    source = "composer.json"
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
        for index, item in enumerate(output):
            event("response.output_item.added", {"output_index": index, "item": item})
            if item["type"] == "message":
                event("response.output_text.delta", {"item_id": item["id"], "output_index": index,
                                                     "content_index": 0, "delta": item["content"][0]["text"]})
            event("response.output_item.done", {"output_index": index, "item": item})
        event("response.completed", {"response": response})
        self.wfile.flush()
        self.close_connection = True

    def decide(self, body: Dict[str, Any]):
        items = body.get("input") or []
        tools = body.get("tools") or []
        last = max((n for n, item in enumerate(items) if item.get("type") in ("function_call", "custom_tool_call")
                    or item.get("role") == "assistant"), default=-1)
        since = [text_of(item) for item in items[last + 1:]]
        number = len(self.requests) + 1
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
        return [shell_call(tools, f"cat {self.source}", f"call_read_{number}")], since


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
    server = ThreadingHTTPServer(("127.0.0.1", 0), Scripted)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    (home / "config.toml").write_text(
        'model = "gpt-5.1-codex"\nmodel_provider = "bench"\napproval_policy = "never"\n\n'
        '[model_providers.bench]\nname = "bench"\n'
        f'base_url = "http://127.0.0.1:{server.server_address[1]}/v1"\n'
        'wire_api = "responses"\nenv_key = "BENCH_API_KEY"\n\n'
        f'[projects.{json.dumps(str(project))}]\ntrust_level = "trusted"\n', encoding="utf-8")
    env = bench.clean_env()
    env.update(HOME=str(work / "home"), CODEX_HOME=str(home), TMPDIR=str(work / "tmp"), BENCH_API_KEY="offline")
    (work / "tmp").mkdir(exist_ok=True)
    os.environ.update(HOME=str(work / "home"), CODEX_HOME=str(home))
    try:
        trust = approve_hooks(args.codex, project)
    except Exception as error:  # the run below shows whether hooks ran without it
        trust = {"error": f"{type(error).__name__}: {error}"}
    done = subprocess.run([args.codex, "exec", "-C", str(project), "--skip-git-repo-check", "--sandbox",
                           args.sandbox, "How are invoices numbered?"], cwd=project, env=env,
                          stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=args.timeout)
    server.shutdown()
    steps = [request for request in Scripted.requests if any(MARKER in text for text in request["since"])]
    carried = [text for text in steps[0]["since"] if MARKER in text] if steps else []
    journal_file = project / "memory-bank/local/native-results.ndjson"
    journal = [json.loads(line) for line in journal_file.read_text().splitlines()] if journal_file.exists() else []
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
    }
    report["ok"] = report["saved"] and len(carried) == 1
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


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
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--work", help=argparse.SUPPRESS)
    parser.add_argument("--source", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inside:
        return inside(args)
    if sys.platform != "linux" or not shutil.which("unshare") or not shutil.which("ip"):
        print("codex_bench: needs Linux with unshare and ip (user and network namespaces)", file=sys.stderr)
        return 2
    args.codex = codex_binary(args.codex)
    if args.target:
        problem = bench.target_problem(Path(args.target).expanduser().resolve())
        if problem:
            print(f"codex_bench: {problem}", file=sys.stderr)
            return 2
    shm = Path("/dev/shm")
    base = shm if shm.is_dir() and os.access(shm, os.W_OK) else None
    work = Path(tempfile.mkdtemp(prefix="codex-bench-", dir=str(base) if base else None))
    try:
        if args.target:
            project = bench.prepare_target(work, Path(args.target).expanduser().resolve(), args.sync,
                                           args.exclude, args.timeout)
        else:
            project = bench.prepare(work, args.edition)
        done = subprocess.run(["unshare", "-rn", sys.executable, str(Path(__file__).resolve()), "--inside",
                               "--work", str(work), "--codex", args.codex, "--sandbox", args.sandbox,
                               "--source", bench.source_in(project), "--timeout", str(args.timeout)],
                              timeout=args.timeout + 120)
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        return done.returncode
    finally:
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
