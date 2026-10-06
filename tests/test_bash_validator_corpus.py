#!/usr/bin/env python3
"""Behavioural corpus for every shipped bash-validator.sh.

tests/fixtures/bash-validator-corpus.json lists shell commands an agent
actually emits, each expected to be blocked or allowed. This suite runs every
case through every shipped copy of the hook - the .claude, .cursor and .codex
copy of the Laravel, Symfony, PHP Core and WordPress editions and of
Infrastructure-Creator - exactly as its host runs it: `bash <hook>` with that
host's JSON payload on stdin (Claude Code PreToolUse, Cursor
beforeShellExecution, Codex PreToolUse).

A case's expectation holds in the editions it lists ("all" = every one); in
the other editions the opposite holds, so framework rules are proven not to
leak into editions they do not belong to.

The hooks keep one generic section (parser, normaliser, git/rm/gh/composer/
SQL/.env rules) byte-identical between explicit markers; that identity is
asserted here too, and so is that parsing time grows linearly with the
command (a long command must not push the hook past its timeout).

Run from the repository root: python3 -m unittest tests.test_bash_validator_corpus
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import time
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CORPUS = REPO_ROOT / "tests" / "fixtures" / "bash-validator-corpus.json"
BASH = shutil.which("bash") or "/bin/bash"
HOOK_TIMEOUT = 30

EDITIONS = {
    "laravel": "Laravel",
    "symfony": "Symfony",
    "php-core": "PHP Core",
    "wordpress": "Cms/wordpress",
    "infrastructure-creator": "Infrastructure-Creator",
}
HOST_DIRS = (".claude", ".cursor", ".codex")

GENERIC_START = "# >>> bash-validator generic section >>>"
GENERIC_END = "# <<< bash-validator generic section <<<"

# The only stderr a block may produce: static text plus the rule category.
BLOCK_LINES = {
    "edition": re.compile(
        r"\ABLOCKED: Command refused \(rule category: (?P<category>.+)\)\.\n"
        r"   This operation is blocked\. See AGENTS\.md\.\n\Z"
    ),
    "creator": re.compile(
        r"\A\[bash-validator\] BLOCKED: command refused \(rule category: (?P<category>.+)\)\. "
        r"Run it manually, with intent, if it is really needed\.\n\Z"
    ),
}


def claude_payload(command: str, cwd: Path) -> dict:
    """Claude Code PreToolUse for the Bash tool."""
    return {
        "session_id": "corpus-session",
        "transcript_path": "/tmp/corpus-transcript.jsonl",
        "cwd": str(cwd),
        "permission_mode": "default",
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command, "description": "corpus case"},
        "tool_use_id": "toolu_corpus",
    }


def cursor_payload(command: str, cwd: Path) -> dict:
    """Cursor beforeShellExecution: the command sits at the top level."""
    return {
        "conversation_id": "corpus-conversation",
        "generation_id": "corpus-generation",
        "model": "corpus-model",
        "hook_event_name": "beforeShellExecution",
        "cursor_version": "1.7.0",
        "workspace_roots": [str(cwd)],
        "user_email": None,
        "transcript_path": None,
        "command": command,
        "cwd": str(cwd),
        "sandbox": False,
    }


def codex_payload(command: str, cwd: Path) -> dict:
    """Codex PreToolUse: Bash carries tool_input.command."""
    return {
        "session_id": "corpus-session",
        "transcript_path": None,
        "cwd": str(cwd),
        "hook_event_name": "PreToolUse",
        "model": "corpus-model",
        "permission_mode": "default",
        "turn_id": "corpus-turn",
        "tool_name": "Bash",
        "tool_use_id": "call_corpus",
        "tool_input": {"command": command},
    }


PAYLOADS = {".claude": claude_payload, ".cursor": cursor_payload, ".codex": codex_payload}


def load_corpus() -> dict:
    return json.loads(CORPUS.read_text(encoding="utf-8"))


def shipped_copies() -> list[tuple[str, str, Path]]:
    """(edition id, host directory, hook path) for every shipped copy."""
    return [
        (edition, host, REPO_ROOT / folder / host / "hooks" / "bash-validator.sh")
        for edition, folder in EDITIONS.items()
        for host in HOST_DIRS
    ]


def expected(case: dict, edition: str) -> str:
    listed = case["editions"] == "all" or edition in case["editions"]
    if listed:
        return case["expect"]
    return "pass" if case["expect"] == "block" else "block"


def run_hook(hook: Path, host: str, command: str) -> subprocess.CompletedProcess:
    root = hook.parents[2]
    payload = PAYLOADS[host](command, root)
    return subprocess.run(
        [BASH, str(hook)],
        input=json.dumps(payload),
        capture_output=True,
        text=True,
        cwd=str(root),
        timeout=HOOK_TIMEOUT,
    )


def generic_section(text: str) -> str:
    start = text.index(GENERIC_START)
    end = text.index(GENERIC_END, start) + len(GENERIC_END)
    return text[start:end]


class CorpusShapeTest(unittest.TestCase):
    def test_corpus_is_well_formed(self) -> None:
        corpus = load_corpus()
        cases = corpus["cases"]
        self.assertGreaterEqual(len(cases), 80)
        self.assertEqual(sorted(corpus["editions"]), sorted(EDITIONS))
        self.assertIn("--force-with-lease", corpus["decisions"])
        seen = set()
        for index, case in enumerate(cases):
            with self.subTest(index=index, command=case.get("command")):
                self.assertTrue(
                    set(case) <= {"command", "expect", "editions", "note", "category"}
                )
                self.assertIsInstance(case["command"], str)
                self.assertTrue(case["command"].strip())
                self.assertIn(case["expect"], ("block", "pass"))
                self.assertTrue(case["note"].strip())
                if case["editions"] != "all":
                    self.assertTrue(case["editions"])
                    self.assertTrue(set(case["editions"]) <= set(EDITIONS))
                if "category" in case:
                    self.assertEqual(case["expect"], "block")
                self.assertNotIn(case["command"], seen, "duplicate case")
                seen.add(case["command"])

    def test_every_edition_has_block_and_pass_cases(self) -> None:
        cases = load_corpus()["cases"]
        for edition in EDITIONS:
            outcomes = {expected(case, edition) for case in cases}
            with self.subTest(edition=edition):
                self.assertEqual(outcomes, {"block", "pass"})


class CorpusBehaviourTest(unittest.TestCase):
    """Every case through every shipped copy, failures reported together."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.cases = load_corpus()["cases"]
        cls.copies = shipped_copies()
        cls.sources = {hook: hook.read_text(encoding="utf-8") for _, _, hook in cls.copies}
        jobs = [
            (index, edition, host, hook)
            for index in range(len(cls.cases))
            for edition, host, hook in cls.copies
        ]
        workers = min(32, (os.cpu_count() or 2) * 2)
        with ThreadPoolExecutor(max_workers=workers) as pool:
            results = pool.map(
                lambda job: run_hook(job[3], job[2], cls.cases[job[0]]["command"]), jobs
            )
            cls.results = list(zip(jobs, results))

    def test_cases_against_every_copy(self) -> None:
        for (index, edition, host, hook), result in self.results:
            case = self.cases[index]
            want = expected(case, edition)
            label = "{}/{}".format(EDITIONS[edition], host)
            with self.subTest(copy=label, command=case["command"], expect=want):
                self.assertEqual(result.stdout, "")
                if want == "pass":
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stderr, "")
                    continue
                self.assertEqual(result.returncode, 2, "not blocked: " + case["note"])
                shape = "creator" if edition == "infrastructure-creator" else "edition"
                match = BLOCK_LINES[shape].match(result.stderr)
                self.assertIsNotNone(match, result.stderr)
                category = match.group("category")
                # Only static rule text is printed, never the command body.
                self.assertIn(category, self.sources[hook])
                if "category" in case:
                    self.assertTrue(
                        category.startswith(case["category"]),
                        "{!r} is not a {!r} rule".format(category, case["category"]),
                    )


