"""Composer commands: the native CLIs' own lists (fake CLIs, no model calls), routing and Codex prompt expansion."""
import json
import os
from pathlib import Path
import sys
import tempfile
import textwrap
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness" / "src"))
from harness import commands
from harness.sessions import SessionError

FAKE_CLAUDE = r'''#!/usr/bin/env python3
import json, os, sys, time
log = os.environ["FAKE_LOG"]
mode = os.environ.get("FAKE_MODE", "")
with open(log, "a") as handle:
    handle.write(json.dumps({"argv": sys.argv[1:], "cwd": os.getcwd()}) + "\n")
for line in sys.stdin:
    with open(log, "a") as handle:
        handle.write(json.dumps({"stdin": json.loads(line)}) + "\n")
    message = json.loads(line)
    if message.get("type") != "control_request":
        continue
    if mode == "hang":
        time.sleep(60)
    if mode == "garbage":
        print("not json", flush=True)
        sys.exit(3)
    if mode == "orphan":
        import subprocess
        child = subprocess.Popen(["sleep", "30"])
        with open(log, "a") as handle:
            handle.write(json.dumps({"orphan": child.pid}) + "\n")
    if mode == "error":
        print(json.dumps({"type": "control_response", "response": {"subtype": "error", "request_id": message["request_id"], "error": "no"}}), flush=True)
        continue
    commands = [
        {"name": "php-review", "description": "Review PHP changes. (project)", "argumentHint": "<path>"},
        {"name": "clear", "description": "Start a new session", "argumentHint": "[name]", "aliases": ["reset", "new", "fresh"], "builtin": True},
        {"name": "compact", "description": "Free up context", "argumentHint": "<optional custom summarization instructions>", "builtin": True},
        {"name": "model", "description": "Set the AI model", "argumentHint": "<model>", "builtin": True},
        {"name": "__remote-workflow", "description": "internal", "builtin": True},
        {"name": "bad name", "description": "spaces are not a command"},
        {"name": "unite-cms:init", "description": "Plugin command (plugin)", "argumentHint": ""},
    ] + ([{"name": os.environ["FAKE_EXTRA"], "description": "Installed a moment ago (project)"}] if os.environ.get("FAKE_EXTRA") else [])
    print(json.dumps({"type": "system", "subtype": "noise"}), flush=True)
    print(json.dumps({"type": "control_response", "response": {"subtype": "success", "request_id": message["request_id"],
                      "response": {"commands": commands, "models": []}}}), flush=True)
'''

FAKE_CODEX = r'''#!/usr/bin/env python3
import json, os, sys
log = os.environ["FAKE_LOG"]
assert sys.argv[1:] == ["app-server"], sys.argv
for line in sys.stdin:
    message = json.loads(line)
    with open(log, "a") as handle:
        handle.write(json.dumps({"stdin": message, "cwd": os.getcwd()}) + "\n")
    if message.get("method") == "initialize":
        print(json.dumps({"id": message["id"], "result": {"userAgent": "fake"}}), flush=True)
    elif message.get("method") == "skills/list":
        cwd = message["params"]["cwds"][0]
        skills = [
            {"name": "php-review", "description": "Review PHP.", "path": cwd + "/.agents/skills/php-review/SKILL.md", "scope": "repo", "enabled": True},
            {"name": "advisor:review-workflow", "description": "Plugin skill.", "path": "/home/x/.codex/plugins/a/SKILL.md", "scope": "user", "enabled": True},
            {"name": "disabled-one", "description": "Off.", "path": "/x/SKILL.md", "scope": "user", "enabled": False},
            {"name": "bad name", "description": "No.", "path": "/x/SKILL.md", "scope": "user"},
        ]
        print(json.dumps({"id": message["id"], "result": {"data": [{"cwd": cwd, "skills": skills, "errors": []}]}}), flush=True)
'''


