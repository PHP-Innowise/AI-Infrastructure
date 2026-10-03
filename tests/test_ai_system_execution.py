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
if "--workspace" in sys.argv:
    assert not prompt, "Cursor must use argv rather than stdin"
    prompt = sys.argv[sys.argv.index("--") + 1]
value = json.loads(prompt.split("Dispatch input:\\n", 1)[1])
root = pathlib.Path.cwd()
marker = root / "fail-worker"
if marker.exists():
    print("untrusted error output is never persisted")
    sys.exit(7)
if (root / "auth-failure-worker").exists() and "--json-schema" in sys.argv:
    print(json.dumps({"type": "assistant", "error": "authentication_failed", "message": {"role": "assistant", "content": []}}))
    print(json.dumps({"type": "result", "subtype": "success", "is_error": True,
                      "result": "Failed to authenticate: OAuth session expired and could not be refreshed"}))
    sys.exit(1)
if (root / "interrupt-worker").exists():
    import time
    time.sleep(5)
if (root / "invalid-worker").exists():
    print('{"garbage": true}')
    sys.exit(0)
access = value.get("access", {"scope": "service", "readable": [], "writable": []})
shared = access["scope"] == "all"
changed = []
if value["mode"] == "edit":
    (root / "spec.md").write_text("Scoped edit for " + value["service"])
    changed.append(value["service"] + "/spec.md" if shared else "spec.md")
    for marker, report_it in (("cross-write", True), ("unreported-cross-write", False)):
        if (root / marker).exists():
            for sid in access["writable"]:
                if sid != value["service"]:
                    (pathlib.Path(value["roots"][sid]) / "spec.md").write_text("Cross edit by " + value["service"])
                    if report_it:
                        changed.append(sid + "/spec.md")
    if (root / "unprefixed-report").exists():
        changed = ["spec.md"]
phase = value["phase"]
checks = [{"name": "Fixture check", "status": "passed", "detail": "Synthetic adapter check passed"}]
if (root / "skip-verification").exists() and phase == "verify":
    checks[0]["status"] = "not_run"
report = {"status": "completed", "summary": "Completed " + phase, "checks": checks,
          "changed_files": changed,
          "service_order": list(reversed(value["context"]["services"])) if phase == "contracts" else []}
if "--json" in sys.argv:
    print(json.dumps({"type": "thread.started", "thread_id": "fixture"}))
    print(json.dumps({"type": "item.started", "item": {"id": "item_1", "type": "command_execution",
                      "command": "bash -lc 'ls -la'", "status": "in_progress"}}))
    print(json.dumps({"type": "item.completed", "item": {"id": "item_1", "type": "command_execution",
                      "command": "bash -lc 'ls -la'", "aggregated_output": "PRIVATE OUTPUT", "exit_code": 0,
                      "status": "completed"}}))
    print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": json.dumps(report)}}))
    print(json.dumps({"type": "turn.completed", "usage": {"input_tokens": 20, "cached_input_tokens": 5,
                      "output_tokens": 4}}))
elif "--json-schema" in sys.argv:
    assert "--verbose" in sys.argv and sys.argv[sys.argv.index("--output-format") + 1] == "stream-json"
    print(json.dumps({"type": "system", "subtype": "init", "session_id": "fixture", "model": "fixture-model"}))
    print(json.dumps({"type": "assistant", "message": {"role": "assistant", "content": [
        {"type": "text", "text": "Inspecting " + phase},
        {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"file_path": str(root / "spec.md")}}]}}))
    print(json.dumps({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "PRIVATE FILE CONTENT", "is_error": False}]}}))
    print(json.dumps({"type": "result", "subtype": "success", "is_error": False, "total_cost_usd": 0.01,
                      "usage": {"input_tokens": 10, "output_tokens": 5}, "result": "", "structured_output": report}))
