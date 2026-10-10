"""The memory draft a linked run leaves, and what the page may send back to save."""

import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import memory_draft
from harness.sessions import SessionError

LEARNING = {"type": "finding", "title": "Cobalt allocation needs one owner",
            "consequence": "Every cobalt allocation names exactly one owner.",
            "sources": ["specs/authority.md"]}


def reply(draft, before="Done."):
    return f"{before}\n\n```memory-draft\n{json.dumps(draft)}\n```\n"


class StubSessions:
    def __init__(self, events):
        self.events = events

    def recent_events(self, sid, kinds, limit=30):
        return [event for event in self.events if event["kind"] in kinds][-limit:]


class ParseTests(unittest.TestCase):
    def test_reported_memory_use_is_bounded_and_never_proves_reading(self):
        path = 'memory-bank/chunks/known.md'
        draft = memory_draft.parse(reply({'progress': 'Checked', 'used_memory': [path, path, '../foreign.md']}))
        evidence = memory_draft.usage(draft, {'semantic': [{'path': path}]})
        self.assertEqual([path], evidence['reported_used'])
        self.assertEqual('agent-reported', evidence['attestation'])
        self.assertEqual(1, evidence['delivered'])
        self.assertFalse(memory_draft.usage({}, {})['reported'])

    def test_the_watch_names_delivered_memory_as_the_runs_tools_read_it(self):
        skill = '.claude/skills/allocation/SKILL.md'
        capsule = {'semantic': [{'path': 'specs/cobalt.md'}], 'procedural': [{'path': skill}],
                   'selected': [{'path': 'specs/cobalt.md'}], 'episodic': [{'id': 3, 'summary': 'A local episode'}]}
        # Any tool's copy of a skill is the skill delivered, and so is a Skill load of it by name.
        self.assertEqual({'specs/cobalt.md': 'specs/cobalt.md', 'skill:allocation': skill,
                          **{tree + 'allocation/SKILL.md': skill for tree in memory_draft.SKILL_TREES}},
                         memory_draft.watch(capsule))
        # An attached accelerator's skill is an absolute path into the clone.
        attached = memory_draft.watch({'procedural': [{'path': '/clone/PHP Core/.agents/skills/allocation/SKILL.md'}]})
        self.assertEqual(('/clone/PHP Core/.agents/skills/allocation/SKILL.md',) * 2,
                         (attached['skill:allocation'], attached['/clone/PHP Core/.codex/skills/allocation/SKILL.md']))
        self.assertEqual({}, memory_draft.watch(None))

    def test_opened_memory_is_the_receipts_count_and_the_notice_says_what_it_is(self):
        capsule = {'semantic': [{'path': 'specs/cobalt.md'}, {'path': 'memory-bank/chunks/MEM-1-x.md'}]}
        draft = {'used_memory': ['specs/cobalt.md', 'specs/elsewhere.md']}
        record = lambda opened, observation, *unseen: {'delivered': 2, 'opened': opened, 'observation': observation,
                                                       'unseen': list(unseen)}
        tail = ', 1 reported used. Neither proves the claim was used.'
        for read, expected, line in (
                (record(1, 'observed'), (1, 'observed'), 'Memory use: 2 delivered, 1 opened by the agent' + tail),
                (record(1, 'lower_bound', 'shell'), (1, 'lower_bound'),
                 'Memory use: 2 delivered, at least 1 opened by the agent (shell commands not inspected)' + tail),
                (record(0, 'lower_bound', 'shell', 'outside'), (0, 'lower_bound'),
                 'Memory use: 2 delivered, none opened with a read tool '
                 '(shell commands and reads outside the project not inspected)' + tail),
                (record(0, 'unknown', 'shell', 'helpers'), (None, 'unknown'),
                 'Memory use: 2 delivered, opening unknown (no read reported; shell commands and helper agents not inspected)' + tail),
                (None, (None, 'unrecorded'), 'Memory use: 2 delivered, opening not recorded' + tail),
                (record(5, 'observed'), (2, 'observed'), 'Memory use: 2 delivered, 2 opened by the agent' + tail),
                (record('1', 'observed'), (None, 'unknown'), 'Memory use: 2 delivered, opening unknown '
                 '(no read reported; some reads not inspected)' + tail)):
            with self.subTest(read=read):
                evidence = memory_draft.usage(draft, capsule, read)
                self.assertEqual(expected, (evidence['opened'], evidence['observation']))
                self.assertEqual((2, ['specs/cobalt.md']), (evidence['delivered'], evidence['reported_used']))
                self.assertEqual(line, memory_draft.use_notice(evidence))
        self.assertTrue(memory_draft.use_notice(memory_draft.usage({}, capsule, record(0, 'observed'))).endswith(
            '0 opened by the agent. Neither proves the claim was used.'))

    def test_the_last_block_is_the_draft_and_it_is_clipped_to_the_form(self):
        earlier = reply({"progress": "stale", "next_steps": [], "learnings": []})
        draft = {"progress": "  Checked   at allocation. ", "next_steps": ["a", "b", "c", "d"],
                 "learnings": [LEARNING, {**LEARNING, "title": ""}, {**LEARNING, "type": "task"},
                               {**LEARNING, "title": "x" * 500}, LEARNING, LEARNING]}
        parsed = memory_draft.parse(earlier + reply(draft))
        self.assertEqual("Checked at allocation.", parsed["progress"])
        self.assertEqual(["a", "b", "c"], parsed["next_steps"])
        # A learning without a title or with an unknown type is dropped; at most three stay.
        self.assertEqual(3, len(parsed["learnings"]))
        self.assertEqual(memory_draft.TITLE_LIMIT, len(parsed["learnings"][1]["title"]))

    def test_anything_but_a_json_object_in_the_block_is_no_draft(self):
        for text in ("no block here", "```memory-draft\nnot json\n```", "```memory-draft\n[1, 2]\n```",
                     reply({"progress": "x"}) + "x" * memory_draft.TEXT_LIMIT, None):
            with self.subTest(text=str(text)[:40]):
                self.assertIsNone(memory_draft.parse(text))

    def test_wrongly_typed_fields_become_empty_rather_than_failing(self):
        parsed = memory_draft.parse(reply({"progress": 3, "next_steps": "one", "learnings": {"a": 1}}))
        self.assertEqual({"progress": "", "next_steps": [], "learnings": []}, parsed)


