"""Knowledge › Memory use: an in-process reader of counts and dates, and the runtime check behind a button."""

from datetime import date, datetime, timedelta, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import memory_use, sessions
from harness.knowledge import KnowledgeBusy, KnowledgeManager
from tests.test_harness_knowledge import install_knowledge_fixture

TASK = "11111111-1111-4111-8111-111111111111"
OTHER_TASK = "22222222-2222-4222-8222-222222222222"
FINDING = "33333333-3333-4333-8333-333333333333"
QUERY = "QUERY-SECRET cobalt allocation"


def frontmatter(fields, body="# Chunk\n\nBody.\n"):
    return "---\n" + json.dumps(fields, indent=2) + "\n---\n\n" + body


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


class MemoryUseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "project"
        self.project.mkdir()
        for name, value in (("discover_providers", []), ("build_command", None)):
            patcher = patch.object(sessions.providers, name, return_value=value) if value is not None else \
                patch.object(sessions.providers, name, side_effect=AssertionError("Memory use never runs a model CLI"))
            patcher.start()
            self.addCleanup(patcher.stop)
        worker = patch.object(sessions.Sessions, "_worker", return_value=None)
        worker.start()
        self.addCleanup(worker.stop)
        self.store = sessions.Sessions(self.root / "state", [self.project])
        self.addCleanup(self.store.close)
        self.manager = KnowledgeManager(self.store)
        self.addCleanup(self.manager.close)
        self.project_id = next(iter(self.store.projects))
        self.today = date.today()

    def write(self, relative, text):
        path = self.project / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def chunk(self, identifier, slug, **changes):
        fields = {"id": identifier, "title": f"Title {identifier}", "type": "domain", "status": "active", "tags": [],
                  "created": "2026-09-01", "last_verified": "2026-09-01",
                  "review_after": (self.today + timedelta(days=200)).isoformat(), "sources": []}
        fields.update(changes)
        return self.write(f"memory-bank/chunks/{identifier}-{slug}.md", frontmatter(fields))

    def record(self, relative, identifier, kind, status, privacy="team", resolved=None):
        transitions = [{"from": None, "to": "open", "at": "2026-09-01T10:00:00+00:00", "actor": "fixture", "reason": "created"}]
        if resolved:
            transitions.append({"from": "open", "to": status, "at": resolved, "actor": "fixture", "reason": "done"})
        fields = {"id": identifier, "type": kind, "status": status, "title": "BRAIN TITLE SECRET", "privacy": privacy,
                  "progress": "BRAIN BODY SECRET", "transitions": transitions}
        return self.write(f"project-brain/{relative}/{identifier}.md", frontmatter(fields, "BRAIN BODY SECRET\n"))

    def manifest(self, store, name, at, selected=(), excluded=(), **fields):
        manifest = {"schema_version": 3, "id": name, "created_at": at, "query": QUERY, "task_id": TASK, "task_revision": 1,
                    "selected": list(selected), "excluded": list(excluded), **fields}
        folder = "project-brain/control" if store == "governed" else "memory-bank/local"
        return self.write(f"{folder}/retrieval-manifests/{name}.json", json.dumps(manifest))

    def read(self, tasks=frozenset()):
        payload = memory_use.read(self.manager, self.project_id, None, tasks)
        json.dumps(payload, allow_nan=False)
        return payload

    def test_chunks_report_provenance_review_dates_and_changed_sources_as_file_facts(self):
        self.write("docs/a.md", "A")
        self.write("docs/b.md", "B")
        (self.project / "docs/link.md").symlink_to("a.md")
        self.chunk("MEM-20260901-aaaaaaaa", "auto", tags=["project-brain", "promoted", "auto-promoted"],
                   source_digests=[{"path": "docs/a.md", "sha256": sha("A")}])
        self.chunk("MEM-20260902-bbbbbbbb", "reviewed", tags=["project-brain", "promoted"],
                   source_digests=[{"path": "docs/b.md", "sha256": sha("an older B")}])
        self.chunk("MEM-0001", "legacy", review_after=(self.today - timedelta(days=9)).isoformat(), last_verified="2025-01-01")
        self.chunk("MEM-0002", "missing-source", source_digests=[{"path": "docs/gone.md", "sha256": sha("gone")}])
        self.chunk("MEM-0003", "linked-source", source_digests=[{"path": "docs/link.md", "sha256": sha("A")}])
        self.write("memory-bank/chunks/MEM-20260903-cccccccc-broken.md", "no frontmatter here\n")
        (self.project / "memory-bank/chunks/MEM-0004-linked.md").symlink_to(self.project / "docs/a.md")
        payload = self.read()
        chunks = {item["id"]: item for item in payload["chunks"]["items"]}
        self.assertEqual(sorted(chunks), ["MEM-0001", "MEM-0002", "MEM-0003", "MEM-20260901-aaaaaaaa",
                                          "MEM-20260902-bbbbbbbb", "MEM-20260903-cccccccc"])
        self.assertEqual({key: chunks[key]["sources_changed"] for key in chunks},
                         {"MEM-20260901-aaaaaaaa": False, "MEM-20260902-bbbbbbbb": True, "MEM-0001": None,
                          "MEM-0002": True, "MEM-0003": True, "MEM-20260903-cccccccc": None})
        self.assertEqual((True, True), (chunks["MEM-20260901-aaaaaaaa"]["auto"], chunks["MEM-20260901-aaaaaaaa"]["promoted"]))
        self.assertEqual((False, True), (chunks["MEM-20260902-bbbbbbbb"]["auto"], chunks["MEM-20260902-bbbbbbbb"]["promoted"]))
        self.assertEqual(((self.today - timedelta(days=9)).isoformat(), "2025-01-01"),
                         (chunks["MEM-0001"]["review_after"], chunks["MEM-0001"]["last_verified"]))
        broken = chunks["MEM-20260903-cccccccc"]
        self.assertEqual((None, None, None), (broken["title"], broken["status"], broken["review_after"]))
        self.assertEqual("chunks/MEM-20260901-aaaaaaaa-auto.md", chunks["MEM-20260901-aaaaaaaa"]["path"])
        self.assertEqual(sum(item["bytes"] for item in chunks.values()), payload["chunks"]["bytes"])
        # Without a runtime there is nothing to check and no governed Brain to read.
        self.assertEqual((False, None, None), (payload["check_available"], payload["brain"], payload["promotions"]))

    def test_brain_and_promotions_leave_counts_and_dates_but_no_titles_bodies_or_ids(self):
        (self.project / "memory-bank/chunks").mkdir(parents=True)
        self.record("dynamic/tasks", "44444444-4444-4444-8444-444444444444", "task", "active")
        self.record("dynamic/findings", FINDING, "finding", "resolved", resolved="2026-09-20T08:00:00+02:00")
        self.record("archive/finding", "55555555-5555-4555-8555-555555555555", "finding", "resolved",
                    privacy="private", resolved="2026-09-10T08:00:00+00:00")
        self.record("dynamic/decisions", "66666666-6666-4666-8666-666666666666", "decision", "accepted",
                    resolved="2026-09-22T08:00:00+00:00")
        self.write("project-brain/archive/handoffs/77777777-7777-4777-8777-777777777777.md",
                   frontmatter({"type": "handoff", "status": "active", "title": "BRAIN TITLE SECRET"}))
        for name, proposal in (
                ("p-auto", {"status": "applied", "review_mode": "automatic", "created_at": "2026-09-20T09:00:00+00:00",
                            "updated_at": "2026-09-21T09:00:00+00:00", "reviewed_at": None, "title": "BRAIN TITLE SECRET",
                            "destination_memory_id": "MEM-20260921-33333333", "source_records": [{"id": FINDING, "type": "finding"}]}),
                ("p-human", {"status": "proposed", "review_mode": "human", "created_at": "2026-10-01T09:00:00+00:00",
                             "updated_at": "2026-10-01T09:00:00+00:00", "reviewed_at": None, "destination_memory_id": None,
                             "source_records": [{"id": "44444444-4444-4444-8444-444444444444", "type": "task"}]}),
                ("p-rejected", {"status": "rejected", "review_mode": "human", "created_at": "2026-09-23T09:00:00+00:00",
                                "updated_at": "2026-09-24T09:00:00+00:00", "reviewed_at": "2026-09-24T09:00:00+00:00",
                                "source_records": [{"id": "66666666-6666-4666-8666-666666666666", "type": "decision"}]})):
            self.write(f"project-brain/control/promotions/{name}.json", json.dumps(proposal))
        payload = self.read()
        text = json.dumps(payload)
        for secret in ("BRAIN TITLE SECRET", "BRAIN BODY SECRET", FINDING, "44444444-4444"):
            self.assertNotIn(secret, text)
        rows = payload["brain"]["items"]
        self.assertEqual(4, len(rows))
        private = [row for row in rows if row.get("private")]
        self.assertEqual([{"archived": True, "open": False, "resolved_at": "2026-09-10T08:00:00+00:00",
                           "promotable": True, "promoted": False, "private": True}], private)
        public = {row["type"]: row for row in rows if not row.get("private")}
        self.assertEqual((True, None), (public["task"]["open"], public["task"]["resolved_at"]))
        # Moments are normalised to UTC so every store sorts as text.
        self.assertEqual(("2026-09-20T06:00:00+00:00", True, True),
                         (public["finding"]["resolved_at"], public["finding"]["promotable"], public["finding"]["promoted"]))
        # A rejected proposal does not count as promoted; an accepted decision is not open.
        self.assertEqual((False, False, True), (public["decision"]["promoted"], public["decision"]["open"], public["decision"]["promotable"]))
        promotions = sorted(payload["promotions"]["items"], key=lambda item: item["created_at"])
        self.assertEqual([("automatic", "applied", "2026-09-21T09:00:00+00:00", "MEM-20260921-33333333"),
                          ("human", "rejected", None, None), ("human", "proposed", None, None)],
                         [(item["mode"], item["status"], item["applied_at"], item["memory_id"]) for item in promotions])

    def test_retrievals_merge_harness_pairs_name_routes_and_drop_queries_and_other_paths(self):
        self.chunk("MEM-20260901-aaaaaaaa", "kept")
        policy = {"path": "AGENTS.md", "category": "policy", "estimated_tokens": 40, "source_hash": "p"}
        chunk = {"path": "memory-bank/chunks/MEM-20260901-aaaaaaaa-kept.md", "category": "durable", "estimated_tokens": 30, "source_hash": "c"}
        task = {"path": f"project-brain/dynamic/tasks/{TASK}.md", "category": "dynamic", "estimated_tokens": 20, "source_hash": "t"}
        cut = [{"path": "memory-bank/chunks/MEM-20260902-bbbbbbbb-cut.md", "reason": "layer-limit"},
               {"path": "specs/private-plan.md", "reason": "budget"}]
        harness = {"host": "cli", "entry_point": "retrieve"}
        self.manifest("governed", "prepare", "2026-09-29T10:00:00+00:00", [policy, chunk, task], cut, **harness)
        self.manifest("local", "freshness", "2026-09-29T10:05:00+00:00", [policy, chunk, task], cut, **harness)
        self.manifest("local", "hook", "2026-09-30T08:00:00+00:00",
                      [chunk, {"path": "specs/y.md", "category": "evidence", "estimated_tokens": 9, "source_hash": "y"}],
                      host="claude", entry_point="hook-context", task_id=OTHER_TASK, local_episode_count=1)
        self.manifest("local", "legacy", "2026-09-28T07:00:00+00:00", [policy], schema_version=1)
        self.manifest("governed", "manual", "2026-10-01T07:00:00+00:00", [policy], host="cli", entry_point="retrieve",
                      task_id=OTHER_TASK)
        self.write("memory-bank/local/retrieval-manifests/corrupt.json", "{not json")
        payload = self.read(frozenset({TASK}))
        text = json.dumps(payload)
        for secret in ("QUERY-SECRET", "specs/private-plan.md", "specs/y.md", "AGENTS.md", TASK):
            self.assertNotIn(secret, text)
        retrievals = payload["retrievals"]
        self.assertEqual((4, 1), (retrievals["found"], retrievals["merged"]))
        rows = retrievals["items"]
        self.assertEqual(["not recorded", "Harness", "Claude hook", "CLI"], [row["route"] for row in rows])
        self.assertEqual([1, 3, 3, 3], [row["version"] for row in rows])
        legacy, prepared, hook, manual = rows
        self.assertEqual((1, 1, 1, 1), (prepared["rules"], prepared["bank"], prepared["brain"], prepared["merged"]))
        self.assertEqual((["MEM-20260901-aaaaaaaa"], [["MEM-20260902-bbbbbbbb", "layer-limit"]]), (prepared["chunks"], prepared["cuts"]))
        # A local episode is Project Brain memory even though the manifest only counts it.
        self.assertEqual((1, 1, 1), (hook["brain"], hook["bank"], hook["rules"]))
        self.assertNotEqual(prepared["task"], hook["task"])
        self.assertEqual(hook["task"], manual["task"])

    def test_unattended_harness_memory_and_provider_hooks_name_their_routes(self):
        policy = {"path": "AGENTS.md", "category": "policy", "estimated_tokens": 40, "source_hash": "p"}
        off = {"decision": "retrieve", "mode": "off", "reason": "gate-off"}
        shadow = {"decision": "retrieve", "mode": "shadow", "reason": "new-selection"}
        # Unattended Harness memory retrieves through `refresh` as the provider, gate off, each turn in
        # the local store; two turns with the same selection are two retrievals, not a freshness check.
        self.manifest("governed", "reviewed", "2026-09-29T09:00:00+00:00", [policy], host="cli", entry_point="retrieve", gate=off)
        self.manifest("local", "turn-1", "2026-09-29T10:00:00+00:00", [policy], host="codex", entry_point="refresh", gate=off)
        self.manifest("local", "turn-2", "2026-09-29T10:05:00+00:00", [policy], host="codex", entry_point="refresh", gate=off)
        # The Claude read hook calls `refresh` with its own host under the configured gate.
        self.manifest("local", "hook", "2026-09-30T08:00:00+00:00", [policy], host="claude", entry_point="refresh",
                      gate=shadow, task_id=OTHER_TASK)
        # Ungated provider refreshes for a task the Harness never linked are a hook, not the Harness.
        self.manifest("local", "foreign", "2026-09-30T09:00:00+00:00", [policy], host="cursor", entry_point="refresh",
                      gate=off, task_id=OTHER_TASK)
        retrievals = self.read(frozenset({TASK}))["retrievals"]
        self.assertEqual((5, 0), (retrievals["found"], retrievals["merged"]))
        self.assertEqual(["Harness", "Harness", "Harness", "Claude hook", "Cursor hook"],
                         [row["route"] for row in retrievals["items"]])

    def test_caps_keep_the_newest_and_say_so(self):
        for number in range(3):
            self.chunk(f"MEM-2026090{number + 1}-0000000{number}", "capped")
            self.manifest("local", f"m{number}", f"2026-09-2{number}T10:00:00+00:00")
            self.record("dynamic/tasks", f"4444444{number}-4444-4444-8444-444444444444", "task", "active")
        with patch.object(memory_use, "CHUNK_LIMIT", 2), patch.object(memory_use, "RETRIEVAL_LIMIT", 2), \
                patch.object(memory_use, "RECORD_LIMIT", 2):
            payload = self.read()
        self.assertEqual((2, True), (len(payload["chunks"]["items"]), payload["chunks"]["truncated"]))
        self.assertEqual((2, True), (len(payload["brain"]["items"]), payload["brain"]["truncated"]))
        self.assertEqual((["2026-09-21T10:00:00+00:00", "2026-09-22T10:00:00+00:00"], 3, 2),
                         ([row["at"] for row in payload["retrievals"]["items"]], payload["retrievals"]["found"],
                          payload["retrievals"]["limit"]))

    def test_redirected_stores_are_refused_and_unknown_stays_unknown(self):
        outside = self.root / "outside"
        (outside / "chunks").mkdir(parents=True)
        (outside / "chunks/MEM-0009-outside.md").write_text(frontmatter({"id": "MEM-0009", "title": "Outside"}))
        (self.project / "memory-bank").mkdir()
        (self.project / "memory-bank/chunks").symlink_to(outside / "chunks", target_is_directory=True)
        (self.project / "project-brain/control").mkdir(parents=True)
        (self.project / "project-brain/control/retrieval-manifests").symlink_to(outside, target_is_directory=True)
        self.write("memory-bank/local/refresh-health.ndjson", "\n".join(json.dumps(line) for line in (
            {"at": "2026-09-30T10:00:00+00:00", "omitted": {"procedural": 0, "semantic": 2, "episodic": 0}},
            {"at": "2026-09-30T11:00:00+00:00", "omitted": {"procedural": 0, "semantic": 0, "episodic": 0}},
            {"at": "2026-09-30T12:00:00+00:00", "omitted": None}, {"at": "not a date"})) + "\n")
        payload = self.read()
        self.assertIsNone(payload["chunks"])
        self.assertNotIn("Outside", json.dumps(payload))
        self.assertEqual([], payload["retrievals"]["items"])
        self.assertEqual([True, False, None], [line["dropped"] for line in payload["health"]["items"]])
        self.write("project-brain/config/runtime.json", json.dumps({"mode": "lightweight"}))
        lightweight = self.read()
        self.assertEqual(("lightweight", None, None), (lightweight["mode"], lightweight["brain"], lightweight["promotions"]))

    def test_check_eligibility_asks_the_installed_runtime_and_a_busy_lock_conflicts(self):
        install_knowledge_fixture(self.project)
        overdue = (self.today - timedelta(days=3)).isoformat()
        self.write("memory-bank/chunks/MEM-20260101-aaaaaaaa-overdue.md", frontmatter({
            "id": "MEM-20260101-aaaaaaaa", "title": "Cobalt allocation rule", "type": "convention", "status": "active",
            "scope": ["application"], "tags": ["allocation"], "created": "2020-01-01", "last_verified": "2025-01-01",
            "review_after": overdue, "sources": ["specs/authority.md"], "supersedes": [], "superseded_by": None},
            "# Cobalt rule\n\nThe cobalt allocation rule requires one owner.\n"))

        def run(action, **fields):
            response = self.manager.run(self.project_id, {"action": action, **fields})
            self.assertTrue(response["ok"], response)
            return response["result"]
        record = run("brain-create", record_type="decision", external_id="D-OBSERVED", title="Cobalt allocation decision",
                     authority="observed", privacy="team", sources=["specs/authority.md"])
        run("brain-update", record_id=record["id"], revision=record["revision"], transition="accepted",
            progress="Allocate exactly one owner to prevent duplicate work")
        payload = self.read()
        self.assertTrue(payload["check_available"])
        result = memory_use.check(self.manager, self.project_id)
        self.assertEqual(({"authority": 1}, 0, {"MEM-20260101-aaaaaaaa": "overdue-review"}),
                         (result["held_back"], result["eligible"], result["skips"]))
        self.assertNotIn(record["id"], json.dumps(result))
        self.assertTrue(self.manager.lock.acquire(blocking=False))
        try:
            with self.assertRaises(KnowledgeBusy):
                memory_use.check(self.manager, self.project_id)
            # Reading never waits for the lock: a background read must not fail a linked launch's freshness check.
            self.assertEqual(1, len(self.read()["chunks"]["items"]))
        finally:
            self.manager.lock.release()

    def test_demo_project_written_by_the_runtime_reads_back_as_its_story(self):
        from tests import harness_memory_demo
        story = harness_memory_demo.build(self.project)
        # Manifests are written directly, so the runtime's own schema must accept every one.
        validated = subprocess.run([sys.executable, "-c", "import json, sys\nfrom pathlib import Path\n"
                                    "root = Path(sys.argv[1]); sys.path.insert(0, str(root / 'memory-bank/scripts'))\n"
                                    "import brain_runtime\npaths = sorted(root.glob('**/retrieval-manifests/*.json'))\n"
                                    "for path in paths: brain_runtime.validate_schema_file(root, 'retrieval-manifest.schema.json', json.loads(path.read_text()))\n"
                                    "print(len(paths))", str(self.project)], capture_output=True, text=True, timeout=120)
        self.assertEqual((0, str(story["retrievals"] + story["merged"])), (validated.returncode, validated.stdout.strip()), validated.stderr)
        payload = self.read(frozenset(story["harness_tasks"]))
        chunks, rows = payload["chunks"]["items"], payload["retrievals"]["items"]
        statuses = {status: sum(item["status"] == status for item in chunks) for status in ("active", "needs-review", "superseded", "archived")}
        self.assertEqual((story["chunks"], story["drafts"]), (len(chunks), statuses["needs-review"]))
        self.assertEqual({"active": 63, "needs-review": 2, "superseded": 3, "archived": 2}, statuses)
        # Two cited files changed after attestation, and two chunks crossed their review date three days ago.
        self.assertEqual(2, sum(item["sources_changed"] is True for item in chunks))
        overdue = [item for item in chunks if item["status"] == "active" and item["review_after"] < payload["today"]]
        self.assertEqual(2, len(overdue))
        self.assertEqual(42, sum(item["auto"] for item in chunks))
        # An overdue chunk leaves retrieval but no longer fails the writes after it, so nothing stalls.
        promotions = payload["promotions"]["items"]
        stalled = [item for item in promotions if item["mode"] == "automatic" and item["status"] in ("proposed", "reviewed")]
        self.assertEqual((42, 0, 7, 1), (sum(item["mode"] == "automatic" and item["status"] == "applied" for item in promotions),
                                          len(stalled), sum(item["mode"] == "human" and item["status"] == "applied" for item in promotions),
                                          sum(item["mode"] == "human" and item["status"] == "proposed" for item in promotions)))
        self.assertEqual(story["records"], len(payload["brain"]["items"]))
        self.assertEqual((story["retrievals"], story["merged"]), (len(rows), payload["retrievals"]["merged"]))
        routes = {route: sum(row["route"] == route for row in rows) for route in ("Claude hook", "Harness", "CLI", "not recorded")}
        self.assertEqual({"Claude hook": story["routes"]["claude"], "Harness": story["routes"]["harness"], "CLI": story["routes"]["cli"],
                          "not recorded": story["routes"]["v1"] + story["routes"]["v2"]}, routes)
        self.assertEqual((story["with_chunks"], story["selections"], story["cuts"]),
                         (sum(bool(row["chunks"]) for row in rows), sum(len(row["chunks"]) for row in rows), sum(len(row["cuts"]) for row in rows)))
        tasks = {}
        for row in rows:
            for identity in row["chunks"]:
                tasks.setdefault(identity, set()).add(row["task"])
        self.assertEqual((story["distinct"], story["reused"]), (len(tasks), sum(len(value) > 1 for value in tasks.values())))
        health = payload["health"]["items"]
        self.assertEqual((story["health"], story["dropped"]), (len(health), sum(line["dropped"] is True for line in health)))
        checked = memory_use.check(self.manager, self.project_id)
        self.assertEqual({"privacy": 3, "authority": 1}, checked["held_back"])
        self.assertEqual({"overdue-review": 2, "needs-review": 2, "superseded": 3, "archived": 2},
                         {reason: list(checked["skips"].values()).count(reason) for reason in set(checked["skips"].values())})

    def test_history_folds_each_retrieval_once_and_outlives_pruned_manifests(self):
        self.chunk("MEM-20260901-aaaaaaaa", "kept")
        chunk = {"path": "memory-bank/chunks/MEM-20260901-aaaaaaaa-kept.md", "category": "durable", "estimated_tokens": 30, "source_hash": "c"}
        policy = {"path": "AGENTS.md", "category": "policy", "estimated_tokens": 40, "source_hash": "p"}
        # Yesterday's local noon: two retrievals an hour apart share a local day at any time of day.
        now = datetime.now().astimezone().replace(hour=12, minute=0, second=0, microsecond=0) - timedelta(days=1)
        moment = lambda days, hours=0: (now - timedelta(days=days, hours=hours)).isoformat()
        first = self.manifest("local", "hook-1", moment(2), [chunk, policy], host="claude", entry_point="hook-context")
        self.manifest("local", "hook-2", moment(2, 1), [policy], [{"path": "memory-bank/chunks/MEM-20260901-aaaaaaaa-kept.md", "reason": "budget"}],
                      host="claude", entry_point="hook-context")
        self.manifest("local", "ancient", moment(memory_use.HISTORY_DAYS + 5), [chunk])
        payload = self.read()
        self.assertNotIn("key", json.dumps(payload["retrievals"]))
        self.assertNotIn("local:", json.dumps(payload))
        days = payload["history"]["days"]
        self.assertEqual(1, len(days))
        day = days[0]
        self.assertEqual((2, 1, 1, 2, 1, {"Claude hook": 2}, {"MEM-20260901-aaaaaaaa": 1}),
                         (day["retrievals"], day["with_chunk"], day["bank"], day["rules"], day["cuts"], day["routes"], day["chunks"]))
        # Reading again folds nothing twice; a new retrieval adds; a pruned manifest's day stays.
        self.assertEqual(days, self.read()["history"]["days"])
        first.unlink()
        self.manifest("local", "hook-3", moment(0), [chunk], host="claude", entry_point="hook-context")
        history = {item["day"]: item for item in self.read()["history"]["days"]}
        self.assertEqual(2, history[day["day"]]["retrievals"])
        self.assertEqual((1, {"MEM-20260901-aaaaaaaa": 1}), (history[memory_use._local_day(moment(0))]["retrievals"],
                                                              history[memory_use._local_day(moment(0))]["chunks"]))
        # Only names inside the horizon are remembered.
        with self.store.lock:
            seen = sorted(row[0] for row in self.store.db.execute("SELECT manifest FROM retrieval_seen"))
        self.assertEqual(["local:hook-1.json", "local:hook-2.json", "local:hook-3.json"], seen)

    def test_a_finished_launch_folds_its_project_history(self):
        (self.project / "memory-bank/chunks").mkdir(parents=True)
        self.manifest("local", "hook-1", datetime.now(timezone.utc).isoformat(), [], host="claude", entry_point="hook-context")
        memory_use.fold_project(self.manager, self.project_id)
        with self.store.lock:
            rows = self.store.db.execute("SELECT bank,data FROM retrieval_days").fetchall()
        self.assertEqual([("memory-bank", 1)], [(row[0], json.loads(row[1])["retrievals"]) for row in rows])

    def test_held_back_sentences_become_rule_names(self):
        self.assertEqual(["authority", "privacy", "cited source changed", "no content", "status", "type", "other"],
                         [memory_use.held_back_reason(text) for text in (
                             "authority is observed, not verified", "privacy private is not allowed for retrieval",
                             "cited source changed since the record was written: specs/a.md",
                             "record carries no content beyond its own title", "finding status open is not promotable",
                             "task records are not promotable", None)])


if __name__ == "__main__":
    unittest.main()
