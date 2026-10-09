"""Usage › Context: per-launch integers for what fills the context window, and how much of it is memory."""

from contextlib import closing
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import context_usage, providers, sessions
from harness.context_usage import CodexFill, ContextTracker, capsule_text_parts

# A runtime from before excerpts printed each item as '  path — title' under its layer.
HOOK_TEXT = ("working: T-1 — Retry webhooks\nLast turn: tests passed\nsemantic:\n"
             "  memory-bank/chunks/MEM-0001-retry.md — Retry with backoff\n  specs/payments.md — Payments\n"
             "  project-brain/dynamic/findings/f.md — Finding\nepisodic:\n  episode 12 — Earlier fix\n")
EDITION_SCRIPTS = Path(__file__).resolve().parents[1] / "PHP Core/memory-bank/scripts"
RENDER = '''
import json, sys
import context
data = json.load(sys.stdin)
excerpts = {}
for path, (text, heading) in data["excerpts"].items():
    excerpts[path] = context.Excerpt(text)
    excerpts[path].heading = heading
print(json.dumps(context.render_capsule_lines(data["capsule"], excerpts)))
'''


def rendered(capsule, excerpts):
    """The capsule as the edition's own runtime renders it for its hooks and the Harness (render_capsule_lines)."""
    result = subprocess.run([sys.executable, "-B", "-c", RENDER], cwd=EDITION_SCRIPTS, capture_output=True, text=True,
                            input=json.dumps({"capsule": capsule, "excerpts": excerpts}), check=True, timeout=60)
    return "\n".join(json.loads(result.stdout))


def item(path, category, layer, title, **extra):
    return {"path": path, "layer": layer, "kind": "spec", "title": title, "snippet": "", "category": category,
            "estimated_tokens": 40, "record_id": None, "conflicts": [], **extra}


# Every kind of item the renderer prints, with titles, marks and excerpts that carry ' — ' themselves.
CHUNK = item("memory-bank/chunks/MEM-20260101-aaaaaaaa-retry.md", "durable", "semantic", "Retry with backoff — always")
SPEC = item("specs/payments.md", "evidence", "semantic", "Payments", attestation="agent")
FINDING = item("project-brain/dynamic/findings/f.md", "dynamic", "semantic", "Retries double-charge",
               source_changed=["app/Retry.php"])
SKILL = item(".agents/skills/payments/SKILL.md", "policy", "procedural", "Payments skill", kind="skill")
RENDERED_CAPSULE = {
    "query": "retry webhooks", "task_id": "T-1", "warnings": ["slow refresh"], "last_turn": "tests passed",
    "task_record": "project-brain/dynamic/tasks/t-1.md",
    "working": {"task_id": "T-1", "goal": "Retry webhooks", "progress": "Tests pass", "next_steps": ["Ship"],
                "files": ["app/Retry.php"]},
    "procedural": [SKILL], "semantic": [CHUNK, SPEC, FINDING],
    "episodic": [{"id": 12, "summary": "Earlier fix — rolled back"}],
    "selected": [SKILL, CHUNK, SPEC, FINDING], "categories": {"policy": [SKILL], "durable": [CHUNK], "evidence": [SPEC],
                                                             "dynamic": [FINDING]}}
RENDERED_EXCERPTS = {CHUNK["path"]: ["Retry three times — then give up.", "Retry policy — limits"],
                     SPEC["path"]: ["Payments retry on 5xx responses.", ""],
                     FINDING["path"]: ["Two retries charged the card twice.", ""],
                     "episode 12": ["We rolled back the — change.", ""]}


def assistant(identity, read, created=0, parent=None, **extra):
    return {"type": "assistant", "parent_tool_use_id": parent, "session_id": "native-context",
            "message": {"id": identity, "model": "fixture-model", "role": "assistant", "content": [{"type": "text", "text": "Working"}],
                        "usage": {"input_tokens": 10, "cache_read_input_tokens": read, "cache_creation_input_tokens": created, "output_tokens": 40}},
            **extra}


