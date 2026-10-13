"""The merge archive's own files, on POSIX and on native Windows.

A merged chat's frozen history is a private file, state_dir/merges/<task>/context.json,
created, read and removed only through folder handles (harness.chat_merge over
scripts/portable_fs): descriptors on POSIX, NT handles on Windows. The merge tests
(tests/test_harness_merge.py) build a whole in-process Harness store and assert POSIX
modes and links, so they ran only on Linux, and the Windows half of these calls - the
handle-relative folder creation, the delete disposition that removes the file and its
folder, the listing that recovery walks - ran in no CI job. These tests need only the
record table, so the Windows job runs them too.

Run: python3 -m unittest tests.test_harness_merge_archive
"""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import unittest
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import chat_merge, sessions  # noqa: E402

# The record table, as harness.sessions creates it.
RECORDS = '''CREATE TABLE session_merges (
    session_id TEXT PRIMARY KEY, request_id TEXT UNIQUE NOT NULL,
    request_hash TEXT NOT NULL, bundle TEXT NOT NULL,
    summary TEXT NOT NULL, context TEXT NOT NULL)'''


class Store:
    """What the archive functions use of a Harness store: its state folder, records and lock."""

    def __init__(self, state_dir):
        self.state_dir = state_dir
        self.db = sqlite3.connect(':memory:')
        self.db.row_factory = sqlite3.Row
        self.db.execute(RECORDS)
        self.lock = threading.RLock()


def link_folder(link, target):
    """A link to a folder: a symbolic link on POSIX, a junction on Windows (no privilege needed)."""
    if os.name == 'nt':
        return os.system('cmd /d /c mklink /J "' + str(link) + '" "' + str(target) + '" >NUL') == 0
    link.symlink_to(target, target_is_directory=True)
    return True


class MergeArchiveFilesTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.state = self.base / 'state'
        self.state.mkdir()
        self.store = Store(self.state)
        self.addCleanup(self.store.db.close)

    def prepared(self, text='Use SQL'):
        source = {'id': str(uuid.uuid4()), 'title': 'Source', 'provider': 'codex', 'branch': 'main',
                  'status': 'completed', 'event_watermark': 3,
                  'messages': [{'event_id': 1, 'role': 'user', 'text': text},
                               {'event_id': 2, 'role': 'assistant', 'text': 'Decision: ' + text}]}
        source['sha256'] = hashlib.sha256(chat_merge.encoded(source)).hexdigest()
        bundle = {'version': 1, 'captured_at': '2026-10-10T00:00:00Z', 'project_id': 'project',
                  'conflicts': 'not-evaluated', 'sources': [source, {**source, 'id': str(uuid.uuid4())}]}
        request_id = str(uuid.uuid4())
        return {'bundle': bundle, 'request_id': request_id,
                'request_hash': hashlib.sha256(request_id.encode()).hexdigest()}

    def persist(self, prepared=None):
        sid = str(uuid.uuid4())
        prepared = prepared or self.prepared()
        chat_merge.persist(self.store, sid, prepared)
        return sid, chat_merge.encoded(prepared['bundle'])

    def test_an_archive_is_written_read_and_removed_through_folder_handles(self):
        sid, body = self.persist()
        path = chat_merge.archive_path(self.store, sid)
        self.assertEqual(body, path.read_bytes())
        self.assertEqual(body, chat_merge.read(self.store, sid))
        self.assertIn(json.dumps(str(path)), chat_merge.launch_context(self.store, sid))
        self.assertEqual([str(path.parent)], chat_merge.directories(self.store, sid))
        if os.name == 'posix':
            self.assertEqual(0o600, path.stat().st_mode & 0o777)
            self.assertEqual(0o700, path.parent.stat().st_mode & 0o777)
        chat_merge.cleanup(self.store, sid)
        self.assertFalse(path.parent.exists())
        self.assertIsNone(chat_merge.read(self.store, sid))
        self.assertEqual([], chat_merge.directories(self.store, sid))
        # Removing what is already gone is no error.
        chat_merge.cleanup(self.store, sid)

    def test_an_archive_that_changed_or_has_a_second_name_is_refused(self):
        sid, body = self.persist()
        path = chat_merge.archive_path(self.store, sid)
        second = self.base / 'second-name.json'
        os.link(path, second)
        self.assertIsNone(chat_merge.read(self.store, sid))
        with self.assertRaises(sessions.SessionError):
            chat_merge.launch_context(self.store, sid)
        second.unlink()
        self.assertEqual(body, chat_merge.read(self.store, sid))
        path.write_text('{}', encoding='utf-8')
        with self.assertRaises(sessions.SessionError):
            chat_merge.launch_context(self.store, sid)

    def test_a_record_that_fails_leaves_no_archive_and_keeps_its_error(self):
        prepared = self.prepared()
        self.persist(prepared)
        # The same request id twice: the record's UNIQUE constraint refuses it
        # after the archive was written, and the archive goes with it.
        sid = str(uuid.uuid4())
        with self.assertRaises(sqlite3.IntegrityError):
            chat_merge.persist(self.store, sid, {**self.prepared(), 'request_id': prepared['request_id']})
        self.assertFalse(chat_merge.archive_path(self.store, sid).parent.exists())
        self.assertEqual(1, len(os.listdir(self.state / 'merges')))

    def test_recovery_removes_only_unreferenced_plain_archives(self):
        kept, _ = self.persist()
        merges = self.state / 'merges'
        orphan = merges / str(uuid.uuid4())
        orphan.mkdir()
        (orphan / chat_merge.ARCHIVE).write_text('private history', encoding='utf-8')
        empty = merges / str(uuid.uuid4())
        empty.mkdir()
        foreign = merges / 'manual-backup'
        foreign.mkdir()
        (foreign / chat_merge.ARCHIVE).write_text('keep', encoding='utf-8')
        crowded = merges / str(uuid.uuid4())
        crowded.mkdir()
        (crowded / chat_merge.ARCHIVE).write_text('{}', encoding='utf-8')
        (crowded / 'notes.txt').write_text('keep', encoding='utf-8')
        outside = self.base / 'outside'
        outside.mkdir()
        (outside / chat_merge.ARCHIVE).write_text('not the Harness state', encoding='utf-8')
        linked = merges / str(uuid.uuid4())
        has_link = link_folder(linked, outside)

        chat_merge.recover(self.store)

        self.assertFalse(orphan.exists())
        self.assertFalse(empty.exists())
        self.assertTrue(chat_merge.archive_path(self.store, kept).is_file())
        self.assertEqual('keep', (foreign / chat_merge.ARCHIVE).read_text(encoding='utf-8'))
        self.assertEqual(['context.json', 'notes.txt'], sorted(os.listdir(crowded)))
        if has_link:
            self.assertTrue(os.path.lexists(linked))
        self.assertEqual('not the Harness state', (outside / chat_merge.ARCHIVE).read_text(encoding='utf-8'))

    def test_a_merge_folder_behind_a_link_is_never_written_through(self):
        outside = self.base / 'outside'
        outside.mkdir()
        if not link_folder(self.state / 'merges', outside):
            self.skipTest('this Windows runner cannot create a junction')
        with self.assertRaises(sessions.SessionError):
            self.persist()
        self.assertEqual([], os.listdir(outside))
        self.assertEqual(0, self.store.db.execute('SELECT count(*) FROM session_merges').fetchone()[0])
        chat_merge.recover(self.store)
        self.assertTrue(os.path.lexists(self.state / 'merges'))


if __name__ == '__main__':
    unittest.main()