class LatestTests(unittest.TestCase):
    def test_only_the_last_run_counts(self):
        sessions = StubSessions([
            {"kind": "user", "text": "first", "id": 1},
            {"kind": "result", "text": reply({"progress": "first run"}), "id": 2},
            {"kind": "user", "text": "follow-up", "id": 3},
            {"kind": "text", "text": "Working on it.", "id": 4},
        ])
        self.assertEqual({"state": "missing", "draft": None, "event_id": None},
                         memory_draft.latest(sessions, "sid"))
        sessions.events.append({"kind": "text", "text": reply({"progress": "second run"}), "id": 5})
        latest = memory_draft.latest(sessions, "sid")
        self.assertEqual(("drafted", 5, "second run"),
                         (latest["state"], latest["event_id"], latest["draft"]["progress"]))

    def test_a_block_that_does_not_parse_is_reported_as_unreadable(self):
        sessions = StubSessions([{"kind": "user", "text": "go", "id": 1},
                                 {"kind": "text", "text": "```memory-draft\n{oops\n```", "id": 2}])
        self.assertEqual("unreadable", memory_draft.latest(sessions, "sid")["state"])
        self.assertEqual("none", memory_draft.latest(StubSessions([]), "sid")["state"])


class SubmissionTests(unittest.TestCase):
    def test_a_reviewed_draft_is_normalized(self):
        saved = memory_draft.submission({"progress": " Done. ", "next_steps": [" Ship "],
                                         "learnings": [{**LEARNING, "sources": ["specs/authority.md#L3",
                                                                                "specs/authority.md#L3"]}],
                                         "verified": True})
        self.assertEqual("Done.", saved["progress"])
        self.assertEqual(["Ship"], saved["next_steps"])
        self.assertEqual(["specs/authority.md#L3"], saved["learnings"][0]["sources"])

    def test_learnings_are_verified_evidence_so_the_person_must_say_so(self):
        with self.assertRaisesRegex(SessionError, "Confirm that you checked"):
            memory_draft.submission({"learnings": [LEARNING]})
        with self.assertRaisesRegex(SessionError, "Confirm that you checked"):
            memory_draft.submission({"learnings": [LEARNING], "verified": "yes"})

    def test_invalid_submissions_are_refused(self):
        cases = (
            {"progress": "x", "unexpected": 1},
            {"progress": "x" * (memory_draft.PROGRESS_LIMIT + 1)},
            {"next_steps": ["a", "b", "c", "d"]},
            {"next_steps": ["x" * (memory_draft.STEP_LIMIT + 1)]},
            {"next_steps": [""]},
            {"learnings": [{**LEARNING, "sources": []}], "verified": True},
            {"learnings": [{**LEARNING, "sources": ["../outside.md"]}], "verified": True},
            {"learnings": [{**LEARNING, "sources": ["/etc/passwd"]}], "verified": True},
            {"learnings": [{**LEARNING, "type": "task"}], "verified": True},
            {"learnings": [{**LEARNING, "extra": 1}], "verified": True},
            {"learnings": [LEARNING] * 4, "verified": True},
            {},
            {"progress": "  "},
            [],
        )
        for data in cases:
            with self.subTest(data=str(data)[:80]):
                with self.assertRaises(SessionError):
                    memory_draft.submission(data)

    def test_sources_must_be_regular_files_in_the_workspace(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "specs").mkdir()
            (root / "specs/authority.md").write_text("# Authority\n", encoding="utf-8")
            draft = memory_draft.submission({"learnings": [LEARNING], "verified": True})
            memory_draft.check_sources(root, draft)
            for source in ("specs/missing.md", "specs"):
                with self.subTest(source=source):
                    missing = memory_draft.submission(
                        {"learnings": [{**LEARNING, "sources": [source]}], "verified": True})
                    with self.assertRaises(SessionError):
                        memory_draft.check_sources(root, missing)

    def test_a_draft_may_not_cite_derived_memory_or_secrets_on_any_write_path(self):
        # The runtime's record-result refuses these; an older runtime's
        # command chain would not, so the Harness refuses them first.
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            for relative in ("memory-bank/chunks/derived.md", ".env.local", "config/app.pem", "app/Order.php"):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("x", encoding="utf-8")
            draft = {"progress": "", "next_steps": [], "verified": True, "learnings": [
                {"type": "finding", "title": title, "consequence": "c", "sources": [source]}
                for title, source in (("Derived", "memory-bank/chunks/derived.md"), ("Env", ".env.local"),
                                      ("Key", "config/app.pem"), ("Order", "app/Order.php"))]}
            kept, skipped = memory_draft.usable(root, draft)
            self.assertEqual(["Order"], [item["title"] for item in kept["learnings"]])
            self.assertEqual({"Derived", "Env", "Key"}, {item["title"] for item in skipped})
            with self.assertRaises(SessionError):
                memory_draft.check_sources(root, {"learnings": [draft["learnings"][0]]})

    def test_external_ids_name_the_task_and_stay_unique(self):
        first = memory_draft.external_id("feature/cobalt", "finding")
        self.assertTrue(first.startswith("feature/cobalt-finding-"), first)
        self.assertNotEqual(first, memory_draft.external_id("feature/cobalt", "finding"))
        self.assertLessEqual(len(memory_draft.external_id("t" * 300, "decision")), 128)