CLAUDE_EVENTS = [
    {"type": "system", "subtype": "init", "session_id": "native-context", "model": "fixture-model"},
    {"type": "system", "subtype": "hook_started", "hook_event": "UserPromptSubmit", "hook_id": "h1", "hook_name": "context"},
    {"type": "system", "subtype": "hook_response", "hook_event": "UserPromptSubmit", "hook_id": "h1", "hook_name": "context",
     "stdout": HOOK_TEXT, "stderr": "", "outcome": "success"},
    assistant("m1", 30000, 1990),
    # One message streams one event per content block; it is one model call.
    assistant("m1", 30000, 1990),
    # A subagent's call fills its own context, not the main one.
    assistant("sub-1", 120000, parent="toolu_1"),
    assistant("m2", 40990),
    {"type": "system", "subtype": "compact_boundary", "compact_metadata": {"trigger": "auto", "pre_tokens": 150000, "post_tokens": 30000}},
    assistant("m3", 30990),
    assistant("meta", 99000, is_meta=True),
    {"type": "result", "subtype": "success", "is_error": False, "session_id": "native-context", "result": "Done",
     "usage": {"input_tokens": 30, "cache_read_input_tokens": 90000, "cache_creation_input_tokens": 9970, "output_tokens": 500},
     "modelUsage": {"fixture-model": {"contextWindow": 200000}, "fixture-small": {"contextWindow": 100000}}},
]

FAKE_CLI = r'''
import datetime, json, os, pathlib, sqlite3, sys
config = json.loads(sys.argv[1])
sys.stdin.read()
if config["provider"] == "claude":
    events = json.loads(pathlib.Path(config["events"]).read_text()) if config["behavior"] == "events" else [
        {"type": "system", "subtype": "init", "session_id": "native-context"},
        {"type": "result", "subtype": "success", "is_error": False, "session_id": "native-context", "result": "Done"}]
    for event in events:
        print(json.dumps(event), flush=True)
    sys.exit(0)
home = pathlib.Path(os.environ["CODEX_HOME"]); folder = home / "sessions"; folder.mkdir(parents=True, exist_ok=True)
rollout = folder / "rollout-native-context.jsonl"
now = datetime.datetime.now(datetime.timezone.utc)
if config["behavior"] == "slow-codex":
    import time
    rollout.write_text(json.dumps({"type": "session_meta", "payload": {"id": "native-context", "cwd": str(pathlib.Path.cwd()), "source": "exec"}}) + "\n")
    with sqlite3.connect(home / "state_5.sqlite") as db:
        db.execute("CREATE TABLE IF NOT EXISTS threads (id TEXT PRIMARY KEY, rollout_path TEXT, cwd TEXT)")
        db.execute("INSERT OR REPLACE INTO threads VALUES (?,?,?)", ("native-context", str(rollout), str(pathlib.Path.cwd())))
    print(json.dumps({"type": "thread.started", "thread_id": "native-context"}), flush=True)
    for step in range(12):
        with rollout.open("a") as handle:
            handle.write(json.dumps({"type": "event_msg", "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(), "payload": {"type": "token_count",
                "info": {"model_context_window": 258400, "last_token_usage": {"input_tokens": 30000 + step * 1000}}}}) + "\n")
        time.sleep(.25)
    print(json.dumps({"type": "turn.completed", "usage": {"input_tokens": 100, "cached_input_tokens": 50}}), flush=True)
    sys.exit(0)
print(json.dumps({"type": "thread.started", "thread_id": "native-context"}), flush=True)
def at(seconds): return (now + datetime.timedelta(seconds=seconds)).isoformat()
def count(fill): return {"type": "event_msg", "timestamp": at(0), "payload": {"type": "token_count", "info": {
    "model_context_window": 258400, "last_token_usage": {"input_tokens": fill, "cached_input_tokens": fill // 2, "output_tokens": 9}}}}
records = [{"type": "session_meta", "payload": {"id": "native-context", "cwd": str(pathlib.Path.cwd()), "source": "exec"}},
    {"type": "event_msg", "timestamp": at(-3600), "payload": {"type": "token_count", "info": {"model_context_window": 258400,
        "last_token_usage": {"input_tokens": 999999}}}},
    count(38616), count(52000), {"type": "compacted", "timestamp": at(0), "payload": {"message": ""}}, count(21000), count(26000),
    {"type": "event_msg", "timestamp": at(0), "payload": {"type": "token_count", "info": None, "rate_limits": {}}}]
if config["behavior"] == "long-codex":
    # More tool output than the 4 MiB the Harness reads from the rollout's end, then the launch's last call.
    records += [{"type": "response_item", "timestamp": at(0), "payload": {"type": "function_call_output", "output": "x" * 4096}}] * 1100
    records.append(count(30000))
rollout.write_text("".join(json.dumps(record) + "\n" for record in records))
with sqlite3.connect(home / "state_5.sqlite") as db:
    db.execute("CREATE TABLE IF NOT EXISTS threads (id TEXT PRIMARY KEY, rollout_path TEXT, cwd TEXT)")
    db.execute("INSERT OR REPLACE INTO threads VALUES (?,?,?)", ("native-context", str(rollout), str(pathlib.Path.cwd())))
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": "Done"}}), flush=True)
print(json.dumps({"type": "turn.completed", "usage": {"input_tokens": 50000, "cached_input_tokens": 40000, "output_tokens": 100}}), flush=True)
'''


