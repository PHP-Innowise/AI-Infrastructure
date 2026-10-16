"""Fresh Git-index-only membership regressions; all Git writes stay in temporary repos."""
from __future__ import annotations

import importlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / 'PHP Core/memory-bank/scripts'
ELM = 'app/elm.py'
OAK = 'app/oak.py'
IMAGES = {ELM: '# Elm Orchard allocation\nOWNER_COUNT = 1\n',
          OAK: '# Oak Orchard allocation\nOWNER_COUNT = 1\n'}


class TaskMembershipIndexTests(unittest.TestCase):
    def setUp(self):
        clean = {key: value for key, value in os.environ.items()
                 if not key.startswith(('GIT_', 'ACCELERATOR_'))}
        environment = mock.patch.dict(os.environ, clean, clear=True)
        environment.start(); self.addCleanup(environment.stop)
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.brain = importlib.import_module('brain_runtime')
            self.context = importlib.import_module('context')
            self.r = importlib.import_module('context_retrieval')
            self.roots = importlib.import_module('workspace_roots')
        finally:
            sys.path.pop(0)
        folder = tempfile.TemporaryDirectory(prefix='task-index-orchard-')
        self.addCleanup(folder.cleanup)
        self.base = Path(folder.name)
        self.repo = self.base / 'project'; self.repo.mkdir()
        self.git('init', '--quiet')
        self.prepare(self.repo)
        self.record = self.brain.create_record(self.repo, 'task', 'ORCHARD-INDEX',
                'Orchard allocation', [ELM, OAK], [], owner='alice', privacy='public')
        self.path = self.brain.dynamic_path(self.repo, self.record).relative_to(self.repo).as_posix()
        self.connection = self.context.connect(self.repo / 'memory-bank/local/context.db')
        self.addCleanup(self.connection.close)

    def prepare(self, repository):
        for part in ('project-brain/config', 'project-brain/schemas'):
            shutil.copytree(ROOT / 'PHP Core' / part, repository / part)
        (repository / 'memory-bank/chunks').mkdir(parents=True)
        for name, image in IMAGES.items():
            target = repository / name; target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(image, encoding='utf-8')
        (repository / 'specs').mkdir()
        (repository / 'specs/dune.md').write_text('# Dune gateway\n\nA dune boundary needs one owner.\n')
        for number in range(18):
            (repository / f'specs/mist-{number}.md').write_text(
                f'# Mist {number}\n\nIndependent scenery paragraph {number} about fog.\n')
        config = repository / 'project-brain/config/runtime.json'
        data = json.loads(config.read_text()); data['automatic_promotion'] = False
        data['retrieval_gate'] = 'off'; config.write_text(json.dumps(data))

    def git(self, *args, check=True, repository=None):
        return subprocess.run(['git', '-C', str(repository or self.repo), '-c', 'core.autocrlf=false',
                               *args], capture_output=True, check=check)

    def stage(self, *paths):
        self.git('add', '--', *paths)

    def index(self, incremental=True):
        return self.context.index_repository(self.connection, self.repo, incremental=incremental)

    def edges(self, kind='file'):
        return {row[0] for row in self.connection.execute(
            'SELECT ref_path FROM document_links WHERE path = ? AND ref_kind = ?', (self.path, kind))}

    def snapshot(self):
        return ((self.repo / self.path).read_bytes(),
                tuple((self.repo / path).read_bytes() for path in IMAGES))

    def cache(self):
        return json.loads(self.r.load_index_state(self.connection).get(self.r.FILE_LINK_TRACKED_KEY, '{}'))

    def batches(self, calls):
        return [call for call in calls.call_args_list if 'cat-file' in call.args[0]]

    def test_index_only_stage_and_remove_refresh_full_and_incremental(self):
        self.stage(ELM); before = self.snapshot()
        self.index(); self.assertEqual({ELM}, self.edges())
        self.stage(OAK); self.index(incremental=False)
        self.assertEqual({ELM, OAK}, self.edges())
        self.git('update-index', '--force-remove', ELM); self.index()
        self.assertEqual({OAK}, self.edges())
        self.index(); self.assertEqual({OAK}, self.edges())
        self.assertEqual(before, self.snapshot())
        self.assertEqual(1, self.brain.find_record(self.repo, self.record['id'])[1]['revision'])

    def test_warm_reuse_and_task_revision_cost(self):
        self.stage(ELM)
        with mock.patch.object(self.r.subprocess, 'run', wraps=subprocess.run) as calls:
            self.index(); self.index()
            self.assertEqual(1, len(self.batches(calls)))
            metadata = [c for c in calls.call_args_list if '--shared-index-path' in c.args[0]]
            self.assertEqual(3, len(metadata))  # cold2, warm1, separate from other Git operations
            record = self.brain.update_record(self.repo, self.record['id'], expected_revision=1,
                progress='Orchard reviewed.', next_steps=[], files=[], sources=[], actor='alice')
            self.assertEqual(2, record['revision'])
            self.index(); self.assertEqual(2, len(self.batches(calls)))
        self.assertEqual([ELM], self.cache()['tracked'])

    def test_unstaged_content_change_keeps_membership_and_warm_reuse(self):
        self.stage(ELM); self.index()
        (self.repo / ELM).write_text(IMAGES[ELM] + '# Edited working content\n')
        with mock.patch.object(self.r.subprocess, 'run', wraps=subprocess.run) as calls:
            self.index(); self.assertEqual(0, len(self.batches(calls)))
        self.assertEqual({ELM}, self.edges())

    def test_alternate_index_switch_absolute_and_relative(self):
        self.stage(ELM); self.index()
        for relative in (False, True):
            alternate = self.repo / ('alternate-relative' if relative else 'alternate-absolute')
            selection = alternate.name if relative else str(alternate)
            with mock.patch.dict(os.environ, {'GIT_INDEX_FILE': selection}):
                self.stage(OAK); self.index(); self.assertEqual({OAK}, self.edges())
            self.index(); self.assertEqual({ELM}, self.edges())

    def test_gitfile_indirection_uses_actual_index(self):
        external = self.base / 'git-data'
        self.git('init', '--quiet', '--separate-git-dir', str(external))
        self.assertTrue((self.repo / '.git').is_file())
        self.stage(ELM); self.index()
        self.stage(OAK); self.index()
        self.assertEqual({ELM, OAK}, self.edges())
        self.assertEqual(str(external / 'index'), self.r._file_link_index_signature(self.repo)[3][0])

    def test_linked_worktree_index_is_separate(self):
        # Only this temporary repository receives a fixture commit to create a real worktree.
        self.stage(ELM)
        self.git('-c', 'user.name=Orchard fixture', '-c', 'user.email=orchard@example.invalid',
                 'commit', '--quiet', '-m', 'public orchard worktree fixture')
        linked = self.base / 'linked'
        self.git('worktree', 'add', '--quiet', '--detach', str(linked))
        (linked / OAK).write_text(IMAGES[OAK])
        self.git('add', '--', OAK, repository=linked)
        original = self.r._file_link_index_signature(self.repo)
        other = self.r._file_link_index_signature(linked)
        self.assertNotEqual(original[3][0], other[3][0])
        self.assertEqual({ELM}, self.r._tracked_paths(self.repo, [ELM, OAK]))
        self.assertEqual({ELM, OAK}, self.r._tracked_paths(linked, [ELM, OAK]))

    def test_attached_subproject_frame_and_index_only_change(self):
        sub = self.repo / 'nested'; sub.mkdir()
        self.prepare(sub)
        with mock.patch.dict(os.environ, {'ACCELERATOR_HOME': str(self.roots.TOOLING_ROOT),
                'ACCELERATOR_STATE_DIR': str(self.repo), 'ACCELERATOR_PROJECT_DIR': str(sub)}):
            self.git('add', '--', 'nested/' + OAK)
            self.index(); self.assertEqual({OAK}, self.edges())
            self.git('update-index', '--force-remove', 'nested/' + OAK)
            self.index(); self.assertEqual(set(), self.edges())
            self.assertEqual(str(sub), self.r._file_link_index_signature(self.repo)[1])
        self.stage(ELM); self.index(); self.assertEqual({ELM}, self.edges())

    def test_effective_worktree_frame_changes_without_index_or_task_changes(self):
        sub = self.repo / 'nested'; sub.mkdir(); self.prepare(sub)
        self.git('add', '--', 'nested/' + ELM)
        before = (self.repo / '.git/index').read_bytes(); task_before = (self.repo / self.path).read_bytes()
        attached = {'ACCELERATOR_HOME': str(self.roots.TOOLING_ROOT),
                    'ACCELERATOR_STATE_DIR': str(self.repo), 'ACCELERATOR_PROJECT_DIR': str(sub)}
        with mock.patch.dict(os.environ, attached):
            self.index(); self.assertEqual({ELM}, self.edges())
            initial = self.r._file_link_index_signature(self.repo)
            with mock.patch.dict(os.environ, {'GIT_WORK_TREE': str(sub)}):
                self.assertNotEqual(initial, self.r._file_link_index_signature(self.repo))
                self.index(); self.assertEqual(set(), self.edges())
            self.index(); self.assertEqual({ELM}, self.edges())
            self.git('config', 'core.worktree', str(sub))
            self.index(); self.assertEqual(set(), self.edges())
        self.assertEqual(before, (self.repo / '.git/index').read_bytes())
        self.assertEqual(task_before, (self.repo / self.path).read_bytes())

    def test_sha256_and_version4_index(self):
        shutil.rmtree(self.repo / '.git'); self.git('init', '--quiet', '--object-format=sha256')
        self.stage(ELM); self.git('update-index', '--index-version=4'); self.index()
        signature = self.r._file_link_index_signature(self.repo)
        self.assertEqual('sha256', signature[2]); self.assertEqual(64, len(signature[3][-1]))
        self.assertEqual('00000004', signature[3][2][8:16])
        self.assertEqual({ELM}, self.edges())

    def test_split_index_shared_timestamp_is_not_generation(self):
        self.stage(ELM); self.git('update-index', '--split-index')
        signature = self.r._file_link_index_signature(self.repo)
        self.assertIsNotNone(signature); self.assertIsNotNone(signature[4])
        self.index(); shared = Path(signature[4][0]); image = shared.stat()
        os.utime(shared, ns=(image.st_atime_ns, image.st_mtime_ns + 10_000_000))
        self.assertEqual(signature, self.r._file_link_index_signature(self.repo))
        with mock.patch.object(self.r.subprocess, 'run', wraps=subprocess.run) as calls:
            self.index(); self.assertEqual(0, len(self.batches(calls)))
        self.stage(OAK); self.index(); self.assertEqual({ELM, OAK}, self.edges())

    def test_split_missing_shared_bypasses_cache_and_failed_git_removes_edges(self):
        self.stage(ELM); self.git('update-index', '--split-index'); self.index()
        shared = Path(self.r._file_link_index_signature(self.repo)[4][0]); shared.unlink()
        self.assertIsNone(self.r._file_link_index_signature(self.repo))
        self.index(); self.assertEqual(set(), self.edges()); self.assertEqual({}, self.cache())

    def test_skiphash_zero_footer_uses_fresh_membership_without_caching(self):
        self.git('config', 'index.skipHash', 'true'); self.stage(ELM)
        self.assertIsNone(self.r._file_link_index_signature(self.repo))
        with mock.patch.object(self.r.subprocess, 'run', wraps=subprocess.run) as calls:
            self.index(); self.stage(OAK); self.index()
            self.assertEqual(2, len(self.batches(calls)))
        self.assertEqual({ELM, OAK}, self.edges()); self.assertEqual({}, self.cache())

    def test_unknown_metadata_uses_fresh_batch_and_clears_old_proof(self):
        self.stage(ELM); self.index(); self.git('update-index', '--force-remove', ELM)
        with mock.patch.object(self.r, '_file_link_index_signature', return_value=None), \
                mock.patch.object(self.r.subprocess, 'run', wraps=subprocess.run) as calls:
            self.index(); self.assertEqual(1, len(self.batches(calls)))
        self.assertEqual(set(), self.edges()); self.assertEqual({}, self.cache())

    def test_detected_race_or_post_unknown_suppresses_rows_not_citations(self):
        self.brain.create_record(self.repo, 'finding', 'ORCHARD-CITATION',
            'Dune boundary citation', [], [ELM], owner='alice', privacy='public')
        self.stage(ELM)
        for final in (None, ('different native generation',)):
            with self.subTest(final=final):
                self.r.store_index_state(self.connection, {self.r.FILE_LINK_TRACKED_KEY: '{}'})
                with mock.patch.object(self.r, '_file_link_index_signature', side_effect=[('before',), final]):
                    self.index()
                self.assertEqual(set(), self.edges()); self.assertEqual({}, self.cache())
                self.assertGreater(self.connection.execute(
                    "SELECT COUNT(*) FROM document_links WHERE ref_kind='source' AND ref_path=?", (ELM,)).fetchone()[0], 0)

    def test_failed_batch_never_publishes_cache(self):
        self.stage(ELM); self.index(); self.stage(OAK)
        with mock.patch.object(self.r, '_tracked_paths', return_value=None):
            self.index()
        self.assertEqual(set(), self.edges()); self.assertEqual({}, self.cache())

    def test_legacy_malformed_and_oversized_cache_cannot_reuse(self):
        self.stage(ELM); self.index(); good = self.cache()
        corrupt = [{'key': good['key'], 'tracked': None},
                   {'key': good['key'], 'tracked': [ELM, ELM]},
                   {'key': good['key'], 'tracked': [7]},
                   {'key': good['key'], 'tracked': ['foreign.py']},
                   {'key': 'legacy-key', 'tracked': [OAK]}]
        for raw in ['{' , '[' * 1500, ' ' * 20000, *(json.dumps(item) for item in corrupt)]:
            with self.subTest(cache=raw[:30]):
                self.r.store_index_state(self.connection, {self.r.FILE_LINK_TRACKED_KEY: raw})
                with mock.patch.object(self.r.subprocess, 'run', wraps=subprocess.run) as calls:
                    self.index(); self.assertEqual(1, len(self.batches(calls)))
                self.assertEqual({ELM}, self.edges())

    def test_metadata_timeout_bad_output_and_unsupported_environment_are_unknown(self):
        self.stage(ELM)
        with mock.patch.object(self.r.subprocess, 'run', side_effect=subprocess.TimeoutExpired('git', 3)):
            self.assertIsNone(self.r._file_link_index_signature(self.repo))
        for output in (b'--path-format=absolute\n', b'\xff\n', b'x' * 20000):
            with mock.patch.object(self.r.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, output)):
                self.assertIsNone(self.r._file_link_index_signature(self.repo))
        for name in ('GIT_OBJECT_DIRECTORY', 'GIT_NAMESPACE', 'GIT_CONFIG_COUNT'):
            with mock.patch.dict(os.environ, {name: ''}):
                self.assertIsNone(self.r._file_link_index_signature(self.repo))

    def test_missing_malformed_or_nonregular_index_has_no_reuse_proof(self):
        self.assertIsNone(self.r._file_link_index_signature(self.repo))
        self.stage(ELM); image = self.repo / '.git/index'; original = image.read_bytes()
        for payload in (b'', b'DIRC' + (9).to_bytes(4, 'big') + original[8:],
                        original[:12] + b'\0' * 20):
            image.write_bytes(payload)
            self.assertIsNone(self.r._git_index_image(image, 20))
        image.unlink(); image.mkdir()
        self.assertIsNone(self.r._git_index_image(image, 20))

    def test_replacement_only_loose_and_packed_changes_invalidate_membership(self):
        self.stage(ELM); self.index()
        blob = self.git('rev-parse', ':' + ELM).stdout.decode().strip()
        tree = self.git('write-tree').stdout.decode().strip()
        before = (self.repo / '.git/index').read_bytes()
        self.git('replace', '-f', blob, tree)
        self.index(); self.assertEqual(set(), self.edges())
        self.assertEqual({}, self.cache()); self.assertEqual(before, (self.repo / '.git/index').read_bytes())
        self.git('pack-refs', '--all', '--prune')
        replacement = self.repo / '.git/refs/replace'
        if replacement.exists(): replacement.rmdir()
        self.index(); self.assertEqual(set(), self.edges()); self.assertEqual([], self.cache()['tracked'])
        self.git('replace', '-d', blob); self.index(); self.assertEqual({ELM}, self.edges())
        self.assertEqual(before, (self.repo / '.git/index').read_bytes())

    def test_protected_untracked_and_gitlink_paths_cannot_be_file_edges(self):
        record = self.brain.update_record(self.repo, self.record['id'], expected_revision=1,
            progress=None, next_steps=[], files=['.env', 'var/cache/a.py', '../outside.py', 'app/submodule'],
            sources=[], actor='alice')
        self.assertEqual(2, record['revision'])
        for path in ('.env', 'var/cache/a.py'):
            member = self.repo / path; member.parent.mkdir(parents=True, exist_ok=True); member.write_text('public fixture\n')
        self.stage('.env', 'var/cache/a.py')
        tree = self.git('write-tree').stdout.decode().strip()
        self.git('update-index', '--add', '--cacheinfo', '160000,' + tree + ',app/submodule')
        self.index(); self.assertEqual(set(), self.edges())

    def test_refresh_membership_reaches_actual_automatic_reader_and_preserves_citations(self):
        current = self.brain.create_record(self.repo, 'task', 'ORCHARD-CURRENT',
            'Inspect dune gateway', [ELM, OAK], [], owner='alice', privacy='public')
        own = {self.brain.dynamic_path(self.repo, current).relative_to(self.repo).as_posix()}
        self.brain.create_record(self.repo, 'finding', 'ORCHARD-CITATION',
            'Dune boundary citation', [], [ELM], owner='alice', privacy='public')
        self.index()
        def selected(seed, origin):
            candidates, _ = self.r._automatic_link_candidates(self.connection, self.repo,
                self.brain.load_config(self.repo), [(seed, origin)], [], own_paths=own, coverage={self.path: 1})
            return {item['path']: item['selection'] for item in candidates}
        self.assertNotIn(self.path, selected(OAK, 'prompt'))
        self.stage(OAK); self.index()
        self.assertIn(self.path, selected(OAK, 'prompt'))
        self.assertEqual('prompt-link', selected(OAK, 'prompt')[self.path])
        self.assertEqual('touch-link', selected(OAK, 'touch')[self.path])
        self.assertEqual([OAK], self.r.prompt_path_seeds(self.connection, self.repo, 'Inspect app/oak.py'))
        self.git('update-index', '--force-remove', OAK); self.index(incremental=False)
        self.assertNotIn(self.path, selected(OAK, 'prompt'))
        self.assertEqual([], self.r.prompt_path_seeds(self.connection, self.repo, 'Inspect app/oak.py'))
        self.assertGreater(self.connection.execute(
            "SELECT COUNT(*) FROM document_links WHERE ref_kind='source' AND ref_path=?", (ELM,)).fetchone()[0], 0)


    def test_sql_failure_rolls_back_file_links_and_cache_proof_together(self):
        self.stage(ELM); self.index(); previous = self.cache()
        self.stage(OAK)
        with mock.patch.object(self.r, 'store_index_state', side_effect=sqlite3.OperationalError('public injected SQL failure')):
            with self.assertRaises(sqlite3.OperationalError):
                self.index()
        self.assertEqual({ELM}, self.edges()); self.assertEqual(previous, self.cache())
        self.index(); self.assertEqual({ELM, OAK}, self.edges())

    def test_no_file_candidates_perform_no_git_membership_operations(self):
        member = self.repo / self.path; member.unlink()
        with mock.patch.object(self.r.subprocess, 'run', wraps=subprocess.run) as calls:
            self.index()
        self.assertEqual([], self.batches(calls))
        self.assertEqual([], [c for c in calls.call_args_list if '--shared-index-path' in c.args[0]])

    def test_full_capsule_render_and_manifest_follow_index_only_membership(self):
        source = 'app/elm_gate.py'; image = '# Elm transfer\nOWNER_LIMIT = 1\n'
        (self.repo / source).write_text(image)
        target = self.brain.create_record(self.repo, 'task', 'ELM-ORCHARD',
            'Elm transfer allocation', [source], [], owner='alice', privacy='public')
        target = self.brain.update_record(self.repo, target['id'], expected_revision=1,
            progress='Check owner allocation.', next_steps=[], files=[], sources=[], actor='alice')
        current = self.brain.create_record(self.repo, 'task', 'DUNE-CURRENT',
            'Dune gateway release', [], [], owner='alice', privacy='public')
        self.context.bind_governed_task(self.connection, current['external_id'], current['id'], 1)
        path = self.brain.dynamic_path(self.repo, target).relative_to(self.repo).as_posix()
        before = (self.repo / path).read_bytes()
        for step, delivered in enumerate((False, True, False)):
            if step == 1: self.stage(source)
            if step == 2: self.git('update-index', '--force-remove', source)
            capsule = self.context.assemble_capsule(self.connection, self.repo,
                mode='governed', query='dune gateway', prompt='Review dune gateway and app/elm_gate.py',
                task_id='DUNE-CURRENT', limit=3, ephemeral=True, refresh_index=True,
                automatic_links=True, render=True, gate_mode='off', entry_point='refresh',
                session_id='orchard-capture-' + str(step))
            selected = {item['path']: item for item in capsule['semantic']}
            manifest = json.loads((self.repo / capsule['manifest']).read_text())
            persisted = {item['path']: item for item in manifest['selected']}
            rendered = '\n'.join(self.context.render_capsule_lines(
                capsule, self.context.capsule_excerpts(self.connection, capsule)))
            self.assertEqual(delivered, path in selected)
            self.assertEqual(delivered, path in persisted)
            self.assertEqual(delivered, path in rendered)
            self.assertIn('specs/dune.md', selected)
            if delivered:
                self.assertEqual('prompt-link', selected[path]['selection'])
                self.assertEqual(source, selected[path]['via'])
                self.assertEqual('prompt-link', persisted[path]['selection'])
                self.assertIsNone(persisted[path]['rank'])
                self.assertIn('Goal: Elm transfer allocation.', rendered)
                self.assertIn('Files: app/elm_gate.py', rendered)
                self.assertIn('linked through ' + source, rendered)
            self.assertEqual(before, (self.repo / path).read_bytes())
            self.assertEqual(image, (self.repo / source).read_text())


if __name__ == '__main__':
    unittest.main()
