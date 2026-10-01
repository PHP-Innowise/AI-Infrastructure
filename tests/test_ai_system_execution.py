"""Actual native Brain CLI + deterministic subprocess workers, no paid AI calls."""
from __future__ import annotations

from copy import deepcopy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import ai_system_execution as execution
from ai_system_lib import System, SystemError, digest, encoded


ADAPTER = '''#!/usr/bin/env python3
import json, os, pathlib, sys
prompt = sys.stdin.read()
value = json.loads(prompt.split("Dispatch input:\\n", 1)[1])
root = pathlib.Path.cwd()
marker = root / "fail-worker"
if marker.exists():
    print("untrusted error output is never persisted")
    sys.exit(7)
if (root / "interrupt-worker").exists():
    import time
    time.sleep(5)
if (root / "invalid-worker").exists():
    print('{"garbage": true}')
    sys.exit(0)
if value["mode"] == "edit":
    (root / "spec.md").write_text("Scoped edit for " + value["service"])
phase = value["phase"]
checks = [{"name": "Fixture check", "status": "passed", "detail": "Synthetic adapter check passed"}]
if (root / "skip-verification").exists() and phase == "verify":
    checks[0]["status"] = "not_run"
report = {"status": "completed", "summary": "Completed " + phase, "checks": checks,
          "changed_files": ["spec.md"] if value["mode"] == "edit" else [],
          "service_order": list(reversed(value["context"]["services"])) if phase == "contracts" else []}
if "--json" in sys.argv:
    print(json.dumps({"type": "thread.started", "thread_id": "fixture"}))
    print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": json.dumps(report)}}))
    print(json.dumps({"type": "turn.completed"}))
else:
    print(json.dumps(report))
'''


class ExecutionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ai-system-execution-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.workspace = self.root / "system"
        self.workspace.mkdir()
        self.config = self.workspace / "system.json"
        services = []
        for sid in ("orders", "payments"):
            root = self.root / sid
            root.mkdir()
            (root / "spec.md").write_text("Original " + sid)
            (root / "AGENTS.md").write_text("# Local policy\nOnly requested changes.\n")
            passport = {"schema_version": 1, "id": sid, "description": sid,
                "owner": "team", "relationships_complete": True,
                "capabilities": [], "provides": [], "consumes": [],
                "sources": [{"path": "spec.md", "kind": "spec"}, {"path": "AGENTS.md", "kind": "policy"}]}
            (root / "ai-service.json").write_text(json.dumps(passport))
            services.append({"id": sid, "root": str(root), "manifest": "ai-service.json"})
        self.config.write_text(json.dumps({"schema_version": 1, "name": "Fixture",
                                         "services": services, "shared_sources": []}))
        self.adapter = self.root / "adapter"
        self.adapter.write_text(ADAPTER)
        self.adapter.chmod(0o700)
        self.run_dir = self.root / "run"

    def system(self):
        return System(self.config, [self.root])

    def prepare(self, mode="edit", provider="command", timeout=10):
        system = self.system()
        plan = system.plan("Implement cancellation", "chg-test", ["orders", "payments"])
        state = execution.create_run(system, plan, self.run_dir, provider, self.adapter, mode, timeout)
        return system, state

    def run_all(self, mode="edit", provider="command"):
        system, state = self.prepare(mode, provider)
        return execution.drive_run(system, self.run_dir, state)

    def test_end_to_end_native_tasks_order_receipts_and_idempotent_resume(self):
        state = self.run_all()
        self.assertEqual("completed", state["status"])
        self.assertEqual(["contracts", "service-payments", "service-orders", "verify"],
                         [s["id"] for s in state["steps"]])
        self.assertEqual(3, len(state["closed_tasks"]))
        for sid, reference in state["tasks"].items():
            brain = execution.Brain(Path(state["roots"][sid]), "ai-system-" + state["run_id"])
            record = brain.call("brain-get", "--record-id", reference["uuid"])
            self.assertEqual("completed", record["status"])
            self.assertEqual(reference["external_id"], record["external_id"])
            self.assertGreater(record["revision"], 1)
        self.assertTrue(self.system().verify(execution.load(self.run_dir, "checkpoint.json"))["fresh"])
        files = {p.name: p.read_bytes() for p in self.run_dir.glob("*.json")}
        resumed = execution.drive_run(self.system(), self.run_dir, state)
        self.assertEqual("completed", resumed["status"])
        self.assertEqual(files, {p.name: p.read_bytes() for p in self.run_dir.glob("*.json")})
        self.assertEqual("Scoped edit for orders", (self.root / "orders/spec.md").read_text())

    def test_codex_adapter_argv_and_jsonl(self):
        original = execution.run_process
        worker_commands = []
        def spy(command, cwd, stdin, timeout):
            if command[0] == str(self.adapter):
                worker_commands.append(command)
            return original(command, cwd, stdin, timeout)
        with mock.patch.object(execution, "run_process", side_effect=spy):
            state = self.run_all(provider="codex")
        self.assertEqual("completed", state["status"])
        self.assertEqual(4, len(worker_commands))
        self.assertIn("workspace-write", worker_commands[1])
        self.assertIn("read-only", worker_commands[0])
        self.assertIn("--output-schema", worker_commands[0])
        self.assertIn("--skip-git-repo-check", worker_commands[0])
        self.assertTrue(all(command[-1] == "-" for command in worker_commands))
        self.assertFalse(any("dangerously" in arg for command in worker_commands for arg in command))

    def test_read_only_run_does_not_edit_service_sources(self):
        state = self.run_all(mode="read-only")
        self.assertEqual("completed", state["status"])
        self.assertEqual("Original orders", (self.root / "orders/spec.md").read_text())
        self.assertEqual("Original payments", (self.root / "payments/spec.md").read_text())

    def test_failure_stops_and_resume_skips_completed_steps(self):
        marker = self.root / "orders/fail-worker"
        marker.touch()
        system, initial = self.prepare()
        failed = execution.drive_run(system, self.run_dir, initial)
        self.assertEqual("blocked", failed["status"])
        self.assertEqual("pending", failed["steps"][-1]["status"])
        completed = [s["id"] for s in failed["steps"] if s["status"] == "completed"]
        self.assertEqual(["contracts", "service-payments"], completed)
        self.assertNotIn("untrusted error", "".join(p.read_text() for p in self.run_dir.glob("*.json")))
        with self.assertRaisesRegex(SystemError, "retry-step"):
            execution.drive_run(self.system(), self.run_dir, failed)
        marker.unlink()
        state = execution.drive_run(self.system(), self.run_dir, failed, "service-orders")
        self.assertEqual("completed", state["status"])
        self.assertEqual([1, 1, 2, 1], [s["attempt"] for s in state["steps"]])

    def test_stale_plan_refused_before_any_tasks_or_launch(self):
        system = self.system()
        plan = system.plan("Change orders", "stale", ["orders"])
        (self.root / "orders/spec.md").write_text("Changed elsewhere")
        with self.assertRaisesRegex(SystemError, "stale"):
            execution.create_run(self.system(), plan, self.run_dir, "command", self.adapter, "edit", 10)
        self.assertFalse(self.run_dir.exists())
        self.assertFalse((self.workspace / "project-brain").exists())

    def test_resume_requires_explicit_acceptance_for_partial_changes(self):
        system, state = self.prepare()
        state = execution.execute_run(system, self.run_dir, state)  # contracts only
        step = next(s for s in state["steps"] if s["id"] == "service-payments")
        step.update(status="running", attempt=1, dispatch_id="a" * 32, input_sha256="b" * 64)
        execution.save(self.run_dir, "run.json", state)
        (self.root / "payments/spec.md").write_text("Partial ambiguous write")
        with self.assertRaisesRegex(SystemError, "retry-step"):
            execution.drive_run(self.system(), self.run_dir, state)
        with self.assertRaisesRegex(SystemError, "accept-source-changes"):
            execution.drive_run(self.system(), self.run_dir, state, "service-payments")
        finished = execution.drive_run(self.system(), self.run_dir, state, "service-payments", True)
        self.assertEqual("completed", finished["status"])
        self.assertEqual(2, finished["steps"][1]["attempt"])

    def test_external_changes_and_catalog_edits_cannot_be_adopted(self):
        system, state = self.prepare()
        state = execution.execute_run(system, self.run_dir, state)
        step = state["steps"][1]
        step.update(status="interrupted", attempt=1, dispatch_id="a" * 32, input_sha256="b" * 64)
        execution.save(self.run_dir, "run.json", state)
        (self.root / "orders/spec.md").write_text("Wrong service mutation")
        with self.assertRaisesRegex(SystemError, "outside"):
            execution.drive_run(self.system(), self.run_dir, state, step["id"], True)

    def test_incomplete_verification_does_not_complete_native_tasks(self):
        (self.workspace / "skip-verification").touch()
        failed = self.run_all()
        self.assertEqual("blocked", failed["status"])
        self.assertEqual([], failed["closed_tasks"])
        self.assertEqual("worker_blocked_or_verification_incomplete", failed["error"])
        for sid, reference in failed["tasks"].items():
            brain = execution.Brain(Path(failed["roots"][sid]), "ai-system-" + failed["run_id"])
            self.assertNotEqual("completed", brain.call("brain-get", "--record-id", reference["uuid"])["status"])

    def test_timeout_is_receipted_and_no_later_workers_run(self):
        (self.workspace / "interrupt-worker").touch()
        system, state = self.prepare(timeout=1)
        state = execution.drive_run(system, self.run_dir, state)
        self.assertEqual("blocked", state["status"])
        receipt = execution.load(self.run_dir, "contracts-a1.json")
        self.assertEqual("timeout", receipt["error"])
        self.assertTrue(all(s["attempt"] == 0 for s in state["steps"][1:]))

    def test_invalid_worker_result_is_not_persisted(self):
        (self.workspace / "invalid-worker").touch()
        state = self.run_all()
        self.assertEqual("invalid_or_sensitive_worker_result", state["error"])
        self.assertIsNone(execution.load(self.run_dir, "contracts-a1.json")["report"])

    def test_binding_recreated_without_duplicate_native_task(self):
        system, state = self.prepare()
        state = execution.execute_run(system, self.run_dir, state)
        reference = state["tasks"]["orders"]
        task_uuid = reference["uuid"]
        # The ignored SQLite binding is disposable, authoritative records are not.
        (self.root / "orders/memory-bank/local/context.db").unlink()
        state = execution.drive_run(self.system(), self.run_dir, state)
        self.assertEqual("completed", state["status"])
        self.assertEqual(task_uuid, state["tasks"]["orders"]["uuid"])

    def test_storage_links_and_lock_collision_are_refused(self):
        system, state = self.prepare()
        with execution.lock_file(self.run_dir, ".run.lock"):
            with self.assertRaisesRegex(SystemError, "Another executor"):
                execution.drive_run(system, self.run_dir, state)
        outside = self.root / "outside"
        outside.mkdir()
        (self.workspace / "project-brain").symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(SystemError, "linked"):
            execution.drive_run(system, self.run_dir, state)
        self.assertEqual([], list(outside.iterdir()))

    def test_original_plan_and_executable_are_pinned(self):
        system, state = self.prepare()
        self.adapter.write_text(ADAPTER + "\n# changed launcher\n")
        with self.assertRaisesRegex(SystemError, "executable changed"):
            execution.drive_run(system, self.run_dir, state)

    def test_success_receipt_recovers_without_repeating_worker(self):
        system, state = self.prepare()
        state = execution.execute_run(system, self.run_dir, state)
        original_save = execution.save
        def crash(root, name, value, new=False):
            original_save(root, name, value, new)
            if name == "service-payments-a1.json":
                raise SystemError("Simulated crash after terminal receipt")
        with mock.patch.object(execution, "save", side_effect=crash):
            with self.assertRaisesRegex(SystemError, "Simulated crash"):
                execution.drive_run(self.system(), self.run_dir, state)
        with self.assertRaisesRegex(SystemError, "uncheckpointed"):
            execution.drive_run(self.system(), self.run_dir, state)
        resumed = execution.drive_run(self.system(), self.run_dir, state, accept_source_changes=True)
        self.assertEqual("completed", resumed["status"])
        self.assertEqual([1, 1, 1, 1], [s["attempt"] for s in resumed["steps"]])
        self.assertFalse((self.run_dir / "service-payments-a2.json").exists())

    def test_native_cas_race_retries_metadata_without_rerunning_worker(self):
        original_call = execution.Brain.call
        raced = []
        def race(brain, *arguments, **kwargs):
            if arguments[0] == "update" and not raced:
                raced.append(True)
                current = original_call(brain, "get", "--task-id", arguments[2])
                original_call(brain, "update", "--task-id", arguments[2],
                              "--revision", str(current["revision"]), "--progress", "Concurrent metadata update")
            return original_call(brain, *arguments, **kwargs)
        with mock.patch.object(execution.Brain, "call", new=race):
            state = self.run_all()
        self.assertTrue(raced)
        self.assertEqual("completed", state["status"])
        self.assertEqual([1, 1, 1, 1], [s["attempt"] for s in state["steps"]])

    def test_maximum_catalog_ids_fit_native_task_ids(self):
        system = self.system()
        plan = system.plan("Implement cancellation", "c" * 80, ["orders"])
        state = execution.create_run(system, plan, self.run_dir, "command", self.adapter, "read-only", 10)
        self.assertTrue(all(len(r["external_id"]) <= 128 for r in state["tasks"].values()))
        state = execution.drive_run(system, self.run_dir, state)
        self.assertEqual("completed", state["status"])

    def test_unreported_changes_during_worker_are_not_adopted(self):
        original = execution.run_process
        def external_edit(command, cwd, stdin, timeout):
            result = original(command, cwd, stdin, timeout)
            if command[0] == str(self.adapter) and cwd.name == "payments":
                (cwd / "AGENTS.md").write_text("Externally changed policy")
            return result
        with mock.patch.object(execution, "run_process", side_effect=external_edit):
            with self.assertRaisesRegex(SystemError, "outside"):
                self.run_all()
        state = execution.load(self.run_dir, "run.json")
        self.assertEqual("blocked", state["status"])
        self.assertEqual(0, next(s for s in state["steps"] if s["id"] == "service-orders")["attempt"])

    def test_lost_native_completion_response_is_reconciled_without_worker_calls(self):
        original_complete = execution.Brain.complete
        crashes = []
        def crash(brain, reference, run_id):
            original_complete(brain, reference, run_id)
            if not crashes:
                crashes.append(True)
                raise SystemExit("Abrupt crash after native completion")
        with mock.patch.object(execution.Brain, "complete", new=crash):
            with self.assertRaises(SystemExit):
                self.run_all()
        state = execution.load(self.run_dir, "run.json")
        self.assertEqual([], state["closed_tasks"])
        self.assertTrue(all(s["status"] == "completed" for s in state["steps"]))
        state = execution.drive_run(self.system(), self.run_dir, state)
        self.assertEqual("completed", state["status"])
        self.assertEqual([1, 1, 1, 1], [s["attempt"] for s in state["steps"]])

    def test_completed_receipts_cannot_be_skipped_or_order_changed(self):
        system, state = self.prepare()
        state = execution.execute_run(system, self.run_dir, state)
        state["steps"][1], state["steps"][2] = state["steps"][2], state["steps"][1]
        execution.save(self.run_dir, "run.json", state)
        with self.assertRaisesRegex(SystemError, "Saved order"):
            execution.drive_run(system, self.run_dir, state)

    def test_malformed_input_fails_closed(self):
        system, state = self.prepare()
        state["tasks"] = []
        execution.save(self.run_dir, "run.json", state)
        with self.assertRaises(SystemError):
            execution.drive_run(system, self.run_dir, state)
        with self.assertRaises(SystemError):
            execution.validate_report({"status": "completed", "summary": "Valid summary", "checks": [],
                "changed_files": [], "service_order": [{}]}, {"id": "contracts", "mode": "read-only"}, ["orders"])

    def test_sensitive_and_oversized_output_are_not_saved(self):
        # Direct provider output tests need no native state mutation.
        secret = "sk-" + "a" * 30
        report = {"status": "completed", "summary": secret, "checks": [], "changed_files": [], "service_order": []}
        with self.assertRaises(SystemError):
            execution.validate_report(report, {"id": "service-orders", "mode": "edit"}, ["orders"])
        large = self.root / "large-output"
        large.write_text("#!/usr/bin/env python3\nimport sys\nsys.stdout.write('x' * 3000000)\n")
        large.chmod(0o700)
        result = execution.run_process([str(large)], self.root, "", 10)
        self.assertEqual("output_limit", result["error"])
        self.assertLessEqual(len(result["stdout"]), execution.MAX_OUTPUT)

    def test_shared_git_checkout_workspace_lock(self):
        subprocess.run(["git", "init", "--quiet", str(self.root)], check=True)
        system = self.system()
        with execution.workspace_locks(system, ["orders"]):
            with self.assertRaisesRegex(SystemError, "Another executor"):
                with execution.workspace_locks(system, ["payments"]):
                    self.fail("Both workers obtained the same Git checkout")

    def test_context_refresh_cannot_adopt_racing_external_change(self):
        system, state = self.prepare()
        original = execution.refresh_checkpoint
        def racing_refresh(current, plan, accepted=()):
            (self.root / "orders/spec.md").write_text("External racing mutation")
            return original(current, plan, accepted)
        with mock.patch.object(execution, "refresh_checkpoint", side_effect=racing_refresh):
            with self.assertRaisesRegex(SystemError, "during canonical"):
                execution.drive_run(system, self.run_dir, state)
        saved = execution.load(self.run_dir, "run.json")
        self.assertTrue(all(s["attempt"] == 0 for s in saved["steps"]))
        self.assertFalse((self.workspace / "project-brain").exists())

    def test_status_remains_readable_after_provider_removal(self):
        system, state = self.prepare()
        self.adapter.unlink()
        historical = execution.validate_state(system, self.run_dir, for_execution=False)
        self.assertEqual("prepared", historical["status"])
        self.assertFalse((self.workspace / "project-brain").exists())
        with self.assertRaises(SystemError):
            execution.validate_state(system, self.run_dir)

    def test_cli_execute_and_status(self):
        system = self.system()
        plan = system.plan("Implement cancellation", "cli-test", ["orders"])
        plan_path = self.root / "input-plan.json"
        plan_path.write_text(json.dumps(plan))
        common = [sys.executable, str(ROOT / "scripts/ai_system.py")]
        args = ["--system", str(self.config), "--allow-root", str(self.root), "--run-dir", str(self.run_dir)]
        result = subprocess.run(common + ["execute"] + args + ["--plan", str(plan_path), "--provider", "command", "--executable", str(self.adapter)],
                                text=True, capture_output=True, timeout=30)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual("completed", json.loads(result.stdout)["status"])
        status = subprocess.run(common + ["run-status"] + args, text=True, capture_output=True)
        self.assertEqual(0, status.returncode, status.stderr)
        self.assertEqual("completed", json.loads(status.stdout)["status"])


if __name__ == "__main__":
    unittest.main()
