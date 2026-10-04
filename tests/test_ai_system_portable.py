"""System Orchestration on the portable file and process layers.

These tests run on Linux and, in the windows-harness CI job, natively on
Windows. There the worker is Codex installed the way npm installs it: a
codex.cmd shim that is never run, resolved to node and the package script.
"""
from __future__ import annotations

import http.client
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "harness/src"))
import ai_system_execution as execution
from ai_system_lib import System, SystemError, read_file
from harness import providers, web
from tests.test_ai_system_execution import ADAPTER

WINDOWS = os.name == "nt"
# The npm package script hands argv and stdin to the shared Python fixture adapter,
# which reads the UTF-8 prompt as UTF-8 on Windows too.
CODEX_JS = """
const {spawnSync} = require('child_process');
const chunks = [];
process.stdin.on('data', chunk => chunks.push(chunk));
process.stdin.on('end', () => {
  const result = spawnSync(process.env.AI_SYSTEM_FIXTURE_PYTHON,
    [process.env.AI_SYSTEM_FIXTURE_ADAPTER, ...process.argv.slice(2)],
    {input: Buffer.concat(chunks), stdio: ['pipe', 'inherit', 'inherit'],
     env: {...process.env, PYTHONUTF8: '1'}});
  process.exit(result.status === null ? 1 : result.status);
});
"""


def scratch(test, prefix):
    """A temporary folder. Windows may still hold a file in it for a moment after a test."""
    root = Path(tempfile.mkdtemp(prefix=prefix))
    test.addCleanup(shutil.rmtree, root, ignore_errors=True)
    return root