class UnattendedSaveTests(unittest.TestCase):
    """What a run's draft keeps when nobody reviews it, and what the conversation says."""

    def test_the_instruction_tells_the_agent_who_reads_its_draft_next(self):
        # Every linked run's draft is saved when it completes, a reviewed session's too.
        self.assertTrue(memory_draft.instruction().endswith("with no review, so leave out anything you did not verify."))
        self.assertTrue(memory_draft.instruction().startswith(memory_draft.REQUEST))
        self.assertNotIn("A person reviews the draft", memory_draft.instruction())

    def test_a_learning_the_workspace_cannot_back_is_left_out_rather_than_failing_the_save(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "specs").mkdir()
            (root / "specs/authority.md").write_text("# Authority\n", encoding="utf-8")
            draft = {"progress": "Checked\x01 at\nallocation.", "next_steps": ["Ship\x7f it", " ", "b", "c", "d"],
                     "learnings": [{**LEARNING, "sources": ["specs/authority.md#L1", "specs/missing.md", "../escape.md",
                                                            "specs/authority.md#L1"]},
                                   {**LEARNING, "title": "Unsourced", "sources": ["specs/missing.md"]},
                                   {**LEARNING, "title": "cobalt  ALLOCATION needs one owner"},
                                   {**LEARNING, "type": "decision", "title": "Owners are explicit"}]}
            kept, skipped = memory_draft.usable(root, draft, known=[["decision", "owners are explicit"]])
            self.assertEqual(("Checked at allocation.", ["Ship it", "b", "c"]), (kept["progress"], kept["next_steps"]))
            self.assertEqual([["specs/authority.md#L1"]], [learning["sources"] for learning in kept["learnings"]])
            self.assertTrue(kept["verified"])
            self.assertEqual([("Unsourced", "unsourced"), ("cobalt ALLOCATION needs one owner", "repeated"),
                              ("Owners are explicit", "repeated")],
                             [(item["title"], item["reason"]) for item in skipped])
            # The kept draft passes the same checks as a reviewed one.
            memory_draft.check_sources(root, memory_draft.submission(kept))
            nothing, _ = memory_draft.usable(root, {"progress": "", "next_steps": [], "learnings": []})
            self.assertEqual((False, []), (nothing["verified"], nothing["learnings"]))

    def test_the_conversation_line_says_what_was_saved_promoted_and_left_out(self):
        self.assertIn("no memory draft", memory_draft.summary("missing"))
        self.assertIn("could not be read", memory_draft.summary("unreadable"))
        result = {"ok": True, "error": None,
                  "saved": {"task": {"id": "t", "revision": 3},
                            "records": [{"type": "finding", "title": "One owner", "status": "resolved"}],
                            "promotion": {"enabled": True, "promoted": [{"memory_id": "MEM-20261004-aaaaaaaa"}],
                                          "blocked": [{"reason": "near-duplicate"}], "failed": []}},
                  "skipped": [{"title": "x", "reason": "unsourced"}, {"title": "y", "reason": "repeated"}]}
        self.assertEqual("Saved to project memory: the task's progress and next steps; finding \u201cOne owner\u201d. "
                         "1 learning(s) were already saved from this session. "
                         "Left out 1 learning(s) citing no file in the workspace. "
                         "Promoted to the Memory Bank as MEM-20261004-aaaaaaaa. "
                         "1 held back from the Memory Bank; they stay in Project Brain.",
                         memory_draft.summary("drafted", result))
        self.assertEqual("Nothing new to save to project memory from this run. "
                         "Automatic promotion is off for this project, so it stays in Project Brain.",
                         memory_draft.summary("drafted", {"ok": True, "saved": {"task": None, "records": [],
                                                          "promotion": {"enabled": False}}, "skipped": []}))
        # A replay of a save leaves later revisions and removals as they are, and says so.
        replayed = memory_draft.summary("drafted", {"ok": True, "skipped": [], "saved": {"task": None, "records": [
            {"type": "finding", "title": "One owner", "state": "differs"},
            {"type": "decision", "title": "Retry once", "state": "missing"}]}})
        self.assertIn("Kept as recorded, with a different consequence than this run's: finding “One owner”", replayed)
        self.assertIn("Removed from project memory since this run saved it: decision “Retry once”.", replayed)
        stopped = memory_draft.summary("drafted", {"ok": False, "error": "The runtime refused content matching its "
                                                   "secret-protection rules.", "saved": {"task": {"id": "t"}, "records": []}})
        self.assertTrue(stopped.endswith("Stopped: The runtime refused content matching its secret-protection rules. "
                                         "What is listed was saved."), stopped)


if __name__ == "__main__":
    unittest.main()
