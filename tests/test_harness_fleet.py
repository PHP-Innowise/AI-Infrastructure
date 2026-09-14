"""Fleet session boundaries plus optional offline LangGraph integration tests."""

from contextlib import closing
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import sessions


LENSES = ["code-reviewer", "security-reviewer", "performance-optimization"]
RUNTIME = sessions.fleet_runtime()

FAKE_NATIVE = r'''
import fcntl, json, os, pathlib, subprocess, sys, time
config = json.loads(pathlib.Path(CONFIG_PATH).read_text())
provider = config["provider"]
arguments = sys.argv[1:]
prompt = arguments[arguments.index("--") + 1] if provider == "cursor" else sys.stdin.read()
lens = next(value for value in ("code-reviewer", "security-reviewer", "performance-optimization") if value in prompt)
allowance = float(arguments[arguments.index("--max-budget-usd") + 1]) if "--max-budget-usd" in arguments else None
child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)"]) if config.get("spawn_child") else None
with open(config["log"], "a+", encoding="utf-8") as log:
    fcntl.flock(log, fcntl.LOCK_EX)
    log.seek(0)
    previous = [json.loads(line) for line in log if line.strip()]
    attempt = 1 + sum(item["lens"] == lens for item in previous)
    log.write(json.dumps({"provider": provider, "lens": lens, "attempt": attempt, "argv": arguments,
                          "cwd": os.getcwd(), "allowance": allowance, "pid": os.getpid(),
                          "group": os.getpgrp(), "child": child.pid if child else None,
                          **({"prompt": prompt} if config.get("capture_prompt") else {})}) + "\n")
    log.flush()
    fcntl.flock(log, fcntl.LOCK_UN)
if config.get("sleep"):
    time.sleep(config["sleep"])
failed = lens in config.get("fail_first", []) and attempt == 1
if failed:
    time.sleep(.2)
text = json.dumps({"findings": [{"file": lens + ".py" if config.get("distinct_files") else "fixture.py", "line": 1, "severity": "low",
                   "claim": config.get("claim", "Fixture finding " + lens),
                   "evidence": "Offline native executable fixture"}]}, ensure_ascii=False)
def emit(event):
    print(json.dumps(event), flush=True)
if provider == "codex":
    emit({"type": "thread.started", "thread_id": "fixture-native"})
    emit({"type": "item.completed", "item": {"type": "agent_message", "text": text}})
    emit({"type": "turn.failed", "error": {"message": "Fixture failure"}} if failed else {"type": "turn.completed"})
else:
    emit({"type": "system", "subtype": "init", "session_id": "fixture-native"})
    emit({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}})
    result = {"type": "result", "subtype": "error" if failed else "success", "is_error": failed,
              "result": text, "session_id": "fixture-native"}
    cost = config.get("costs", {}).get(lens, config.get("cost", .1))
    if provider == "claude" and cost is not None:
        result["total_cost_usd"] = cost
    emit(result)
sys.exit(7 if failed else 0)
'''