elif "--workspace" in sys.argv:
    print(json.dumps({"type": "assistant", "model_call_id": "fixture", "message":
        {"role": "assistant", "content": [{"type": "text", "text": "Interim analysis"}]}}))
    print(json.dumps({"type": "result", "subtype": "success", "is_error": False,
                      "result": json.dumps(report)}))
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

    def test_oversized_cursor_plan_is_refused_before_journal_or_native_tasks(self):
        for sid in ('orders', 'payments'):
            root = self.root / sid
            passport = json.loads((root / 'ai-service.json').read_text())
            for number in range(8):
                source = 'large-' + str(number) + '.md'
                (root / source).write_text('😀' * 2000)
                passport['sources'].append({'path': source, 'kind': 'spec'})
            (root / 'ai-service.json').write_text(json.dumps(passport))
        system = self.system()
        plan = system.plan('Inspect contract', 'chg-large', ['orders', 'payments'], budget=64000)
        with self.assertRaisesRegex(SystemError, 'argument byte limit'):
            execution.create_run(system, plan, self.run_dir, 'cursor', self.adapter, 'read-only', 10)
        self.assertFalse(self.run_dir.exists())
        for root in (self.workspace, self.root / 'orders', self.root / 'payments'):
            self.assertFalse((root / 'project-brain').exists())

    def test_claude_and_cursor_native_tasks_argv_and_modes(self):
        original = execution.run_process
        for provider in ('claude', 'cursor'):
            for mode in ('read-only', 'edit'):
                with self.subTest(provider=provider, mode=mode):
                    self.run_dir = self.root / ('run-' + provider + '-' + mode)
                    calls = []
                    def spy(command, cwd, stdin, timeout, **options):
                        if command[0] == str(self.adapter):
                            calls.append((command, cwd, stdin))
                        return original(command, cwd, stdin, timeout, **options)
                    with mock.patch.object(execution, 'run_process', side_effect=spy):
                        state = self.run_all(provider=provider, mode=mode)
                    self.assertEqual('completed', state['status'])
                    self.assertEqual(provider, execution.validate_state(self.system(), self.run_dir)['provider'])
                    self.assertEqual(3, len(state['closed_tasks']))
                    self.assertEqual(4, len(calls))
                    for index, (command, cwd, stdin) in enumerate(calls):
                        service_mode = mode if index in (1, 2) else 'read-only'
                        self.assertEqual(Path(state['roots'][state['steps'][index]['service']]), cwd)
                        if provider == 'claude':
                            self.assertEqual('stream-json', command[command.index('--output-format') + 1])
                            self.assertIn('--verbose', command)
                            # Coordination reads every selected service; own-scope service workers read their own.
                            granted = (command[command.index('--add-dir') + 1:command.index('--json-schema')]
                                       if '--add-dir' in command else [])
                            self.assertEqual([state['roots'][sid] for sid in state['selected']] if index in (0, 3) else [],
                                             granted)
                            self.assertEqual('acceptEdits' if service_mode == 'edit' else 'plan',
                                             command[command.index('--permission-mode') + 1])
                            self.assertIn('--json-schema', command)
                            self.assertIn('Agent', command)
                            self.assertIn('Dispatch input:', stdin)
                        else:
                            self.assertEqual('', stdin)
                            self.assertEqual(str(cwd), command[command.index('--workspace') + 1])
                            self.assertIn('--disable-auto-update', command)
                            self.assertEqual('enabled', command[command.index('--sandbox') + 1])
                            self.assertEqual(service_mode == 'read-only', '--mode' in command)
                            self.assertNotIn('--force', command)
                            self.assertNotIn('--add-dir', command)
                            self.assertIn('Dispatch input:', command[-1])

    def test_codex_adapter_argv_and_jsonl(self):
        original = execution.run_process
        worker_commands = []
        def spy(command, cwd, stdin, timeout, **options):
            if command[0] == str(self.adapter):
                worker_commands.append(command)
            return original(command, cwd, stdin, timeout, **options)
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
        def external_edit(command, cwd, stdin, timeout, **options):
            result = original(command, cwd, stdin, timeout, **options)
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


    def spy_commands(self):
        original, commands = execution.run_process, []
        def spy(command, cwd, stdin, timeout, **options):
            if command[0] == str(self.adapter):
                commands.append((command, cwd, stdin))
            return original(command, cwd, stdin, timeout, **options)
        return mock.patch.object(execution, "run_process", side_effect=spy), commands

    def test_shared_access_grants_every_service_and_adopts_reported_cross_service_edits(self):
        (self.root / "payments/cross-write").touch()
        system = self.system()
        plan = system.plan("Implement cancellation", "chg-shared", ["orders", "payments"])
        state = execution.create_run(system, plan, self.run_dir, "claude", self.adapter, "edit", 10, "all")
        patcher, commands = self.spy_commands()
        with patcher:
            state = execution.drive_run(system, self.run_dir, state)
        self.assertEqual("completed", state["status"], state["error"])
        self.assertEqual("all", execution.validate_state(self.system(), self.run_dir)["access"])
        roots = state["roots"]
        for (command, cwd, stdin), step in zip(commands, state["steps"]):
            granted = command[command.index("--add-dir") + 1:command.index("--json-schema")] if "--add-dir" in command else []
            expected = [roots[sid] for sid in state["selected"] if roots[sid] != str(cwd)]
            self.assertEqual(expected, granted, step["id"])
            value = json.loads(stdin.split("Dispatch input:\n", 1)[1])
            writable = list(state["selected"]) if step["service"] != "__system__" else []
            self.assertEqual({"scope": "all", "readable": list(state["selected"]), "writable": writable}, value["access"])
            self.assertIn("<service-id>/<path relative to that service root>", stdin)
        receipt = execution.load(self.run_dir, "service-payments-a1.json")
        self.assertEqual(["payments/spec.md", "orders/spec.md"], receipt["report"]["changed_files"])
        self.assertEqual("Scoped edit for orders", (self.root / "orders/spec.md").read_text())
        self.assertTrue(self.system().verify(execution.load(self.run_dir, "checkpoint.json"))["fresh"])

    def test_shared_access_refuses_unreported_or_unprefixed_changes(self):
        for marker, expected in (("unreported-cross-write", "outside the current worker scope"),
                                 ("unprefixed-report", "invalid_or_sensitive_worker_result")):
            with self.subTest(marker=marker):
                self.run_dir = self.root / ("run-" + marker)
                (self.root / "payments" / marker).touch()
                system = self.system()
                plan = system.plan("Implement cancellation", "chg-" + marker, ["orders", "payments"])
                state = execution.create_run(system, plan, self.run_dir, "command", self.adapter, "edit", 10, "all")
                try:
                    state = execution.drive_run(system, self.run_dir, state)
                except SystemError as error:
                    self.assertIn(expected, str(error))
                else:
                    self.assertEqual(expected, state["error"])
                saved = execution.load(self.run_dir, "run.json")
                self.assertEqual("blocked", saved["status"])
                self.assertEqual(0, next(s for s in saved["steps"] if s["id"] == "service-orders")["attempt"])
                (self.root / "payments" / marker).unlink()
                for sid in ("orders", "payments"):
                    (self.root / sid / "spec.md").write_text("Original " + sid)

    def test_codex_shared_edit_scope_adds_only_writable_roots(self):
        system = self.system()
        plan = system.plan("Implement cancellation", "chg-codex-shared", ["orders", "payments"])
        state = execution.create_run(system, plan, self.run_dir, "codex", self.adapter, "edit", 10, "all")
        patcher, commands = self.spy_commands()
        with patcher:
            state = execution.drive_run(system, self.run_dir, state)
        self.assertEqual("completed", state["status"], state["error"])
        for (command, cwd, _), step in zip(commands, state["steps"]):
            granted = [command[i + 1] for i, part in enumerate(command) if part == "--add-dir"]
            expected = [] if step["service"] == "__system__" else [
                state["roots"][sid] for sid in state["selected"] if sid != step["service"]]
            self.assertEqual(expected, granted, step["id"])
            self.assertEqual("-", command[-1])

    def test_cursor_cannot_be_granted_other_service_folders(self):
        system = self.system()
        plan = system.plan("Implement cancellation", "chg-cursor", ["orders", "payments"])
        with self.assertRaisesRegex(SystemError, "cannot be granted"):
            execution.create_run(system, plan, self.run_dir, "cursor", self.adapter, "edit", 10, "all")
        with self.assertRaisesRegex(SystemError, "access scope"):
            execution.create_run(system, plan, self.run_dir, "codex", self.adapter, "edit", 10, "everything")
        self.assertFalse(self.run_dir.exists())
        self.assertFalse((self.workspace / "project-brain").exists())

    def test_observer_sees_each_dispatch_and_its_output_but_cannot_change_receipts(self):
        system = self.system()
        plan = system.plan("Implement cancellation", "chg-observer", ["orders", "payments"])
        state = execution.create_run(system, plan, self.run_dir, "claude", self.adapter, "read-only", 10)
        events = []
        def observer(event):
            events.append(event)
            raise RuntimeError("Display failures never change the run")
        state = execution.drive_run(system, self.run_dir, state, observer=observer)
        self.assertEqual("completed", state["status"])
        started = [e for e in events if e["type"] == "dispatch_started"]
        finished = [e for e in events if e["type"] == "dispatch_finished"]
        self.assertEqual([s["id"] for s in state["steps"]], [e["phase"] for e in started])
        self.assertEqual([s["id"] for s in state["steps"]], [e["phase"] for e in finished])
        self.assertTrue(all(e["ok"] and e["report"]["status"] == "completed" for e in finished))
        self.assertEqual(list(state["selected"]), started[0]["readable"])
        self.assertEqual([], started[0]["writable"])
        for phase in [s["id"] for s in state["steps"]]:
            kinds = [e["type"] for e in events if e["phase"] == phase]
            self.assertEqual("dispatch_started", kinds[0])
            self.assertEqual("dispatch_finished", kinds[-1])
            lines = [json.loads(e["line"]) for e in events if e["phase"] == phase and e["type"] == "output"]
            self.assertEqual(["system", "assistant", "user", "result"], [line["type"] for line in lines])
        self.assertEqual([1, 1, 1, 1], [s["attempt"] for s in state["steps"]])

    def test_journals_without_access_scope_remain_own_service_runs(self):
        system, state = self.prepare(mode="read-only", provider="command")
        saved = execution.load(self.run_dir, "run.json")
        self.assertEqual("service", saved.pop("access"))
        execution.save(self.run_dir, "run.json", saved)
        legacy = execution.validate_state(system, self.run_dir)
        self.assertEqual("service", execution.access_scope(legacy))
        self.assertEqual("service", execution.run_summary(legacy, self.run_dir)["access"])
        finished = execution.drive_run(system, self.run_dir, legacy)
        self.assertEqual("completed", finished["status"])
        saved = execution.load(self.run_dir, "run.json")
        saved["access"] = "everything"
        execution.save(self.run_dir, "run.json", saved)
        with self.assertRaises(SystemError):
            execution.validate_state(self.system(), self.run_dir)

    def test_process_lines_reach_the_observer_without_changing_the_result(self):
        script = self.root / "lines"
        script.write_text("#!/usr/bin/env python3\nimport sys\nsys.stdout.write('one\\ntwo\\nthree')\n")
        script.chmod(0o700)
        lines = []
        result = execution.run_process([str(script)], self.root, "", 10, on_line=lines.append)
        self.assertEqual([b"one", b"two", b"three"], lines)
        self.assertEqual(b"one\ntwo\nthree", result["stdout"])
        self.assertNotIn("stderr_tail", result)
        noisy = self.root / "noisy"
        noisy.write_text("#!/usr/bin/env python3\nimport sys\nsys.stderr.write('x' * 9000 + 'last words')\n")
        noisy.chmod(0o700)
        tail = execution.run_process([str(noisy)], self.root, "", 10, stderr_tail=True)["stderr_tail"]
        self.assertEqual((4096, b"last words"), (len(tail), tail[-10:]))
        def broken(line):
            raise RuntimeError("display failure")
        result = execution.run_process([str(script)], self.root, "", 10, on_line=broken)
        self.assertEqual((0, None, b"one\ntwo\nthree"), (result["returncode"], result["error"], result["stdout"]))

if __name__ == "__main__":
    unittest.main()
