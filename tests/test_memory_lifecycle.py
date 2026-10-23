"""One task across two sessions, a commit and a second branch, through the installed hooks.

The rollout rehearsal (scripts/memory_rollout_rehearsal.py) runs one turn and the next session's
recall. A task lives longer than that: a learning in one session, a progress and a next step in
the next turn, a commit, a new session, another branch with its own task, and the way back. This
follows that life with Claude Code, where a person reads the session, and with Codex, whose save
continuation comes back through the prompt hook.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GIT = ("git", "-c", "user.name=Lifecycle test", "-c", "user.email=lifecycle-test@example.invalid",
       "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null")


def save_command(step: str, result: dict) -> tuple:
    """The save a Stop step asks for, ready to run, and its stdin: Claude
    Code's command takes the JSON in place of <JSON>, the others' a pipe."""
    command = re.search(r"exact command: (\(cd .*?--json\)|cd .*?--json)\n", step).group(1)
    data = json.dumps(result)
    if "'<JSON>'" in command:
        return command.replace("'<JSON>'", "'" + data.replace("'", "'\\''") + "'"), None
    return command, data


@unittest.skipUnless(os.name == "posix" and shutil.which("bash") and shutil.which("git"), "POSIX shell hooks")
class MemoryLifecycleTest(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="memory lifecycle ")
        self.addCleanup(temporary.cleanup)
        base = Path(temporary.name)
        (base / "home").mkdir()
        (base / "tmp").mkdir()
        # A suite run from a Claude Code or Codex session inherits that session's marks.
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("GIT_", "CONTEXT_", "ACCELERATOR_", "CLAUDE", "CODEX"))}
        self.env.update(HOME=str(base / "home"), TMPDIR=str(base / "tmp"), GIT_OPTIONAL_LOCKS="0",
                        PYTHONDONTWRITEBYTECODE="1")
        self.project = base / "shop"
        (self.project / "src").mkdir(parents=True)
        (self.project / "composer.json").write_text('{"name": "acme/shop"}\n', encoding="utf-8")
        (self.project / "src/Invoice.php").write_text("<?php\nfinal class Invoice {}\n", encoding="utf-8")
        (self.project / "src/Ledger.php").write_text("<?php\nfinal class Ledger {}\n", encoding="utf-8")
        self.git("init", "-q", "-b", "feature/SHOP-7-invoices")
        self.git("add", ".")
        self.git("commit", "-qm", "Shop baseline")
        installed = subprocess.run(
            [sys.executable, str(ROOT / "scripts/install_accelerator.py"), "--edition", "PHP Core",
             "--target", str(self.project), "--merge-existing"],
            cwd=ROOT, env=self.env, capture_output=True, text=True,
        )
        self.assertEqual(0, installed.returncode, installed.stdout[-600:] + installed.stderr[-600:])

    def git(self, *args: str) -> str:
        return subprocess.run([*GIT, *args], cwd=self.project, env=self.env, capture_output=True,
                              text=True, check=True).stdout.strip()

    def hook(self, host: str, name: str, payload: dict, env: dict) -> str:
        tree = ".claude" if host == "claude" else ".codex"
        if host == "codex":
            payload = dict(payload, turn_id=str(uuid.uuid4()), model="test", permission_mode="default")
        done = subprocess.run(["bash", str(self.project / tree / "hooks" / name)],
                              input=json.dumps(payload).encode(), cwd=self.project, env=env,
                              capture_output=True, timeout=120)
        self.assertEqual(0, done.returncode, done.stderr.decode(errors="replace")[-400:])
        return done.stdout.decode()

    def turn(self, host: str, session: str, prompt: str, result: dict = None) -> str:
        env = dict(self.env, CLAUDE_PROJECT_DIR=str(self.project))
        if host == "claude":
            env.update(CLAUDE_CODE_SESSION_ATTENDED="1", CLAUDE_CODE_ENTRYPOINT="claude-desktop")
        base = {"session_id": session, "cwd": str(self.project)}
        capsule = self.hook(host, "working-memory-read.sh",
                            dict(base, hook_event_name="UserPromptSubmit", prompt=prompt), env)
        stop = self.hook(host, "working-memory-write.sh",
                         dict(base, hook_event_name="Stop", stop_hook_active=False), env)
        if result is not None:
            output = json.loads(stop.strip().splitlines()[0])
            if host == "codex":
                reason = output["reason"]
                # The continuation comes back as a prompt; the turn's own prompt brought the capsule.
                echoed = self.hook(host, "working-memory-read.sh",
                                   dict(base, hook_event_name="UserPromptSubmit", prompt=reason), env)
                self.assertEqual("", echoed.strip())
            else:
                # Hook feedback, not a block: Claude Code would label a block a hook error.
                self.assertNotIn("decision", output)
                reason = output["hookSpecificOutput"]["additionalContext"]
            command, data = save_command(reason, result)
            saved = subprocess.run(["bash", "-c", command], input=data, cwd=self.project,
                                   env=env, capture_output=True, text=True, timeout=120)
            self.assertEqual(0, saved.returncode, saved.stdout[-400:] + saved.stderr[-400:])
        self.hook(host, "working-memory-write.sh", dict(base, hook_event_name="Stop", stop_hook_active=True), env)
        return capsule

    def follow_a_task(self, host: str) -> None:
        first = self.git("branch", "--show-current")
        self.turn(host, "one-" + host, "How are invoices numbered?", {
            "progress": "Read src/Invoice.php; numbering is per tenant.", "verified": True,
            "learnings": [{"type": "finding", "title": "Invoice numbers are per tenant",
                           "consequence": "Invoice numbering restarts for each tenant.",
                           "sources": ["src/Invoice.php"]}]})
        with open(self.project / "src/Ledger.php", "a", encoding="utf-8") as handle:
            handle.write("// per tenant\n")
        self.turn(host, "one-" + host, "Make the ledger per tenant.", {
            "progress": "Made src/Ledger.php per tenant; tests next.", "verified": True, "learnings": [],
            "next_steps": ["Run the ledger tests."]})
        self.git("add", "src/Ledger.php")
        self.git("commit", "-qm", "Ledger per tenant")

        capsule = self.turn(host, "two-" + host, "What do we know about invoice numbers per tenant?")
        self.assertIn("Made src/Ledger.php per tenant", capsule)
        self.assertIn("Run the ledger tests.", capsule)
        self.assertIn("Invoice numbers are per tenant", capsule)

        self.git("checkout", "-qb", "feature/SHOP-8-refunds")
        capsule = self.turn(host, "three-" + host, "Start the refunds.", {
            "progress": "Refunds started.", "verified": True, "learnings": []})
        self.assertNotIn("Made src/Ledger.php", capsule)

        self.git("checkout", "-q", first)
        capsule = self.turn(host, "four-" + host, "Continue with the ledger.")
        self.assertIn("Made src/Ledger.php per tenant", capsule)
        self.assertNotIn("Refunds started", capsule)

        journal = (self.project / "memory-bank/local/native-results.ndjson").read_text(encoding="utf-8")
        events = [json.loads(line) for line in journal.splitlines()]
        self.assertEqual(3, sum(1 for line in events if line["stage"] == "save" and line["event"] == "saved"))
        validate = subprocess.run([sys.executable, "memory-bank/scripts/context.py", "validate"],
                                  cwd=self.project, env=self.env, capture_output=True, text=True, timeout=600)
        self.assertEqual(0, validate.returncode, validate.stdout[-400:] + validate.stderr[-400:])

    def test_cursor_keeps_the_last_prompts_rule_through_a_save_continuation(self) -> None:
        # Cursor reads its rule before the prompt hook runs, so the rule a
        # prompt writes is what the next request carries. Its save
        # continuation comes back as a prompt: a capsule rendered for the
        # instruction's words replaced the rule the person's next request got.
        (self.project / "billing.md").write_text(
            "# Billing\n\nInvoices are numbered per tenant and restart each year.\n", encoding="utf-8")
        self.git("add", "billing.md")
        self.git("commit", "-qm", "Billing")

        def cursor(name: str, payload: dict) -> str:
            payload = dict(payload, conversation_id="lifecycle", workspace_roots=[str(self.project)],
                           cursor_version="2.0.0")
            done = subprocess.run(["bash", str(self.project / ".cursor/hooks" / name)],
                                  input=json.dumps(payload).encode(), cwd=self.project, env=self.env,
                                  capture_output=True, timeout=120)
            self.assertEqual(0, done.returncode, done.stderr.decode(errors="replace")[-400:])
            return done.stdout.decode()

        def prompt(generation: str, text: str) -> str:
            return cursor("working-memory-read.sh", {"generation_id": generation,
                                                     "hook_event_name": "beforeSubmitPrompt", "prompt": text})

        def stop(generation: str, loop: int) -> str:
            return cursor("working-memory-write.sh", {"generation_id": generation, "hook_event_name": "stop",
                                                      "status": "completed", "loop_count": loop})

        prompt("g1", "How are invoices numbered per tenant?")
        followup = json.loads(stop("g1", 0))["followup_message"]
        command, data = save_command(followup, {
            "progress": "Read billing.md; invoice numbering is per tenant.", "verified": True,
            "learnings": [{"type": "finding", "title": "Invoices are numbered per tenant",
                           "consequence": "Invoice numbering restarts per tenant and per year.",
                           "sources": ["billing.md"]}]})
        saved = subprocess.run(["bash", "-c", command], input=data, cwd=self.project, env=self.env,
                               capture_output=True, text=True, timeout=120)
        self.assertEqual(0, saved.returncode, saved.stdout[-400:] + saved.stderr[-400:])
        stop("g1", 1)

        prompt("g2", "Which tenant rules apply to invoice numbers?")
        followup = json.loads(stop("g2", 0))["followup_message"]
        rule = self.project / ".cursor/rules/working-memory.mdc"
        before = rule.read_bytes()
        self.assertIn(b"Invoices are numbered per tenant", before)
        self.assertEqual({"continue": True}, json.loads(prompt("g3", followup)))
        self.assertEqual(before, rule.read_bytes())

    def test_claude_code_follows_a_task_across_sessions_a_commit_and_a_branch(self) -> None:
        self.follow_a_task("claude")

    def test_codex_follows_a_task_and_its_save_continuation_gets_no_capsule(self) -> None:
        self.follow_a_task("codex")


if __name__ == "__main__":
    unittest.main()
