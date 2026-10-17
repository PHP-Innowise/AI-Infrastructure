"""A working task whose uncommitted record was swept away comes back.

`git stash -u` or `git clean` removes a task record the Stop hook created and
nobody committed, while the local binding still names it. Every later Stop
then failed with "record not found" and the prompt hook delivered nothing at
all; now the read serves the turn as for a task not recorded yet, saying why,
and the next checkpoint records the task again.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
EDITION = ROOT / 'PHP Core'
TASK = 'feature/plover-orphan'


@unittest.skipUnless(shutil.which('git'), 'Git')
class OrphanedTaskTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='plover-orphan-')
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root = self.base / 'project'
        self.root.mkdir()
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        self.env.update(HOME=str(self.base / 'home'), XDG_CONFIG_HOME=str(self.base / 'xdg'),
                        TMPDIR=str(self.base), PYTHONDONTWRITEBYTECODE='1', GIT_OPTIONAL_LOCKS='0')
        (self.base / 'home').mkdir()
        for rel in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/config',
                    'project-brain/schemas', 'project-brain/templates'):
            shutil.copytree(EDITION / rel, self.root / rel, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        (self.root / '.gitignore').write_text('memory-bank/local/\nproject-brain/local/\n', encoding='utf-8')
        self.git('init', '-q', '-b', TASK)
        self.git('config', 'user.email', 'plover@example.invalid')
        self.git('config', 'user.name', 'Plover')
        self.git('add', '-A')
        self.git('commit', '-q', '-m', 'fixture')

    def git(self, *args):
        subprocess.run(['git', '-C', str(self.root), *args], env=self.env, check=True, capture_output=True, timeout=60)

    def cli(self, *args, check=True):
        return subprocess.run([sys.executable, str(self.root / 'memory-bank/scripts/context.py'), *args],
                              cwd=self.root, env=self.env, check=check, capture_output=True, text=True, timeout=120)

    def work(self, name, check=True):
        (self.root / 'src').mkdir(exist_ok=True)
        (self.root / 'src' / name).write_text('<?php\n', encoding='utf-8')
        return self.cli('turn', '--task-id', TASK, '--flush-after', '1', check=check)

    def tasks(self):
        return sorted((self.root / 'project-brain/dynamic/tasks').glob('*.md'))

    def test_a_swept_task_is_served_as_unrecorded_and_recorded_again(self):
        self.work('First.php')
        [record] = self.tasks()
        uuid = record.stem
        record.unlink()  # what `git stash -u` does to an uncommitted record
        read = self.cli('refresh', '--query', 'What changed in the plover sources?', '--task-id', TASK,
                        '--ephemeral', '--json')
        packet = json.loads(read.stdout)
        capsule = packet['capsule']
        self.assertEqual('warming', capsule.get('kind'))
        self.assertTrue(any(uuid in warning and 'is gone' in warning for warning in capsule['warnings']),
                        capsule['warnings'])
        flushed = self.work('Second.php')
        self.assertEqual(0, flushed.returncode, flushed.stderr)
        [again] = self.tasks()
        self.assertNotEqual(uuid, again.stem)
        read = json.loads(self.cli('refresh', '--query', 'What changed in the plover sources?', '--task-id', TASK,
                                   '--ephemeral', '--json').stdout)
        self.assertEqual(again.stem, read['capsule']['task_uuid'])
        self.assertNotEqual('warming', read['capsule'].get('kind'))

    def test_a_record_that_is_only_invalid_keeps_its_binding(self):
        self.work('First.php')
        [record] = self.tasks()
        record.write_text(record.read_text(encoding='utf-8').replace('"status": "active"', '"status": "bogus"'),
                          encoding='utf-8')
        flushed = self.work('Second.php', check=False)
        self.assertNotEqual(0, flushed.returncode)
        self.assertEqual([record], self.tasks())  # no twin created beside it


    def test_an_unreadable_record_is_named_relatively_and_never_duplicated(self):
        self.work('First.php')
        [record] = self.tasks()
        record.write_bytes(record.read_bytes().replace(b'## Goal', b'## Goal \xff\xfe', 1))
        read = self.cli('refresh', '--query', 'What changed in the plover sources?', '--task-id', TASK,
                        '--ephemeral', '--json')
        warnings = json.loads(read.stdout)['capsule']['warnings']
        relative = 'project-brain/dynamic/tasks/' + record.name
        self.assertTrue(any(relative in warning and 'invalid UTF-8' in warning for warning in warnings), warnings)
        self.assertNotIn(str(self.base), read.stdout)
        flushed = self.work('Second.php', check=False)
        self.assertNotEqual(0, flushed.returncode)
        self.assertIn(relative, flushed.stderr)
        self.assertNotIn(str(self.base), flushed.stderr)
        self.assertEqual([record], self.tasks())  # the unreadable record is not replaced by a twin
    def damage(self):
        database = self.root / 'memory-bank/local/context.db'
        data = bytearray(database.read_bytes())
        for offset in range(4096, len(data), 512):
            data[offset:offset + 16] = b'\xde\xad' * 8
        database.write_bytes(bytes(data))

    def test_a_damaged_local_database_is_moved_aside_and_rebuilt(self):
        # "database disk image is malformed" on every prompt until a person
        # moved the file: in governed mode all it holds is derived.
        self.work('First.php')
        [record] = self.tasks()
        self.damage()
        read = self.cli('refresh', '--query', 'What changed in the plover sources?', '--task-id', TASK,
                        '--ephemeral', '--json')
        capsule = json.loads(read.stdout)['capsule']
        self.assertEqual(record.stem, capsule['task_uuid'])
        self.assertTrue(any('was damaged' in warning and 'context.db.quarantine-' in warning
                            for warning in capsule['warnings']), capsule['warnings'])
        local = self.root / 'memory-bank/local'
        self.assertEqual(1, len(list(local.glob('context.db.quarantine-*'))))
        self.assertTrue((local / 'context.db').is_file())
        # The next checkpoint binds the record again; no twin is made.
        flushed = self.work('Second.php')
        self.assertEqual(0, flushed.returncode, flushed.stderr)
        self.assertEqual([record], self.tasks())
        again = json.loads(self.cli('refresh', '--query', 'What changed in the plover sources?', '--task-id',
                                    TASK, '--ephemeral', '--json').stdout)['capsule']
        self.assertFalse(any('was damaged' in warning for warning in again['warnings']))

    def test_a_damaged_lightweight_database_is_left_for_a_person(self):
        # Lightweight working tasks live only in that file: it is not moved.
        self.cli('--mode', 'lightweight', 'refresh', '--query', 'plover', '--task-id', TASK, '--ephemeral')
        self.damage()
        self.cli('--mode', 'lightweight', 'refresh', '--query', 'plover', '--task-id', TASK, '--ephemeral',
                 check=False)
        self.assertEqual([], list((self.root / 'memory-bank/local').glob('context.db.quarantine-*')))


if __name__ == '__main__':
    unittest.main()
