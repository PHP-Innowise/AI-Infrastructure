"""Native Windows acceptance checks for the optional Harness runtime.

These tests deliberately use Git Bash, a real Node executable, Windows
junctions and the Windows process runtime.  They do not mock Windows APIs or
run a batch launcher through ``cmd.exe``.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from urllib.request import ProxyHandler, Request, build_opener


ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = ROOT / "harness-server"
sys.path.insert(0, str(ROOT / "harness" / "src"))
from harness import windows_commands


@unittest.skipUnless(os.name == "nt", "native Windows acceptance coverage")
class WindowsHarnessAcceptanceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        git = shutil.which("git")
        bundled = Path(git).parents[1] / "bin/bash.exe" if git else None
        self.bash = str(bundled) if bundled and bundled.is_file() else shutil.which("bash")
        self.assertIsNotNone(self.bash, "windows-latest must provide Git Bash")
        self.state = self.root / "state folder"
        self.project = self.root / "project folder"
        self.project.mkdir()
        self.info = None

    def launch(self, *args, env=None, timeout=40):
        return subprocess.run([self.bash, str(LAUNCHER), *map(str, args)], cwd=self.root,
                              env={**os.environ, **(env or {})}, capture_output=True,
                              text=True, check=False, timeout=timeout)

    def start(self, *extra, env=None):
        completed = self.launch("start", "--state-dir", self.state, "--project", self.project,
                                "--port", "0", *extra, env=env)
        self.assertEqual(completed.returncode, 0, completed.stderr + completed.stdout)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            metadata = self.state / "server.json"
            if metadata.exists():
                self.info = json.loads(metadata.read_text(encoding="utf-8"))
                return
            time.sleep(.05)
        self.fail("native Harness server did not publish its state")

    def stop(self):
        if self.info is not None:
            completed = self.launch("stop", "--state-dir", self.state)
            self.assertEqual(completed.returncode, 0, completed.stderr + completed.stdout)
            self.info = None

    def tearDown(self):
        self.stop()

    def api(self, path, body=None):
        headers = {"Host": f"127.0.0.1:{self.info['port']}"}
        data = None
        if body is not None:
            headers.update({"Content-Type": "application/json", "X-Harness-Token": self.info["token"]})
            data = json.dumps(body).encode()
        request = Request(f"http://127.0.0.1:{self.info['port']}{path}", data=data, headers=headers)
        with build_opener(ProxyHandler({})).open(request, timeout=5) as response:
            return json.load(response)

    def wait_for(self, predicate, message):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(.05)
        self.fail(message)

    def test_git_bash_starts_stops_and_refuses_a_second_state_owner(self):
        self.start()
        self.assertTrue(self.api("/api/health")["ok"])
        competing = self.launch("serve", "--state-dir", self.state, "--project", self.project,
                                "--port", "0", timeout=10)
        self.assertEqual(competing.returncode, 1, competing.stderr + competing.stdout)
        self.assertIn("already owns this state directory", competing.stderr)
        self.stop()
        status = self.launch("status", "--state-dir", self.state)
        self.assertEqual(status.returncode, 1)
        self.assertIn("Stopped", status.stdout)

    def test_launcher_refuses_junction_state_without_touching_external_sentinel(self):
        outside = self.root / "outside"
        outside.mkdir()
        sentinel = outside / "sentinel.txt"
        sentinel.write_text("EXTERNAL SENTINEL", encoding="utf-8")
        junction = self.root / "junction-state"
        linked = subprocess.run(["cmd", "/d", "/c", "mklink", "/J", str(junction), str(outside)],
                                capture_output=True, text=True, check=False)
        self.assertEqual(linked.returncode, 0, linked.stderr)
        completed = self.launch("start", "--state-dir", junction, "--project", self.project, "--port", "0")
        self.assertEqual(completed.returncode, 1)
        self.assertNotIn("EXTERNAL SENTINEL", completed.stderr)
        self.assertEqual(sentinel.read_text(encoding="utf-8"), "EXTERNAL SENTINEL")

    def npm_fixture(self):
        prefix = self.root / "npm prefix"
        prefix.mkdir()
        shim = prefix / "codex.cmd"
        batch_marker = self.root / "batch-executed"
        shim.write_text('@echo off\necho batch > "%HARNESS_BATCH_MARKER%"\nexit /b 86\n', encoding="utf-8")
        package = prefix / "node_modules" / "@openai" / "codex"
        script = package / "bin" / "codex.js"
        script.parent.mkdir(parents=True)
        script.write_text(
            "const fs=require('fs');let input='';process.stdin.setEncoding('utf8');"
            "process.stdin.on('data',v=>input+=v);process.stdin.on('end',()=>{"
            "fs.writeFileSync(process.env.HARNESS_NODE_LOG,JSON.stringify({argv:process.argv.slice(2),stdin:input}));"
            "if(process.argv.includes('--version')) console.log('codex-cli fixture');"
            "else if(process.argv.includes('--help')) console.log('codex CLI fixture help');"
            "else {console.log(JSON.stringify({type:'thread.started',thread_id:'native-fixture'}));"
            "console.log(JSON.stringify({type:'turn.completed',usage:{input_tokens:1,output_tokens:1}}));}});",
            encoding="utf-8")
        (package / "package.json").write_text(json.dumps({"name": "@openai/codex", "bin": {"codex": "bin/codex.js"}}), encoding="utf-8")
        return shim, script, batch_marker

    def test_npm_shim_uses_direct_node_argv_and_preserves_literal_prompt(self):
        shim, script, batch_marker = self.npm_fixture()
        marker = self.root / "node-argv.json"
        literal = 'literal &|<>^%! "quotes with spaces"'
        relative_shim = os.path.relpath(shim, self.root)
        self.start("--codex-bin", relative_shim, env={"HARNESS_NODE_LOG": str(marker), "HARNESS_BATCH_MARKER": str(batch_marker)})
        bootstrap = self.api("/api/bootstrap")
        project_id = bootstrap["projects"][0]["id"]
        created = self.api("/api/sessions", {"project_id": project_id, "provider": "codex", "prompt": literal,
                                              "mode": "plan", "workflow": "native", "project_context": False})
        sid = created["session"]["id"]
        session = self.wait_for(lambda: self.api("/api/sessions/" + sid)["session"]
                                if self.api("/api/sessions/" + sid)["session"]["status"] in {"completed", "failed"} else None,
                                "fixture Codex session did not finish")
        self.assertEqual(session["status"], "completed")
        payload = json.loads(marker.read_text(encoding="utf-8"))
        self.assertIn(literal, payload["stdin"])
        self.assertIn("--cd", payload["argv"])
        self.assertEqual(payload["argv"][payload["argv"].index("--cd") + 1], str(self.project.resolve()))
        self.assertEqual(payload["argv"][-1], "-")
        self.assertFalse(batch_marker.exists(), "the untrusted .cmd shim must never execute")
        check_literal = 'check &|<>^%! "quoted spaces"'
        command = windows_commands.command_argv([str(shim), check_literal], "codex")
        self.assertEqual(command[:2], [str(shutil.which("node.exe") or shutil.which("node")), str(script)])
        self.assertEqual(command[-1], check_literal)
        self.assertNotIn("cmd.exe", command)
        escaped = self.root / "escape.cmd"
        escaped.write_text("@echo unsafe", encoding="utf-8")
        (script.parent.parent / "package.json").write_text(json.dumps({"name": "@openai/codex", "bin": {"codex": "../escape.cmd"}}), encoding="utf-8")
        with self.assertRaises(ValueError):
            windows_commands.command_argv([str(shim)], "codex")

    def test_project_brain_mutation_lock_serializes_two_native_processes(self):
        repository = self.root / "brain repo"
        (repository / "project-brain").mkdir(parents=True)
        first = self.root / "first"
        second = self.root / "second"
        runtime = ROOT / "PHP Core" / "memory-bank" / "scripts"
        code = ("import pathlib,sys,time;sys.path.insert(0,sys.argv[1]);import brain_runtime;"
                "repo=pathlib.Path(sys.argv[2]);mark=pathlib.Path(sys.argv[3]);"
                "\nwith brain_runtime.mutation_lock(repo):mark.write_text('entered');time.sleep(float(sys.argv[4]))")
        first_process = subprocess.Popen([sys.executable, "-c", code, str(runtime), str(repository), str(first), "1"])
        try:
            self.wait_for(first.exists, "first process did not enter Project Brain mutation lock")
            second_process = subprocess.Popen([sys.executable, "-c", code, str(runtime), str(repository), str(second), "0"])
            try:
                time.sleep(.25)
                self.assertFalse(second.exists(), "second process bypassed the native mutation lock")
                self.assertEqual(first_process.wait(timeout=5), 0)
                self.assertEqual(second_process.wait(timeout=5), 0)
                self.assertTrue(second.exists())
            finally:
                if second_process.poll() is None:
                    second_process.kill(); second_process.wait(timeout=3)
        finally:
            if first_process.poll() is None:
                first_process.kill(); first_process.wait(timeout=3)


if __name__ == "__main__":
    unittest.main()
