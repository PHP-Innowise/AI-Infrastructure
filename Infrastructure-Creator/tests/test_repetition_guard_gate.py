#!/usr/bin/env python3
"""Every generated bash-validator.sh runs the editions' command repetition guard.

A failing command rerun forever touches no file, so loop-detection.sh never
sees that loop. The four editions' validators count a command's identical
reruns in a session once their rules let it through - a warning at the
sixth, a refusal at the twelfth - leave a standalone status query uncounted,
and start a command's count over when loop-detection.sh recorded a file edit
since the command last ran. hook-forge step 2 copies that guard from its own
assets/bash-validator-repetition-guard.sh, byte for byte, between the
bv_validate call's `fi` and the final `exit 0`, and the hook assigns BV_HOST
its edition, which names the counter directory and, on Cursor, the
warning's channel. Step 5 has loop-detection.sh compute that directory with
the guard's own lines and keep its edit-* counters there.

bootstrap-verifier holds the guard to the asset byte for byte, checks where
it sits and what BV_HOST names, and holds loop-detection.sh to the guard's
directory lines and its counter name - without running a hook. The tests
here also run the built pair through bash, as the hosts do.

Run from this directory: python3 test_repetition_guard_gate.py
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))
from test_bash_validator_gate import (  # noqa: E402
    BASH,
    EDITIONS,
    GENERATOR_VALIDATOR,
    HOOK_FORGE,
    REPETITION_GUARD,
    TARGET_RULES,
    GateFixture,
    built,
    generator_parts,
    line_of,
    repetition_guard,
    validator,
)

GUARD_OPENING, GUARD_CLOSING = validator.REPETITION_GUARD_MARKERS
COMMAND = "vendor/bin/phpunit --filter Broken"

# loop-detection.sh's counting half as hook-forge step 5 asks for it: the
# guard's four directory lines with the host spelled out, and one
# edit-<session>-<path hash> counter per file, written on every edit.
LOOP_DETECTION = r'''#!/bin/bash
# Loop Detection Hook (Acme Shop): counts the edits each file gets in a session.
INPUT=
IFS= read -r -d '' INPUT || :
FILE_PATH=$(printf '%s' "$INPUT" \
  | sed -n -E 's/.*"(file_path|notebook_path|path)"[[:space:]]*:[[:space:]]*"([^"]*)".*/\2/p' | head -1)
[ -n "$FILE_PATH" ] || exit 0
SESSION_KEY=$(printf '%s' "$INPUT" \
  | sed -n -E 's/.*"(session_id|conversation_id)"[[:space:]]*:[[:space:]]*"([^"]*)".*/\2/p' | head -1)
SESSION_KEY=${SESSION_KEY//[^A-Za-z0-9]/}
SESSION_KEY=${SESSION_KEY:0:64}
[ -n "$SESSION_KEY" ] || SESSION_KEY=shared
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
REPO_KEY=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
TRACK_BASE=${TMPDIR:-/tmp}
TRACK_DIR="${TRACK_BASE%/}/<tool>-loop-detection-${EUID:-0}-$REPO_KEY"
mkdir -m 700 "$TRACK_DIR" 2>/dev/null
if [ -L "$TRACK_DIR" ] || [ ! -d "$TRACK_DIR" ] || [ ! -O "$TRACK_DIR" ]; then
  exit 0
fi
TRACK_FILE="$TRACK_DIR/edit-$SESSION_KEY-$(printf '%s' "$FILE_PATH" | cksum | cut -d' ' -f1)"
[ -L "$TRACK_FILE" ] && exit 0
COUNT=0
[ -f "$TRACK_FILE" ] && COUNT=$(cat "$TRACK_FILE" 2>/dev/null)
case "$COUNT" in *[!0-9]*|'') COUNT=0 ;; esac
[ "${#COUNT}" -gt 9 ] && COUNT=0
COUNT=$((10#$COUNT + 1))
echo "$COUNT" > "$TRACK_FILE" 2>/dev/null
exit 0
'''


def loop_detection(tool: str) -> bytes:
    return LOOP_DETECTION.replace("<tool>", tool).encode()


def edit_payload(path: str = "/work/app/Fixing.php", **fields) -> dict:
    return {
        "tool_name": "Edit",
        "tool_input": {"file_path": path, "old_string": "a", "new_string": "b"},
        **fields,
    }


class GuardFixture(GateFixture):
    """A target's bash-validator.sh and loop-detection.sh, built per edition."""

    def write_detector(self, edition: str, data: bytes, *, owned: bool = True) -> Path:
        relative = f"{validator.EDITION_HOOK_WIRING[edition][0]}/loop-detection.sh"
        path = self.target / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o755)
        if owned:
            self.files[relative] = {"sha256": "x"}
        return path

    def write_pair(self, edition: str) -> tuple:
        return (
            self.write(edition, built(host=edition)),
            self.write_detector(edition, loop_detection(edition)),
        )

    def track_dir(self, tool: str) -> Path:
        """The counter directory, computed in the target as the hooks compute it."""
        key = subprocess.run(
            [BASH, "-c", "printf %s \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\""
             " | cksum | cut -d' ' -f1"],
            cwd=self.target, capture_output=True, text=True, check=True, timeout=30,
        ).stdout.strip()
        return self.tmpdir / f"{tool}-loop-detection-{os.geteuid()}-{key}"

    def outcome(self, result: subprocess.CompletedProcess) -> tuple:
        return result.returncode, result.stdout, result.stderr

    def warning(self, edition: str, result: subprocess.CompletedProcess) -> str:
        """The warning as its host shows it, after checking the channel."""
        if edition == "cursor":
            # No channel to the agent for a command Cursor lets run.
            self.assertEqual((1, ""), (result.returncode, result.stdout), result.stderr)
            return result.stderr
        # Exit 0 with additionalContext: Claude Code and Codex show the model.
        self.assertEqual((0, ""), (result.returncode, result.stderr))
        output = json.loads(result.stdout)["hookSpecificOutput"]
        self.assertEqual("PreToolUse", output["hookEventName"])
        return output["additionalContext"]


