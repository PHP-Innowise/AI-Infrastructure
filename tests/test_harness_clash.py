"""Clash option: ledger logic offline, cycles through the session queue with fixture CLIs, HTTP boundary."""
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
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import clash, providers, sessions, web

FAKE_PARTICIPANT = r'''#!/usr/bin/env python3
import json, os, pathlib, re, sys, time
config = json.loads(pathlib.Path(CONFIG_PATH).read_text())
provider = config["provider"]
arguments = sys.argv[1:]
prompt = arguments[arguments.index("--") + 1] if provider == "cursor" else sys.stdin.read()
role = "challenger" if "You are the CHALLENGER" in prompt else "response" if "respond to the challenger" in prompt else "opening"
match = re.search(r"round (\d+) of (\d+)", prompt)
round_number = int(match.group(1)) if match and role != "opening" else 0
with open(config["log"], "a", encoding="utf-8") as log:
    log.write(json.dumps({"provider": provider, "role": role, "round": round_number, "argv": arguments,
                          "cwd": os.getcwd(), "pid": os.getpid(), "prompt": prompt}) + "\n")
if config.get("sleep_on") == role:
    time.sleep(30)
if config.get("write") and role in ("opening", "response"):
    with open(config["write"], "a", encoding="utf-8") as handle:
        handle.write(role + " " + str(round_number) + "\n")
script = config.get("script", {})
text = script.get(role + ":" + str(round_number)) or script.get(role) or "Fixture " + role + " summary."
native = config.get("native_id", "fixture-" + provider)
def emit(event):
    print(json.dumps(event), flush=True)
if provider == "codex":
    emit({"type": "thread.started", "thread_id": native})
    emit({"type": "item.completed", "item": {"type": "command_execution", "status": "completed", "exit_code": 0}})
    emit({"type": "item.completed", "item": {"type": "agent_message", "text": text}})
    emit({"type": "turn.completed", "usage": {"input_tokens": 10, "output_tokens": 5}})
else:
    emit({"type": "system", "subtype": "init", "session_id": native})
    emit({"type": "assistant", "message": {"role": "assistant", "content": [
        {"type": "tool_use", "name": "Read", "id": "tool-1"}, {"type": "text", "text": text}]}})
    emit({"type": "result", "subtype": "success", "is_error": False, "result": text, "session_id": native,
          "total_cost_usd": config.get("cost", 0.05), "usage": {"input_tokens": 20, "output_tokens": 7}})
sys.exit(0)
'''

CATALOG = {
    "claude": {"models": [{"id": "fixture-claude", "label": "Claude fixture", "efforts": ["high", "ultracode"]}],
               "efforts": ["high", "ultracode"], "detail": "Offline Claude fixture catalog"},
    "codex": {"models": [{"id": "fixture-model", "label": "Fixture", "efforts": ["low", "high"]}],
              "efforts": ["low", "high"], "detail": "Offline fixture catalog"},
    "cursor": {"models": [], "efforts": [], "detail": "Offline Cursor fixture catalog"},
}
CLASH = {"challenger": "codex", "rounds": 2, "challenger_model": None, "challenger_thinking_effort": None}


def fenced(payload):
    return "Prose before the block.\n```json\n" + json.dumps(payload) + "\n```\n"


def state_for(stage, **changes):
    state = clash.new_state({"provider": "claude", "model": None, "thinking_effort": None, "clash": dict(CLASH)}, stage)
    state.update(changes)
    return state