class _FleetFixture:
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "project"
        self.project.mkdir()
        (self.project / "README.md").write_text("Original fixture project\n", encoding="utf-8")
        self.managers = []
        providers = patch.object(sessions.providers, "discover_providers", return_value=[
            {"id": name, "name": name, "available": False, "executable": f"/never-launched/{name}"}
            for name in ("claude", "codex", "cursor")
        ])
        providers.start()
        self.addCleanup(providers.stop)
        native = patch.object(sessions.providers, "build_command", side_effect=AssertionError("Unexpected native model call"))
        self.native = native.start()
        self.addCleanup(native.stop)
        self.addCleanup(self.close_managers)

    def close_managers(self):
        for manager in self.managers:
            manager.close()

    def manager(self, execute=True):
        if execute:
            manager = sessions.Sessions(self.root / "state", [self.project], timeout=30)
        else:
            with patch.object(sessions.Sessions, "_worker", return_value=None):
                manager = sessions.Sessions(self.root / "state", [self.project], timeout=30)
        self.managers.append(manager)
        return manager

    def options(self, manager, **changes):
        return {"project_id": next(iter(manager.projects)), "provider": "codex", "mode": "plan",
                "workflow": "fleet-review", "prompt": "Review the fixture without modifying it",
                "agents_enabled": False,
                "fleet": {"lenses": list(LENSES), "dry_run": True, "budget_usd": None, "worker_timeout": 30},
                **changes}

    def wait_for_status(self, manager, sid, expected, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            session = manager.get(sid)
            if session["status"] in expected and manager.jobs.unfinished_tasks == 0:
                return session
            if session["status"] == "failed" and "failed" not in expected:
                self.fail(f"Fleet failed before reaching {expected}: {manager.events(sid)}")
            time.sleep(.02)
        self.fail(f"Fleet did not reach {expected}: {manager.get(sid)}, events={manager.events(sid)}")

    def test_budget_edit_preserves_checkpoint_and_ledger(self):
        manager=self.manager(execute=False)
        sid=manager.create(self.options(manager))['id']; manager._status(sid,'failed')
        directory=manager.state_dir/'fleet'/sid; directory.mkdir(parents=True)
        ledger=directory/'budget.sqlite'; checkpoint=directory/'checkpoints.sqlite'
        ledger.write_bytes(b'existing reservations'); checkpoint.write_bytes(b'completed reviewers')
        updated=manager.set_budgets(sid,{'revision':0,'budgets':{'usd':2,'tokens':500,'seconds':60},'worker_timeout':40})
        self.assertEqual(updated['fleet']['budget_usd'],2); self.assertEqual(updated['fleet']['worker_timeout'],40)
        self.assertEqual(updated['fleet']['lenses'],list(LENSES))
        self.assertEqual(ledger.read_bytes(),b'existing reservations'); self.assertEqual(checkpoint.read_bytes(),b'completed reviewers')

    def completed_reviewers(self, manager, sid):
        return [event["lens"] for event in manager.events(sid)
                if event.get("kind") == "fleet_reviewer" and event.get("status") == "completed"]

    def fake_native(self, manager, provider, name=None, **settings):
        name = name or provider
        config = self.root / f"{name}.json"
        log = self.root / f"{name}.jsonl"
        config.write_text(json.dumps({"provider": provider, "log": str(log), **settings}), encoding="utf-8")
        executable = self.root / f"{name}-cli"
        executable.write_text(f"#!{sys.executable}\nCONFIG_PATH = {str(config)!r}\n" + FAKE_NATIVE, encoding="utf-8")
        executable.chmod(0o755)
        manager.providers[provider].update(available=True, executable=str(executable))
        return log

    def fixture_runtime(self):
        # Keep the external process's model validation independent of user model caches.
        wrapper = self.root / "fixture-runtime"
        wrapper.write_text(
            f"#!{RUNTIME['executable']}\nimport runpy, sys\n"
            f"sys.path.insert(0, {str(Path(sessions.__file__).resolve().parents[1])!r})\n"
            "from harness import providers\n"
            "providers.model_options = lambda provider: {'models': [], 'efforts': [] if provider == 'cursor' else ['high'], 'detail': 'Fixture metadata'}\n"
            "sys.argv = sys.argv[1:]\nrunpy.run_path(sys.argv[0], run_name='__main__')\n",
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        return {**RUNTIME, "executable": str(wrapper)}

    @staticmethod
    def native_calls(log):
        return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

    def wait_until(self, predicate, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(.02)
        self.fail("Timed out waiting for owned Fleet fixture state")


class FleetValidationTests(_FleetFixture, unittest.TestCase):
    def setUp(self):
        super().setUp()
        runtime = patch.object(sessions, "fleet_runtime", return_value={
            "available": True, "executable": "/never-launched/runtime", "detail": "Fixture runtime",
            "lenses": list(LENSES), "max_worker_timeout": 3600,
        })
        runtime.start()
        self.addCleanup(runtime.stop)

    def test_invalid_fleet_options_do_not_create_sessions_or_files(self):
        manager = self.manager(execute=False)
        manager.providers["codex"]["available"] = True
        valid = self.options(manager)
        invalid_fleet = [None, [], "fleet"]
        changes = [{"lenses": value} for value in (None, [], "code-reviewer", ["unknown"],
                                                  ["code-reviewer", "code-reviewer"], [None])]
        changes += [{"dry_run": value} for value in (None, 0, 1, "true")]
        changes += [{"budget_usd": value} for value in (False, 0, -1, float("nan"), float("inf"), "1")]
        changes += [{"worker_timeout": value} for value in (None, False, 0, -1, 3601, "30", 1.5)]
        changes += [{"extra": True}]
        invalid_fleet += [{**valid["fleet"], **change} for change in changes]
        before = list(manager.db.iterdump())
        for fleet in invalid_fleet:
            with self.subTest(fleet=fleet), self.assertRaises(sessions.SessionError):
                manager.create({**valid, "fleet": fleet})
        for changes in ({"mode": "edit"}, {"project_id": "unregistered"}, {"project_id": {}},
                        {"provider": "unknown"}, {"workflow": "native"}, {"model": "unused-dry-run-model"}):
            with self.subTest(changes=changes), self.assertRaises(sessions.SessionError):
                manager.create({**valid, **changes})
        self.assertEqual(manager.list(), [])
        self.assertEqual(manager.jobs.qsize(), 0)
        self.assertEqual(list(manager.db.iterdump()), before)
        self.assertEqual([path.name for path in self.project.iterdir()], ["README.md"])
        self.assertFalse(any((manager.state_dir / "fleet").glob("*")))
        self.native.assert_not_called()

    def test_real_fleet_requires_helpers_provider_and_supported_budget_and_effort(self):
        manager = self.manager(execute=False)
        base = self.options(manager)
        real = {**base, "fleet": {**base["fleet"], "dry_run": False}}
        with self.assertRaises(sessions.SessionError):
            manager.create({**real, "agents_enabled": True})
        for provider in manager.providers.values():
            provider["available"] = True
        with self.assertRaises(sessions.SessionError):
            manager.create(real)
        for provider in ("codex", "cursor"):
            with self.subTest(provider=provider), self.assertRaises(sessions.SessionError):
                manager.create({**real, "provider": provider, "agents_enabled": True,
                                "fleet": {**real["fleet"], "budget_usd": 1.0}})
        with patch.object(sessions.providers, "model_options", return_value={
            "models": [], "efforts": ["high", "ultracode"], "detail": "Fixture catalog",
        }), self.assertRaises(sessions.SessionError):
            manager.create({**real, "provider": "claude", "agents_enabled": True,
                            "thinking_effort": "ultracode"})
        self.assertEqual(manager.list(), [])
        self.assertEqual(manager.jobs.qsize(), 0)
        # The accepted configuration remains queued; the fixture never launches a worker.
        created = manager.create({**real, "provider": "claude", "agents_enabled": True,
                                  "fleet": {**real["fleet"], "budget_usd": 1.25, "worker_timeout": 3600}})
        self.assertEqual(created["fleet"]["budget_usd"], 1.25)
        self.assertEqual(created["fleet"]["worker_timeout"], 3600)
        self.assertEqual(created["status"], "queued")
        self.native.assert_not_called()


@unittest.skipUnless(RUNTIME["available"], "Optional Fleet/LangGraph runtime is not installed")
class FleetRuntimeTests(_FleetFixture, unittest.TestCase):
    def assert_demonstration_findings(self, session, lenses):
        result = session["fleet_result"]
        self.assertEqual(len(result["findings"]), len(lenses))
        self.assertEqual({item["lens"] for item in result["findings"]}, set(lenses))
        for finding in result["findings"]:
            self.assertEqual(finding["file"], "DEMO.md")
            self.assertIn("Offline demonstration finding", finding["claim"])
            self.assertIn("Synthetic finding", finding["evidence"])
        self.assertEqual(result["cost_usd"], 0)
        self.assertEqual(result["skipped"], [])

    def test_native_provider_fixtures_reach_gate_with_readonly_model_effort_and_cwd(self):
        manager = self.manager()
        metadata = lambda provider: {"models": [], "efforts": [] if provider == "cursor" else ["high"],
                                     "detail": "Offline fixture metadata"}
        with patch.object(sessions, "fleet_runtime", return_value=self.fixture_runtime()), \
                patch.object(sessions.providers, "model_options", side_effect=metadata):
            for provider in ("claude", "codex", "cursor"):
                with self.subTest(provider=provider):
                    log = self.fake_native(manager, provider, capture_prompt=True)
                    settings = {"lenses": [LENSES[0]], "dry_run": False,
                                "budget_usd": None, "worker_timeout": 10}
                    sid = manager.create(self.options(manager, provider=provider, fleet=settings,
                        attachments=[{'name':'review.txt','data':'YXR0YWNobWVudA=='}],
                        agents_enabled=True, model="fixture-model", thinking_effort=None if provider == "cursor" else "high"))["id"]
                    waiting = self.wait_for_status(manager, sid, {"awaiting_approval"})
                    findings = waiting["fleet_result"]["findings"]
                    self.assertEqual([item["claim"] for item in findings], ["Fixture finding " + LENSES[0]])
                    self.assertEqual(waiting["fleet_result"]["cost_usd"], .1 if provider == "claude" else None)
                    calls = self.native_calls(log)
                    self.assertEqual(len(calls), 1)
                    self.assertEqual(calls[0]["cwd"], str(self.project))
                    _, content, attachment_path = manager.attachments.current(sid)[0]
                    self.assertEqual(content, b'attachment')
                    self.assertIn(attachment_path, calls[0]['prompt'])
                    arguments = calls[0]["argv"]
                    self.assertEqual(arguments[arguments.index("--model") + 1], "fixture-model")
                    if provider == "claude":
                        self.assertEqual(arguments[arguments.index('--add-dir')+1], str(Path(attachment_path).parent))
                        self.assertEqual(arguments[arguments.index("--permission-mode") + 1], "plan")
                        self.assertEqual(arguments[arguments.index("--effort") + 1], "high")
                        self.assertIn("--disallowedTools", arguments)
                        self.assertTrue({"Agent", "Task", "Workflow"}.issubset(arguments))
                    elif provider == "codex":
                        self.assertEqual(arguments[arguments.index("--sandbox") + 1], "read-only")
                        self.assertEqual(arguments[arguments.index("--cd") + 1], str(self.project))
                        self.assertIn('model_reasoning_effort="high"', arguments)
                        self.assertIn("agents.enabled=false", arguments)
                    else:
                        self.assertIn("--disable-auto-update", arguments)
                        self.assertEqual(arguments[arguments.index("--sandbox") + 1], "enabled")
                        self.assertEqual(arguments[arguments.index("--mode") + 1], "plan")
                        self.assertNotIn("--effort", arguments)
                    manager.decide(sid, False)
                    self.wait_for_status(manager, sid, {"rejected"})
        self.native.assert_not_called()

    def test_linked_fleet_uses_reviewed_context_and_existing_task_in_selected_nested_bank(self):
        from tests.test_harness_knowledge import install_knowledge_fixture, metadata

        edition = self.project / "PHP Core"
        edition.mkdir()
        install_knowledge_fixture(edition)
        manager = self.manager()
        log = self.fake_native(manager, "codex", capture_prompt=True)
        settings = {"lenses": [LENSES[0]], "dry_run": False, "budget_usd": None, "worker_timeout": 10}
        linked = {"bank": "PHP Core/memory-bank", "task_id": "TASK-LINKED-FLEET", "query": "cobalt allocation",
                  "create": True, "goal": "Review the cobalt allocation rule"}
        with patch.object(sessions, "fleet_runtime", return_value=self.fixture_runtime()):
            sid = manager.create(self.options(manager, fleet=settings, agents_enabled=True, brain=linked))["id"]
            prepared = self.wait_for_status(manager, sid, {"awaiting_context"})
            self.assertEqual(self.native_calls(log), [])
            task_id = prepared["brain"]["task"]["id"]
            manager.run_context(sid, prepared["brain"]["context_id"])
            waiting = self.wait_for_status(manager, sid, {"awaiting_approval"})
            self.assertTrue(waiting["fleet_result"]["brain_available"])
            self.assertEqual(waiting["fleet_result"]["task_id"], "TASK-LINKED-FLEET")
            tasks = list((edition / "project-brain/dynamic/tasks").glob("*.md"))
            self.assertEqual([path.stem for path in tasks], [task_id])
            self.assertFalse((self.project / "project-brain").exists())
            calls = self.native_calls(log)
            self.assertEqual(len(calls), 1)
            self.assertEqual(calls[0]["cwd"], str(self.project))
            self.assertIn(json.dumps(prepared["brain"]["capsule"], ensure_ascii=False), calls[0]["prompt"])
            self.assertNotEqual(metadata(tasks[0])["status"], "completed")
            manager.decide(sid, True)
            completed = self.wait_for_status(manager, sid, {"completed"})
            self.assertEqual(completed["fleet_result"]["task_id"], "TASK-LINKED-FLEET")
            self.assertIn("Fixture finding", manager.report(sid))
            self.assertEqual(len(self.native_calls(log)), 1)
            self.assertNotEqual(metadata(tasks[0])["status"], "completed")
        self.native.assert_not_called()

    def test_failed_native_lens_is_not_a_finding_and_retry_keeps_completed_peer_and_cost(self):
        manager = self.manager()
        log = self.fake_native(manager, "claude", fail_first=[LENSES[1]],
                               costs={LENSES[0]: 0.0, LENSES[1]: .1})
        settings = {"lenses": LENSES[:2], "dry_run": False, "budget_usd": .4, "worker_timeout": 10}
        with patch.object(sessions, "fleet_runtime", return_value=self.fixture_runtime()):
            sid = manager.create(self.options(manager, provider="claude", fleet=settings,
                                               agents_enabled=True, agent_count=2))["id"]
            failed = self.wait_for_status(manager, sid, {"failed"})
            self.assertFalse(failed["fleet_result"])
            self.assertEqual(self.completed_reviewers(manager, sid), [LENSES[0]])
            self.assertTrue(any(event.get("kind") == "fleet_reviewer" and event.get("lens") == LENSES[1]
                                and event.get("status") == "failed" for event in manager.events(sid)))
            with self.assertRaises(sessions.SessionError):
                manager.report(sid)
            calls = self.native_calls(log)
            self.assertEqual(len(calls), 2)
            self.assertLessEqual(sum(call["allowance"] for call in calls), .4)
            manager.resume(sid)
            waiting = self.wait_for_status(manager, sid, {"awaiting_approval"})
            calls = self.native_calls(log)
            self.assertEqual([call["lens"] for call in calls].count(LENSES[0]), 1)
            self.assertEqual([call["lens"] for call in calls].count(LENSES[1]), 2)
            self.assertEqual({item["claim"] for item in waiting["fleet_result"]["findings"]},
                             {"Fixture finding " + lens for lens in LENSES[:2]})
            self.assertAlmostEqual(waiting["fleet_result"]["cost_usd"], .2)
            with closing(sqlite3.connect(manager.state_dir / "fleet" / sid / "budget.sqlite")) as ledger:
                reservations = ledger.execute("SELECT lens,allowance,cost FROM reservations").fetchall()
            self.assertEqual(len(reservations), 3)
            self.assertAlmostEqual(sum(row[2] for row in reservations), .2)
            manager.decide(sid, True)
            approved = self.wait_for_status(manager, sid, {"completed"})
            self.assertAlmostEqual(approved["fleet_result"]["cost_usd"], .2)
            self.assertIn("$0.20", manager.report(sid))
            self.assertEqual(len(self.native_calls(log)), 3)

    def test_resumed_native_reviewer_uses_edited_cap_and_keeps_uncapped_spending(self):
        manager=self.manager()
        with patch.object(sessions,'fleet_runtime',return_value=self.fixture_runtime()):
            for initial in (None,.1):
                log=self.fake_native(manager,'claude',name='changed-'+str(initial),cost=.15,fail_first=[LENSES[0]])
                settings={'lenses':[LENSES[0]],'dry_run':False,'budget_usd':initial,'worker_timeout':10}
                sid=manager.create(self.options(manager,provider='claude',fleet=settings,agents_enabled=True))['id']
                self.wait_for_status(manager,sid,{'failed'})
                manager.set_budgets(sid,{'revision':0,'budgets':{'usd':.5,'tokens':500,'seconds':20}})
                manager.resume(sid); result=self.wait_for_status(manager,sid,{'awaiting_approval'})
                calls=self.native_calls(log)
                self.assertEqual(len(calls),2); self.assertAlmostEqual(calls[1]['allowance'],.35)
                self.assertAlmostEqual(result['fleet_result']['cost_usd'],.3)

    def test_exhausted_or_unmetered_failed_allocation_cannot_launch_another_reviewer(self):
        manager = self.manager()
        with patch.object(sessions, "fleet_runtime", return_value=self.fixture_runtime()):
            for name, cost in (("exhausted", .1), ("unmetered", None), ("overspent", .162)):
                with self.subTest(allocation=name):
                    log = self.fake_native(manager, "claude", name=name, cost=cost, fail_first=[LENSES[0]])
                    settings = {"lenses": [LENSES[0]], "dry_run": False,
                                "budget_usd": .1, "worker_timeout": 10}
                    sid = manager.create(self.options(manager, provider="claude", fleet=settings,
                                                       agents_enabled=True, agent_count=1))["id"]
                    self.wait_for_status(manager, sid, {"failed"})
                    self.assertEqual(len(self.native_calls(log)), 1)
                    if name == 'overspent':
                        self.assertTrue(any(event.get('kind') == 'fleet_reviewer'
                                            and event.get('limit_reached') == 'USD'
                                            for event in manager.events(sid)))
                    manager.resume(sid)
                    retry = self.wait_for_status(manager, sid, {"failed"})
                    self.assertFalse(retry["fleet_result"])
                    self.assertEqual(len(self.native_calls(log)), 1)
                    with closing(sqlite3.connect(manager.state_dir / "fleet" / sid / "budget.sqlite")) as ledger:
                        rows = ledger.execute("SELECT allowance,cost FROM reservations").fetchall()
                    self.assertEqual(rows, [(.1, cost)])
                    with self.assertRaises(sessions.SessionError):
                        manager.report(sid)

    def test_large_unicode_findings_can_be_approved_without_resending_the_result_as_runner_input(self):
        manager = self.manager()
        claim = "ф" * 20000
        log = self.fake_native(manager, "claude", claim=claim, distinct_files=True)
        settings = {"lenses": list(LENSES), "dry_run": False, "budget_usd": None, "worker_timeout": 10}
        with patch.object(sessions, "fleet_runtime", return_value=self.fixture_runtime()):
            sid = manager.create(self.options(manager, provider="claude", fleet=settings, agents_enabled=True))["id"]
            waiting = self.wait_for_status(manager, sid, {"awaiting_approval"})
            self.assertGreater(len(json.dumps(waiting).encode("utf-8")), 262144)
            self.assertEqual([item["claim"] for item in waiting["fleet_result"]["findings"]], [claim] * len(LENSES))
            manager.decide(sid, True)
            self.wait_for_status(manager, sid, {"completed"})
            self.assertIn(claim, manager.report(sid))
            self.assertEqual(len(self.native_calls(log)), len(LENSES))

    @unittest.skipUnless(Path("/proc").is_dir(), "Owned process cleanup checks require procfs")
    def test_crashed_fleet_parent_stops_native_descendants_and_releases_runner_lock_for_restart(self):
        manager = self.manager()
        log = self.fake_native(manager, "codex", name="crash", sleep=120, spawn_child=True)
        settings = {"lenses": [LENSES[0]], "dry_run": False, "budget_usd": None, "worker_timeout": 120}
        with patch.object(sessions, "fleet_runtime", return_value=self.fixture_runtime()):
            sid = manager.create(self.options(manager, fleet=settings, agents_enabled=True))["id"]

            def first_call():
                try:
                    calls = self.native_calls(log)
                    return calls[0] if calls else None
                except json.JSONDecodeError:
                    return None

            call = self.wait_until(first_call)
            self.assertNotEqual(call["group"], os.getpgrp())

            def kill_owned_group():
                try:
                    os.killpg(call["group"], signal.SIGKILL)
                except ProcessLookupError:
                    pass

            self.addCleanup(kill_owned_group)
            with manager.lock:
                parent = manager.active[sid]
                os.killpg(parent.pid, signal.SIGKILL)
            self.wait_for_status(manager, sid, {"failed"})

            def stopped(pid):
                try:
                    return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] == "Z"
                except FileNotFoundError:
                    return True

            self.wait_until(lambda: stopped(call["pid"]) and stopped(call["child"]))
            manager.close()
            self.managers.remove(manager)
            restarted = self.manager()
            self.fake_native(restarted, "codex", name="crash")
            restarted.resume(sid)
            self.wait_for_status(restarted, sid, {"awaiting_approval"})
            self.assertEqual(len(self.native_calls(log)), 2)
            restarted.decide(sid, False)
            self.wait_for_status(restarted, sid, {"rejected"})

    def test_dry_run_checkpoint_resumes_after_restart_without_repeating_reviewers_and_publishes_only_on_approval(self):
        manager = self.manager()
        options = self.options(manager)
        sid = manager.create(options)["id"]
        waiting = self.wait_for_status(manager, sid, {"awaiting_approval"})
        self.assertEqual(waiting["fleet"], options["fleet"])
        self.assert_demonstration_findings(waiting, LENSES)
        self.assertFalse(waiting["fleet_result"].get("report"))
        self.assertCountEqual(self.completed_reviewers(manager, sid), LENSES)
        with self.assertRaises(sessions.SessionError):
            manager.report(sid)
        with self.assertRaises(sessions.SessionError):
            manager.send(sid, "Fleet reviews cannot receive chat followups")
        with self.assertRaises(sessions.SessionError):
            manager.resume(sid)
        checkpoint_dir = manager.state_dir / "fleet" / sid
        self.assertTrue(checkpoint_dir.is_dir())
        checkpoint = checkpoint_dir / "checkpoints.sqlite"
        self.assertTrue(checkpoint.is_file())
        self.assertEqual(checkpoint.stat().st_mode & 0o077, 0)
        self.assertEqual(checkpoint_dir.stat().st_mode & 0o077, 0)
        self.assertFalse(list(checkpoint_dir.rglob("*.md")))
        # A parent crash after the saved gate but before recording its final status
        # must resume the existing graph instead of dispatching completed lenses again.
        manager._status(sid, "interrupted")
        manager.close()
        self.managers.remove(manager)
        restarted = self.manager()
        self.assertEqual(restarted.get(sid)["fleet_result"], waiting["fleet_result"])
        restarted.resume(sid)
        resumed = self.wait_for_status(restarted, sid, {"awaiting_approval"})
        self.assert_demonstration_findings(resumed, LENSES)
        self.assertCountEqual(self.completed_reviewers(restarted, sid), LENSES)
        restarted.decide(sid, True)
        approved = self.wait_for_status(restarted, sid, {"completed"})
        report = restarted.report(sid)
        self.assertEqual(report, approved["fleet_result"]["report"])
        self.assertIn("Offline demonstration finding", report)
        self.assertIn("Synthetic finding", report)
        self.assertCountEqual(self.completed_reviewers(restarted, sid), LENSES)
        self.assertTrue(list(checkpoint_dir.rglob("*.md")))
        self.assertEqual([path.name for path in self.project.iterdir()], ["README.md"])
        self.assertEqual((self.project / "README.md").read_text(), "Original fixture project\n")
        self.native.assert_not_called()

    def test_rejection_keeps_report_private_and_concurrent_decisions_accept_only_one(self):
        manager = self.manager()
        one_lens = {"lenses": [LENSES[0]], "dry_run": True, "budget_usd": None, "worker_timeout": 30}
        rejected_id = manager.create(self.options(manager, fleet=one_lens))["id"]
        self.wait_for_status(manager, rejected_id, {"awaiting_approval"})
        manager.decide(rejected_id, False)
        rejected = self.wait_for_status(manager, rejected_id, {"rejected"})
        self.assertFalse(rejected["fleet_result"].get("report"))
        with self.assertRaises(sessions.SessionError):
            manager.report(rejected_id)
        self.assertFalse(list((manager.state_dir / "fleet" / rejected_id).rglob("*.md")))

        sid = manager.create(self.options(manager, fleet=one_lens))["id"]
        self.wait_for_status(manager, sid, {"awaiting_approval"})
        barrier = threading.Barrier(3)
        outcomes = []

        def decide():
            barrier.wait(5)
            try:
                manager.decide(sid, True)
                outcomes.append("accepted")
            except sessions.SessionError:
                outcomes.append("refused")

        threads = [threading.Thread(target=decide) for _ in range(2)]
        for thread in threads:
            thread.start()
        barrier.wait(5)
        for thread in threads:
            thread.join(5)
            self.assertFalse(thread.is_alive())
        self.assertCountEqual(outcomes, ["accepted", "refused"])
        self.wait_for_status(manager, sid, {"completed"})
        self.assertTrue(manager.report(sid))
        self.assertEqual(self.completed_reviewers(manager, sid), [LENSES[0]])
        self.native.assert_not_called()

    @unittest.skipUnless(shutil.which("git"), "Git worktree integration requires git")
    def test_fleet_runtime_executes_in_the_session_worktree_and_keeps_primary_dirty_files(self):
        environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        environment.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                            "GIT_AUTHOR_NAME": "Harness fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
                            "GIT_COMMITTER_NAME": "Harness fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid"})
        for arguments in (("init", "--quiet"), ("add", "README.md"),
                          ("commit", "--quiet", "-m", "Initialize Fleet fixture")):
            subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgSign=false",
                            "-C", str(self.project), *arguments], env=environment, check=True,
                           stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        (self.project / "README.md").write_text("Dirty primary Fleet fixture\n", encoding="utf-8")
        cwd_log = self.root / "runtime-cwd.log"
        wrapper = self.root / "fleet-python"
        wrapper.write_text(
            f"#!{sys.executable}\nimport os, sys\n"
            f"with open({str(cwd_log)!r}, 'a', encoding='utf-8') as log:\n    log.write(os.getcwd() + '\\n')\n"
            f"os.execv({RUNTIME['executable']!r}, [{RUNTIME['executable']!r}, *sys.argv[1:]])\n",
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        manager = self.manager()
        with patch.object(sessions, "fleet_runtime", return_value={**RUNTIME, "executable": str(wrapper)}):
            sid = manager.create(self.options(manager, workspace="worktree"))["id"]
            waiting = self.wait_for_status(manager, sid, {"awaiting_approval"})
            worktree = manager.state_dir / "worktrees" / sid
            self.assertEqual(waiting["project_path"], str(worktree))
            self.assertEqual((worktree / "README.md").read_text(), "Original fixture project\n")
            manager.decide(sid, False)
            self.wait_for_status(manager, sid, {"rejected"})
        self.assertEqual(cwd_log.read_text().splitlines(), [str(worktree), str(worktree)])
        self.assertEqual((self.project / "README.md").read_text(), "Dirty primary Fleet fixture\n")
        self.assertEqual([path.name for path in worktree.iterdir() if path.name != ".git"], ["README.md"])
        self.native.assert_not_called()


if __name__ == "__main__":
    unittest.main()
