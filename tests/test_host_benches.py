"""The offline host benches, without the hosts.

scripts/claude_code_bench.py and scripts/codex_bench.py drive the installed
Claude Code and Codex, and CI has neither. This holds what they rely on: each
finds the save command in the step the runtime writes for every host, their
stand-ins take a turn the way each host sends it (Claude Code's hook feedback,
Codex's XML-escaped <hook_prompt> and code mode), and a target the rollout
rehearsal refuses is refused.
"""
import contextlib
import html
import io
import importlib
import json
import subprocess
import sys
import tempfile
import threading
import types
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import bench_checkpoint  # noqa: E402
import claude_code_bench  # noqa: E402
import codex_bench  # noqa: E402

TOKEN = "0123456789abcdef0123456789abcdef"


def instruction(host: str, root: Path) -> str:
    """The save step the editions' runtime writes for a host."""
    scripts = ROOT / "PHP Core/memory-bank/scripts"
    names = {path.stem for path in scripts.glob("*.py")}
    previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}
    sys.path.insert(0, str(scripts))
    try:
        native = importlib.import_module("native_results")
        return native._instruction(root, "k" * 64, {"token": TOKEN}, brief=False, host=host)
    finally:
        sys.path.remove(str(scripts))
        for name in names:
            sys.modules.pop(name, None)
        sys.modules.update(previous)


class SaveCommandTest(unittest.TestCase):
    def test_each_bench_finds_the_save_command_for_every_host(self) -> None:
        root = Path("/home/user/shop project")
        for host in ("claude", "codex", "cursor"):
            with self.subTest(host=host):
                found = claude_code_bench.SAVE.search(instruction(host, root))
                self.assertIsNotNone(found)
                command = found.group(1)
                self.assertTrue(command.endswith("--json") or command.endswith("--json)"))
                self.assertIn("native-result --request", command)
                if host == "claude":
                    self.assertIn("'<JSON>'", command)
                else:
                    self.assertTrue(command.startswith("(cd "))
        self.assertIs(codex_bench.SAVE, claude_code_bench.SAVE)


class ClaudeStandInTest(unittest.TestCase):
    """Claude Code's Messages API turn: read, answer, save as told, done."""

    def setUp(self) -> None:
        self.stand_in = SimpleNamespace(project="/work/project", source="src/Invoice.php")
        self.tools = [{"name": "Bash"}, {"name": "Read"}]

    def decide(self, messages):
        return claude_code_bench.Scripted.decide(self.stand_in, {"tools": self.tools, "messages": messages})

    def test_a_turn_reads_answers_saves_and_ends(self) -> None:
        prompt = [{"role": "user", "content": "How are invoices numbered?"}]
        [read], stop, _ = self.decide(prompt)
        self.assertEqual(("tool_use", "Read", "tool_use"), (read["type"], read["name"], stop))
        self.assertEqual("/work/project/src/Invoice.php", read["input"]["file_path"])
        turn = prompt + [{"role": "assistant", "content": [read]},
                         {"role": "user", "content": [{"type": "tool_result", "tool_use_id": read["id"],
                                                       "content": "<?php"}]},
                         {"role": "user", "content": "<total_tokens>1 tokens left</total_tokens>"}]
        [answer], stop, _ = self.decide(turn)
        self.assertEqual(("text", "end_turn"), (answer["type"], stop))
        step = "<system-reminder>\nStop hook additional context: " + instruction("claude", Path("/work/project"))
        turn += [{"role": "assistant", "content": [answer]}, {"role": "user", "content": step}]
        [save], stop, _ = self.decide(turn)
        self.assertEqual(("tool_use", "Bash"), (save["type"], save["name"]))
        command = save["input"]["command"]
        self.assertNotIn("<JSON>", command)
        self.assertIn("echo -E '", command)
        self.assertIn("счёт", command)
        turn += [{"role": "assistant", "content": [save]},
                 {"role": "user", "content": [{"type": "tool_result", "tool_use_id": save["id"], "content": "{}"}]}]
        [done], stop, _ = self.decide(turn)
        self.assertEqual(("Saved.", "end_turn"), (done["text"], stop))

    def test_a_request_without_tools_gets_text(self) -> None:
        [reply], stop, _ = claude_code_bench.Scripted.decide(self.stand_in, {"messages": []})
        self.assertEqual(("text", "end_turn"), (reply["type"], stop))

    def test_with_queued_the_stand_in_takes_its_time_over_the_first_save_only(self) -> None:
        stand_in = SimpleNamespace(save_delay=7.0, save_begun=threading.Event())
        with mock.patch.object(claude_code_bench.time, "sleep") as sleep:
            for name in ("toolu_read", "toolu_save", "toolu_save"):
                claude_code_bench.Scripted.pause(stand_in, [{"type": "tool_use", "id": name}])
        sleep.assert_called_once_with(7.0)
        self.assertTrue(stand_in.save_begun.is_set())

    def test_a_queued_run_passes_only_when_the_save_in_flight_lands(self) -> None:
        # The second question came while the save was written: the save must
        # still land, and nothing may say it was not saved.
        landed = ["prompt:armed", "stop:requested", "prompt:armed:previous-in-flight", "save:saved:in-flight",
                  "stop:skipped:recovery-saved"]
        refused = ["prompt:armed", "stop:requested", "prompt:armed:previous-requested",
                   "save:refused:native-result:request-unavailable", "stop:skipped:recovery-armed"]
        warning = "Previous native memory result was not saved completely."
        self.assertTrue(claude_code_bench.queued_ok(landed, "Saved."))
        # A runtime that kept the request where it was journals a plain save.
        self.assertTrue(claude_code_bench.queued_ok([event.replace(":in-flight", "") for event in landed], "Saved."))
        self.assertFalse(claude_code_bench.queued_ok(refused, warning))
        self.assertFalse(claude_code_bench.queued_ok(landed, warning))