class PortableSystemTests(unittest.TestCase):
    def setUp(self):
        self.root = scratch(self, "ai-system-portable-")

    def npm_codex(self):
        """A Codex CLI as npm installs it on this platform, running the shared fixture adapter."""
        adapter = self.root / "adapter.py"
        adapter.write_text(ADAPTER, encoding="utf-8")
        if not WINDOWS:
            adapter.chmod(0o700)
            return adapter
        prefix = self.root / "npm prefix"
        package = prefix / "node_modules" / "@openai" / "codex"
        (package / "bin").mkdir(parents=True)
        (package / "package.json").write_text(json.dumps({"name": "@openai/codex", "bin": {"codex": "bin/codex.js"}}),
                                              encoding="utf-8")
        (package / "bin" / "codex.js").write_text(CODEX_JS, encoding="utf-8")
        environment = mock.patch.dict(os.environ, {"AI_SYSTEM_FIXTURE_PYTHON": sys.executable,
                                                   "AI_SYSTEM_FIXTURE_ADAPTER": str(adapter)})
        environment.start()
        self.addCleanup(environment.stop)
        shim = prefix / "codex.cmd"
        # Never run: a batch file would go through cmd.exe, so Harness resolves it to node.
        shim.write_text("@echo off\r\nexit /b 86\r\n", encoding="utf-8")
        return shim

    def link_directory(self, link, target):
        if WINDOWS:
            created = subprocess.run(["cmd", "/d", "/c", "mklink", "/J", str(link), str(target)],
                                     capture_output=True, text=True, check=False)
            self.assertEqual(0, created.returncode, created.stderr)
        else:
            link.symlink_to(target, target_is_directory=True)

    def services(self):
        """A system workspace with two independent services."""
        workspace = self.root / "system"
        workspace.mkdir()
        services = []
        for sid in ("orders", "payments"):
            root = self.root / sid
            root.mkdir()
            (root / "spec.md").write_text("Original " + sid, encoding="utf-8")
            passport = {"schema_version": 1, "id": sid, "description": sid, "owner": "team",
                        "relationships_complete": True, "capabilities": [], "provides": [], "consumes": [],
                        "sources": [{"path": "spec.md", "kind": "spec"}]}
            (root / "ai-service.json").write_text(json.dumps(passport), encoding="utf-8")
            services.append({"id": sid, "root": str(root), "manifest": "ai-service.json"})
        config = workspace / "system.json"
        config.write_text(json.dumps({"schema_version": 1, "name": "Portable", "services": services,
                                      "shared_sources": []}), encoding="utf-8")
        return config

    def test_journal_is_written_once_replaced_under_a_reader_and_locked(self):
        directory = self.root / "journal"
        directory.mkdir()
        execution.save(directory, "run.json", {"value": 1}, new=True)
        with self.assertRaises(SystemError):
            execution.save(directory, "run.json", {"value": 2}, new=True)
        execution.save(directory, "run.json", {"value": 3})
        self.assertEqual({"value": 3}, execution.load(directory, "run.json"))
        # Windows refuses to replace a file a reader holds open; the save waits it out.
        reader = open(directory / "run.json", "rb")
        threading.Timer(.3, reader.close).start()
        execution.save(directory, "run.json", {"value": 4})
        self.assertEqual({"value": 4}, execution.load(directory, "run.json"))
        self.assertEqual(["run.json"], sorted(path.name for path in directory.iterdir()))
        with execution.lock_file(directory, ".run.lock"):
            with self.assertRaisesRegex(SystemError, "Another executor"):
                with execution.lock_file(directory, ".run.lock"):
                    self.fail("Two executors held the same run")
        with execution.lock_file(directory, ".run.lock"):
            pass

    def test_services_sharing_a_checkout_share_one_workspace_lock(self):
        config = self.services()
        subprocess.run(["git", "init", "--quiet", str(self.root)], check=True)
        system = System(config, [self.root])
        with execution.workspace_locks(system, ["orders"]):
            with self.assertRaisesRegex(SystemError, "Another executor"):
                with execution.workspace_locks(system, ["payments"]):
                    self.fail("Both workers obtained the same Git checkout")

    def test_links_and_junctions_are_refused(self):
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "secret.txt").write_text("outside", encoding="utf-8")
        service = self.root / "service"
        service.mkdir()
        self.link_directory(service / "linked", outside)
        with self.assertRaises((OSError, SystemError)):
            read_file(service, "linked/secret.txt")
        (service / "spec.md").write_text("spec", encoding="utf-8")
        os.link(service / "spec.md", service / "alias.md")
        with self.assertRaisesRegex(SystemError, "without hard links"):
            read_file(service, "alias.md")
        with self.assertRaises((OSError, SystemError)):
            execution.save(service / "linked", "run.json", {}, new=True)
        self.link_directory(service / "project-brain", outside)
        with self.assertRaisesRegex(SystemError, "Refused linked"):
            execution.guard_brain(service)
        self.assertEqual(["secret.txt"], [path.name for path in outside.iterdir()])

    def test_worker_output_stderr_and_timeout_reap_the_whole_tree(self):
        lines = []
        script = "import sys; print('first'); print('second'); sys.stderr.write('problem\\n')"
        result = execution.run_process([sys.executable, "-c", script], self.root, "", 30,
                                       on_line=lines.append, stderr_tail=True)
        self.assertEqual((0, None), (result["returncode"], result["error"]), result)
        self.assertEqual([b"first", b"second"], [line.rstrip(b"\r") for line in lines])
        self.assertIn(b"problem", result["stderr_tail"])
        marker = self.root / "survived"
        late = "import sys, time; time.sleep(3); open(sys.argv[1], 'w').write('survived')"
        worker = ("import subprocess, sys, time\n"
                  "subprocess.Popen([sys.executable, '-c', " + repr(late) + ", sys.argv[1]])\n"
                  "print('started', flush=True)\n"
                  "time.sleep(60)\n")
        started = time.monotonic()
        result = execution.run_process([sys.executable, "-c", worker, str(marker)], self.root, "", 2)
        self.assertEqual("timeout", result["error"], result)
        self.assertLess(time.monotonic() - started, 20)
        time.sleep(4)
        self.assertFalse(marker.exists(), "A worker's child outlived the dispatch")

    def test_cli_creates_without_overwriting_and_prints_utf8_json(self):
        def cli(*arguments):
            return subprocess.run([sys.executable, str(ROOT / "scripts/ai_system.py"), *arguments],
                                  capture_output=True, check=False, timeout=60)
        workspace = self.root / "workspace"
        created = cli("init", "--root", str(workspace), "--name", "Система заказов")
        self.assertEqual(0, created.returncode, created.stderr)
        self.assertEqual(2, cli("init", "--root", str(workspace), "--name", "Again").returncode)
        self.assertNotIn(b"\r\n", (workspace / "system.json").read_bytes())
        catalog = cli("catalog", "--system", str(workspace / "system.json"))
        self.assertEqual(0, catalog.returncode, catalog.stderr)
        self.assertEqual("Система заказов", json.loads(catalog.stdout.decode("utf-8"))["system"])

    def test_run_closes_native_tasks_for_a_task_written_in_russian(self):
        codex = self.npm_codex()
        config = self.services()
        system = System(config, [self.root])
        plan = system.plan("Отменить заказ и вернуть оплату", "chg-portable", ["orders", "payments"])
        run_dir = self.root / "run"
        state = execution.create_run(system, plan, run_dir, "codex", str(codex), "edit", 120)
        state = execution.drive_run(system, run_dir, state)
        self.assertEqual("completed", state["status"], state["error"])
        self.assertEqual(["contracts", "service-payments", "service-orders", "verify"],
                         [step["id"] for step in state["steps"]])
        self.assertEqual(3, len(state["closed_tasks"]))
        self.assertEqual("Scoped edit for orders", (self.root / "orders/spec.md").read_text(encoding="utf-8"))
        self.assertEqual(4, len(execution.load(run_dir, "handoff.json")["receipts"]))