@unittest.skipIf(os.name == "nt", "The fake CLIs are POSIX scripts")
class CommandCatalogTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "project"
        self.project.mkdir()
        self.log = self.root / "log.jsonl"
        for name, body in (("claude", FAKE_CLAUDE), ("codex", FAKE_CODEX)):
            script = self.root / name
            script.write_text(body)
            script.chmod(0o755)
        environment = patch.dict(os.environ, {"FAKE_LOG": str(self.log), "CODEX_HOME": str(self.root / "codex-home")})
        environment.start()
        self.addCleanup(environment.stop)

        class Store:
            providers = {"claude": {"available": True, "executable": str(self.root / "claude")},
                         "codex": {"available": True, "executable": str(self.root / "codex")},
                         "cursor": {"available": False, "executable": None}}
            runner_lock = None
        self.store = Store()
        self.catalog = commands.Catalog(self.store)

    def entries(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def session(self, provider="claude", **changes):
        return {"provider": provider, "project_path": str(self.project), "agents_enabled": False, "agent_count": 3,
                "thinking_effort": None, **changes}

    def test_claude_lists_its_own_commands_from_one_initialize_request_without_a_turn(self):
        listing = self.catalog.listing("claude", self.project)
        self.assertIsNone(listing["error"])
        self.assertEqual([(item["name"], item["kind"], item["action"]) for item in listing["commands"]], [
            ("php-review", "native", None), ("clear", "page", "new"), ("compact", "native", None),
            ("model", "page", "model"), ("unite-cms:init", "native", None)])
        clear = listing["commands"][1]
        self.assertEqual((clear["aliases"], clear["hint"], clear["builtin"]), (["reset", "new", "fresh"], "[name]", True))
        launch, request = self.entries()
        # The probe asks print mode with stream-json input and the launch settings, in the workspace, and sends no
        # message. It runs no hooks: a SessionStart hook would act on the checkout and the sessions running in it.
        self.assertEqual(launch["cwd"], str(self.project))
        self.assertEqual(launch["argv"][:7], ["--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--settings"])
        self.assertEqual(json.loads(launch["argv"][7]), {**json.loads(commands.claude_settings()), "disableAllHooks": True})
        self.assertTrue(json.loads(launch["argv"][7])["disableWorkflows"])
        self.assertEqual(request["stdin"], {"type": "control_request", "request_id": "harness-commands", "request": {"subtype": "initialize"}})
        # The list is reused for a minute: a second listing starts no CLI.
        self.catalog.listing("claude", self.project)
        self.assertEqual(len(self.entries()), 2)
        self.catalog.listing("claude", self.project, commands.claude_settings(True, 4, "ultracode"))
        self.assertEqual(len(self.entries()), 4)
        # Installing or changing skills forgets the lists.
        self.catalog.clear()
        self.catalog.listing("claude", self.project)
        self.assertEqual(len(self.entries()), 6)

    def test_what_a_probe_leaves_running_is_stopped_with_it(self):
        with patch.dict(os.environ, {"FAKE_MODE": "orphan"}):
            self.assertIsNone(self.catalog.listing("claude", self.project)["error"])
        orphan = next(entry["orphan"] for entry in self.entries() if "orphan" in entry)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                os.kill(orphan, 0)
            except ProcessLookupError:
                break
            time.sleep(.05)
        with self.assertRaises(ProcessLookupError):
            os.kill(orphan, 0)

    def test_a_cli_that_fails_hangs_or_is_missing_gives_an_error_not_a_list(self):
        for mode, message in (("error", "did not list"), ("garbage", "exited without answering")):
            with self.subTest(mode=mode), patch.dict(os.environ, {"FAKE_MODE": mode}):
                catalog = commands.Catalog(self.store)
                listing = catalog.listing("claude", self.project)
                self.assertEqual(listing["commands"], [])
                self.assertIn(message, listing["error"])
        with patch.dict(os.environ, {"FAKE_MODE": "hang"}), patch.object(commands, "PROBE_SECONDS", 1):
            started = time.monotonic()
            listing = commands.Catalog(self.store).listing("claude", self.project)
            self.assertIn("did not answer in time", listing["error"])
            self.assertLess(time.monotonic() - started, 10)
        self.store.providers["claude"] = {"available": False, "executable": None}
        self.assertIn("unavailable", commands.Catalog(self.store).listing("claude", self.project)["error"])

    def test_claude_routes_its_commands_natively_and_refuses_a_restart(self):
        session = self.session()
        for message in ("/php-review src/Cart.php", "/compact", "/unite-cms:init"):
            with self.subTest(message=message):
                route = self.catalog.route(session, message)
                self.assertEqual((route["mode"], route["requests"]), ("native", []))
                self.assertIn("as typed", route["notice"])
        for message in ("/php-review, focus", "/nope x", "/bad name"):
            with self.subTest(message=message):
                route = self.catalog.route(session, message)
                self.assertEqual((route["mode"], route["text"]), ("text", message))
                self.assertIn("is not a Claude Code command here", route["notice"])
        self.assertEqual(self.catalog.route(session, "plain words"), {"mode": "text", "text": "plain words", "requests": [], "notice": None})
        # The page carries these out: a restart (also under an alias only the CLI's list names), the model, the effort.
        for message in ("/clear", "/reset now", "/new", "/fresh"):
            with self.subTest(message=message), self.assertRaisesRegex(SessionError, "New session"):
                self.catalog.route(session, message)
        for message in ("/model sonnet", "/model picker is broken, fix it", "/effort high"):
            with self.subTest(message=message), self.assertRaisesRegex(SessionError, "session settings"):
                self.catalog.route(session, message)
        # A skill installed after the list was read is found by a fresh list, not sent as text.
        with patch.dict(os.environ, {"FAKE_EXTRA": "just-installed"}), patch.object(commands, "RECHECK_SECONDS", 0):
            self.assertEqual(self.catalog.route(session, "/just-installed now")["mode"], "native")
        with patch.dict(os.environ, {"FAKE_MODE": "error"}):
            route = commands.Catalog(self.store).route(session, "/php-review x")
        self.assertEqual(route["mode"], "text")
        self.assertIn("could not list its commands", route["notice"])

    def test_codex_skills_come_from_its_app_server_and_a_mention_anywhere_requests_them(self):
        listing = self.catalog.listing("codex", self.project)
        self.assertEqual([item["name"] for item in listing["skills"]], ["php-review", "advisor:review-workflow"])
        self.assertEqual([(item["name"], item["kind"]) for item in listing["commands"]],
                         [("new", "page"), ("model", "page"), ("diff", "page"), ("status", "page"), ("init", "prompt")])
        exchange = [entry["stdin"] for entry in self.entries()]
        self.assertEqual([message.get("method") for message in exchange], ["initialize", "initialized", "skills/list"])
        self.assertEqual(exchange[2]["params"], {"cwds": [str(self.project)]})
        route = self.catalog.route(self.session("codex"), "Check $php-review, then $advisor:review-workflow. Not US$5 or $nope")
        self.assertEqual([item["name"] for item in route["requests"]], ["php-review", "advisor:review-workflow"])
        self.assertEqual(route["text"], "Check $php-review, then $advisor:review-workflow. Not US$5 or $nope")
        self.assertEqual(route["notice"], "Codex skills requested: php-review, advisor:review-workflow.")
        self.assertIn(f"- php-review ({self.project}/.agents/skills/php-review/SKILL.md): read that file and follow it.",
                      commands.request(route["requests"]))
        self.assertEqual(self.catalog.route(self.session("codex"), "no skills here")["requests"], [])
        # PHP and code in a message are not skill mentions, but a sentence may go on right after a name.
        code = 'Fix it: $php-review = new Review(); if ($php-review) { echo "$php-review"; } `$php-review` $php-review->run() $php-review[0]'
        self.assertEqual(self.catalog.route(self.session("codex"), code)["requests"], [])
        self.assertEqual([item["name"] for item in self.catalog.route(self.session("codex"), "Use $php-review: check rounding")["requests"]], ["php-review"])
        self.assertEqual(self.catalog.route(self.session("codex"), "```\n$php-review\n```")["requests"], [])

    def test_codex_prompts_and_init_expand_as_the_codex_app_would(self):
        prompts = self.root / "codex-home" / "prompts"
        prompts.mkdir(parents=True)
        (prompts / "draftpr.md").write_text("---\ndescription: Draft a PR\nargument-hint: FILES=<paths> PR_TITLE=<title>\n---\n"
                                            "Open a PR titled $PR_TITLE for $FILES. Notes: $1. Cost $$5.\n")
        (prompts / "plain.md").write_text("Summarise $ARGUMENTS in one line.\n")
        (prompts / "nested").mkdir()
        (prompts / "nested" / "hidden.md").write_text("Not top level.\n")
        names = [item["name"] for item in self.catalog.listing("codex", self.project)["commands"]]
        self.assertEqual(names[-2:], ["prompts:draftpr", "prompts:plain"])
        route = self.catalog.route(self.session("codex"), '/prompts:draftpr FILES="a.php b.php" PR_TITLE=Fix urgent')
        self.assertEqual(route["text"], "Open a PR titled Fix for a.php b.php. Notes: urgent. Cost $5.")
        self.assertIn("expanded", route["notice"])
        self.assertEqual(self.catalog.route(self.session("codex"), "/prompts:plain the log file")["text"], "Summarise the log file in one line.")
        with self.assertRaisesRegex(SessionError, "PR_TITLE=…"):
            commands.Catalog.check("codex", "/prompts:draftpr FILES=a")
        with self.assertRaisesRegex(SessionError, "no Codex prompt"):
            commands.Catalog.check("codex", "/prompts:missing")
        with self.assertRaisesRegex(SessionError, "quotes"):
            commands.Catalog.check("codex", '/prompts:plain "open')
        route = self.catalog.route(self.session("codex"), "/init focus on the PHP tests")
        self.assertTrue(route["text"].startswith(commands.INIT_PROMPT))
        self.assertTrue(route["text"].endswith("Also: focus on the PHP tests"))

    def test_cursor_takes_a_leading_skill_from_its_folder(self):
        folder = self.project / ".cursor/skills/php-review"
        folder.mkdir(parents=True)
        (folder / "SKILL.md").write_text("---\nname: php-review\ndescription: Review.\nargument-hint: <path>\n---\n# Review\n")
        listing = self.catalog.listing("cursor", self.project)
        self.assertEqual([(item["name"], item["kind"], item["hint"]) for item in listing["commands"]], [("php-review", "skill", "<path>")])
        route = self.catalog.route(self.session("cursor"), "/php-review src")
        self.assertEqual(([item["name"] for item in route["requests"]], route["mode"]), (["php-review"], "text"))
        self.assertEqual(self.catalog.route(self.session("cursor"), "Use /php-review later")["requests"], [])

    def test_frontmatter_reads_names_folded_descriptions_hints_and_flags(self):
        meta = commands.frontmatter("---\nname: php-review\ndescription: >-\n  Reviews PHP code\n  for defects.\n"
                                    "argument-hint: \"<path>\"\ndisable-model-invocation: true\n---\n# Body\nText\n")
        self.assertEqual({key: meta[key] for key in ("name", "description", "hint", "model", "user")},
                         {"name": "php-review", "description": "Reviews PHP code for defects.", "hint": "<path>", "model": False, "user": True})
        self.assertEqual(meta["body"], "# Body\nText\n")
        self.assertEqual(commands.frontmatter("# No frontmatter\ndescription: not metadata\n")["description"], "")
        long = commands.frontmatter("---\ndescription: " + "word " * 200 + "\n---\n")["description"]
        self.assertEqual((len(long), long[-1]), (commands.DESCRIPTION_CHARS, "…"))


if __name__ == "__main__":
    unittest.main()