class ParseCostTest(unittest.TestCase):
    """The parser's time grows linearly with the command, so a long command
    cannot push the hook past its 5 s timeout, which fails open. Measured
    as a ratio (4x the input must cost well under 16x the time) so that a
    slow machine does not fail it; the generic section is identical in
    every copy, so one copy stands for all."""

    HOOK = REPO_ROOT / "Laravel" / ".claude" / "hooks" / "bash-validator.sh"

    @staticmethod
    def dense(kind: str, n: int) -> str:
        return {
            "expansions in double quotes": 'python3 -c "' + "$a $b ${c} " * n + '"',
            "ANSI-C quoted string": "printf $'" + "line\\n" * (n * 2) + "'",
            "long word list": "ls " + "file.txt " * n,
        }[kind]

    def best_time(self, command: str) -> float:
        times = []
        for _ in range(2):
            started = time.perf_counter()
            result = run_hook(self.HOOK, ".claude", command)
            times.append(time.perf_counter() - started)
            self.assertEqual(result.returncode, 0, result.stderr)
        return min(times)

    def test_parse_time_grows_linearly(self) -> None:
        for kind in ("expansions in double quotes", "ANSI-C quoted string", "long word list"):
            with self.subTest(kind=kind):
                small = self.best_time(self.dense(kind, 2000))
                large = self.best_time(self.dense(kind, 8000))
                self.assertLess(large / small, 6.0, "4x the input took %.1fx the time" % (large / small))

    def test_large_heredoc_write_passes_quickly(self) -> None:
        body = "\n".join(
            "    public function test%d(): void { $this->assertSame(%d, f(%d)); } // \"x\" 'y'" % (i, i, i)
            for i in range(1500)
        )
        command = "cat > tests/Feature/BigTest.php <<'PHP'\n<?php\nclass BigTest {\n" + body + "\n}\nPHP"
        self.assertGreater(len(command), 100_000)
        self.assertLess(self.best_time(command), 2.5)


class GenericSectionTest(unittest.TestCase):
    def test_generic_section_is_identical_in_every_copy(self) -> None:
        sections = {}
        for edition, host, hook in shipped_copies():
            text = hook.read_text(encoding="utf-8")
            with self.subTest(hook=str(hook.relative_to(REPO_ROOT))):
                self.assertEqual(text.count(GENERIC_START), 1)
                self.assertEqual(text.count(GENERIC_END), 1)
                sections[(edition, host)] = generic_section(text)
        reference = sections[("php-core", ".claude")]
        for key, section in sections.items():
            with self.subTest(copy=key):
                self.assertEqual(section, reference)

    def test_framework_rules_live_below_the_generic_section(self) -> None:
        for edition, host, hook in shipped_copies():
            text = hook.read_text(encoding="utf-8")
            section = generic_section(text)
            tail = text[text.index(GENERIC_END) :]
            with self.subTest(hook=str(hook.relative_to(REPO_ROOT))):
                self.assertNotIn("BV_FRAMEWORK_RULES=(", section)
                self.assertIn("BV_FRAMEWORK_RULES=(", tail)
                self.assertIn("BV_CONSOLE_ENTRIES=", tail)


if __name__ == "__main__":
    unittest.main()
