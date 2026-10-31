"""Offline tests for scripts/memory_outcome_ab.py: no model, no network."""
from __future__ import annotations

import contextlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock

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

    def test_a_finished_answer_naming_gold_is_an_answer_only_without_a_touch(self) -> None:
        counter = ab.Counter(self.corpus, ["src/Gold.php"], cap=40)
        counter.feed(event("item.started", self.command("1", "context.py retrieve rounding")))
        counter.feed(event("item.completed", {"id": "m1", "type": "agent_message", "text": "Looking."}))
        counter.feed(event("item.completed", {"id": "m2", "type": "agent_message",
                                              "text": "The rounding lives in `Gold.php`, saved last session."}))
        self.assertFalse(counter.answered())
        counter.feed({"type": "turn.completed", "usage": {}})
        self.assertTrue(counter.answered())
        elsewhere = ab.Counter(self.corpus, ["src/Gold.php"], cap=40)
        elsewhere.feed(event("item.completed", {"id": "m", "type": "agent_message", "text": "See src/Other.php."}))
        elsewhere.feed({"type": "turn.completed", "usage": {}})
        self.assertFalse(elsewhere.answered())


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
        with tempfile.TemporaryDirectory() as directory:
            corpus = Path(directory)
            (corpus / ".codex").mkdir()
            (corpus / ".codex" / "hooks.json").write_text(json.dumps({"hooks": {"UserPromptSubmit": [
                {"hooks": [{"type": "command", "command": read}]}]}}), encoding="utf-8")
            navigation = ab.codex_command("codex", corpus, "navigation", "do it", None)
            ab.set_capsule_profile(corpus, "navigation")
            config = json.loads((corpus / "project-brain/config/runtime.json").read_text(encoding="utf-8"))
        self.assertIn("--dangerously-bypass-hook-trust", navigation)
        self.assertEqual("navigation", config["capsule_profile"])
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

    def test_the_ticket_arm_is_compared_with_recall_and_its_line_counted(self) -> None:
        def run(calls, ran, ticket):
            return {"stopped": "gold", "calls_to_gold": calls, "capsule_delivered": False, "hook_ran": ran,
                    "ticket_files_delivered": ticket}
        items = {f"T{n}": {"status": "ok", "arms": {
            "recall": [run(6, True, False)], "ticket": [run(3, True, n % 3 != 0)],
        }} for n in range(21)}
        # A ticket run whose hook never ran got no treatment.
        items["T0"]["arms"]["ticket"][0]["hook_ran"] = False
        summary = ab.summarize({"meta": {"cap": 40}, "items": items}, ("recall", "ticket"))
        self.assertEqual(20, summary["pairs"])
        self.assertEqual(14, summary["ticket_files_delivered"])
        self.assertEqual("ticket-helps", summary["verdict"])

    def test_the_cost_of_each_arm_is_reported_beside_the_verdict(self) -> None:
        def run(calls, seconds, usage=None, tokens=0):
            return {"stopped": "gold" if usage is None else "finished", "calls_to_gold": calls,
                    "capsule_delivered": True, "hook_ran": True, "seconds": seconds, "usage": usage,
                    "capsule_tokens": tokens}
        finished = {"input_tokens": 40000, "cached_input_tokens": 30000, "output_tokens": 900}
        items = {f"T{n}": {"status": "ok", "arms": {
            "off": [{**run(6, 30.0), "capsule_delivered": False, "hook_ran": False}],
            "capsule": [run(3, 20.0, finished if n == 0 else None, 400)],
        }} for n in range(4)}
        summary = ab.summarize({"meta": {"cap": 40}, "items": items})
        self.assertEqual({"runs": 4, "seconds_mean": 20.0, "runs_with_usage": 1, "capsule_tokens_mean": 400.0,
                          "input_tokens_mean": 40000.0, "cached_input_tokens_mean": 30000.0,
                          "output_tokens_mean": 900.0}, summary["cost"]["capsule"])
        self.assertEqual((0, 30.0), (summary["cost"]["off"]["runs_with_usage"], summary["cost"]["off"]["seconds_mean"]))
        self.assertEqual("insufficient", summary["verdict"])
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / "result.json"
            result.write_text(json.dumps({"meta": {"cap": 40}, "items": items}), encoding="utf-8")
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                ab.main(["report", str(result)])
        self.assertIn("cost, not part of the verdict:", printed.getvalue())
        self.assertIn("capsule: 20.0 s a run; Codex usage on 1 of 4 runs", printed.getvalue())

    def test_the_capsule_arm_names_the_full_profile(self) -> None:
        # The editions' default became recall: left to it, `capsule` would
        # have measured recall under the old name.
        with tempfile.TemporaryDirectory() as directory:
            corpus = Path(directory)
            ab.configure_corpus(corpus, {"capsule_profile": "recall"})
            ab.configure_corpus(corpus, ab.ARM_SETTINGS["capsule"])
            config = json.loads((corpus / "project-brain/config/runtime.json").read_text(encoding="utf-8"))
        self.assertEqual("full", config["capsule_profile"])
        self.assertEqual(set(ab.HOOKED_ARMS), set(ab.ARM_SETTINGS))

    def test_the_health_journal_tells_whether_ticket_files_came(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            health = Path(directory) / "refresh-health.ndjson"
            health.write_text("".join(json.dumps(line) + "\n" for line in (
                {"source": "refresh", "capsule": {"ticket_files": 2}},
                {"source": "hook", "hook_status": 0},
                {"source": "refresh", "capsule": {"ticket_files": 0}},
            )), encoding="utf-8")
            self.assertTrue(ab.ticket_files_delivered(health, 0))
            self.assertFalse(ab.ticket_files_delivered(health, 1))
            self.assertFalse(ab.ticket_files_delivered(Path(directory) / "absent.ndjson", 0))

    def test_navigation_is_compared_with_the_full_capsule(self) -> None:
        def run(calls, delivered, ran):
            return {"stopped": "gold", "calls_to_gold": calls, "capsule_delivered": delivered, "hook_ran": ran}
        items = {f"T{n}": {"status": "ok", "arms": {
            "capsule": [run(5, True, True)], "navigation": [run(3, False, True)], "off": [run(6, False, False)],
        }} for n in range(21)}
        summary = ab.summarize({"meta": {"cap": 40}, "items": items}, ("capsule", "navigation"))
        self.assertEqual(21, summary["pairs"])
        self.assertEqual(-2.0, summary["mean_difference"])
        self.assertEqual("navigation-helps", summary["verdict"])
        self.assertEqual(21, summary["navigation_fewer"])

    def test_a_navigation_run_that_delivered_documents_is_not_navigation(self) -> None:
        run = {"stopped": "gold", "calls_to_gold": 3, "capsule_delivered": True, "hook_ran": True}
        self.assertFalse(ab.valid_arm("navigation", [run]))
        self.assertTrue(ab.valid_arm("capsule", [run]))
        self.assertFalse(ab.valid_arm("off", [{**run, "capsule_delivered": False}]))

    def test_the_recall_metric_counts_an_answer_naming_a_needed_file(self) -> None:
        answer = {"stopped": "finished", "calls_to_gold": None, "calls_to_answer": 2}
        self.assertEqual(41, ab.censored(answer, 40))
        self.assertEqual(2, ab.censored(answer, 40, "recall"))
        self.assertEqual(41, ab.censored({"calls_to_gold": None, "calls_to_answer": None}, 40, "recall"))
        self.assertEqual(5, ab.censored({"calls_to_gold": 5, "calls_to_answer": None}, 40, "recall"))

    def test_recall_arms_are_compared_by_the_results_own_metric(self) -> None:
        def run(gold, answer, delivered, ran, saved=False):
            return {"stopped": "finished", "calls_to_gold": gold, "calls_to_answer": answer,
                    "capsule_delivered": delivered, "hook_ran": ran, "saved_delivered": saved}
        items = {f"T{n}": {"status": "ok", "arms": {
            "navigation": [run(None, None, False, True), run(9, None, False, True)],
            "recall": [run(None, 1, True, True, True), run(None, 3, False, True)],
        }} for n in range(20)}
        result = {"meta": {"cap": 40, "metric": "recall"}, "items": items}
        summary = ab.summarize(result, ("navigation", "recall"))
        self.assertEqual("recall", summary["metric"])
        self.assertEqual(20, summary["pairs"])
        self.assertEqual(25.0, summary["mean_navigation"])
        self.assertEqual(2.0, summary["mean_recall"])
        self.assertEqual("recall-helps", summary["verdict"])
        self.assertEqual((0, 20), (summary["touched_navigation"], summary["touched_recall"]))
        self.assertEqual((0, 20), (summary["saved_delivered_navigation"], summary["saved_delivered_recall"]))
        self.assertEqual(41.0, ab.summarize(result, ("navigation", "recall"), "gold")["mean_recall"])

    def test_a_recall_run_needs_the_hook_but_not_a_manifest(self) -> None:
        ran = {"stopped": "gold", "calls_to_gold": 3, "capsule_delivered": False, "hook_ran": True}
        self.assertTrue(ab.valid_arm("recall", [ran, {**ran, "capsule_delivered": True}]))
        self.assertFalse(ab.valid_arm("recall", [{**ran, "hook_ran": False}]))

    def test_the_report_refuses_an_unknown_comparison(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / "result.json"
            result.write_text(json.dumps({"meta": {"cap": 40}, "items": {}}), encoding="utf-8")
            self.assertEqual(2, ab.main(["report", str(result), "--compare", "off,documents"]))
            self.assertEqual(0, ab.main(["report", str(result), "--compare", "off,navigation"]))

    def test_the_sign_test_is_exact_and_two_sided(self) -> None:
        self.assertEqual(1.0, ab.sign_test([0, 0]))
        self.assertAlmostEqual(0.0625, ab.sign_test([-1, -1, -1, -1, -1]), places=6)
        self.assertEqual(ab.bootstrap([1.0, 2.0, 3.0]), ab.bootstrap([1.0, 2.0, 3.0]))


class DemoProject(unittest.TestCase):
    """A Git project and its synthetic Claude Code transcripts."""

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


class SampleTest(DemoProject):
    """Cold starts from a synthetic Claude Code transcript."""

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


class TicketArmTest(unittest.TestCase):
    """The `ticket` arm, its corpus branch, and the cold starts it can act on."""

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        base = Path(self.directory.name)
        self.project = base / "Desktop" / "demo"
        (self.project / "src").mkdir(parents=True)
        self.env = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
        # Every commit before the sessions, so each has a corpus commit.
        self.commit("2026-08-01T10:00:00Z", "init", "src/Gold.php", "src/Named.php", "src/Other.php")
        self.commit("2026-08-10T10:00:00Z", "CMS-741: start the rounding", "src/Other.php")
        self.commit("2026-08-11T10:00:00Z", "CMS-742: only a named file", "src/Named.php")
        self.transcripts = base / "projects"
        self.folder = ab.transcript_directory(self.transcripts, self.project)
        self.folder.mkdir(parents=True)
        for number, (prompt, branch) in enumerate((
            ("CMS-741: fix the rounding; Named.php is fine as it is.", "main"),
            ("Fix the rounding; Named.php is fine as it is.", "feature/cms-741-rounding"),
            ("Fix the rounding; Named.php is fine as it is.", "feature/cms-900-rounding"),
            ("CMS-742: fix the rounding; Named.php is fine as it is.", "main"),
        )):
            self.session(number, prompt, branch)

    def tearDown(self) -> None:
        self.directory.cleanup()

    def commit(self, when: str, subject: str, *paths: str) -> None:
        for path in paths:
            target = self.project / path
            target.write_text(f"<?php // {subject}\n", encoding="utf-8")
        dated = {**self.env, "GIT_AUTHOR_DATE": when, "GIT_COMMITTER_DATE": when}
        if not (self.project / ".git").exists():
            subprocess.run(["git", "-C", str(self.project), "init", "-q"], check=True, env=dated)
        subprocess.run(["git", "-C", str(self.project), "add", "-A"], check=True, env=dated)
        subprocess.run(["git", "-C", str(self.project), "-c", "user.name=t", "-c", "user.email=t@example.invalid",
                        "commit", "-qm", subject], check=True, env=dated)

    def session(self, number: int, prompt: str, branch: str) -> None:
        root = str(self.project)
        ts = f"2026-09-0{number + 1}T10:00:00Z"
        records = [
            {"type": "user", "timestamp": ts, "cwd": root, "gitBranch": branch, "message": {"content": prompt}},
            {"type": "assistant", "cwd": root, "gitBranch": branch, "message": {"content": [
                {"type": "tool_use", "name": "Edit", "input": {"file_path": f"{root}/src/Gold.php"}}]}},
        ]
        (self.folder / f"session-{number}.jsonl").write_text(
            "".join(json.dumps(record) + "\n" for record in records), encoding="utf-8")

    def test_the_arm_is_recall_with_the_ticket_files_line(self) -> None:
        corpus = Path(self.directory.name) / "corpus"
        (corpus / ".codex").mkdir(parents=True)
        (corpus / ".codex" / "hooks.json").write_text(json.dumps({"hooks": {"UserPromptSubmit": [
            {"hooks": [{"type": "command", "command": "sh working-memory-read.sh"}]}]}}), encoding="utf-8")
        ab.configure_corpus(corpus, {"retrieval_gate": "shadow"})
        ab.configure_corpus(corpus, ab.ARM_SETTINGS["ticket"])
        config = json.loads((corpus / "project-brain/config/runtime.json").read_text(encoding="utf-8"))
        self.assertEqual({"capsule_profile": "recall", "retrieval_gate": "shadow", "ticket_files": True}, config)
        self.assertIn("--dangerously-bypass-hook-trust", ab.codex_command("codex", corpus, "ticket", "do it", None))

    def test_the_corpus_branch_keeps_the_sessions_ticket(self) -> None:
        self.assertEqual("ab/T1", ab.corpus_branch({"id": "T1"}))
        self.assertEqual("ab/T1", ab.corpus_branch({"id": "T1", "branch": "main"}))
        self.assertEqual("ab/T1-CMS-741", ab.corpus_branch({"id": "T1", "branch": "feature/cms-741-rounding"}))

    def test_a_cold_start_records_its_branch_and_what_the_line_could_show(self) -> None:
        tasks = list(ab.cold_starts(self.project, self.transcripts, None, ab.instant("2026-10-07")))
        self.assertEqual(["main", "feature/cms-741-rounding", "feature/cms-900-rounding", "main"],
                         [task["branch"] for task in tasks])
        shown = [ab.ticket_commit_files(self.project, task) for task in tasks]
        # The prompt's ticket, the branch's ticket, a ticket no commit names,
        # and a ticket whose only file the prompt names.
        self.assertEqual([(["CMS-741"], ["src/Other.php"]), (["CMS-741"], ["src/Other.php"]),
                          (["CMS-900"], []), (["CMS-742"], [])], shown)

    def test_files_the_capsule_already_shows_as_recent_leave_the_line_empty(self) -> None:
        root = str(self.project)
        earlier = [
            {"type": "user", "timestamp": "2026-08-31T10:00:00Z", "cwd": root, "message": {"content": "Look around"}},
            {"type": "assistant", "cwd": root, "message": {"content": [
                {"type": "tool_use", "name": "Read", "input": {"file_path": f"{root}/src/Other.php"}}]}},
        ]
        (self.folder / "session-earlier.jsonl").write_text(
            "".join(json.dumps(record) + "\n" for record in earlier), encoding="utf-8")
        turns = ab.project_turns(self.project, self.transcripts, None, ("claude",))
        self.assertEqual(["src/Other.php"], ab.recent_at(turns, ab.instant("2026-09-01"), lambda path: True))
        self.assertEqual([], ab.recent_at(turns, ab.instant("2026-08-31"), lambda path: True))
        task = next(task for task in ab.cold_starts(self.project, self.transcripts, None, ab.instant("2026-10-07"))
                    if task["prompt"].startswith("CMS-741"))
        self.assertEqual((["CMS-741"], ["src/Other.php"]), ab.ticket_commit_files(self.project, task))
        self.assertEqual((["CMS-741"], []), ab.ticket_commit_files(self.project, task, turns))

    def test_sample_keeps_only_the_cold_starts_the_arm_can_act_on(self) -> None:
        out = Path(self.directory.name) / "sample.json"
        code = ab.main(["sample", "--projects", "demo", "--projects-root", str(self.project.parent),
                        "--transcripts", str(self.transcripts), "--before", "2026-10-07", "--n", "10",
                        "--seed", "1", "--require-ticket-commits", "--out", str(out)])
        self.assertEqual(0, code)
        sample = json.loads(out.read_text(encoding="utf-8"))
        self.assertTrue(sample["require_ticket_commits"])
        self.assertEqual([["CMS-741"], ["CMS-741"]], [task["tickets"] for task in sample["tasks"]])


def zstd_bytes(data: bytes):
    """`data` compressed as Codex stores a finished rollout, or None without a coder."""
    coder = getattr(ab.stand, "_zstd", None)
    if coder is not None:
        return coder.compress(data)
    command = shutil.which("zstd")
    if command:
        return subprocess.run([command, "-q", "-c"], input=data, capture_output=True, check=True).stdout
    return None


def codex_message(role: str, words: str) -> dict:
    kind = "output_text" if role == "assistant" else "input_text"
    return {"type": "message", "role": role, "content": [{"type": kind, "text": words}]}


def codex_rollout(root: str, prompt: str, ts: str) -> list:
    """A Codex rollout as 0.150-0.163 wrote one: its instructions, plugin and
    environment messages in the user role, the prompt, a read, a notification
    mid-turn, a patch, the answer."""
    def item(payload: dict, at: str = ts) -> dict:
        return {"timestamp": at, "type": "response_item", "payload": payload}

    patch = "*** Begin Patch\n*** Update File: src/Gold.php\n@@\n-a\n+b\n*** End Patch\n"
    return [
        {"timestamp": ts, "type": "session_meta", "payload": {"id": "s", "cwd": root, "timestamp": ts}},
        item(codex_message("user", f"# AGENTS.md instructions for {root}\n\n<INSTRUCTIONS>rules</INSTRUCTIONS>")),
        item(codex_message("user", "<recommended_plugins>\nHere is a plugin\n</recommended_plugins>")),
        item(codex_message("user", "<environment_context><cwd>" + root + "</cwd></environment_context>")),
        item(codex_message("user", prompt)),
        item({"type": "function_call", "name": "shell",
              "arguments": json.dumps({"command": ["bash", "-lc", "cat src/Named.php"], "workdir": root})}),
        item(codex_message("user", "<task-notification>a background command ended</task-notification>")),
        item({"type": "custom_tool_call", "name": "apply_patch", "input": patch}),
        item(codex_message("assistant", "Done.")),
        item(codex_message("user", "Now the tests"), "2026-09-02T11:05:00Z"),
    ]


class CodexSampleTest(DemoProject):
    """Cold starts from Codex rollouts, the compressed ones Codex keeps included."""

    def setUp(self) -> None:
        super().setUp()
        self.codex = Path(self.directory.name) / "codex"
        folder = self.codex / "2026" / "09" / "02"
        folder.mkdir(parents=True)
        lines = codex_rollout(str(self.project), "Fix the totals; Named.php is fine as it is.",
                              "2026-09-02T11:00:00Z")
        data = "".join(json.dumps(record) + "\n" for record in lines).encode("utf-8")
        packed = zstd_bytes(data)
        self.compressed = packed is not None
        name = "rollout-2026-09-02T11-00-00-0a1b.jsonl"
        if packed is None:
            (folder / name).write_bytes(data)
        else:
            (folder / (name + ".zst")).write_bytes(packed)
        # A rollout of another folder is no session of this project.
        elsewhere = codex_rollout(str(self.project.parent / "elsewhere"), "Fix the totals",
                                  "2026-09-03T11:00:00Z")
        (folder / "rollout-2026-09-03T11-00-00-0c2d.jsonl").write_text(
            "".join(json.dumps(record) + "\n" for record in elsewhere), encoding="utf-8")

    def tasks(self, hosts) -> list:
        return list(ab.cold_starts(self.project, self.transcripts, None, ab.instant("2026-10-07"),
                                   self.codex, hosts))

    def test_a_codex_rollout_is_a_cold_start(self) -> None:
        (task,) = self.tasks(("codex",))
        self.assertEqual(("codex", ["src/Gold.php"], ["codex"]), (task["host"], task["gold"], task["seed_hosts"]))
        self.assertTrue(task["prompt"].startswith("Fix the totals"))
        self.assertEqual("rollout-2026-09-02T11-00-00-0a1b", task["session"])

    def test_the_default_draws_from_claude_code_alone(self) -> None:
        default = list(ab.cold_starts(self.project, self.transcripts, None, ab.instant("2026-10-07")))
        self.assertEqual(["claude"], [task["host"] for task in default])
        self.assertEqual(["claude", "codex"], sorted(task["host"] for task in self.tasks(("claude", "codex"))))

    def test_the_compressed_rollout_is_read(self) -> None:
        if not self.compressed:
            self.skipTest("no zstd coder: neither Python 3.14 nor the zstd command")
        sessions = ab.project_sessions(self.project, self.transcripts, self.codex, ("codex",))
        self.assertEqual([("codex", ".zst")], [(host, path.suffix) for host, path in sessions])

    def test_seeding_reads_the_hosts_the_task_names(self) -> None:
        recorded = []
        with mock.patch.object(ab.runtime(), "record_recent_files",
                               side_effect=lambda connection, files: recorded.append(list(files))), \
                mock.patch.object(ab.runtime(), "load_recent_files", return_value=[]), \
                mock.patch.object(ab.sqlite3, "connect"):
            corpus = Path(self.directory.name) / "corpus"
            shutil.copytree(self.project, corpus, ignore=shutil.ignore_patterns(".git"))
            later = {"ts": "2026-09-05T00:00:00Z"}
            ab.seed_recent_files(later, corpus, self.project, self.transcripts, self.codex)
            claude_only = [files for files in recorded]
            recorded.clear()
            ab.seed_recent_files(dict(later, seed_hosts=["claude", "codex"]), corpus, self.project,
                                 self.transcripts, self.codex)
        # The Claude Code session read and edited both files; the Codex one adds its own turn.
        self.assertEqual(1, len(claude_only))
        self.assertEqual(2, len(recorded))

    def test_an_unknown_host_is_refused(self) -> None:
        with self.assertRaises(ab.OutcomeError):
            ab.parse_hosts("claude,cursor")

    def test_only_what_a_person_wrote_starts_a_codex_turn(self) -> None:
        def said(words: str) -> dict:
            return {"type": "response_item", "payload": codex_message("user", words)}

        for words in ("# AGENTS.md instructions for /w\n\nrules", "<recommended_plugins>x</recommended_plugins>",
                      "<hook_prompt>save</hook_prompt>", "<turn_aborted>\nstopped\n</turn_aborted>",
                      "<task-notification>\n<task-id>b1</task-id>\n</task-notification>",
                      "<heartbeat>\n<automation_id>nightly</automation_id>\n</heartbeat>",
                      "<system-reminder>a side session ended</system-reminder>\nnote"):
            with self.subTest(words=words[:20]):
                self.assertFalse(ab.human_prompt(said(words)))
        for words in ("Fix the totals", '<pasted_content id="1">log</pasted_content> why?', "# Plan\nDo it",
                      '<pasted_content id="2">only the log</pasted_content>',
                      "# In app browser:\n- url: http://localhost\n\n## My request for Codex:\nFix the totals",
                      "# Files mentioned by the user:\n\n## a.png: /w/a.png\n\n## My request:\nFix the totals",
                      '<in-app-browser-context source="page">url</in-app-browser-context>\n\nFix the totals'):
            with self.subTest(words=words[:20]):
                self.assertTrue(ab.human_prompt(said(words)))


class RecallSourcesTest(DemoProject):
    """First turns that answered from files they read, from synthetic transcripts."""

    def setUp(self) -> None:
        super().setUp()
        for number in range(12):
            (self.project / "src" / f"Other{number}.php").write_text("<?php\n", encoding="utf-8")
        subprocess.run(["git", "-C", str(self.project), "add", "-A"], check=True,
                       env={**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"})
        # The corpus for a question is the last commit before it.
        dated = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
                 "GIT_AUTHOR_DATE": "2026-09-01T00:00:00Z", "GIT_COMMITTER_DATE": "2026-09-01T00:00:00Z"}
        subprocess.run(["git", "-C", str(self.project), "-c", "user.name=t", "-c", "user.email=t@example.invalid",
                        "commit", "-q", "-m", "dated"], check=True, env=dated)
        root = str(self.project)
        folder = ab.transcript_directory(self.transcripts, self.project)
        answer = ("The rounding happens in Gold.php: amounts are rounded half-up to two places before "
                  "the invoice total is summed, so line totals and the grand total can differ by a cent. "
                  "Named.php only formats the result for display. ") * 2
        reader = [
            {"type": "user", "timestamp": "2026-09-02T09:00:00Z", "cwd": root,
             "message": {"content": "Where does the invoice rounding happen?"}},
            {"type": "assistant", "timestamp": "2026-09-02T09:01:00Z", "cwd": root, "message": {"content": [
                {"type": "tool_use", "name": "Read", "input": {"file_path": f"{root}/src/Gold.php"}},
                {"type": "tool_use", "name": "Read", "input": {"file_path": f"{root}/src/Named.php"}},
            ]}},
            {"type": "assistant", "timestamp": "2026-09-02T09:02:00Z", "cwd": root,
             "message": {"content": [{"type": "text", "text": answer}]}},
            {"type": "user", "timestamp": "2026-09-02T09:10:00Z", "cwd": root, "message": {"content": "Thanks"}},
        ]
        (folder / "session-2.jsonl").write_text("\n".join(json.dumps(record) for record in reader) + "\n",
                                                encoding="utf-8")
        short = [dict(reader[0], timestamp="2026-09-03T09:00:00Z"), reader[1],
                 {**reader[2], "message": {"content": [{"type": "text", "text": "Gold.php."}]}}]
        (folder / "session-3.jsonl").write_text("\n".join(json.dumps(record) for record in short) + "\n",
                                                encoding="utf-8")
        # Later work on twelve other files pushes the two out of the ten the
        # capsule shows; the session after it is the later question's moment.
        busy = [
            {"type": "user", "timestamp": "2026-09-05T09:00:00Z", "cwd": root,
             "message": {"content": "Tidy the other services"}},
            {"type": "assistant", "timestamp": "2026-09-05T09:01:00Z", "cwd": root, "message": {"content": [
                {"type": "tool_use", "name": "Edit", "input": {"file_path": f"{root}/src/Other{number}.php"}}
                for number in range(12)]}},
        ]
        (folder / "session-4.jsonl").write_text("\n".join(json.dumps(record) for record in busy) + "\n",
                                                encoding="utf-8")
        later = {"type": "user", "timestamp": "2026-09-06T09:00:00Z", "cwd": root,
                 "message": {"content": "Next thing"}}
        (folder / "session-5.jsonl").write_text(json.dumps(later) + "\n", encoding="utf-8")

    def sources(self) -> list:
        return list(ab.recall_sources(self.project, self.transcripts, None, ab.instant("2026-10-07")))

    def test_a_turn_that_read_and_answered_is_a_source_and_one_that_changed_is_not(self) -> None:
        sources = self.sources()
        # session-1 changed files, session-3 answered in two words.
        self.assertEqual(["session-2"], [source["session"] for source in sources])
        source = sources[0]
        self.assertEqual(["src/Gold.php", "src/Named.php"], source["read"])
        self.assertEqual(["src/Gold.php", "src/Named.php"], source["named"])
        self.assertEqual("2026-09-02T09:03:00Z", source["ts"])
        self.assertEqual(ab.task_id("demo", "2026-09-02T09:03:00Z"), source["id"])
        # session-3 still shows the two files; after session-4 they are gone.
        self.assertEqual("2026-09-06T09:00:00Z", source["later"])

    def scenario(self, **changes) -> dict:
        return {"id": self.sources()[0]["id"], "learning": {
            "type": "finding", "title": "Invoice rounding happens per line",
            "consequence": "Amounts are rounded half-up to two places per line before the total is summed.",
            "sources": ["src/Gold.php"]},
            "needed": ["src/Gold.php"], "prompt": "Why can the invoice total differ by a cent?", **changes}

    def test_an_authored_scenario_must_fit_its_source(self) -> None:
        source = self.sources()[0]
        self.assertIsNone(ab.recall_problem(source, self.scenario()))
        for changes, reason in (
            ({"prompt": "Look at Gold.php again"}, "names a needed file"),
            ({"needed": ["src/Named.php"]}, "among the learning's sources"),
            ({"learning": {**self.scenario()["learning"], "sources": ["src/Other.php"]}}, "the source turn read"),
            ({"learning": {**self.scenario()["learning"], "type": "event"}}, "finding or decision"),
            ({"learning": {**self.scenario()["learning"], "consequence": "user: paste\nassistant: log"}},
             "record-result refuses"),
        ):
            with self.subTest(reason=reason):
                self.assertIn(reason, ab.recall_problem(source, self.scenario(**changes)) or "")

    def test_recall_set_keeps_valid_scenarios_with_their_save(self) -> None:
        base = Path(self.directory.name)
        sources_path, authored, out = base / "sources.json", base / "authored.json", base / "set.json"
        sources = self.sources()
        # A file the turn read but the corpus does not hold: record-result
        # would refuse a learning that cites it.
        ghost = {**sources[0], "id": "T-ghost", "read": [*sources[0]["read"], "src/Ghost.php"]}
        ab.stand.write_json(sources_path, {"sources": [*sources, ghost]})
        cites_ghost = self.scenario(id="T-ghost", learning={**self.scenario()["learning"],
                                                             "sources": ["src/Gold.php", "src/Ghost.php"]})
        ab.stand.write_json(authored, [self.scenario(), {"id": "T-unknown", "skip": "no"}, cites_ghost])
        root = ["--projects-root", str(Path(self.directory.name) / "Desktop")]
        self.assertEqual(0, ab.main(["recall-set", "--sources", str(sources_path), "--scenarios", str(authored),
                                     *root, "--n", "5", "--seed", "1", "--out", str(out)]))
        document = json.loads(out.read_text(encoding="utf-8"))
        self.assertEqual("recall", document["kind"])
        self.assertEqual({"T-unknown": "no such source",
                          "T-ghost": "learning sources missing from the corpus: src/Ghost.php"}, document["refused"])
        task = document["tasks"][0]
        self.assertEqual(["src/Gold.php"], task["gold"])
        self.assertEqual([{"learnings": [self.scenario()["learning"]], "verified": True}], task["saves"])
        later = base / "later.json"
        self.assertEqual(0, ab.main(["recall-set", "--sources", str(sources_path), "--scenarios", str(authored),
                                     *root, "--regime", "later", "--n", "5", "--seed", "1", "--out", str(later)]))
        moved = json.loads(later.read_text(encoding="utf-8"))["tasks"][0]
        self.assertEqual(("2026-09-06T09:00:00Z", "later", task["id"]), (moved["ts"], moved["regime"], moved["source"]))
        self.assertEqual(ab.task_id("demo", f"2026-09-06T09:00:00Z#{task['id']}"), moved["id"])
        self.assertEqual(task["saves"], moved["saves"])
        excluded = base / "excluded.json"
        self.assertEqual(0, ab.main(["recall-set", "--sources", str(sources_path), "--scenarios", str(authored),
                                     *root, "--exclude-id", task["id"], "--n", "5", "--seed", "1", "--out", str(excluded)]))
        self.assertEqual([], json.loads(excluded.read_text(encoding="utf-8"))["tasks"])


class LaterMomentTest(unittest.TestCase):
    """The later question waits until the capsule's recent files hide the subject."""

    class Repository:
        def commit_before(self, moment):
            return "c0ffee"

        def tree(self, commit):
            return "7ree"

    def later(self, present: list) -> object:
        when = ab.instant
        history = [(when("2026-09-01T10:00:00Z"), ["src/Gold.php"]),
                   (when("2026-09-02T10:00:00Z"), [f"src/X{number}.php" for number in range(10)])]
        starts = [when("2026-09-01T09:59:00Z"), when("2026-09-03T09:00:00Z")]
        with mock.patch.object(ab.stand, "read_git", return_value="\n".join(present)):
            return ab.later_moment(Path("/demo"), self.Repository(), history, starts,
                                   when("2026-09-01T10:01:00Z"), ["src/Gold.php"], when("2026-10-07"))

    def test_ten_newer_files_hide_the_subject(self) -> None:
        self.assertEqual("2026-09-03T09:00:00Z",
                         self.later(["src/Gold.php", *(f"src/X{number}.php" for number in range(10))]))

    def test_files_gone_from_the_corpus_hide_nothing(self) -> None:
        # The capsule skips recent files the corpus no longer has, so the
        # subject is still the one it shows.
        self.assertIsNone(self.later(["src/Gold.php"]))

    def test_a_subject_gone_from_the_corpus_has_no_later_question(self) -> None:
        self.assertIsNone(self.later([f"src/X{number}.php" for number in range(10)]))


class PlantSavesTest(unittest.TestCase):
    """A save recorded through the runtime's own record-result, then committed and indexed."""

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        base = Path(self.directory.name)
        self.corpus = base / "corpus"
        shutil.copytree(ROOT / "PHP Core" / "memory-bank" / "scripts", self.corpus / "memory-bank" / "scripts",
                        ignore=shutil.ignore_patterns("__pycache__"))
        (self.corpus / "memory-bank" / "chunks").mkdir()
        (self.corpus / "memory-bank" / "README.md").write_text("# Memory Bank\n", encoding="utf-8")
        (self.corpus / "memory-bank" / "INDEX.md").write_text(
            "# Memory Index\n\n| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n"
            "| --- | --- | --- | --- | --- | --- | --- | --- |\n", encoding="utf-8")
        (self.corpus / ".gitignore").write_text("memory-bank/local/\n", encoding="utf-8")
        (self.corpus / "project-brain" / "config").mkdir(parents=True)
        (self.corpus / "project-brain" / "config" / "runtime.json").write_text(
            json.dumps({"mode": "governed", "automatic_promotion": True}), encoding="utf-8")
        (self.corpus / "src").mkdir()
        (self.corpus / "src" / "Gold.php").write_text("<?php\n// invoice rounding\n", encoding="utf-8")
        self.cache = base / "cache"
        (self.cache / "home").mkdir(parents=True)
        self.task = {"id": "T0123456789", "ts": "2026-09-02T09:03:00Z", "saves": [{"learnings": [{
            "type": "finding", "title": "Invoice rounding happens per line",
            "consequence": "Amounts are rounded half-up per line before the total is summed.",
            "sources": ["src/Gold.php"]}], "verified": True}]}
        self.env = ab.stand.runtime_env(self.cache, ab.stand.parse_instant(self.task["ts"]), None)
        ab.stand.commit_corpus(self.corpus, self.env, None, None, ab.corpus_branch(self.task))

    def tearDown(self) -> None:
        self.directory.cleanup()

    def test_the_save_lands_on_the_branch_task_promoted_and_committed(self) -> None:
        planted = ab.plant_saves(self.task, self.corpus, self.env, 60, Path(self.directory.name) / "saves")
        self.assertEqual((1, 1), (planted["records"], planted["promoted"]))
        chunks = [path for path in planted["documents"] if path.startswith("memory-bank/chunks/")]
        self.assertEqual(1, len(chunks), planted["documents"])
        status = subprocess.run(["git", "status", "--porcelain"], cwd=str(self.corpus), env=self.env,
                                capture_output=True, text=True)
        self.assertEqual("", status.stdout)
        refreshed = ab.stand.run_runtime(self.corpus, ["refresh", "--query", "invoice rounding per line",
                                                       "--task-id", ab.corpus_branch(self.task), "--ephemeral",
                                                       "--capsule-profile", "recall"], self.env, 60)
        self.assertEqual(0, refreshed["exit"], refreshed["stderr"])
        self.assertIn("Invoice rounding happens per line", refreshed["stdout"].decode("utf-8"))

    def test_a_set_without_saves_plants_nothing(self) -> None:
        self.assertEqual({}, ab.plant_saves({"id": "T1", "ts": self.task["ts"]}, self.corpus, self.env, 60,
                                            Path(self.directory.name) / "saves"))


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
                with open(manifests.parent / "refresh-health.ndjson", "a") as health:
                    health.write(json.dumps({{"source": "refresh"}}) + "\\n")
                if "--navigation" not in sys.argv:
                    (manifests / "hook.json").write_text(json.dumps(
                        {{"entry_point": "refresh", "token_estimates": {{"total": 812}},
                         "selected": [{{"path": "memory-bank/chunks/MEM-1-saved.md"}}]}}))
            (manifests / "agent.json").write_text(json.dumps({{"entry_point": "retrieve"}}))
            def emit(event):
                print(json.dumps(event), flush=True)
            emit({{"type": "thread.started"}})
            if "--answer" in sys.argv:
                emit({{"type": "item.started", "item": {{"id": "1", "type": "command_execution", "command": "/usr/bin/bash -lc 'ls'"}}}})
                emit({{"type": "item.completed", "item": {{"id": "m", "type": "agent_message", "text": "It is in src/Gold.php."}}}})
                emit({{"type": "turn.completed", "usage": {{}}}})
                sys.exit(0)
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
        self.assertTrue(outcome["hook_ran"])
        self.assertEqual(812, outcome["capsule_tokens"])
        self.assertEqual({"refresh": 1, "retrieve": 1}, outcome["manifests"])

    def test_a_navigation_run_has_a_hook_line_and_no_delivery(self) -> None:
        command = [sys.executable, str(self.fake), "--dangerously-bypass-hook-trust", "--navigation", "prompt"]
        outcome = ab.run_arm(command, self.corpus, ["src/Gold.php"], cap=40, timeout=20)
        self.assertTrue(outcome["hook_ran"])
        self.assertFalse(outcome["capsule_delivered"])
        self.assertEqual(0, outcome["capsule_tokens"])

    def test_an_agent_retrieve_is_not_a_delivered_capsule(self) -> None:
        command = [sys.executable, str(self.fake), "prompt"]
        outcome = ab.run_arm(command, self.corpus, ["src/Gold.php"], cap=40, timeout=20)
        self.assertFalse(outcome["capsule_delivered"])
        self.assertFalse(outcome["hook_ran"])

    def test_an_answer_naming_the_needed_file_is_counted_and_the_save_seen(self) -> None:
        command = [sys.executable, str(self.fake), "--dangerously-bypass-hook-trust", "--answer", "prompt"]
        outcome = ab.run_arm(command, self.corpus, ["src/Gold.php"], cap=40, timeout=20,
                             saved=["memory-bank/chunks/MEM-1-saved.md"])
        self.assertEqual("finished", outcome["stopped"])
        self.assertIsNone(outcome["calls_to_gold"])
        self.assertEqual(1, outcome["calls_to_answer"])
        self.assertTrue(outcome["saved_delivered"])
        unseen = ab.run_arm(command, self.corpus, ["src/Gold.php"], cap=40, timeout=20,
                            saved=["memory-bank/chunks/MEM-2-other.md"])
        self.assertFalse(unseen["saved_delivered"])

    def test_the_deadline_stops_a_quiet_session(self) -> None:
        command = [sys.executable, str(self.fake), "prompt"]
        outcome = ab.run_arm(command, self.corpus, ["src/Missing.php"], cap=40, timeout=2)
        self.assertEqual("timeout", outcome["stopped"])
        self.assertIsNone(outcome["calls_to_gold"])


class RunRefusesTest(unittest.TestCase):
    def test_a_set_with_one_id_twice_is_refused_before_any_run(self) -> None:
        # Two sources asked at the same later session once shared an ID, and
        # the second result overwrote the first.
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            task = {"id": "T1", "project": "demo", "ts": "2026-09-06T09:00:00Z", "prompt": "q", "gold": ["a"]}
            (base / "set.json").write_text(json.dumps({"kind": "recall", "tasks": [task, dict(task)]}),
                                           encoding="utf-8")
            fake = base / "codex"
            fake.write_text("#!/bin/sh\necho codex-cli 0\n", encoding="utf-8")
            fake.chmod(0o755)
            code = ab.main(["run", "--set", str(base / "set.json"), "--out", str(base / "result.json"),
                            "--codex", str(fake), "--cache", str(base / "cache")])
            self.assertEqual(2, code)
            self.assertFalse((base / "result.json").exists())


class OutputsStayOutsideTheCloneTest(unittest.TestCase):
    def test_a_set_inside_the_clone_is_refused(self) -> None:
        code = ab.main(["sample", "--projects", "x", "--before", "2026-10-07", "--n", "1", "--seed", "1",
                        "--out", str(ROOT / "set.json")])
        self.assertEqual(2, code)
        self.assertFalse((ROOT / "set.json").exists())


if __name__ == "__main__":
    unittest.main()