class ClashLogicTests(unittest.TestCase):
    def test_validate_normalizes_and_rejects_invalid_settings(self):
        with patch.object(providers, "model_options", side_effect=lambda provider: CATALOG[provider]):
            self.assertIsNone(clash.validate(None, "native", "claude"))
            self.assertIsNone(clash.validate(None, "plan", "claude"))
            settings = clash.validate({"challenger": "codex", "challenger_model": "", "challenger_thinking_effort": ""}, "native", "claude")
            self.assertEqual(settings, CLASH)
            self.assertEqual(clash.validate({"challenger": "codex", "rounds": 3, "challenger_model": "fixture-model",
                                             "challenger_thinking_effort": "low"}, "review", "claude")["challenger_thinking_effort"], "low")
            for invalid in ({}, "codex", {"challenger": "claude"}, {"challenger": "gemini"}, {"challenger": "codex", "rounds": 0},
                            {"challenger": "codex", "rounds": "2"}, {"challenger": "codex", "rounds": clash.MAX_ROUNDS + 1},
                            {"challenger": "codex", "challenger_model": "-flag"},
                            {"challenger": "codex", "challenger_model": "fixture-model", "challenger_thinking_effort": "max"},
                            {"challenger": "codex", "extra": 1}, {"stage": "implement", "challenger": "codex"}):
                with self.subTest(invalid=invalid), self.assertRaises(sessions.SessionError):
                    clash.validate(invalid, "native", "claude")
            for workflow in ("plan", "sdd", "fleet-review", "clash"):
                with self.subTest(workflow=workflow), self.assertRaisesRegex(sessions.SessionError, "Workspace and Review"):
                    clash.validate({"challenger": "codex"}, workflow, "claude")
            with self.assertRaisesRegex(sessions.SessionError, "Ultracode"):
                clash.validate({"challenger": "claude", "challenger_thinking_effort": "ultracode"}, "native", "codex")
        self.assertEqual((clash.stage_for("edit"), clash.stage_for("plan")), ("implement", "review"))

    def test_json_extraction_prefers_the_last_fenced_block_and_prose_strips_it(self):
        text = ('Example: ```json\n{"verdict": "accept", "objections": []}\n```\nActually no.\n'
                '```json\n{"verdict": "reject", "objections": [{"id": 1, "claim": "Bug", "severity": "high"}], "resolved": []}\n```\n')
        self.assertEqual(clash.extract_json(text, ("verdict",))["verdict"], "reject")
        bare = 'Summary.\n{"responses": [{"id": 1, "action": "fixed", "note": "done"}]}'
        self.assertEqual(clash.extract_json(bare, ("responses",))["responses"][0]["action"], "fixed")
        self.assertIsNone(clash.extract_json("no structure {here", ("verdict",)))
        self.assertIsNone(clash.extract_json('{"other": 1}', ("verdict",)))
        self.assertEqual(clash.prose(text), "Example: Actually no.")
        self.assertEqual(clash.prose(bare), "Summary.")
        self.assertEqual(clash.prose(None), "")

    def test_implementation_ledger_transitions_and_convergence(self):
        state = state_for("implement")
        clash.apply_challenge(state, {"verdict": "reject", "objections": [
            {"id": 7, "severity": "high", "file": "app.py", "line": 3, "claim": "Null check missing", "evidence": "Read app.py"},
            {"severity": "bogus", "claim": "Weak name", "line": "x"}, "junk"]}, 1)
        self.assertEqual([(item["id"], item["severity"], item["status"]) for item in state["items"]],
                         [(1, "high", "open"), (2, "unrated", "open")])
        self.assertEqual(state["verdict"], "reject")
        self.assertFalse(clash.converged(state))
        clash.apply_response(state, {"responses": [{"id": 1, "action": "fixed", "note": "Added guard"}, {"id": 1, "action": "rebutted"},
                                                    {"id": 9, "action": "fixed"}, {"id": 2, "action": "promise"}]}, 1)
        self.assertEqual([item["status"] for item in state["items"]], ["fixed", "open"])
        self.assertEqual(state["items"][1]["history"][-1]["action"], "no response")
        # Accept with an unrated objection still open does not converge; the note says so.
        clash.apply_challenge(state, {"verdict": "accept", "objections": [{"id": 2, "severity": "low", "claim": "Weak name"}], "resolved": [1]}, 2)
        self.assertEqual([item["status"] for item in state["items"]], ["resolved", "open"])
        self.assertEqual(state["items"][1]["severity"], "low")
        self.assertTrue(clash.converged(state))
        self.assertEqual(state["verdict_note"], "")
        state["items"][1]["severity"] = "medium"
        clash.apply_challenge(state, {"verdict": "accept", "objections": [{"id": 2, "claim": "Weak name"}]}, 3)
        self.assertFalse(clash.converged(state))
        self.assertIn("does not converge", state["verdict_note"])
        # Silence about a rebutted objection withdraws it; about a fixed one resolves it; a withdrawn one can be reopened.
        state = state_for("implement")
        clash.apply_challenge(state, {"verdict": "reject", "objections": [{"id": 1, "claim": "A"}, {"id": 2, "claim": "B"}, {"id": 3, "claim": "C"}]}, 1)
        clash.apply_response(state, {"responses": [{"id": 1, "action": "rebutted"}, {"id": 2, "action": "fixed"}, {"id": 3, "action": "deferred", "note": "Out of scope"}]}, 1)
        clash.apply_challenge(state, {"objections": []}, 2)
        self.assertEqual([item["status"] for item in state["items"]], ["withdrawn", "resolved", "deferred"])
        self.assertIsNone(state["verdict"])
        self.assertIn("no valid verdict", state["verdict_note"])
        clash.apply_challenge(state, {"verdict": "reject", "objections": [{"id": 1, "claim": "A again", "severity": "medium"}]}, 3)
        self.assertEqual((state["items"][0]["status"], state["items"][0]["history"][-1]["action"], state["items"][0]["claim"]),
                         ("open", "reopened", "A again"))
        self.assertEqual(len(clash.unresolved(state)), 1)

    def test_review_ledger_transitions_and_convergence(self):
        state = state_for("review")
        clash.apply_review_turn(state, "reviewer", {"findings": [{"id": 1, "severity": "high", "claim": "SQL built by concatenation", "file": "repo.php"},
                                                                 {"id": 2, "severity": "low", "claim": "Unused import"}]}, 0)
        self.assertEqual([item["status"] for item in state["items"]], ["proposed", "proposed"])
        clash.apply_review_turn(state, "challenger", {"verdict": "reject", "assessments": [
            {"id": 1, "stance": "dispute", "evidence": "Parameters are bound"}, {"id": 2, "stance": "agree"}, {"id": 2, "stance": "dispute"}],
            "additional": [{"severity": "medium", "claim": "Missing index", "file": "schema.sql"}]}, 1)
        self.assertEqual([(item["id"], item["origin"], item["status"]) for item in state["items"]],
                         [(1, "reviewer", "disputed"), (2, "reviewer", "confirmed"), (3, "challenger", "proposed")])
        self.assertFalse(clash.converged(state))
        clash.apply_review_turn(state, "reviewer", {"responses": [{"id": 1, "action": "defend", "note": "Line 40 concatenates"}, {"id": 2, "action": "withdraw"}],
                                                    "assessments": [{"id": 3, "stance": "agree"}, {"id": 1, "stance": "agree"}]}, 1)
        self.assertEqual([item["status"] for item in state["items"]], ["defended", "confirmed", "confirmed"])
        clash.apply_review_turn(state, "challenger", {"verdict": "accept", "assessments": [{"id": 1, "stance": "agree"}]}, 2)
        self.assertEqual([item["status"] for item in state["items"]], ["confirmed", "confirmed", "confirmed"])
        self.assertTrue(clash.converged(state))
        # Accepting while a defended finding stays unassessed does not converge.
        state = state_for("review")
        clash.apply_review_turn(state, "reviewer", {"findings": [{"claim": "X"}]}, 0)
        clash.apply_review_turn(state, "challenger", {"verdict": "accept"}, 1)
        self.assertFalse(clash.converged(state))
        self.assertEqual(state["items"][0]["history"][-1]["action"], "not assessed")
        self.assertIn("does not converge", state["verdict_note"])

    def test_prompts_ledger_bounds_report_and_carried_state(self):
        state = state_for("implement", task="Add a login form", summary="Implemented it.")
        opening = clash.opening_prompt(state, "Add a login form")
        self.assertIn("You are the IMPLEMENTER (Claude Code)", opening)
        self.assertNotIn("respond to the challenger", opening)
        challenge = clash.challenge_prompt(state, 1, {"text": "diff --git a/x b/x", "truncated": True})
        self.assertIn("You are the CHALLENGER (Codex), round 1 of 2", challenge)
        self.assertIn("diff --git", challenge)
        self.assertIn("truncated", challenge)
        self.assertNotIn("Objection ledger", challenge)
        clash.apply_challenge(state, {"verdict": "reject", "objections": [{"id": 1, "claim": "Bug " * 200, "evidence": "E " * 400}]}, 1)
        response = clash.response_prompt(state, 1)
        self.assertIn("respond to the challenger", response)
        self.assertIn('"status": "open"', response)
        self.assertLessEqual(len(clash.ledger_for_prompt(state["items"], limit=900)), 900)
        self.assertIn("truncated", clash.ledger_for_prompt(state["items"], limit=300))
        follow = clash.continue_prompt(state, "Also add tests")
        self.assertIn("continuing the clash", follow)
        self.assertIn("Also add tests", follow)
        review = state_for("review", task="Review src/")
        self.assertIn("You are REVIEWER A (Claude Code)", clash.opening_prompt(review, "Review src/"))
        self.assertIn("reviewer B, Codex", clash.challenge_prompt(review, 2))
        self.assertIn("defend", clash.response_prompt(review, 1))
        state["outcome"] = "unresolved"
        clash.record_turn(state, {"cycle": 1, "round": 1, "role": "challenger", "provider": "codex", "ok": True, "summary": "Found one bug",
                                  "cost_usd": None, "seconds": 2.5, "verdict": "reject"})
        report = clash.render_report(state)
        self.assertIn("# Clash — implementation stage", report)
        self.assertIn("#1 [unrated] open", report)
        self.assertIn("cost unknown, 2.5s; verdict reject — Found one bug", report)
        self.assertIn("Unresolved after", clash.outcome_message(state))
        state["outcome"] = "converged"
        self.assertIn("Converged in round", clash.outcome_message(state))
        previous = {**state, "challenger_session_id": "thread-9"}
        session = {"provider": "claude", "clash": {**CLASH, "rounds": 1}}
        carried = clash.new_state(session, "implement", previous)
        self.assertEqual((carried["cycle"], carried["challenger_session_id"], len(carried["items"]), carried["task"], len(carried["turns"])),
                         (2, "thread-9", 1, "Add a login form", 1))
        # Another stage keeps the turn history but starts a fresh ledger; another challenger keeps the ledger but not its native session.
        switched = clash.new_state(session, "review", previous)
        self.assertEqual((switched["cycle"], switched["items"], switched["task"], len(switched["turns"])), (2, [], "", 1))
        swapped = clash.new_state({"provider": "claude", "clash": {**CLASH, "challenger": "cursor"}}, "implement", previous)
        self.assertEqual((swapped["challenger_session_id"], len(swapped["items"]), swapped["challenger"]["provider"]), (None, 1, "cursor"))


