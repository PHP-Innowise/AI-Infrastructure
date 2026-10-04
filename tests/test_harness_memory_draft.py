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

    def test_external_ids_name_the_task_and_stay_unique(self):
        first = memory_draft.external_id("feature/cobalt", "finding")
        self.assertTrue(first.startswith("feature/cobalt-finding-"), first)
        self.assertNotEqual(first, memory_draft.external_id("feature/cobalt", "finding"))
        self.assertLessEqual(len(memory_draft.external_id("t" * 300, "decision")), 128)


if __name__ == "__main__":
    unittest.main()