class PolicyHeardTest(unittest.TestCase):
    """Every run checks the policy reached the model: a project's own CLAUDE.md
    kept AGENTS.md from every client's Claude Code until it was imported."""

    POLICY = "# AGENTS.md - Policy Rules\n\n- MUST run retrieve.\n- MUST NOT save twice.\n"

    def test_every_line_in_the_conversation_or_the_system_prompt_counts(self) -> None:
        loaded = [{"since": ["Contents of /p/AGENTS.md (project instructions):\n" + self.POLICY], "system": ""}]
        self.assertTrue(claude_code_bench.policy_heard(self.POLICY, loaded))
        lent = [{"since": ["How are invoices numbered?"], "system": "Claude Code.\n" + self.POLICY}]
        self.assertTrue(claude_code_bench.policy_heard(self.POLICY, lent))

    def test_a_line_missing_or_kept_in_an_overflow_file_fails(self) -> None:
        own_claude_md_only = [{"since": ["Contents of /p/CLAUDE.md:\n# Shop\n"], "system": ""}]
        self.assertFalse(claude_code_bench.policy_heard(self.POLICY, own_claude_md_only))
        cut = [{"since": [self.POLICY.replace("- MUST NOT save twice.\n", "")], "system": ""}]
        self.assertFalse(claude_code_bench.policy_heard(self.POLICY, cut))
        overflow = [{"since": [self.POLICY + "\nOutput too large (40KB). Full output saved to /tmp/x.txt"],
                     "system": ""}]
        self.assertFalse(claude_code_bench.policy_heard(self.POLICY, overflow))

    def test_html_comments_claude_code_drops_are_not_asked_for(self) -> None:
        managed = ("## Team notes\n\n<!-- BEGIN ACCELERATOR MANAGED POLICY -->\n" + self.POLICY
                   + "<!-- END ACCELERATOR MANAGED POLICY -->\n")
        loaded = [{"since": ["Contents of /p/AGENTS.md:\n## Team notes\n\n" + self.POLICY], "system": ""}]
        self.assertTrue(claude_code_bench.policy_heard(managed, loaded))
        self.assertFalse(claude_code_bench.policy_heard(managed + "- MUST cite sources.\n", loaded))

    def test_the_loaded_files_come_from_the_first_question_with_each_load_counted(self) -> None:
        requests = [{"since": [], "system": "title"},
                    {"since": ["Contents of /p/CLAUDE.md:\n#\nContents of /p/AGENTS.md (project instructions):\nx",
                               "Contents of /p/AGENTS.md (the project's policy), part 1 of 2:\nx"], "system": ""},
                    {"since": ["Contents of /p/sub/AGENTS.md:\ny"], "system": ""}]
        self.assertEqual(claude_code_bench.policy_files(requests),
                         ["/p/AGENTS.md (project instructions)", "/p/AGENTS.md (the project's policy), part 1 of 2"])
        self.assertEqual(claude_code_bench.policy_files([]), [])

    def test_the_bench_project_keeps_a_claude_md_of_its_own(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            project = claude_code_bench.shop(Path(folder))
            self.assertTrue((project / "CLAUDE.md").is_file())
            self.assertFalse((project / "AGENTS.md").exists())


class CodexHelpersTest(unittest.TestCase):
    """Codex's helpers outlive the bench's run unless it stops them."""

    @unittest.skipUnless(Path("/proc/self/environ").exists(), "Linux /proc")
    def test_only_the_runs_own_helpers_are_stopped(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            ours, theirs = Path(folder) / "codex-home", Path(folder) / "other-home"
            sleep = [sys.executable, "-c", "import time; time.sleep(60)"]
            mine = subprocess.Popen(sleep, env={"CODEX_HOME": str(ours)})
            other = subprocess.Popen(sleep, env={"CODEX_HOME": str(theirs)})
            try:
                self.assertEqual(1, codex_bench.stop_helpers(ours))
                self.assertEqual(-9, mine.wait(timeout=10))
                self.assertIsNone(other.poll())
            finally:
                for process in (mine, other):
                    if process.poll() is None:
                        process.kill()
                    process.wait()


class CodexStandInTest(unittest.TestCase):
    """Codex's Responses API turn, with a tool list and in code mode."""

    def decide(self, tools, items):
        stand_in = SimpleNamespace(requests=[], source="composer.json")
        output, _ = codex_bench.Scripted.decide(stand_in, {"tools": tools, "input": items})
        return output

    def test_the_hook_prompt_is_read_unescaped(self) -> None:
        tools = [{"type": "function", "name": "exec_command"}]
        prompt = [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Explain"}]}]
        [read] = self.decide(tools, prompt)
        self.assertEqual(("function_call", "exec_command"), (read["type"], read["name"]))
        self.assertEqual("cat composer.json", json.loads(read["arguments"])["cmd"])
        turn = prompt + [read, {"type": "function_call_output", "call_id": read["call_id"], "output": "{}"}]
        [answer] = self.decide(tools, turn)
        self.assertEqual("message", answer["type"])
        step = ('<hook_prompt hook_run_id="stop:8">' + html.escape(instruction("codex", Path("/work/project")),
                                                                  quote=False) + "</hook_prompt>")
        self.assertIn("&amp;&amp;", step)
        turn += [answer, {"type": "message", "role": "user", "content": [{"type": "input_text", "text": step}]}]
        [save] = self.decide(tools, turn)
        command = json.loads(save["arguments"])["cmd"]
        self.assertIn(" && python3 memory-bank/scripts/context.py", command)
        self.assertNotIn("&amp;", command)
        turn += [save, {"type": "function_call_output", "call_id": save["call_id"], "output": "{}"}]
        [done] = self.decide(tools, turn)
        self.assertEqual("Saved.", done["content"][0]["text"])

    def test_code_mode_runs_commands_through_its_exec_script(self) -> None:
        prompt = [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Explain"}]}]
        [read] = self.decide([], prompt)
        self.assertEqual(("custom_tool_call", "exec"), (read["type"], read["name"]))
        self.assertEqual('text(await tools.exec_command({cmd:"cat composer.json"}));', read["input"])
        turn = prompt + [read, {"type": "custom_tool_call_output", "call_id": read["call_id"],
                                "output": [{"type": "input_text", "text": "Script completed"}]}]
        [answer] = self.decide([], turn)
        self.assertEqual("message", answer["type"])


class CheckpointStandInTest(unittest.TestCase):
    """--cli checkpoint: the checkpoint skill as a literal model follows it."""

    SKILL = (ROOT / "PHP Core/.agents/skills/checkpoint/SKILL.md").read_text(encoding="utf-8")
    # Step 1 as it stood before a detached HEAD's task came from status.
    STOPPING = ("## Governed Workflow\n\n1. The task ID is `CONTEXT_TASK_ID`, otherwise\n"
                "   `git symbolic-ref --quiet --short HEAD`. Stop with an actionable\n"
                "   detached-HEAD error rather than inventing an ID.\n2. Run ...")
    HINT = ("Memory CLI: this worktree has none - run the commands the policy and skills name with "
            "`python3 /work/shop/memory-bank/scripts/context.py` in place of `python3 memory-bank/scripts/context.py`.")
    STATUS = json.dumps({"automatic_memory": {"task_identity": {"task_id": "codex-thread/01a1776d-b888-7ac2-8a1b"
                                                                           "-9557bdbd1dad"}}})

    def steps(self, skill, heard, answers):
        """The commands the checkpoint runs, given each one's (output, failed)."""
        checkpoint, done, commands = bench_checkpoint.Checkpoint(skill), [], []
        while True:
            planned = checkpoint.next(heard, done)
            if planned is None:
                return checkpoint, commands
            commands.append(planned[1])
            done.append((planned[0], *answers[planned[0]]))

    def test_the_installed_skill_takes_a_detached_heads_task_from_status(self) -> None:
        self.assertTrue(bench_checkpoint.takes_status(self.SKILL))
        self.assertFalse(bench_checkpoint.takes_status(self.STOPPING))

    def test_on_a_branch_it_flushes_and_updates_with_the_cli_it_was_told_of(self) -> None:
        answers = {"branch": ("feature/SHOP-9-refunds\n", False), "flush": ("{}", False), "update": ("{}", False)}
        checkpoint, commands = self.steps(self.SKILL, self.HINT, answers)
        self.assertEqual(3, len(commands))
        self.assertTrue(commands[1].startswith("python3 /work/shop/memory-bank/scripts/context.py turn --task-id "
                                               "feature/SHOP-9-refunds --flush"))
        self.assertIn(" update --task-id feature/SHOP-9-refunds --revision auto", commands[2])
        self.assertEqual(("feature/SHOP-9-refunds", ""), (checkpoint.task, checkpoint.stopped))
        # Told of nothing, it runs the policy's own.
        _, commands = self.steps(self.SKILL, "", answers)
        self.assertTrue(commands[1].startswith("python3 memory-bank/scripts/context.py turn"))

    def test_on_a_detached_head_it_asks_status_where_the_skill_says_so_and_stops_otherwise(self) -> None:
        answers = {"branch": ("", True), "status": (self.STATUS, False), "flush": ("{}", False), "update": ("{}", False)}
        checkpoint, commands = self.steps(self.SKILL, self.HINT, answers)
        self.assertEqual("python3 /work/shop/memory-bank/scripts/context.py status --json", commands[1])
        self.assertEqual("codex-thread/01a1776d-b888-7ac2-8a1b-9557bdbd1dad", checkpoint.task)
        self.assertEqual(4, len(commands))
        checkpoint, commands = self.steps(self.STOPPING, self.HINT, answers)
        self.assertEqual((1, "the skill stops on a detached HEAD"), (len(commands), checkpoint.stopped))

    def test_a_queued_answer_is_recorded_as_queued(self) -> None:
        checkpoint = bench_checkpoint.Checkpoint(self.SKILL)
        checkpoint.record("update", json.dumps({"task_id": "feature/x", "queued": True}), False)
        checkpoint.record("flush", "context: Working task not found: feature/x", True)
        self.assertEqual([("update", True, True), ("flush", False, False)],
                         [(step["step"], step["ok"], step["queued"]) for step in checkpoint.steps])

    def test_a_codex_command_result_is_read_from_either_tool(self) -> None:
        exec_command = ("Chunk ID: 1a2b\nWall time: 0.0123 seconds\nProcess exited with code 1\n"
                        "Original token count: 3\nOutput:\nfatal: not a symbolic ref\n")
        self.assertEqual(("fatal: not a symbolic ref\n", True), codex_bench.command_result(exec_command))
        self.assertEqual(("main\n", False), codex_bench.command_result(exec_command.replace("code 1", "code 0")
                                                                        .replace("fatal: not a symbolic ref", "main")))
        shell = json.dumps({"output": "{}", "metadata": {"exit_code": 0, "duration_seconds": 0.1}})
        self.assertEqual(("{}", False), codex_bench.command_result(shell))

    def test_each_stand_in_checkpoints_before_it_reads(self) -> None:
        answers = {"branch": "feature/SHOP-9-refunds\n", "flush": "{}", "update": "{}"}
        # Claude Code: Bash calls, their results, then the read.
        stand_in = SimpleNamespace(project="/work/project", source="src/Invoice.php",
                                   checkpoint=bench_checkpoint.Checkpoint(self.SKILL))
        stand_in.checkpoint_step = types.MethodType(claude_code_bench.Scripted.checkpoint_step, stand_in)
        messages = [{"role": "user", "content": self.HINT + "\nHow are invoices numbered?"}]
        for name in ("branch", "flush", "update"):
            [call], stop, _ = claude_code_bench.Scripted.decide(stand_in, {"tools": [{"name": "Bash"}],
                                                                           "messages": messages})
            self.assertEqual(("Bash", "tool_use"), (call["name"], stop))
            self.assertTrue(call["id"].endswith("_" + name))
            messages += [{"role": "assistant", "content": [call]},
                         {"role": "user", "content": [{"type": "tool_result", "tool_use_id": call["id"],
                                                       "content": answers[name]}]}]
        [read], _, _ = claude_code_bench.Scripted.decide(stand_in, {"tools": [{"name": "Bash"}], "messages": messages})
        self.assertEqual("Read", read["name"])
        self.assertEqual(["branch", "flush", "update"], [step["step"] for step in stand_in.checkpoint.steps])
        # Codex: exec_command calls, their outputs, then the read.
        stand_in = SimpleNamespace(requests=[], source="composer.json", read="",
                                   checkpoint=bench_checkpoint.Checkpoint(self.SKILL))
        stand_in.checkpoint_step = types.MethodType(codex_bench.Scripted.checkpoint_step, stand_in)
        tools = [{"type": "function", "name": "exec_command"}]
        items = [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": self.HINT}]}]
        for name in ("branch", "flush", "update"):
            [call], _ = codex_bench.Scripted.decide(stand_in, {"tools": tools, "input": items})
            self.assertTrue(call["call_id"].endswith("_" + name))
            items += [call, {"type": "function_call_output", "call_id": call["call_id"],
                             "output": "Process exited with code 0\nOutput:\n" + answers[name]}]
        [read], _ = codex_bench.Scripted.decide(stand_in, {"tools": tools, "input": items})
        self.assertEqual("cat composer.json", json.loads(read["arguments"])["cmd"])
        self.assertEqual(["branch", "flush", "update"], [step["step"] for step in stand_in.checkpoint.steps])


class TargetTest(unittest.TestCase):
    def test_a_target_the_rehearsal_refuses_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary) / "project"
            project.mkdir()
            self.assertIn("no installed accelerator", claude_code_bench.target_problem(project))
            (project / "memory-bank/scripts").mkdir(parents=True)
            (project / "memory-bank/scripts/context.py").write_text("", encoding="utf-8")
            self.assertIsNone(claude_code_bench.target_problem(project))
            (project / ".git").write_text("gitdir: /elsewhere\n", encoding="utf-8")
            self.assertIn("worktree or submodule", claude_code_bench.target_problem(project))
        # An edition inside this clone has the runtime, and is still not a project.
        self.assertIn("this clone", claude_code_bench.target_problem(ROOT / "PHP Core"))


class EditionTest(unittest.TestCase):
    def test_the_wordpress_edition_installs(self) -> None:
        # The bench passed "Cms/wordpress" to the installer, which takes
        # "WordPress": it could not run that edition at all.
        with tempfile.TemporaryDirectory() as temporary:
            project = claude_code_bench.prepare(Path(temporary), "WordPress")
            self.assertTrue((project / "memory-bank/scripts/context.py").is_file())
            self.assertTrue((project / ".claude/hooks/working-memory-write.sh").is_file())

    def test_the_bench_hands_the_installer_the_editions_name(self) -> None:
        seen = []

        def prepare(work: Path, edition: str) -> Path:
            seen.append(edition)
            raise SystemExit(0)

        argv = ["claude_code_bench.py", "--edition", "WordPress", "--claude", sys.executable]
        with mock.patch.object(claude_code_bench, "prepare", prepare), mock.patch.object(sys, "argv", argv), \
                mock.patch.object(claude_code_bench.sys, "platform", "linux"), \
                mock.patch.object(claude_code_bench.shutil, "which", return_value="/usr/bin/true"):
            with self.assertRaises(SystemExit):
                claude_code_bench.main()
        self.assertEqual(["WordPress"], seen)


class AttachedTest(unittest.TestCase):
    def test_an_attached_run_installs_nothing_and_carries_the_harness_overlay(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            project, state = claude_code_bench.prepare_attached(work, "PHP Core")
            self.assertFalse((project / "memory-bank").exists())
            self.assertFalse((project / ".claude").exists())
            self.assertFalse(state.is_relative_to(project))
            overlay = json.loads((work / "overlay.json").read_text(encoding="utf-8"))
            self.assertIn("--add-dir", overlay["arguments"])
            self.assertTrue(any("memory-bank/scripts/context.py" in rule
                                for rule in overlay["settings"]["permissions"]["allow"]))
            self.assertIn("Stop", overlay["settings"]["hooks"])
            self.assertEqual(str(state), overlay["environment"]["ACCELERATOR_STATE_DIR"])

    def test_attached_takes_no_target(self) -> None:
        argv = ["claude_code_bench.py", "--attached", "--target", "/p/next", "--claude", sys.executable]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(claude_code_bench.sys, "platform", "linux"), \
                mock.patch.object(claude_code_bench.shutil, "which", return_value="/usr/bin/true"):
            with contextlib.redirect_stderr(io.StringIO()) as said:
                self.assertEqual(2, claude_code_bench.main())
        self.assertIn("--attached runs a fresh project", said.getvalue())


class CodexAttachedTest(unittest.TestCase):
    def test_codex_attached_installs_nothing_and_takes_no_target(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            project, state = codex_bench.prepare_attached(work, "PHP Core")
            self.assertFalse((project / "memory-bank").exists())
            self.assertFalse((project / ".codex").exists())
            overlay = json.loads((work / "overlay.json").read_text(encoding="utf-8"))
            self.assertIn(str(state), overlay["arguments"])
            self.assertTrue(any(argument.startswith("hooks.") for argument in overlay["arguments"]))
        argv = ["codex_bench.py", "--attached", "--target", "/p/next", "--codex", sys.executable]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(codex_bench.sys, "platform", "linux"), \
                mock.patch.object(codex_bench.shutil, "which", return_value="/usr/bin/true"):
            with contextlib.redirect_stderr(io.StringIO()) as said:
                self.assertEqual(2, codex_bench.main())
        self.assertIn("--attached runs a fresh project", said.getvalue())


class CodexWorktreeTest(unittest.TestCase):
    def refused(self, *options: str) -> str:
        argv = ["codex_bench.py", *options, "--codex", sys.executable]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(codex_bench.sys, "platform", "linux"), \
                mock.patch.object(codex_bench.shutil, "which", return_value="/usr/bin/true"):
            with contextlib.redirect_stderr(io.StringIO()) as said:
                self.assertEqual(2, codex_bench.main())
        return said.getvalue()

    def test_a_worktree_run_needs_an_install_the_sandbox_writes_and_one_turn(self) -> None:
        self.assertIn("--worktree runs Codex", self.refused("--worktree", "--attached"))
        self.assertIn("--worktree runs Codex", self.refused("--worktree", "beside", "--sandbox", "read-only"))
        self.assertIn("--app-server runs one turn", self.refused("--app-server", "--queued", "8"))
        self.assertIn("--worktree codex lets", self.refused("--worktree", "codex", "--queued", "8"))

    def test_the_app_worktree_is_detached_marked_and_owned_as_the_app_writes_it(self) -> None:
        # The app-server has no request for a worktree: the bench makes it the
        # way the Codex app's own code does, and the owner record as it writes it.
        with tempfile.TemporaryDirectory() as temporary:
            project, home = Path(temporary) / "shop", Path(temporary) / "codex-home"
            project.mkdir()
            for step in (["init", "-q", "-b", "main"], ["commit", "-q", "--allow-empty", "-m", "base"]):
                subprocess.run([*claude_code_bench.GIT, *step], cwd=project, check=True, capture_output=True)
            place = codex_bench.app_worktree(home, project)
            self.assertEqual([place], codex_bench.managed_worktrees(home))
            self.assertEqual(home / "worktrees", place.parent.parent)
            self.assertEqual("shop", place.name)
            self.assertTrue((place.parent / ".codex-worktree-name").is_file())
            self.assertEqual(1, subprocess.run(["git", "-C", str(place), "symbolic-ref", "-q", "HEAD"],
                                               capture_output=True).returncode)
            self.assertEqual("", codex_bench.owner_of(place))
            thread = "01a1776d-b888-7ac2-8a1b-9557bdbd1dad"
            codex_bench.own(place, thread)
            self.assertEqual('{\n  "version": 1,\n  "ownerThreadId": "%s"\n}\n' % thread,
                             (codex_bench.git_directory(place) / "codex-thread.json").read_text(encoding="utf-8"))
            self.assertEqual(thread, codex_bench.owner_of(place))

    def test_only_the_approvals_of_the_persons_hooks_file_count_as_the_setups(self) -> None:
        # The setup must leave the approval to the person: an approval of a
        # project's hooks or a plugin's is not one of its entries.
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            mine = f"{home / 'hooks.json'}:session_start:0:0"
            (home / "config.toml").write_text(
                f'model = "m"\n\n[hooks.state."/p/next/.codex/hooks.json:stop:0:0"]\ntrusted_hash = "sha256:1"\n\n'
                f'[hooks.state."ponytail@ponytail:hooks/hooks.json:session_start:0:0"]\ntrusted_hash = "sha256:2"\n',
                encoding="utf-8")
            self.assertEqual([], codex_bench.user_approvals(home))
            with (home / "config.toml").open("a", encoding="utf-8") as config:
                config.write(f'\n[hooks.state."{mine}"]\ntrusted_hash = "sha256:3"\n')
            self.assertEqual([mine], codex_bench.user_approvals(home))
            with mock.patch.dict(sys.modules, {"tomllib": None}):
                self.assertEqual([mine], codex_bench.user_approvals(home))


class PersonTrustTest(unittest.TestCase):
    def test_the_bench_reads_whether_your_claude_code_trusts_the_target(self) -> None:
        # next had never had its trust accepted: there `claude -p` refused
        # the save after every answer, which the bench's own HOME hides.
        with tempfile.TemporaryDirectory() as folder:
            config = Path(folder) / ".claude.json"
            with mock.patch.dict(claude_code_bench.os.environ, {"CLAUDE_CONFIG_DIR": folder}):
                self.assertIsNone(claude_code_bench.person_trust(Path("/p/next")))
                config.write_text(json.dumps({"projects": {"/p/next": {"hasTrustDialogAccepted": False},
                                                           "/p/scb": {"hasTrustDialogAccepted": True}}}))
                self.assertIs(False, claude_code_bench.person_trust(Path("/p/next")))
                self.assertIs(True, claude_code_bench.person_trust(Path("/p/scb")))
                self.assertIs(False, claude_code_bench.person_trust(Path("/p/never-opened")))


class NamespaceTest(unittest.TestCase):
    def test_the_bench_keeps_the_persons_uid_where_unshare_can(self) -> None:
        # Inside `unshare -r` Claude Code runs as root and refuses
        # bypassPermissions: "cannot be used with root/sudo privileges".
        kept = claude_code_bench.namespace("  --map-user=<uid>|<name>  map current user to uid")
        self.assertEqual(["unshare", "-n", "--user"], kept[:3])
        self.assertIn("--keep-caps", kept)
        self.assertNotIn("-r", kept)
        self.assertEqual(["unshare", "-rn"], claude_code_bench.namespace("  -r, --map-root-user"))


if __name__ == "__main__":
    unittest.main()