class GuardAssetTest(unittest.TestCase):
    def test_the_asset_holds_one_guard_that_names_its_directory_by_bv_host(self) -> None:
        self.assertEqual(validator.REPETITION_GUARD, REPETITION_GUARD)
        data = REPETITION_GUARD.read_bytes()
        _, _, problem = validator.marked_block(
            data, validator.REPETITION_GUARD_MARKERS, "repetition guard"
        )
        self.assertEqual(problem, "")
        lines = validator.repetition_guard_lines(repetition_guard())
        self.assertEqual(sorted(lines), sorted(validator.REPETITION_GUARD_DIRECTORY))
        self.assertIn("/$BV_HOST-loop-detection-", lines["TRACK_DIR"])
        result = subprocess.run([BASH, "-n", str(REPETITION_GUARD)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_the_guard_names_no_skill_a_generated_project_may_lack(self) -> None:
        guard = repetition_guard().decode("utf-8")
        for name in ("/debugger", "systematic-debugger"):
            self.assertNotIn(name, guard)

    def test_hook_forge_names_the_asset_markers_setting_and_directory_lines(self) -> None:
        text = HOOK_FORGE.read_text(encoding="utf-8")
        needles = [
            "`assets/bash-validator-repetition-guard.sh`",
            f"`{GUARD_OPENING.decode()}`",
            f"`{GUARD_CLOSING.decode()}`",
            "`BV_HOST`",
            # Step 5 copies the guard's directory lines, the host spelled out.
            "the edition's name in place of `$BV_HOST`",
            '`"$TRACK_DIR/edit-$SESSION_KEY-<hash of its path>"`',
            *(f"`{name}`" for name in validator.REPETITION_GUARD_DIRECTORY),
        ]
        for needle in needles:
            with self.subTest(needle=needle):
                self.assertIn(needle, text)


class GuardGateTest(GuardFixture):
    def test_a_validator_without_the_guard_is_rejected(self) -> None:
        for name, data in (
            ("guard left out", built(guard=b"")),
            ("the generator's own validator", GENERATOR_VALIDATOR.read_bytes()),
        ):
            with self.subTest(variant=name):
                self.write("claude", data)
                self.assert_one_error(
                    "claude",
                    f"has no `{GUARD_OPENING.decode()}` marker",
                    f"copy the repetition guard from {REPETITION_GUARD}",
                    "hook-forge step 2",
                )

    def test_an_edited_or_trimmed_guard_is_rejected_at_its_line(self) -> None:
        guard = repetition_guard()
        poll = guard[guard.index(b"BV_POLL_QUERY="):guard.index(b"\nfi\n", guard.index(b"BV_POLL_QUERY=")) + 4]
        for name, edited, needle in (
            ("threshold", guard.replace(b'"$COUNT" -ge 12', b'"$COUNT" -ge 20', 1), b'"$COUNT" -ge 12'),
            ("polling exemption dropped", guard.replace(poll, b"", 1), b"BV_POLL_QUERY="),
            ("crlf", guard.replace(b"\n", b"\r\n"), GUARD_OPENING),
        ):
            with self.subTest(variant=name):
                self.write("cursor", built(host="cursor", guard=edited))
                line = line_of(built(host="cursor"), needle)
                self.assert_one_error(
                    "cursor",
                    f"line {line}: the repetition guard differs from the one in {REPETITION_GUARD}",
                    "never edited or trimmed",
                )

    def test_a_guard_marker_missing_doubled_or_reversed_is_rejected(self) -> None:
        guard = repetition_guard()
        for name, edited, fragment in (
            ("no closing marker", guard.replace(GUARD_CLOSING, b"# guard end", 1),
             f"has no `{GUARD_CLOSING.decode()}` marker"),
            ("twice", guard + b"\n" + guard, f"has the `{GUARD_OPENING.decode()}` marker 2 times"),
            ("reversed", guard.replace(GUARD_OPENING, b"\0", 1).replace(GUARD_CLOSING, GUARD_OPENING, 1)
             .replace(b"\0", GUARD_CLOSING, 1), "closes the repetition guard before it opens it"),
        ):
            with self.subTest(variant=name):
                self.write("codex", built(host="codex", guard=edited))
                self.assert_one_error("codex", fragment)

    def test_a_guard_before_the_call_is_rejected_and_lets_a_status_query_skip_the_rules(self) -> None:
        _, _, call = generator_parts()
        hook = self.write("claude", built(call=repetition_guard() + b"\n\n" + call, guard=b""))
        self.assert_one_error("claude", "runs the repetition guard before bv_validate")
        # `tail .env` reads like polling: the guard's exemption exits 0
        # before the generic rule against printing .env has looked at it.
        self.assertEqual(self.run_hook(hook, "tail .env").returncode, 0)
        ordered = self.write("claude", built())
        result = self.run_hook(ordered, "tail .env")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("reading a .env file", result.stderr)

    def test_an_exit_before_the_guard_is_rejected_and_counts_nothing(self) -> None:
        _, _, call = generator_parts()
        # The generator's own `exit 0` copied with the call, guard after it.
        hook = self.write("codex", built(host="codex", call=call + b"\nexit 0\n"))
        self.assert_one_error("codex", "exits before the repetition guard, which then counts no command")
        for _ in range(12):
            self.assertEqual(self.outcome(self.run_hook(hook, COMMAND)), (0, "", ""))

    def test_only_exit_0_may_follow_the_guard(self) -> None:
        complete = built()
        self.assertTrue(complete.endswith(GUARD_CLOSING + b"\n\nexit 0\n"))
        self.write("claude", complete[:-len(b"\nexit 0\n")])
        self.assertEqual(self.errors(["claude"]), [])
        self.write("claude", complete[:-len(b"exit 0\n")] + b"exit 1\n")
        self.assert_one_error("claude", "runs `exit 1` after the repetition guard; only `exit 0` follows it")

    def test_bv_host_unset_is_rejected_and_lets_every_counted_command_exit_1(self) -> None:
        hook = self.write("claude", built(host=None))
        self.assert_one_error(
            "claude", "never assigns BV_HOST below the generic section", "assign BV_HOST=claude"
        )
        refused = self.run_hook(hook, "git push --force origin main")
        self.assertEqual(refused.returncode, 2, refused.stderr)
        result = self.run_hook(hook, COMMAND)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("BV_HOST: unbound variable", result.stderr)

    def test_bv_host_must_name_the_edition(self) -> None:
        for rules, ok in (
            (b"BV_HOST='cursor'\n", True),
            (b'BV_HOST="cursor"\n', True),
            (b"BV_HOST=cursor  # this copy's host\n", True),
            (b"BV_HOST=claude\nBV_HOST=cursor\n", True),
            (b"BV_HOST=claude\n", False),
            (b"BV_HOST=cursor\nBV_HOST=claude\n", False),
            # Only a literal is read: a computed name is not checked.
            (b"BV_HOST=$(printf cursor)\n", False),
        ):
            with self.subTest(rules=rules):
                self.write("cursor", built(TARGET_RULES + rules, host=None))
                if ok:
                    self.assertEqual(self.errors(["cursor"]), [])
                else:
                    self.assert_one_error("cursor", "in the cursor edition", "assign BV_HOST=cursor")

    def test_bv_host_counts_only_where_it_always_runs(self) -> None:
        for rules in (
            b"if [ -d .cursor ]; then\n  BV_HOST=cursor\nfi\n",
            b"# BV_HOST=cursor\n",
        ):
            with self.subTest(rules=rules):
                self.write("cursor", built(TARGET_RULES + rules, host=None))
                self.assert_one_error("cursor", "never assigns BV_HOST")


class LoopDetectionPairTest(GuardFixture):
    def test_a_loop_detection_written_to_step_5_passes_in_every_edition(self) -> None:
        for edition in EDITIONS:
            self.write_pair(edition)
        self.assertEqual(self.errors(), [])

    def assert_detector_error(self, edition: str, *fragments: str) -> None:
        relative = f"{validator.EDITION_HOOK_WIRING[edition][0]}/loop-detection.sh"
        errors = self.errors()
        self.assertEqual(len(errors), 1, errors)
        self.assertTrue(errors[0].startswith(f"[{edition}] {relative} "), errors)
        for fragment in (*fragments, "edit-and-rerun", "hook-forge step 5"):
            self.assertIn(fragment, errors[0])

    def test_a_loop_detection_counting_elsewhere_is_rejected(self) -> None:
        line = 'TRACK_DIR="${TRACK_BASE%/}/cursor-loop-detection-${EUID:-0}-$REPO_KEY"'
        # Step 5's earlier one-liner, which named no REPO_KEY or TRACK_BASE.
        one_liner = (
            "REPO_KEY=$(printf '%s' \"$REPO_ROOT\" | cksum | cut -d' ' -f1)\n"
            "TRACK_BASE=${TMPDIR:-/tmp}\n"
            + line + "\n"
        )
        for name, source, missing in (
            ("earlier one-liner", loop_detection("cursor").decode().replace(
                one_liner,
                "TRACK_DIR=\"${TMPDIR:-/tmp}/cursor-loop-detection-$EUID-"
                "$(printf '%s' \"$REPO_ROOT\" | cksum | cut -d' ' -f1)\"\n", 1),
             "REPO_KEY=$(printf"),
            ("another host's directory", loop_detection("claude").decode(), line),
            ("the line in a comment", loop_detection("cursor").decode().replace(line, "# " + line, 1), line),
        ):
            with self.subTest(variant=name):
                for edition in EDITIONS:
                    self.write_pair(edition)
                self.write_detector("cursor", source.encode())
                self.assert_detector_error(
                    "cursor",
                    f"lacks the repetition guard's line `{missing}",
                    ".cursor/hooks/bash-validator.sh",
                )

    def test_a_loop_detection_without_edit_counters_is_rejected(self) -> None:
        for edition in EDITIONS:
            self.write_pair(edition)
        source = loop_detection("codex").replace(b'"$TRACK_DIR/edit-$SESSION_KEY-', b'"$TRACK_DIR/$SESSION_KEY-')
        self.write_detector("codex", source)
        self.assert_detector_error("codex", "keeps no edit- counter in $TRACK_DIR")

    def test_only_selected_owned_loop_detection_hooks_are_opened(self) -> None:
        self.write_pair("claude")
        # A directory where the hook would be: opening it would be an error.
        (self.target / ".cursor/hooks/loop-detection.sh").mkdir(parents=True)
        self.write_detector("codex", b"#!/bin/bash\nexit 0\n", owned=False)
        self.assertEqual(self.errors(), [])
        self.write_detector("codex", b"#!/bin/bash\nexit 0\n")
        self.assertEqual(self.errors(["claude", "cursor"]), [])

    def test_a_guard_asset_that_cannot_be_read_or_cut_fails_closed(self) -> None:
        for edition in EDITIONS:
            self.write_pair(edition)
        self.write_detector("codex", b"#!/bin/bash\nexit 0\n")
        unmarked = self.tmpdir / "unmarked-guard.sh"
        unmarked.write_bytes(b"#!/bin/bash\nexit 0\n")
        blind = self.tmpdir / "blind-guard.sh"
        blind.write_bytes(repetition_guard().replace(b"\nTRACK_DIR=", b"\nCOUNTER_DIR=", 1))
        for reference, fragment in (
            (self.tmpdir / "missing.sh", "cannot be read"),
            (unmarked, f"has no `{GUARD_OPENING.decode()}` marker"),
            (blind, "assigns no TRACK_DIR at the start of a line"),
        ):
            with self.subTest(reference=reference.name), \
                    mock.patch.object(validator, "REPETITION_GUARD", reference):
                # The guard and the pair go unchecked; the section still is.
                errors = self.errors()
                self.assertEqual(len(errors), 1, errors)
                self.assertTrue(errors[0].startswith(
                    f"repetition guard not checked: hook-forge's own {reference} {fragment}"
                ), errors)


class GuardRunsTest(GuardFixture):
    def test_a_rerun_warns_from_the_sixth_and_is_refused_at_the_twelfth(self) -> None:
        for edition in EDITIONS:
            with self.subTest(edition=edition):
                hook, _ = self.write_pair(edition)
                for _ in range(5):
                    self.assertEqual(self.outcome(self.run_hook(hook, COMMAND)), (0, "", ""))
                for count in range(6, 12):
                    warning = self.warning(edition, self.run_hook(hook, COMMAND))
                    self.assertIn(f"this exact command has run {count} times this session", warning)
                blocked = self.run_hook(hook, COMMAND)
                self.assertEqual((2, ""), (blocked.returncode, blocked.stdout))
                self.assertIn("BLOCKED: this exact command has run 12 times", blocked.stderr)
                self.assertIn("find the root cause", blocked.stderr)
                # Any change to the command starts its own count.
                other = self.run_hook(hook, COMMAND + " --stop-on-failure")
                self.assertEqual(self.outcome(other), (0, "", ""))
                # Each edition counts in the directory its BV_HOST names.
                self.assertEqual(len(list(self.track_dir(edition).glob("cmd-*"))), 2)

    def test_an_edit_loop_detection_recorded_starts_the_count_over(self) -> None:
        for edition in EDITIONS:
            with self.subTest(edition=edition):
                hook, detector = self.write_pair(edition)
                for _ in range(11):
                    self.run_hook(hook, COMMAND, session_id="s1")
                time.sleep(0.05)  # file timestamps are coarser than a hook run
                recorded = self.run_payload(detector, edit_payload(session_id="s1"))
                self.assertEqual(self.outcome(recorded), (0, "", ""))
                self.assertEqual(self.outcome(self.run_hook(hook, COMMAND, session_id="s1")), (0, "", ""))

    def test_edit_counters_kept_under_another_name_restart_nothing(self) -> None:
        # Why bootstrap-verifier reads loop-detection.sh: its counts land
        # where the guard never looks, and the fix-and-rerun loop is refused.
        hook, detector = self.write_pair("claude")
        self.write_detector("claude", loop_detection("claude").replace(b"/edit-$SESSION_KEY-", b"/$SESSION_KEY-"))
        for _ in range(11):
            self.run_hook(hook, COMMAND)
        time.sleep(0.05)
        self.assertEqual(self.run_payload(detector, edit_payload()).returncode, 0)
        refused = self.run_hook(hook, COMMAND)
        self.assertEqual(refused.returncode, 2, refused.stderr)
        self.assertIn("has run 12 times", refused.stderr)

    def test_a_status_query_and_a_refused_command_are_not_counted(self) -> None:
        hook = self.write("claude", built())
        for command in (
            "git status",
            "gh pr checks 44",
            "sleep 30 && gh run view 123 --log-failed | tail -n 50",
            "kubectl get pods",
        ):
            with self.subTest(command=command):
                for _ in range(13):
                    self.assertEqual(self.outcome(self.run_hook(hook, command)), (0, "", ""))
        refused = [self.run_hook(hook, "git push --force origin main") for _ in range(13)]
        self.assertEqual({result.returncode for result in refused}, {2})
        self.assertFalse(any("has run" in result.stderr for result in refused))
        # A status query chained with an action is counted like any command.
        codes = [self.run_hook(hook, "gh pr checks 44 && composer test").returncode for _ in range(12)]
        self.assertEqual(codes[-1], 2)

    def test_sessions_keep_their_own_counts(self) -> None:
        hook = self.write("codex", built(host="codex"))
        for _ in range(5):
            self.assertEqual(self.run_hook(hook, COMMAND, session_id="session-a").returncode, 0)
        for _ in range(5):
            self.assertEqual(self.outcome(self.run_hook(hook, COMMAND, session_id="session-b")), (0, "", ""))
        sixth = self.run_hook(hook, COMMAND, session_id="session-a")
        self.assertIn("has run 6 times", self.warning("codex", sixth))

    def test_a_planted_directory_or_linked_counter_is_never_trusted(self) -> None:
        hook = self.write("claude", built())
        directory = self.track_dir("claude")
        planted = self.tmpdir / "planted"
        planted.mkdir()
        directory.symlink_to(planted)
        for _ in range(12):
            self.assertEqual(self.outcome(self.run_hook(hook, COMMAND)), (0, "", ""))
        self.assertEqual(list(planted.iterdir()), [])
        directory.unlink()

        victim = self.tmpdir / "victim.txt"
        victim.write_text("important\n", encoding="utf-8")
        self.assertEqual(self.run_hook(hook, COMMAND).returncode, 0)
        self.assertEqual(directory.stat().st_mode & 0o777, 0o700)
        (counter,) = directory.glob("cmd-*")
        counter.unlink()
        counter.symlink_to(victim)
        self.assertEqual(self.outcome(self.run_hook(hook, COMMAND)), (0, "", ""))
        self.assertEqual(victim.read_text(encoding="utf-8"), "important\n")
        counter.unlink()
        # A leading zero is not octal, and a count is never shell code.
        counter.write_text("08\n", encoding="utf-8")
        self.assertIn("has run 9 times", self.warning("claude", self.run_hook(hook, COMMAND)))
        marker = self.tmpdir / "evaluated"
        counter.write_text(f"a[$(touch {marker})]\n", encoding="utf-8")
        self.run_hook(hook, COMMAND)
        self.assertFalse(marker.exists())
        self.assertEqual(counter.read_text(encoding="utf-8"), "1\n")



if __name__ == "__main__":
    unittest.main()
