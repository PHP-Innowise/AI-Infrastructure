#!/usr/bin/env python3
"""Regression tests for the agent message channel and capsule validation.

This file is byte-identical across the Laravel, Symfony and PHP Core
editions: it locates the edition root relative to itself and never mentions
an edition by name.

The channel is exercised exactly as an orchestrating flow uses it: the
``context.py`` CLI run as a subprocess against a disposable governed
repository — ``msg-send`` / ``msg-read`` / ``msg-dispatch`` / ``capsule``,
plus the phase-ordering guard on ``update``.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "context.py"

VALID_CAPSULE = """\
**Objective** - review the change set through one lens.
**Output format** - findings list plus a Context Summary.
**Tool and source guidance** - read src/ and the task handoff.
**Task boundaries** - report only; modify nothing.
**Decisions and assumptions so far** - scope agreed with the user.
"""


class ChannelTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="channel-test-")
        self.repository = Path(self.temporary.name)
        subprocess.run(
            ["git", "init", "--quiet", str(self.repository)],
            check=True,
            capture_output=True,
            text=True,
        )
        result = self.run_cli(
            "start", "--task-id", "task-1", "--goal", "Orchestrated demo task"
        )
        self.assertEqual(0, result.returncode, result.stderr)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_cli(self, *arguments: str, mode: str | None = None) -> subprocess.CompletedProcess[str]:
        command = [sys.executable, str(SCRIPT), "--root", str(self.repository)]
        if mode:
            command += ["--mode", mode]
        return subprocess.run(
            [*command, *arguments], text=True, capture_output=True
        )

    def send(self, **overrides: str) -> subprocess.CompletedProcess[str]:
        options = {
            "--task-id": "task-1",
            "--from": "coder",
            "--to": "code-reviewer",
            "--type": "finding",
            "--body": "PDO binding fixed in src/Repo.php",
        }
        options.update(overrides)
        arguments = [item for pair in options.items() for item in pair if item]
        return self.run_cli("msg-send", *arguments, "--json")

    def read_json(self, *arguments: str) -> list[dict[str, object]]:
        result = self.run_cli(
            "msg-read", "--task-id", "task-1", *arguments, "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def journal_path(self) -> Path:
        messages = self.repository / "project-brain" / "control" / "messages"
        files = list(messages.glob("*.jsonl"))
        self.assertEqual(1, len(files))
        return files[0]

    def test_send_and_read_roundtrip_with_filters(self) -> None:
        self.assertEqual(0, self.send().returncode)
        self.assertEqual(
            0,
            self.send(
                **{"--from": "main", "--to": "*", "--type": "question",
                   "--body": "Anyone blocked?"}
            ).returncode,
        )
        everything = self.read_json()
        self.assertEqual([1, 2], [message["seq"] for message in everything])
        addressed = self.read_json("--for", "code-reviewer")
        # Direct message plus the broadcast, in journal order.
        self.assertEqual([1, 2], [message["seq"] for message in addressed])
        other = self.read_json("--for", "verify")
        self.assertEqual([2], [message["seq"] for message in other])
        later = self.read_json("--since", "1")
        self.assertEqual([2], [message["seq"] for message in later])
        typed = self.read_json("--type", "question")
        self.assertEqual([2], [message["seq"] for message in typed])

    def test_lightweight_mode_refuses_messages(self) -> None:
        result = self.run_cli(
            "msg-read", "--task-id", "task-1", mode="lightweight"
        )
        self.assertNotEqual(0, result.returncode)
        self.assertIn("governed mode", result.stderr)

    def test_body_validation(self) -> None:
        empty = self.send(**{"--body": "   "})
        self.assertNotEqual(0, empty.returncode)
        oversize = self.send(**{"--body": "x" * 8001})
        self.assertNotEqual(0, oversize.returncode)
        self.assertIn("8000", oversize.stderr)
        secret = self.send(**{"--body": "use api_key: abcd1234efgh"})
        self.assertNotEqual(0, secret.returncode)
        self.assertIn("not stored", secret.stderr)

    def test_actor_validation(self) -> None:
        bad_sender = self.send(**{"--from": "Not A Slug"})
        self.assertNotEqual(0, bad_sender.returncode)
        self.assertIn("slug", bad_sender.stderr)
        broadcast_sender = self.send(**{"--from": "*"})
        self.assertNotEqual(0, broadcast_sender.returncode)

    def test_terminal_task_refuses_send_but_stays_readable(self) -> None:
        self.assertEqual(0, self.send().returncode)
        completed = self.run_cli(
            "complete", "--task-id", "task-1", "--outcome", "Done",
            "--revision", "1",
        )
        self.assertEqual(0, completed.returncode, completed.stderr)
        refused = self.send(**{"--body": "too late"})
        self.assertNotEqual(0, refused.returncode)
        self.assertIn("terminal", refused.stderr)
        self.assertEqual([1], [m["seq"] for m in self.read_json()])

    def test_dispatch_records_digest_and_directions(self) -> None:
        capsule = self.repository / "capsule.md"
        capsule.write_text(VALID_CAPSULE, encoding="utf-8")
        spawn = self.run_cli(
            "msg-dispatch", "--task-id", "task-1", "--agent", "code-reviewer",
            "--event", "spawn", "--capsule-file", str(capsule), "--json",
        )
        self.assertEqual(0, spawn.returncode, spawn.stderr)
        record = json.loads(spawn.stdout)
        self.assertEqual(("main", "code-reviewer", "dispatch"),
                         (record["from_actor"], record["to_actor"], record["type"]))
        self.assertRegex(record["capsule_digest"], r"^[0-9a-f]{64}$")
        done = self.run_cli(
            "msg-dispatch", "--task-id", "task-1", "--agent", "code-reviewer",
            "--event", "complete", "--note", "review done: 2 findings", "--json",
        )
        self.assertEqual(0, done.returncode, done.stderr)
        record = json.loads(done.stdout)
        self.assertEqual(("code-reviewer", "main", "completion"),
                         (record["from_actor"], record["to_actor"], record["type"]))

    def test_spawn_refuses_underspecified_capsule(self) -> None:
        capsule = self.repository / "capsule.md"
        capsule.write_text("just do the thing", encoding="utf-8")
        spawn = self.run_cli(
            "msg-dispatch", "--task-id", "task-1", "--agent", "coder",
            "--event", "spawn", "--capsule-file", str(capsule),
        )
        self.assertNotEqual(0, spawn.returncode)
        self.assertIn("under-specified", spawn.stderr)
        self.assertEqual([], self.read_json())

    def test_capsule_validate_command(self) -> None:
        good = self.repository / "good.md"
        good.write_text(VALID_CAPSULE, encoding="utf-8")
        result = self.run_cli("capsule", "--validate", "--file", str(good), "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertTrue(json.loads(result.stdout)["valid"])
        bad = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.repository),
             "capsule", "--validate", "--file", "-", "--json"],
            input="objective only",
            text=True,
            capture_output=True,
        )
        self.assertEqual(1, bad.returncode)
        verdict = json.loads(bad.stdout)
        self.assertFalse(verdict["valid"])
        self.assertIn("missing mandatory section: output format", verdict["problems"])

    def test_phase_moves_forward_only_unless_overridden(self) -> None:
        forward = self.run_cli(
            "update", "--task-id", "task-1", "--phase", "implementation",
            "--revision", "1",
        )
        self.assertEqual(0, forward.returncode, forward.stderr)
        regression = self.run_cli(
            "update", "--task-id", "task-1", "--phase", "planning"
        )
        self.assertNotEqual(0, regression.returncode)
        self.assertIn("backward", regression.stderr)
        allowed = self.run_cli(
            "update", "--task-id", "task-1", "--phase", "planning",
            "--allow-phase-regression",
        )
        self.assertEqual(0, allowed.returncode, allowed.stderr)
        alias = self.run_cli(
            "update", "--task-id", "task-1", "--phase", "review"
        )
        self.assertEqual(0, alias.returncode, alias.stderr)

    def test_update_actor_prefixes_progress(self) -> None:
        result = self.run_cli(
            "update", "--task-id", "task-1", "--actor", "coder",
            "--progress", "handler wired",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        shown = self.run_cli("get", "--task-id", "task-1", "--json")
        self.assertEqual(
            "[coder] handler wired", json.loads(shown.stdout)["progress"]
        )
        bad = self.run_cli(
            "update", "--task-id", "task-1", "--actor", "Not A Slug",
            "--progress", "x",
        )
        self.assertNotEqual(0, bad.returncode)

    def test_validate_catches_journal_tampering(self) -> None:
        self.assertEqual(0, self.send().returncode)
        self.assertEqual(0, self.send(**{"--body": "second"}).returncode)
        clean = self.run_cli("validate")
        self.assertEqual(0, clean.returncode, clean.stderr)
        journal = self.journal_path()
        lines = journal.read_text(encoding="utf-8").splitlines()
        entry = json.loads(lines[1])
        entry["seq"] = 7
        lines[1] = json.dumps(entry, ensure_ascii=False, sort_keys=True)
        journal.write_text("\n".join(lines) + "\n", encoding="utf-8")
        broken = self.run_cli("validate")
        self.assertNotEqual(0, broken.returncode)
        self.assertIn("seq must increase", broken.stdout + broken.stderr)

    def test_validate_rejects_foreign_journal_name(self) -> None:
        messages = self.repository / "project-brain" / "control" / "messages"
        messages.mkdir(parents=True, exist_ok=True)
        (messages / "not-a-uuid.jsonl").write_text("{}\n", encoding="utf-8")
        result = self.run_cli("validate")
        self.assertNotEqual(0, result.returncode)
        self.assertIn("task UUID", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