@unittest.skipUnless(hasattr(os, "killpg"), "Session workers require POSIX process groups")
class ClashCycleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / "project"
        self.project.mkdir()
        self.log = self.root / "participants.log"
        self.fixtures = {}
        for provider in ("claude", "codex"):
            config = self.root / f"{provider}.json"
            script = self.root / f"fake-{provider}.py"
            script.write_text(FAKE_PARTICIPANT.replace("CONFIG_PATH", json.dumps(str(config))))
            script.chmod(0o755)
            self.fixtures[provider] = config
            self.configure(provider)
        discovery = patch.object(sessions.providers, "discover_providers", return_value=[
            {"id": provider, "name": providers.PROVIDERS[provider], "available": provider != "cursor",
             "executable": str(self.root / f"fake-{provider}.py") if provider != "cursor" else None}
            for provider in ("claude", "codex", "cursor")])
        discovery.start()
        self.addCleanup(discovery.stop)
        catalog = patch.object(sessions.providers, "model_options", side_effect=lambda provider: CATALOG[provider])
        catalog.start()
        self.addCleanup(catalog.stop)
        self.managers = []
        self.addCleanup(self.close_managers)

    def close_managers(self):
        for manager in self.managers:
            manager.close()

    def configure(self, provider, **settings):
        self.fixtures[provider].write_text(json.dumps({"provider": provider, "log": str(self.log), **settings}))

    def manager(self, timeout=30):
        manager = sessions.Sessions(self.root / "state", [self.project], timeout=timeout)
        self.managers.append(manager)
        return manager

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.project), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", *args],
                              check=True, capture_output=True, text=True).stdout

    def entries(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def wait_for(self, predicate, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(.02)
        self.fail("Timed out waiting for the clash fixture")

    def settled(self, manager, sid):
        self.wait_for(lambda: manager.get(sid)["status"] not in sessions.ACTIVE and manager.jobs.unfinished_tasks == 0)
        return manager.get(sid)

    def options(self, manager, **changes):
        return {"project_id": next(iter(manager.projects)), "provider": "claude", "workflow": "native", "mode": "edit",
                "prompt": "Add a greeting to app.py", "clash": {"challenger": "codex", "rounds": 2}, **changes}

    @unittest.skipUnless(shutil.which("git"), "Git workspace tests require git")
    def test_implementation_clash_converges_resumes_both_participants_and_continues(self):
        self.git("init", "-q", "-b", "main")
        (self.project / "app.py").write_text("print('hi')\n")
        self.git("add", "."); self.git("commit", "-q", "-m", "Base")
        objection = {"verdict": "reject", "objections": [{"id": 1, "severity": "high", "file": "app.py", "line": 1,
                     "claim": "Greeting is never called", "evidence": "app.py defines greet() without calling it"}], "resolved": []}
        self.configure("claude", write=str(self.project / "app.py"),
                       script={"opening": "Implemented greet() in app.py and ran the tests.",
                               "response:1": fenced({"responses": [{"id": 1, "action": "fixed", "note": "Called greet() at import"}]})})
        self.configure("codex", script={"challenger:1": "Found a defect.\n" + fenced(objection),
                                        "challenger:2": fenced({"verdict": "accept", "objections": [], "resolved": [1]})})
        manager = self.manager()
        sid = manager.create(self.options(manager))["id"]
        session = self.settled(manager, sid)
        self.assertEqual(session["status"], "completed", manager.events(sid))
        self.assertEqual((session["workflow"], session["mode"]), ("native", "edit"))
        self.assertEqual(session["clash"], CLASH)
        result = session["clash_result"]
        self.assertEqual((result["stage"], result["status"], result["outcome"], result["cycle"], result["round"], result["verdict"]),
                         ("implement", "finished", "converged", 1, 2, "accept"))
        self.assertEqual([(turn["role"], turn["provider"], turn["round"], turn["ok"]) for turn in result["turns"]],
                         [("implementer", "claude", 0, True), ("challenger", "codex", 1, True), ("implementer", "claude", 1, True), ("challenger", "codex", 2, True)])
        self.assertEqual(result["turns"][1]["verdict"], "reject")
        self.assertEqual(result["turns"][0]["cost_usd"], 0.05)
        self.assertEqual([entry["action"] for entry in result["items"][0]["history"]], ["raised", "fixed", "resolved"])
        self.assertEqual(result["items"][0]["status"], "resolved")
        self.assertEqual(result["challenger_session_id"], "fixture-codex")
        self.assertEqual(session["native_session_id"], "fixture-claude")
        self.assertIn("Converged in round 2", result["message"])
        self.assertIn("# Clash — implementation stage", result["report"])
        entries = self.entries()
        self.assertEqual([(e["provider"], e["role"], e["round"]) for e in entries],
                         [("claude", "opening", 0), ("codex", "challenger", 1), ("claude", "response", 1), ("codex", "challenger", 2)])
        self.assertTrue(all(e["cwd"] == str(self.project) for e in entries))
        self.assertNotIn("--resume", entries[0]["argv"])
        self.assertIn("acceptEdits", entries[0]["argv"])
        self.assertIn("read-only", entries[1]["argv"])
        self.assertNotIn("resume", entries[1]["argv"])
        self.assertEqual(entries[2]["argv"][entries[2]["argv"].index("--resume") + 1], "fixture-claude")
        self.assertIn("fixture-codex", entries[3]["argv"])
        self.assertIn("resume", entries[3]["argv"])
        self.assertIn("Implemented greet()", entries[1]["prompt"])
        self.assertIn("diff --git a/app.py b/app.py", entries[1]["prompt"])
        self.assertIn("+opening 0", entries[1]["prompt"])
        self.assertIn("Additional agents are disabled", entries[1]["prompt"])
        self.assertIn('"status": "open"', entries[2]["prompt"])
        self.assertIn('"status": "fixed"', entries[3]["prompt"])
        self.assertIn("Called greet() at import", entries[3]["prompt"])
        events = manager.events(sid)
        turns = [event for event in events if event["kind"] == "clash_turn"]
        self.assertEqual([(t["role"], t["provider"], t["status"]) for t in turns][:2], [("implementer", "claude", "running"), ("implementer", "claude", "completed")])
        self.assertEqual(len(turns), 8)
        self.assertTrue(any(e["kind"] == "text" and e.get("provider") == "codex" and e.get("role") == "challenger" and "Found a defect" in e["text"] for e in events))
        self.assertTrue(any(e["kind"] == "tool" and e.get("provider") == "claude" for e in events))
        self.assertEqual([e["native_session_id"] for e in events if e["kind"] == "session"], ["fixture-claude"])
        self.assertFalse(any(e["kind"] == "clash_state" for e in events))
        self.assertTrue(any("Clash: Claude Code (implementer) versus Codex (challenger)" in e.get("text", "") for e in events))
        history = manager.results.history(sid)
        self.assertEqual([launch["kind"] for launch in history["launches"]], ["clash"])
        self.assertEqual(history["launches"][0]["settings"]["clash"], session["clash"])
        self.assertEqual(session["budget_usage"]["tokens"], 84)
        self.assertEqual(session["budget_usage"]["cost_usd"], 0.1)
        snapshot = manager.results.snapshot(sid)
        self.assertIn("+response 1", snapshot["diff"])
        # A follow-up starts the next cycle: the implementer and the challenger both resume their native sessions.
        for invalid in ({"clash": {"challenger": "claude"}}, {"clash": {"challenger": "codex", "rounds": 0}},
                        {"clash": {"stage": "review", "challenger": "codex"}}, {"agents_enabled": True, "agent_count": 2},
                        {"mode": "plan"}, {"model_routing": {"plan": {"model": None, "thinking_effort": None}, "edit": {"model": None, "thinking_effort": None}}}):
            with self.subTest(invalid=invalid), self.assertRaises(sessions.SessionError):
                manager.send(sid, "Also add tests", invalid)
        self.configure("codex", script={"challenger:1": fenced({"verdict": "reject", "objections": [
            {"id": 2, "severity": "medium", "file": "tests/test_app.py", "claim": "No test covers greet()", "evidence": "tests/ is empty"}], "resolved": []})})
        manager.send(sid, "Also add tests", {"clash": {"challenger": "codex", "rounds": 1}})
        session = self.settled(manager, sid)
        self.assertEqual(session["status"], "completed", manager.events(sid))
        self.assertEqual(session["clash"]["rounds"], 1)
        result = session["clash_result"]
        self.assertEqual((result["outcome"], result["cycle"], result["round"], result["verdict"]), ("unresolved", 2, 1, "reject"))
        self.assertEqual([(item["id"], item["status"]) for item in result["items"]], [(1, "resolved"), (2, "open")])
        self.assertEqual(len(result["turns"]), 6)
        self.assertEqual(result["task"], "Add a greeting to app.py")
        self.assertEqual(result["followup"], "Also add tests")
        entries = self.entries()
        self.assertEqual(entries[4]["argv"][entries[4]["argv"].index("--resume") + 1], "fixture-claude")
        self.assertIn("continuing the clash", entries[4]["prompt"])
        self.assertIn("Also add tests", entries[4]["prompt"])
        self.assertIn("fixture-codex", entries[5]["argv"])
        self.assertIn("Latest human follow-up", entries[5]["prompt"])
        self.assertEqual([launch["kind"] for launch in manager.results.history(sid)["launches"]], ["clash", "clash"])
        manager.close(); self.managers.remove(manager)
        reopened = self.manager()
        restored = reopened.get(sid)
        self.assertEqual(restored["clash"], session["clash"])
        self.assertEqual(restored["clash_result"]["items"], result["items"])

    def test_review_clash_on_the_review_workflow_assesses_defends_and_converges(self):
        self.configure("codex", script={
            "opening": fenced({"findings": [{"id": 1, "severity": "high", "file": "repo.php", "line": 40, "claim": "SQL built by concatenation", "evidence": "Line 40"},
                                            {"id": 2, "severity": "low", "file": "Service.php", "claim": "Unused import", "evidence": "Line 3"}]}),
            "response:1": fenced({"responses": [{"id": 1, "action": "defend", "note": "The variable is user input from the request"}],
                                  "assessments": [{"id": 3, "stance": "agree", "evidence": "Confirmed the missing index"}]})})
        self.configure("claude", script={
            "challenger:1": fenced({"verdict": "reject", "assessments": [{"id": 1, "stance": "dispute", "evidence": "Parameters look bound"}, {"id": 2, "stance": "agree"}],
                                    "additional": [{"id": 3, "severity": "medium", "file": "schema.sql", "claim": "Missing index on user_id", "evidence": "No index"}]}),
            "challenger:2": fenced({"verdict": "accept", "assessments": [{"id": 1, "stance": "agree", "evidence": "Verified the request source"}]})})
        manager = self.manager()
        sid = manager.create(self.options(manager, provider="codex", workflow="review", mode="plan", prompt="Review src/ for persistence bugs",
                                          clash={"challenger": "claude", "rounds": 2, "challenger_model": "fixture-claude",
                                                 "challenger_thinking_effort": "high"}))["id"]
        session = self.settled(manager, sid)
        self.assertEqual(session["status"], "completed", manager.events(sid))
        self.assertEqual((session["workflow"], session["mode"]), ("review", "plan"))
        result = session["clash_result"]
        self.assertEqual((result["stage"], result["outcome"], result["round"], result["verdict"]), ("review", "converged", 2, "accept"))
        self.assertEqual([(item["id"], item["origin"], item["status"]) for item in result["items"]],
                         [(1, "reviewer", "confirmed"), (2, "reviewer", "confirmed"), (3, "challenger", "confirmed")])
        self.assertEqual([entry["action"] for entry in result["items"][0]["history"]], ["raised", "dispute", "defend", "agree"])
        self.assertEqual(result["challenger"]["model"], "fixture-claude")
        self.assertEqual([(turn["role"], turn["provider"]) for turn in result["turns"]],
                         [("reviewer", "codex"), ("challenger", "claude"), ("reviewer", "codex"), ("challenger", "claude")])
        entries = self.entries()
        self.assertIn("read-only", entries[0]["argv"])
        self.assertEqual(entries[1]["argv"][entries[1]["argv"].index("--permission-mode") + 1], "plan")
        self.assertEqual(entries[1]["argv"][entries[1]["argv"].index("--model") + 1], "fixture-claude")
        self.assertEqual(entries[1]["argv"][entries[1]["argv"].index("--effort") + 1], "high")
        self.assertIn("You are REVIEWER A (Codex)", entries[0]["prompt"])
        # The clash prompts define both roles; the Review workflow's own prefix must not be layered on top.
        self.assertFalse(any("Review the requested scope" in entry["prompt"] for entry in entries))
        self.assertIn('"status": "disputed"', entries[2]["prompt"])
        self.assertIn("# Clash — review stage", result["report"])
        self.assertIn("#3 [medium] confirmed — schema.sql (challenger)", result["report"])

    def test_clash_can_be_enabled_and_disabled_between_turns(self):
        self.configure("codex", script={"challenger:1": fenced({"verdict": "accept", "objections": [], "resolved": []})})
        manager = self.manager()
        sid = manager.create(self.options(manager, clash=None))["id"]
        session = self.settled(manager, sid)
        self.assertEqual((session["status"], session["clash"], session["clash_result"]), ("completed", None, None))
        manager.send(sid, "Now let Codex challenge it", {"clash": {"challenger": "codex"}})
        session = self.settled(manager, sid)
        self.assertEqual(session["status"], "completed", manager.events(sid))
        self.assertEqual(session["clash"], CLASH)
        self.assertEqual((session["clash_result"]["cycle"], session["clash_result"]["outcome"]), (1, "converged"))
        entries = self.entries()
        self.assertEqual([(e["provider"], e["role"]) for e in entries], [("claude", "opening"), ("claude", "opening"), ("codex", "challenger")])
        self.assertEqual(entries[1]["argv"][entries[1]["argv"].index("--resume") + 1], "fixture-claude")
        self.assertIn("You are the IMPLEMENTER (Claude Code)", entries[1]["prompt"])
        self.assertNotIn("You are the IMPLEMENTER", entries[0]["prompt"])
        manager.send(sid, "Plain follow-up", {"clash": None})
        session = self.settled(manager, sid)
        self.assertEqual((session["status"], session["clash"]), ("completed", None))
        self.assertEqual(session["clash_result"]["cycle"], 1)
        self.assertNotIn("clash", self.entries()[3]["prompt"].lower())
        self.assertEqual([launch["kind"] for launch in manager.results.history(sid)["launches"]], ["native", "clash", "native"])
        manager.send(sid, "Challenge again", {"clash": {"challenger": "codex", "rounds": 1}})
        session = self.settled(manager, sid)
        self.assertEqual((session["status"], session["clash_result"]["cycle"]), ("completed", 2))
        self.assertIn("continuing the clash", self.entries()[4]["prompt"])
        self.assertIn("fixture-codex", self.entries()[5]["argv"])
        with self.assertRaisesRegex(sessions.SessionError, "unavailable"):
            manager.send(sid, "Swap", {"clash": {"challenger": "cursor"}})

    def test_missing_verdict_is_incomplete_and_a_followup_starts_the_next_cycle(self):
        self.configure("codex", script={"challenger:1": "I would rather not answer in JSON today."})
        manager = self.manager()
        sid = manager.create(self.options(manager))["id"]
        session = self.settled(manager, sid)
        self.assertEqual(session["status"], "failed")
        result = session["clash_result"]
        self.assertEqual((result["status"], result["outcome"], result["cycle"]), ("finished", "incomplete", 1))
        self.assertEqual(len(result["turns"]), 2)
        self.assertTrue(any(e["kind"] == "error" and "structured verdict" in e["text"] for e in manager.events(sid)))
        self.assertIn("Incomplete", result["message"])
        self.configure("codex", script={"challenger:1": fenced({"verdict": "accept", "objections": [], "resolved": []})})
        manager.send(sid, "Try again", {})
        session = self.settled(manager, sid)
        self.assertEqual(session["status"], "completed", manager.events(sid))
        self.assertEqual((session["clash_result"]["outcome"], session["clash_result"]["cycle"]), ("converged", 2))
        self.assertIn("continuing the clash", self.entries()[2]["prompt"])

    def test_cancel_stops_the_running_participant_and_keeps_the_interim_ledger(self):
        self.configure("codex", sleep_on="challenger")
        manager = self.manager()
        sid = manager.create(self.options(manager))["id"]
        entry = self.wait_for(lambda: next((e for e in self.entries() if e["role"] == "challenger"), None))
        manager.cancel(sid)
        session = self.settled(manager, sid)
        self.assertEqual(session["status"], "cancelled")
        self.assertEqual(session["clash_result"]["status"], "running")
        self.assertEqual([turn["role"] for turn in session["clash_result"]["turns"]], ["implementer"])
        self.assertEqual(session["native_session_id"], "fixture-claude")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                os.kill(entry["pid"], 0)
            except ProcessLookupError:
                break
            time.sleep(.05)
        else:
            self.fail("The sleeping challenger outlived the cancelled session")
        with self.assertRaises(sessions.SessionError):
            manager.send(sid, "Continue", {"clash": {"challenger": "claude"}})

    def test_invalid_clash_options_create_no_sessions_or_processes(self):
        manager = self.manager()
        base = self.options(manager)
        for changes in ({"clash": {"challenger": "claude"}}, {"clash": {"challenger": "cursor"}}, {"clash": {"stage": "implement", "challenger": "codex"}},
                        {"clash": {"challenger": "codex", "rounds": None}}, {"agents_enabled": True}, {"thinking_effort": "ultracode", "agents_enabled": True},
                        {"model_routing": {"plan": {"model": None, "thinking_effort": None}, "edit": {"model": None, "thinking_effort": None}}},
                        {"clash": {"challenger": "codex", "challenger_thinking_effort": "ultracode"}},
                        {"workflow": "plan", "mode": "plan"}, {"workflow": "sdd", "sdd": {"phase": "specify", "feature": "login"}},
                        {"workflow": "fleet-review", "mode": "plan"}, {"workflow": "clash", "mode": "plan"}):
            with self.subTest(changes=changes), self.assertRaises(sessions.SessionError):
                manager.create({**base, **changes})
        self.assertEqual(manager.list(), [])
        self.assertFalse(self.log.exists())


class ClashHttpTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / "project"
        self.project.mkdir()
        discovery = patch.object(providers, "discover_providers", return_value=[
            {"id": "claude", "name": "Claude Code", "available": True, "executable": "/never-launched/claude", "detail": "Offline"},
            {"id": "codex", "name": "Codex", "available": True, "executable": "/never-launched/codex", "detail": "Offline"}])
        discovery.start(); self.addCleanup(discovery.stop)
        catalog = patch.object(providers, "model_options", side_effect=lambda provider: CATALOG[provider])
        catalog.start(); self.addCleanup(catalog.stop)
        worker = patch.object(web.Sessions, "_worker", return_value=None)
        worker.start(); self.addCleanup(worker.stop)
        self.server = web.HarnessServer(("127.0.0.1", 0), self.root / "state", [self.project])
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": .01}, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)
        self.project_id = next(iter(self.server.sessions.projects))

    def close_server(self):
        self.server.close(); self.thread.join(2)

    def request(self, path, method="GET", data=None, token=True):
        body = json.dumps(data).encode() if data is not None else None
        headers = {"Host": f"127.0.0.1:{self.server.server_port}"}
        if body is not None:
            headers.update({"Content-Type": "application/json", "Content-Length": str(len(body))})
        if token and method == "POST":
            headers["X-Harness-Token"] = self.server.token
        connection = http.client.HTTPConnection("127.0.0.1", self.server.server_port, timeout=5)
        try:
            connection.request(method, path, body=body, headers=headers)
            response = connection.getresponse()
            return response.status, json.loads(response.read())
        finally:
            connection.close()

    def mark_completed(self, sid, native="native-1"):
        store = self.server.sessions
        with store.lock:
            store.db.execute("UPDATE sessions SET status='completed', native_session_id=? WHERE id=?", (native, sid))
            store.db.commit()

    def test_bootstrap_creation_and_followups_carry_the_clash_option(self):
        status, boot = self.request("/api/bootstrap")
        self.assertEqual(status, 200)
        self.assertEqual(boot["clash"]["workflows"], ["native", "review"])
        self.assertEqual(set(boot["clash"]["stages"]), {"implement", "review"})
        self.assertEqual((boot["clash"]["max_rounds"], boot["clash"]["default_rounds"]), (clash.MAX_ROUNDS, clash.DEFAULT_ROUNDS))
        self.assertNotIn("clash", [workflow["id"] for workflow in boot["workflows"]])
        options = {"project_id": self.project_id, "provider": "claude", "workflow": "native", "prompt": "Implement the widget",
                   "mode": "edit", "clash": {"challenger": "codex", "rounds": 3}}
        self.assertEqual(self.request("/api/sessions", "POST", options, token=False)[0], 403)
        status, payload = self.request("/api/sessions", "POST", {**options, "clash": {"challenger": "claude"}})
        self.assertEqual(status, 400)
        self.assertIn("different provider", payload["error"])
        self.assertEqual(self.request("/api/sessions", "POST", {**options, "agents_enabled": True})[0], 400)
        status, payload = self.request("/api/sessions", "POST", {**options, "workflow": "plan", "mode": "plan"})
        self.assertEqual(status, 400)
        self.assertIn("Workspace and Review", payload["error"])
        status, payload = self.request("/api/sessions", "POST", options)
        self.assertEqual(status, 201, payload)
        session = payload["session"]
        self.assertEqual((session["workflow"], session["mode"], session["clash"]["rounds"], session["clash_result"]), ("native", "edit", 3, None))
        sid = session["id"]
        self.mark_completed(sid)
        status, payload = self.request(f"/api/sessions/{sid}/messages", "POST", {"prompt": "Now the tests", "clash": {"challenger": "codex", "rounds": 1}})
        self.assertEqual(status, 200, payload)
        self.assertEqual(payload["session"]["clash"]["rounds"], 1)
        self.mark_completed(sid)
        status, payload = self.request(f"/api/sessions/{sid}/messages", "POST", {"prompt": "Ordinary turn", "clash": None})
        self.assertEqual(status, 200, payload)
        self.assertIsNone(payload["session"]["clash"])
        self.mark_completed(sid)
        status, payload = self.request(f"/api/sessions/{sid}/messages", "POST", {"prompt": "Switch", "clash": {"stage": "review", "challenger": "codex"}})
        self.assertEqual(status, 400)
        self.assertEqual(self.request(f"/api/sessions/{sid}/messages", "POST", {"prompt": "Switch", "fleet": {}})[0], 400)
        status, payload = self.request(f"/api/sessions/{sid}/results?diff=0")
        self.assertEqual(status, 200)
        self.assertEqual([launch["kind"] for launch in payload["launches"]], [])


if __name__ == "__main__":
    unittest.main()