class PortableHarnessSystemTests(unittest.TestCase):
    """The Harness queue runs a reviewed plan through its runner and guarded workers."""

    def setUp(self):
        self.root = scratch(self, "ai-system-harness-")
        self.project = self.root / "project"
        shutil.copytree(ROOT / "docs/examples/ai-system", self.project)
        codex = PortableSystemTests.npm_codex(self)
        discovery = mock.patch.object(providers, "discover_providers", return_value=[{
            "id": "codex", "name": "Fixture Codex", "available": True, "executable": str(codex),
            "detail": "Deterministic fixture; no model calls"}])
        discovery.start()
        self.addCleanup(discovery.stop)
        models = mock.patch.object(providers, "model_options",
                                   return_value={"models": [], "efforts": [], "detail": "Fixture"})
        models.start()
        self.addCleanup(models.stop)
        self.server = web.HarnessServer(("127.0.0.1", 0), self.root / "state", [self.project])
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": .01}, daemon=True)
        self.thread.start()
        self.addCleanup(self.stop)
        self.project_id = next(iter(self.server.sessions.projects))

    def stop(self):
        self.server.close()
        self.thread.join(5)

    def call(self, path, body=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.server.server_port, timeout=30)
        try:
            headers = {"Host": f"127.0.0.1:{self.server.server_port}"}
            data = None
            if body is not None:
                data = json.dumps(body).encode()
                headers.update({"Content-Type": "application/json", "X-Harness-Token": self.server.token})
            connection.request("POST" if body is not None else "GET", path, data, headers)
            response = connection.getresponse()
            return response.status, json.loads(response.read() or b"null")
        finally:
            connection.close()

    def test_reviewed_plan_runs_with_npm_codex_and_closes_its_tasks(self):
        status, run = self.call("/api/system-runs", {
            "project_id": self.project_id, "config_path": "system.json",
            "task": "Расследовать отмену заказа и её потребителей", "change_id": "change-001",
            "services": ["orders"]})
        self.assertEqual(201, status, run)
        status, queued = self.call("/api/system-runs/" + run["id"], {
            "action": "execute", "revision": run["revision"], "mode": "read-only", "timeout": 120})
        self.assertEqual(200, status, queued)
        deadline = time.monotonic() + 300
        while time.monotonic() < deadline:
            status, result = self.call("/api/system-runs/" + run["id"])
            self.assertEqual(200, status, result)
            if not result["active"]:
                break
            time.sleep(.2)
        self.assertEqual("completed", result["status"], result["events"])
        self.assertEqual(5, len(result["receipts"]))
        self.assertTrue(all(task["closed"] and task["uuid"] for task in result["execution"]["tasks"].values()))


if __name__ == "__main__":
    unittest.main()