def strings(value, path=()):
    """Every string inside a JSON value, with where it sits."""
    if isinstance(value, str):
        yield path, value
    elif isinstance(value, dict):
        for key, item in value.items():
            yield from strings(item, path + (key,))
    elif isinstance(value, list):
        for item in value:
            yield from strings(item, path)


class ContextTrackerTests(unittest.TestCase):
    def test_claude_fill_counts_each_main_call_once_and_keeps_compaction_window_and_cache(self):
        tracker = ContextTracker("claude")
        changed = [tracker.observe(event) for event in CLAUDE_EVENTS]
        # The second block of m1, the subagent call and the meta message change nothing.
        self.assertEqual([False, False, True, True, False, False, True, True, True, False, True], changed)
        snapshot = tracker.snapshot()
        self.assertEqual({"start": 32000, "end": 31000, "peak": 150000, "calls": 3, "window": 200000,
                          "compactions": [{"pre": 150000, "post": 30000, "call": 2}], "cache_share": .9},
                         {key: snapshot[key] for key in ("start", "end", "peak", "calls", "window", "compactions", "cache_share")})
        self.assertEqual(capsule_text_parts(HOOK_TEXT), snapshot["hooks"])

    def test_nothing_reported_stays_unknown_not_zero(self):
        tracker = ContextTracker("claude")
        for event in ({"type": "assistant", "message": {"id": "m1", "content": []}, "parent_tool_use_id": None},
                      {"type": "assistant", "message": {"id": "m2", "usage": {"input_tokens": True}}, "parent_tool_use_id": None},
                      {"type": "assistant", "message": {"id": "m3", "usage": {"input_tokens": 5, "cache_read_input_tokens": -1}}, "parent_tool_use_id": None},
                      {"type": "result", "subtype": "success", "usage": {"input_tokens": 0}}, "not an event"):
            tracker.observe(event)
        self.assertEqual({"start": None, "end": None, "peak": None, "calls": None, "window": None, "compactions": [],
                          "cache_share": None, "hooks": None}, tracker.snapshot())
        codex = ContextTracker("codex")
        codex.observe({"type": "turn.completed", "usage": {"input_tokens": 0, "cached_input_tokens": 0}})
        self.assertIsNone(codex.snapshot()["cache_share"])
        codex.observe({"type": "turn.completed", "usage": {"input_tokens": 50000, "cached_input_tokens": 40000}})
        self.assertEqual(.8, codex.snapshot()["cache_share"])

    def test_hook_lines_split_by_memory_kind_and_add_up(self):
        parts = capsule_text_parts(HOOK_TEXT)
        self.assertEqual(len(HOOK_TEXT), sum(parts.values()))
        self.assertEqual(len("  memory-bank/chunks/MEM-0001-retry.md — Retry with backoff\n"), parts["bank"])
        self.assertEqual(len("  specs/payments.md — Payments\n"), parts["rules"])

    def test_the_rendered_capsule_splits_by_its_items_and_the_excerpts_under_them(self):
        # What the runtime renders today: `- memory <path>[ § section] — title`, the excerpt indented under it.
        text = rendered(RENDERED_CAPSULE, RENDERED_EXCERPTS) + "\n"
        lines = text.splitlines(keepends=True)

        def block(prefix):
            index = next(position for position, line in enumerate(lines) if line.startswith(prefix))
            following = lines[index + 1] if index + 1 < len(lines) else ""
            return lines[index] + (following if following.startswith("  ") else "")

        bank = block("- memory " + CHUNK["path"])
        rules = block("- memory " + SPEC["path"]) + block("- skill " + SKILL["path"])
        # The chunk's line names the section its excerpt quotes; its excerpt and the episode's carry ' — '.
        self.assertIn(" § Retry policy - limits — Retry with backoff — always\n  Retry three times — then give up.\n", bank)
        self.assertIn("- history episode 12 — Earlier fix — rolled back\n  We rolled back the — change.\n", text)
        parts = capsule_text_parts(text)
        self.assertEqual({"bank": len(bank), "rules": len(rules), "brain": len(text) - len(bank) - len(rules)}, parts)
        # A Claude hook prints the same render; its memory is split the same way, not all charged to Project Brain.
        tracker = ContextTracker("claude")
        self.assertTrue(tracker.observe({"type": "system", "subtype": "hook_response", "hook_event": "UserPromptSubmit",
                                         "stdout": text, "outcome": "success"}))
        self.assertEqual(parts, tracker.snapshot()["hooks"])

    def test_codex_rollout_keeps_the_launch_window_and_reads_compaction(self):
        now = datetime.now(timezone.utc)
        line = lambda record, seconds=0: json.dumps({"timestamp": (now + timedelta(seconds=seconds)).isoformat(), **record})
        count = lambda fill, seconds=0: line({"type": "event_msg", "payload": {"type": "token_count", "info": {
            "model_context_window": 258400, "last_token_usage": {"input_tokens": fill}}}}, seconds)
        fill = CodexFill(now.timestamp() - 60, now.timestamp() + 60)
        for record in [count(999999, -7200), count(38616), count(52000), line({"type": "compacted", "payload": {}}),
                       count(21000), "not json", count(26000), count(777777, 7200)]:
            fill.feed(record)
        self.assertEqual({"start": 38616, "end": 26000, "peak": 52000, "calls": 4, "window": 258400,
                          "compactions": [{"pre": 52000, "post": 21000, "call": 2}]}, fill.snapshot())
        self.assertIsNone(CodexFill(0, 1).snapshot())

    def test_live_writes_wait_two_seconds_unless_the_fill_moved_half_a_percent(self):
        tracker = ContextTracker("claude")
        with patch.object(context_usage.time, "monotonic", side_effect=[100.0, 100.5, 100.6, 103.0]):
            tracker.observe(CLAUDE_EVENTS[3]); self.assertTrue(tracker.due())
            tracker.window = 200000
            # 31,610 is within half a percent of a 200,000 window of the 32,000 last written.
            tracker.observe(assistant("m-small", 31600)); self.assertFalse(tracker.due())
            tracker.observe(assistant("m-big", 40000)); self.assertTrue(tracker.due())
            self.assertTrue(tracker.due())


    def test_only_session_launches_of_claude_stream_hook_events(self):
        with tempfile.TemporaryDirectory() as project:
            self.assertIn("--include-hook-events", providers.build_command("claude", "/usr/bin/true", project, "Inspect", hook_events=True))
            self.assertNotIn("--include-hook-events", providers.build_command("claude", "/usr/bin/true", project, "Inspect"))
            self.assertNotIn("--include-hook-events", providers.build_command("codex", "/usr/bin/true", project, "Inspect", hook_events=True))


class ContextLaunchTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "project"
        self.project.mkdir()
        self.fake = self.root / "fake.py"
        self.fake.write_text(FAKE_CLI)
        self.events = self.root / "events.json"
        self.events.write_text(json.dumps(CLAUDE_EVENTS))
        self.calls, self.managers, self.behavior = [], [], None
        for name, value in (("discover_providers", [{"id": "codex", "available": True, "executable": str(self.fake)},
                                                    {"id": "claude", "available": True, "executable": str(self.fake)}]),
                            ("model_options", {"models": [], "efforts": [], "detail": "Offline fixture"})):
            patcher = patch.object(sessions.providers, name, return_value=value)
            patcher.start()
            self.addCleanup(patcher.stop)
        builder = patch.object(sessions.providers, "build_command", side_effect=self.build_command)
        builder.start()
        self.addCleanup(builder.stop)
        home = patch.dict(os.environ, {"CODEX_HOME": str(self.root / "codex-home")})
        home.start()
        self.addCleanup(home.stop)
        self.addCleanup(lambda: [manager.close() for manager in self.managers])

    def build_command(self, provider, executable, project, prompt, **options):
        self.calls.append({"provider": provider, "prompt": prompt, **options})
        behavior = self.behavior or ("events" if self.events.exists() else "plain")
        return [sys.executable, "-u", str(self.fake), json.dumps({"provider": provider, "behavior": behavior, "events": str(self.events)})]

    def manager(self, state=None):
        manager = sessions.Sessions(state or self.root / "state", [self.project], timeout=20)
        self.managers.append(manager)
        return manager

    def run_session(self, manager, **options):
        sid = manager.create({"project_id": next(iter(manager.projects)), "provider": "claude", "prompt": "Fix the retry bug",
                              "project_context": True, **options})["id"]
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            session = manager.get(sid)
            if session["status"] not in sessions.ACTIVE and manager.jobs.unfinished_tasks == 0:
                return session, manager.results.history(sid)["launches"]
            time.sleep(.02)
        self.fail("The fixture launch did not finish")

    def assert_integers_only(self, record):
        # Only the provider and the fixed file names are text; no prompt, file or hook text is stored.
        allowed = {*sessions.CONTEXT_FILES, *(name for names in context_usage.CLI_FILES.values() for name in names), "claude", "codex", "cursor"}
        self.assertEqual([], [(path, value) for path, value in strings(record) if value not in allowed])

    def test_claude_launch_records_its_prompt_parts_fill_and_hook_memory_as_integers(self):
        (self.project / "AGENTS.md").write_text("A" * 13998)
        (self.project / "README.md").write_text("Readme\n")
        (self.project / "CLAUDE.md").write_text("# Claude\n" * 30)
        (self.project / ".claude").mkdir()
        (self.project / ".claude/settings.json").write_text(json.dumps({"hooks": {"UserPromptSubmit": [{"hooks": [{"command": "x"}]}]}}))
        manager = self.manager()
        session, launches = self.run_session(manager)
        self.assertEqual("completed", session["status"])
        self.assertTrue(self.calls[0]["hook_events"])
        context = launches[0]["context"]
        self.assert_integers_only(context)
        ledger, fill = context["ledger"], context["fill"]
        self.assertEqual(("claude", 1, len("Fix the retry bug"), len(self.calls[0]["prompt"])),
                         (context["provider"], context["agents"], ledger["message"], ledger["total"]))
        # Claude loads CLAUDE.md by itself, so the excerpts leave it out; it counts as a CLI file instead.
        self.assertEqual([{"name": "AGENTS.md", "sent": 3000, "full": 13998, "characters": 3000},
                          {"name": "README.md", "sent": 7, "full": 7, "characters": 7}], ledger["excerpts"])
        self.assertEqual(ledger["total"] - ledger["message"] - 3007, ledger["instructions"])
        self.assertEqual((32000, 31000, 150000, 3, 200000), (fill["start"], fill["end"], fill["peak"], fill["calls"], fill["window"]))
        self.assertEqual(capsule_text_parts(HOOK_TEXT), fill["hooks"])
        self.assertEqual(({"installed": True, "measured": True, "bytes": None}, [{"name": "CLAUDE.md", "bytes": 270}]),
                         (context["hooks"], context["cli_files"]))
        self.assertEqual({"fill": 31000, "window": 200000, "compacted": True, "running": False},
                         {key: session["context_last"][key] for key in ("fill", "window", "compacted", "running")})
        # The next turn on the same model knows its window before its own result reports it.
        self.events.unlink()
        manager.send(session["id"], "Follow up")
        deadline = time.monotonic() + 15
        while (manager.get(session["id"])["status"] in sessions.ACTIVE or manager.jobs.unfinished_tasks) and time.monotonic() < deadline:
            time.sleep(.02)
        second = manager.results.history(session["id"])["launches"][1]["context"]["fill"]
        self.assertEqual((200000, None, None), (second["window"], second["end"], second["calls"]))
        self.assertIsNone(manager.results.last_window(session["id"], "another-model"))

    def test_a_rendered_capsule_is_counted_as_the_prompt_carries_it(self):
        manager = self.manager()
        text = rendered(RENDERED_CAPSULE, RENDERED_EXCERPTS)
        session = {"id": None, "workflow": "native", "sdd": None, "project_context": False, "provider": "claude",
                   "brain": {"capsule": RENDERED_CAPSULE, "capsule_text": text}, "agents_enabled": False,
                   "agent_count": 3, "thinking_effort": None, "budgets": {}}
        ledger = {}
        prompt = manager._prompt(session, "Fix the retry", ledger)
        self.assert_integers_only(ledger)
        inserted = sessions.BRAIN_CONTEXT_HEADER + text + "\n\n"
        self.assertIn(inserted, prompt)
        self.assertNotIn('"categories"', prompt)
        capsule = ledger["capsule"]
        # The memory parts describe the text the provider received, header and envelope as Project Brain.
        self.assertEqual((len(inserted), len(inserted)), (capsule["inserted"], sum(capsule["kinds"].values())))
        self.assertEqual(capsule_text_parts(inserted), capsule["kinds"])
        self.assertTrue(capsule["kinds"]["bank"] and capsule["kinds"]["rules"])
        # The JSON it replaced repeated every item three times; measuring it overstated the memory sent.
        self.assertGreater(sum(sessions.capsule_parts(RENDERED_CAPSULE).values()), 2 * capsule["inserted"])

    def test_capsule_attachments_and_instructions_add_up_to_the_prompt(self):
        # A runtime that renders no capsule text: the prompt carries the JSON, and its parts are the JSON's.
        manager = self.manager()
        capsule = {"query": "retry", "working": {"task_id": "T-1", "goal": "Retry — webhooks"},
                   "semantic": [{"path": "memory-bank/chunks/MEM-0001-a.md", "category": "durable", "layer": "semantic", "title": "Retry"}],
                   "procedural": [{"path": "AGENTS.md", "category": "policy", "layer": "procedural", "title": "Rules"}], "episodic": [],
                   "selected": [], "categories": {"durable": [], "policy": []}, "omitted": {"semantic": 1}}
        files = [({"name": "notes.txt", "size": 12}, None, "/attachments/notes.txt")]
        session = {"id": None, "workflow": "plan", "sdd": None, "brain": {"capsule": capsule}, "project_context": False,
                   "provider": "codex", "agents_enabled": False, "agent_count": 3, "thinking_effort": None, "budgets": {}}
        with patch.object(manager.attachments, "current", return_value=files):
            ledger = {}
            text = manager._prompt({**session, "id": "s1"}, "Do it", ledger)
        self.assert_integers_only(ledger)
        inserted = sessions.BRAIN_CONTEXT_HEADER + json.dumps(capsule, ensure_ascii=False) + "\n\n"
        self.assertIn(inserted, text)
        self.assertEqual((len(inserted), len(inserted)), (ledger["capsule"]["inserted"], sum(ledger["capsule"]["kinds"].values())))
        self.assertEqual({"semantic": 1}, ledger["capsule"]["dropped"])
        listing = text[text.index("User-attached"):text.index(sessions.BRAIN_CONTEXT_HEADER)]
        self.assertEqual({"characters": len(listing), "count": 1}, ledger["attachments"])
        self.assertEqual((len(text), len(text) - 5 - len(inserted) - len(listing)), (ledger["total"], ledger["instructions"]))
        self.assertIsNone(ledger["excerpts"])

    def test_codex_fill_arrives_from_its_rollout_and_its_hooks_are_not_measured(self):
        self.events.unlink()
        (self.project / ".codex").mkdir()
        (self.project / ".codex/hooks.json").write_text(json.dumps({"hooks": {"UserPromptSubmit": []}}))
        (self.project / "AGENTS.md").write_text("Agents\n")
        manager = self.manager()
        session, launches = self.run_session(manager, provider="codex", project_context=False)
        self.assertEqual("completed", session["status"])
        self.assertFalse(self.calls[0]["hook_events"])
        context = launches[0]["context"]
        self.assert_integers_only(context)
        self.assertEqual({"start": 38616, "end": 26000, "peak": 52000, "calls": 4, "window": 258400, "cache_share": .8,
                          "compactions": [{"pre": 52000, "post": 21000, "call": 2}], "hooks": None}, context["fill"])
        self.assertEqual(({"installed": True, "measured": False, "bytes": None}, [{"name": "AGENTS.md", "bytes": 7}]),
                         (context["hooks"], context["cli_files"]))

    def test_a_codex_launch_longer_than_the_rollout_tail_keeps_its_start_peak_calls_and_compactions(self):
        # The first calls and the compaction lie before the last 4 MiB of the rollout, which is all the tail read
        # after the launch holds; only the live reads saw them, and they stay.
        self.behavior = "long-codex"
        manager = self.manager()
        session, launches = self.run_session(manager, provider="codex", project_context=False)
        self.assertEqual("completed", session["status"])
        self.assertGreater((self.root / "codex-home/sessions/rollout-native-context.jsonl").stat().st_size, 4 * 1024 * 1024)
        fill = launches[0]["context"]["fill"]
        self.assertEqual({"start": 38616, "end": 30000, "peak": 52000, "calls": 5, "window": 258400, "cache_share": .8,
                          "compactions": [{"pre": 52000, "post": 21000, "call": 2}], "hooks": None}, fill)
        self.assertEqual({"fill": 30000, "window": 258400, "compacted": True},
                         {key: session["context_last"][key] for key in ("fill", "window", "compacted")})
        self.assertEqual([52000], [event["compaction"]["pre"] for event in manager.events(session["id"]) if "compaction" in event])

    def test_a_settled_codex_fill_extends_the_live_reads_by_place_in_the_file(self):
        home = self.root / "codex-home"
        (home / "sessions").mkdir(parents=True)
        rollout = home / "sessions" / "rollout-long.jsonl"
        now = datetime.now(timezone.utc)
        record = lambda kind, payload: json.dumps({"type": kind, "timestamp": now.isoformat(), "payload": payload})
        count = lambda fill: record("event_msg", {"type": "token_count", "info": {"model_context_window": 258400,
                                                                                    "last_token_usage": {"input_tokens": fill}}})
        output = record("response_item", {"type": "function_call_output", "output": "x" * 4096})
        mebibyte = (1024 * 1024) // (len(output) + 1)
        meta = json.dumps({"type": "session_meta", "payload": {"id": "native-long", "cwd": str(self.project), "source": "exec"}})
        # 25,000 sits where both the live reads and the tail reach; 30,000 and the last line, which has no newline,
        # only the tail reaches.
        lines = [meta, count(100000), record("compacted", {}), count(20000), *[output] * (2 * mebibyte), count(25000),
                 *[output] * (2 * mebibyte), count(30000), *[output] * mebibyte]
        rollout.write_text("\n".join(lines) + "\n" + count(31000))
        with closing(sqlite3.connect(home / "state_5.sqlite")) as db:
            db.execute("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, cwd TEXT)")
            db.execute("INSERT INTO threads VALUES (?,?,?)", ("native-long", str(rollout), str(self.project)))
            db.commit()
        with patch.object(context_usage.CodexLive, "LIMIT", 1024 * 1024), patch.object(context_usage.CodexLive, "SETTLE_READS", 2):
            live = context_usage.CodexLive(self.project, now.timestamp() - 1)
            self.assertTrue(live.poll("native-long"))
            self.assertEqual((100000, 20000, 2), tuple(live.fill.snapshot()[key] for key in ("start", "end", "calls")))
            self.assertTrue(live.settle("native-long", time.time()))
        # Three reads of 1 MiB stop short of the 5 MiB rollout, and its last 4 MiB begin before they stopped.
        size = rollout.stat().st_size
        self.assertTrue(size - 4 * 1024 * 1024 < live.offset < size, (size, live.offset))
        self.assertEqual([100000, 20000, 25000, 30000, 31000], live.fill.fills)
        self.assertEqual({"start": 100000, "end": 31000, "peak": 100000, "calls": 5, "window": 258400,
                          "compactions": [{"pre": 100000, "post": 20000, "call": 1}]}, live.fill.snapshot())

    def test_codex_fill_grows_while_the_launch_runs(self):
        self.behavior = "slow-codex"
        manager = self.manager()
        with patch.object(context_usage, "WRITE_SECONDS", .2):
            sid = manager.create({"project_id": next(iter(manager.projects)), "provider": "codex", "prompt": "Grow", "project_context": False})["id"]
            seen, deadline = [], time.monotonic() + 20
            while time.monotonic() < deadline:
                session = manager.get(sid)
                launches = manager.results.history(sid)["launches"]
                fill = (launches[0]["context"] or {}).get("fill") if launches else None
                if session["status"] == "running" and fill and fill["end"] is not None:
                    seen.append(fill["calls"])
                if session["status"] not in sessions.ACTIVE and manager.jobs.unfinished_tasks == 0:
                    break
                time.sleep(.05)
        final = manager.results.history(sid)["launches"][0]["context"]["fill"]
        self.assertEqual((30000, 41000, 12, 258400), (final["start"], final["end"], final["calls"], final["window"]))
        # The page saw the rollout grow before the turn ended.
        self.assertTrue(seen and min(seen) < 12, seen)
        self.assertEqual(sorted(seen), seen)

    def test_live_rollout_reads_only_new_complete_lines(self):
        home = self.root / "codex-home"
        folder = home / "sessions"
        folder.mkdir(parents=True)
        rollout = folder / "rollout-live.jsonl"
        now = datetime.now(timezone.utc)
        meta = json.dumps({"type": "session_meta", "payload": {"id": "native-live", "cwd": str(self.project), "source": "exec"}})
        count = lambda fill: json.dumps({"type": "event_msg", "timestamp": now.isoformat(), "payload": {"type": "token_count",
                                         "info": {"model_context_window": 258400, "last_token_usage": {"input_tokens": fill}}}})
        rollout.write_text(meta + "\n" + count(10000) + "\n")
        live = context_usage.CodexLive(self.project, now.timestamp() - 1)
        self.assertFalse(live.poll(None))
        self.assertFalse(live.poll("native-live"))
        with sqlite3.connect(home / "state_5.sqlite") as db:
            db.execute("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, cwd TEXT)")
            db.execute("INSERT INTO threads VALUES (?,?,?)", ("native-live", str(rollout), str(self.project)))
        live.retry_at = 0
        self.assertTrue(live.poll("native-live"))
        self.assertEqual((10000, 1), (live.fill.snapshot()["end"], live.fill.snapshot()["calls"]))
        with rollout.open("a") as handle:
            handle.write(count(12000) + "\n" + count(13000)[:40])
        self.assertTrue(live.poll("native-live"))
        self.assertEqual((12000, 2), (live.fill.snapshot()["end"], live.fill.snapshot()["calls"]))
        with rollout.open("a") as handle:
            handle.write(count(13000)[40:] + "\n")
        self.assertTrue(live.poll("native-live"))
        self.assertFalse(live.poll("native-live"))
        self.assertEqual((10000, 13000, 3, 258400), tuple(live.fill.snapshot()[key] for key in ("start", "end", "calls", "window")))

    def test_unreported_fill_stays_unknown_and_old_launch_history_reads_as_not_recorded(self):
        self.events.unlink()
        state = self.root / "old-state"
        state.mkdir()
        with sqlite3.connect(state / "sessions.sqlite3") as db:
            db.execute("CREATE TABLE launches (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL, "
                       "started_at TEXT NOT NULL, finished_at TEXT, settings TEXT NOT NULL, usage TEXT)")
            db.execute("INSERT INTO launches VALUES ('old','s-old','native','completed','2026-01-01T00:00:00+00:00',NULL,'{}',NULL)")
        manager = self.manager(state)
        with manager.lock:
            columns = {row["name"] for row in manager.db.execute("PRAGMA table_info(launches)")}
            old = manager.db.execute("SELECT context FROM launches WHERE id='old'").fetchone()
        self.assertIn("context", columns)
        self.assertIsNone(old["context"])
        session, launches = self.run_session(manager)
        fill = launches[0]["context"]["fill"]
        self.assertEqual({"start": None, "end": None, "peak": None, "calls": None, "window": None, "compactions": [],
                          "cache_share": None, "hooks": None}, fill)
        self.assertEqual(({"installed": False, "measured": False, "bytes": None}, []),
                         (launches[0]["context"]["hooks"], launches[0]["context"]["cli_files"]))
        self.assertIsNone(session["context_last"])

    def test_cursor_counts_its_memory_rule_and_stops_at_its_size(self):
        (self.project / ".cursor/rules").mkdir(parents=True)
        (self.project / ".cursor/rules/working-memory.mdc").write_text("working: T-1\n" * 40)
        self.assertEqual({"cli_files": [], "hooks": {"installed": True, "measured": True, "bytes": 520}},
                         context_usage.launch_files(self.project, "cursor"))
        self.assertEqual({"cli_files": [], "hooks": {"installed": False, "measured": False, "bytes": None}},
                         context_usage.launch_files(self.project, "claude"))


if __name__ == "__main__":
    unittest.main()
