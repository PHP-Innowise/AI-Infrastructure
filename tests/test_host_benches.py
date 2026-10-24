"""The offline host benches, without the hosts.

scripts/claude_code_bench.py and scripts/codex_bench.py drive the installed
Claude Code and Codex, and CI has neither. This holds what they rely on: each
finds the save command in the step the runtime writes for every host, their
stand-ins take a turn the way each host sends it (Claude Code's hook feedback,
Codex's XML-escaped <hook_prompt> and code mode), and a target the rollout
rehearsal refuses is refused.
"""
import html
import importlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
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
