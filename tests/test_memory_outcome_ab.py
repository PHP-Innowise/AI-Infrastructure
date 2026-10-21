"""Offline tests for scripts/memory_outcome_ab.py: no model, no network."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import memory_outcome_ab as ab  # noqa: E402


def event(kind: str, item: dict) -> dict:
    return {"type": kind, "item": item}


class PathTest(unittest.TestCase):
    def test_a_bash_lc_wrapper_yields_its_script(self) -> None:
        self.assertEqual("cat src/A.php", ab.unwrap_shell("/usr/bin/bash -lc 'cat src/A.php'"))
        self.assertEqual("rg -n x", ab.unwrap_shell("rg -n x"))

    def test_project_path_keeps_paths_inside_the_root(self) -> None:
        root = "/work/project"
        self.assertEqual("src/A.php", ab.project_path("src/A.php", root, root))
        self.assertEqual("src/A.php", ab.project_path("/work/project/src/A.php:12", "/tmp", root))
        self.assertEqual("src/A.php", ab.project_path("A.php#L3", "/work/project/src", root))
        self.assertIsNone(ab.project_path("/etc/passwd", root, root))
        self.assertIsNone(ab.project_path("../other/x.php", root, root))
        self.assertIsNone(ab.project_path("-n", root, root))

    def test_shell_syntax_is_not_a_file(self) -> None:
        for word in ("s/old/new/", "+x", "app/$S/c.txt", "a b.php"):
            self.assertTrue(ab.SHELL_SYNTAX.search(word), word)
        for word in ("src/A.php", "packages/web/app/util/token.ts"):
            self.assertFalse(ab.SHELL_SYNTAX.search(word), word)


class CounterTest(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.corpus = Path(self.directory.name)
        (self.corpus / "src").mkdir()
        (self.corpus / "src" / "Gold.php").write_text("<?php\n", encoding="utf-8")
        (self.corpus / "src" / "Other.php").write_text("<?php\n", encoding="utf-8")

    def tearDown(self) -> None:
        self.directory.cleanup()

    def command(self, identity: str, script: str) -> dict:
        return {"id": identity, "type": "command_execution", "command": f"/usr/bin/bash -lc '{script}'"}

    def test_the_first_gold_touch_stops_the_run_with_its_call_number(self) -> None:
        counter = ab.Counter(self.corpus, ["src/Gold.php"], cap=40)
        self.assertIsNone(counter.feed(event("item.started", self.command("1", "ls"))))
        self.assertIsNone(counter.feed(event("item.completed", self.command("1", "ls"))))
        self.assertIsNone(counter.feed(event("item.started", self.command("2", "cat src/Other.php"))))
        self.assertEqual("gold", counter.feed(event("item.started", self.command("3", "sed -n 1,20p src/Gold.php"))))
        self.assertEqual(3, counter.first_gold)
        self.assertEqual(3, counter.calls)

    def test_messages_and_reasoning_are_not_calls(self) -> None:
        counter = ab.Counter(self.corpus, ["src/Gold.php"], cap=40)
        counter.feed(event("item.completed", {"id": "m", "type": "agent_message", "text": "src/Gold.php"}))
        counter.feed(event("item.completed", {"id": "r", "type": "reasoning", "text": "x"}))
        self.assertEqual(0, counter.calls)
        self.assertIsNone(counter.first_gold)

    def test_a_file_change_touches_its_path(self) -> None:
        counter = ab.Counter(self.corpus, ["src/Gold.php"], cap=40)
        change = {"id": "c", "type": "file_change",
                  "changes": [{"path": str(self.corpus / "src" / "Gold.php"), "kind": "update"}]}
        self.assertEqual("gold", counter.feed(event("item.completed", change)))
        self.assertEqual(1, counter.first_gold)

    def test_the_cap_stops_a_run_that_never_touches_gold(self) -> None:
        counter = ab.Counter(self.corpus, ["src/Gold.php"], cap=2)
        self.assertIsNone(counter.feed(event("item.started", self.command("1", "ls"))))
        self.assertEqual("cap", counter.feed(event("item.started", self.command("2", "cat src/Other.php"))))
        self.assertIsNone(counter.first_gold)

    def test_turn_completion_records_usage(self) -> None:
        counter = ab.Counter(self.corpus, [], cap=40)
        counter.feed({"type": "turn.completed", "usage": {"input_tokens": 5}})
        self.assertTrue(counter.finished)
        self.assertEqual({"input_tokens": 5}, counter.usage)


class HookArgumentsTest(unittest.TestCase):
    def test_only_the_memory_prompt_hook_is_passed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            corpus = Path(directory)
            (corpus / ".codex").mkdir()
            read = "sh -c 'exec x' sh working-memory-read.sh"
            (corpus / ".codex" / "hooks.json").write_text(json.dumps({"hooks": {
                "SessionStart": [{"hooks": [{"type": "command", "command": "sh local-context.sh"}]}],
                "UserPromptSubmit": [{"hooks": [
                    {"type": "command", "command": read, "additionalContextLimit": 4000},
                    {"type": "command", "command": "sh -c 'exec x' sh context-continuity.sh"},
                ]}],
                "Stop": [{"hooks": [{"type": "command", "command": "sh working-memory-write.sh"}]}],
            }}), encoding="utf-8")
            arguments = ab.hook_arguments(corpus)
            capsule = ab.codex_command("codex", corpus, "capsule", "do it", None)
            off = ab.codex_command("codex", corpus, "off", "do it", None)
        self.assertIn("--dangerously-bypass-hook-trust", arguments)
        flag = next(value for value in arguments if value.startswith("hooks.UserPromptSubmit="))
        self.assertIn("working-memory-read.sh", flag)
        self.assertNotIn("context-continuity", flag)
        self.assertFalse(any("Stop" in value or "SessionStart" in value for value in arguments))
        self.assertIn("features.hooks=false", off)
        self.assertNotIn("--dangerously-bypass-hook-trust", off)
        self.assertEqual("do it", capsule[-1])
        self.assertIn("workspace-write", capsule)


class ReportTest(unittest.TestCase):
    def run_pair(self, calls_to_gold, delivered: bool = False) -> dict:
        return {"stopped": "gold" if calls_to_gold is not None else "cap", "calls_to_gold": calls_to_gold,
                "capsule_delivered": delivered}

    def result(self, pairs) -> dict:
        items = {}
        for number, (off, capsule) in enumerate(pairs):
            items[f"T{number}"] = {"status": "ok", "arms": {
                "off": [self.run_pair(value) for value in off],
                "capsule": [self.run_pair(value, delivered=True) for value in capsule],
            }}
        return {"meta": {"cap": 40}, "items": items}

    def test_a_run_without_a_gold_touch_counts_one_past_the_cap(self) -> None:
        self.assertEqual(41, ab.censored({"calls_to_gold": None}, 40))
        self.assertEqual(7, ab.censored({"calls_to_gold": 7}, 40))

    def test_replicates_are_averaged_per_arm(self) -> None:
        summary = ab.summarize(self.result([((4, 6), (2, 2))]))
        self.assertEqual(1, summary["pairs"])
        self.assertEqual(5.0, summary["mean_off"])
        self.assertEqual(2.0, summary["mean_capsule"])
        self.assertEqual(-3.0, summary["mean_difference"])

    def test_fewer_than_twenty_pairs_is_insufficient(self) -> None:
        summary = ab.summarize(self.result([((5,), (3,))] * 19))
        self.assertEqual("insufficient", summary["verdict"])

    def test_a_consistent_saving_is_capsule_helps(self) -> None:
        summary = ab.summarize(self.result([((6,), (3,))] * 20 + [((4,), (4,))] * 5))
        self.assertEqual("capsule-helps", summary["verdict"])
        self.assertLess(summary["ci95"][1], 0)

    def test_a_consistent_cost_is_capsule_hurts(self) -> None:
        summary = ab.summarize(self.result([((3,), (6,))] * 22))
        self.assertEqual("capsule-hurts", summary["verdict"])

    def test_noise_is_no_difference_shown(self) -> None:
        summary = ab.summarize(self.result([((3,), (5,)), ((5,), (3,))] * 11))
        self.assertEqual("no-difference-shown", summary["verdict"])

    def test_a_pair_whose_capsule_did_not_arrive_is_left_out(self) -> None:
        result = self.result([((5,), (3,))])
        result["items"]["T0"]["arms"]["capsule"][0]["capsule_delivered"] = False
        self.assertEqual(0, ab.summarize(result)["pairs"])

    def test_the_sign_test_is_exact_and_two_sided(self) -> None:
        self.assertEqual(1.0, ab.sign_test([0, 0]))
        self.assertAlmostEqual(0.0625, ab.sign_test([-1, -1, -1, -1, -1]), places=6)
        self.assertEqual(ab.bootstrap([1.0, 2.0, 3.0]), ab.bootstrap([1.0, 2.0, 3.0]))


class SampleTest(unittest.TestCase):
    """Cold starts from a synthetic Claude Code transcript."""

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        base = Path(self.directory.name)
        self.project = base / "Desktop" / "demo"
        (self.project / "src").mkdir(parents=True)
        for name in ("Gold.php", "Named.php"):
            (self.project / "src" / name).write_text("<?php\n", encoding="utf-8")
        environment = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
        for command in (["init", "-q"], ["add", "-A"],
                        ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-qm", "init"]):
            subprocess.run(["git", "-C", str(self.project), *command], check=True, env=environment)
        self.transcripts = base / "projects"
        folder = ab.transcript_directory(self.transcripts, self.project)
        folder.mkdir(parents=True)
        root = str(self.project)
        records = [
            {"type": "user", "timestamp": "2026-09-01T10:00:00Z", "cwd": root,
             "message": {"content": "Fix the rounding; Named.php is fine as it is."}},
            {"type": "assistant", "cwd": root, "message": {"content": [
                {"type": "tool_use", "name": "Read", "input": {"file_path": f"{root}/src/Named.php"}},
                {"type": "tool_use", "name": "Edit", "input": {"file_path": f"{root}/src/Gold.php"}},
                {"type": "tool_use", "name": "Edit", "input": {"file_path": f"{root}/src/Named.php"}},
                {"type": "tool_use", "name": "Bash", "input": {"command": "sed -i 's/a/b/' src/Gold.php; chmod +x tools/run.sh"}},
            ]}},
            {"type": "user", "timestamp": "2026-09-01T10:05:00Z", "cwd": root,
             "message": {"content": "Now the tests"}},
        ]
        (folder / "session-1.jsonl").write_text("\n".join(json.dumps(record) for record in records) + "\n", encoding="utf-8")
        (folder / "subagents").mkdir()
        (folder / "subagents" / "side.jsonl").write_text(json.dumps(records[0]) + "\n", encoding="utf-8")

    def tearDown(self) -> None:
        self.directory.cleanup()

    def test_a_cold_start_keeps_changed_files_the_prompt_does_not_name(self) -> None:
        before = ab.instant("2026-10-07")
        tasks = list(ab.cold_starts(self.project, self.transcripts, None, before))
        self.assertEqual(1, len(tasks))
        self.assertEqual(["src/Gold.php"], tasks[0]["gold"])
        self.assertTrue(tasks[0]["prompt"].startswith("Fix the rounding"))

    def test_the_window_excludes_later_sessions(self) -> None:
        self.assertEqual([], list(ab.cold_starts(self.project, self.transcripts, None, ab.instant("2026-08-01"))))

    def test_bare_dates_are_utc_midnight(self) -> None:
        self.assertEqual("2026-10-07T00:00:00+00:00", ab.instant("2026-10-07").isoformat())
        self.assertIsNone(ab.instant("tomorrow"))


class RunArmTest(unittest.TestCase):
    """A fake `codex` that streams canned events, so stopping is exercised."""

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        base = Path(self.directory.name)
        self.corpus = base / "corpus"
        (self.corpus / "src").mkdir(parents=True)
        (self.corpus / "src" / "Gold.php").write_text("<?php\n", encoding="utf-8")
        manifests = self.corpus / "memory-bank" / "local" / "retrieval-manifests"
        manifests.mkdir(parents=True)
        self.fake = base / "fake-codex.py"
        self.fake.write_text(textwrap.dedent(f"""\
            import json, sys, time
            from pathlib import Path
            manifests = Path({str(manifests)!r})
            if "--dangerously-bypass-hook-trust" in sys.argv:
                (manifests / "hook.json").write_text(json.dumps({{"entry_point": "refresh"}}))
            (manifests / "agent.json").write_text(json.dumps({{"entry_point": "retrieve"}}))
            def emit(event):
                print(json.dumps(event), flush=True)
            emit({{"type": "thread.started"}})
            emit({{"type": "item.started", "item": {{"id": "1", "type": "command_execution", "command": "/usr/bin/bash -lc 'ls'"}}}})
            emit({{"type": "item.started", "item": {{"id": "2", "type": "command_execution", "command": "/usr/bin/bash -lc 'cat src/Gold.php'"}}}})
            time.sleep(30)
            emit({{"type": "turn.completed", "usage": {{}}}})
            """), encoding="utf-8")

    def tearDown(self) -> None:
        self.directory.cleanup()

    def test_the_run_stops_at_the_gold_touch_and_tells_hook_from_agent(self) -> None:
        command = [sys.executable, str(self.fake), "--dangerously-bypass-hook-trust", "prompt"]
        outcome = ab.run_arm(command, self.corpus, ["src/Gold.php"], cap=40, timeout=20)
        self.assertEqual("gold", outcome["stopped"])
        self.assertEqual(2, outcome["calls_to_gold"])
        self.assertLess(outcome["seconds"], 15)
        self.assertTrue(outcome["capsule_delivered"])
        self.assertEqual({"refresh": 1, "retrieve": 1}, outcome["manifests"])

    def test_an_agent_retrieve_is_not_a_delivered_capsule(self) -> None:
        command = [sys.executable, str(self.fake), "prompt"]
        outcome = ab.run_arm(command, self.corpus, ["src/Gold.php"], cap=40, timeout=20)
        self.assertFalse(outcome["capsule_delivered"])

    def test_the_deadline_stops_a_quiet_session(self) -> None:
        command = [sys.executable, str(self.fake), "prompt"]
        outcome = ab.run_arm(command, self.corpus, ["src/Missing.php"], cap=40, timeout=2)
        self.assertEqual("timeout", outcome["stopped"])
        self.assertIsNone(outcome["calls_to_gold"])


class OutputsStayOutsideTheCloneTest(unittest.TestCase):
    def test_a_set_inside_the_clone_is_refused(self) -> None:
        code = ab.main(["sample", "--projects", "x", "--before", "2026-10-07", "--n", "1", "--seed", "1",
                        "--out", str(ROOT / "set.json")])
        self.assertEqual(2, code)
        self.assertFalse((ROOT / "set.json").exists())


if __name__ == "__main__":
    unittest.main()
