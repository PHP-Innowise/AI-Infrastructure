"""Local Git delivery checks; all repositories are disposable and providers stay idle."""

import json
import os
from pathlib import Path
import shutil
import shlex
import threading
import time
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import delivery, sessions


@unittest.skipUnless(shutil.which('git'), 'Delivery tests require Git')
class DeliveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / 'project'
        self.project.mkdir()
        self.git('init', '-q')
        self.git('symbolic-ref', 'HEAD', 'refs/heads/main')
        self.git('config', 'user.name', 'Delivery fixture')
        self.git('config', 'user.email', 'fixture@example.invalid')
        for name in ('selected.txt', 'staged.txt', 'unstaged.txt', 'mixed.txt'):
            (self.project / name).write_text('baseline\n')
        self.git('add', '--', 'selected.txt', 'staged.txt', 'unstaged.txt', 'mixed.txt')
        self.git('commit', '-qm', 'Fixture baseline')
        self.base = self.git('rev-parse', 'HEAD')
        discovery = patch.object(sessions.providers, 'discover_providers', return_value=[{
            'id': 'codex', 'available': True, 'executable': '/never-launched/fixture-provider',
        }])
        discovery.start()
        self.addCleanup(discovery.stop)
        worker = patch.object(sessions.Sessions, '_worker', return_value=None)
        worker.start()
        self.addCleanup(worker.stop)
        self.store = sessions.Sessions(self.root / 'state', [self.project])
        self.addCleanup(self.store.close)
        self.project_id = next(iter(self.store.projects))
        self.sid = self.create(workspace='worktree', worktree_branch='codex/delivery-fixture')
        self.worktree = Path(self.store.get(self.sid)['project_path'])

    def git(self, *arguments, cwd=None):
        environment = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
        environment.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT='0')
        return subprocess.run(['git', '-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgSign=false',
                               '-C', str(cwd or self.project), *arguments], env=environment,
                              stdin=subprocess.DEVNULL, check=True, capture_output=True,
                              text=True).stdout.strip()

    def create(self, **options):
        session = self.store.create({'project_id': self.project_id, 'provider': 'codex',
                                     'prompt': 'Offline delivery fixture', 'project_context': False,
                                     **options})
        self.store.jobs.get_nowait()
        self.store.jobs.task_done()
        self.store.results.baseline(session)
        self.store._status(session['id'], 'completed')
        return session['id']

    def preview_commit(self, paths=None, message='Deliver selected fixture files'):
        current = self.store.delivery.get(self.sid)
        return self.store.delivery.preview_commit(self.sid, {
            'snapshot_id': current['snapshot_id'], 'paths': paths or ['selected.txt'], 'message': message,
        })

    def commit_file(self, content='delivered change\n'):
        (self.worktree / 'selected.txt').write_text(content)
        preview = self.preview_commit()
        self.assertTrue(preview['can_apply'])
        return self.store.delivery.commit(self.sid, {'preview_id': preview['preview_id']})['commit']

    def preview_transfer(self, commit):
        return self.store.delivery.preview_transfer(self.sid, {'commit': commit, 'target_branch': 'main'})

    def project_state(self):
        return (self.git('rev-parse', 'HEAD'), self.git('status', '--porcelain=v1', '--untracked-files=all'),
                self.git('ls-files', '--stage'), (self.project / 'selected.txt').read_bytes())

    def test_selected_commit_preserves_unselected_index_files_and_dirty_primary(self):
        (self.project / 'selected.txt').write_text('dirty primary stays here\n')
        (self.project / 'private.txt').write_text('primary untracked\n')
        primary = self.project_state()
        (self.worktree / 'selected.txt').write_text('selected staged version\n')
        (self.worktree / 'staged.txt').write_text('unselected staged content\n')
        (self.worktree / 'mixed.txt').write_text('unselected index content\n')
        self.git('add', '--', 'selected.txt', 'staged.txt', 'mixed.txt', cwd=self.worktree)
        (self.worktree / 'selected.txt').write_text('selected final content\n')
        (self.worktree / 'mixed.txt').write_text('unselected working content\n')
        (self.worktree / 'unstaged.txt').write_text('unselected unstaged content\n')
        (self.worktree / 'new file.txt').write_text('selected new content\n')
        (self.worktree / 'unselected-new.txt').write_text('unselected new content\n')
        self.git('rm', '--cached', '--', 'unstaged.txt', cwd=self.worktree)
        pending = [item['path'] for item in self.store.delivery.get(self.sid)['files']]
        self.assertEqual(pending.count('unstaged.txt'), 1)
        self.assertEqual(len(pending), len(set(pending)))
        unselected = ['staged.txt', 'mixed.txt', 'unstaged.txt', 'unselected-new.txt']
        contents = {name: (self.worktree / name).read_bytes() for name in unselected}
        index = self.git('ls-files', '--stage', '--', *unselected, cwd=self.worktree)
        preview = self.preview_commit(['selected.txt', 'new file.txt'])
        self.assertEqual(self.git('rev-parse', 'HEAD', cwd=self.worktree), self.base)
        result = self.store.delivery.commit(self.sid, {'preview_id': preview['preview_id']})
        self.assertTrue(result['ok'])
        commit = result['commit']
        self.assertEqual(self.git('diff-tree', '--no-commit-id', '--name-only', '-r', commit).splitlines(),
                         ['new file.txt', 'selected.txt'])
        self.assertEqual(self.git('show', commit + ':selected.txt'), 'selected final content')
        self.assertEqual(self.git('show', commit + ':staged.txt'), 'baseline')
        self.assertEqual(self.git('ls-files', '--stage', '--', *unselected, cwd=self.worktree), index)
        self.assertEqual({name: (self.worktree / name).read_bytes() for name in unselected}, contents)
        self.assertEqual(self.git('status', '--porcelain', '--', 'selected.txt', 'new file.txt', cwd=self.worktree), '')
        self.assertEqual(self.project_state(), primary)

    def test_stale_commit_preview_rejects_changed_content_and_index_without_mutation(self):
        (self.worktree / 'selected.txt').write_text('reviewed content\n')
        preview = self.preview_commit()
        (self.worktree / 'selected.txt').write_text('changed after review\n')
        before = self.git('ls-files', '--stage', cwd=self.worktree)
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.commit(self.sid, {'preview_id': preview['preview_id']})
        self.assertEqual(self.git('rev-parse', 'HEAD', cwd=self.worktree), self.base)
        self.assertEqual(self.git('ls-files', '--stage', cwd=self.worktree), before)
        self.assertEqual((self.worktree / 'selected.txt').read_text(), 'changed after review\n')
        preview = self.preview_commit()
        self.git('add', '--', 'selected.txt', cwd=self.worktree)
        before = self.git('ls-files', '--stage', cwd=self.worktree)
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.commit(self.sid, {'preview_id': preview['preview_id']})
        self.assertEqual(self.git('rev-parse', 'HEAD', cwd=self.worktree), self.base)
        self.assertEqual(self.git('ls-files', '--stage', cwd=self.worktree), before)

    def test_transfer_preserves_target_history_and_only_delivers_selected_commit(self):
        (self.worktree / 'unstaged.txt').write_text('earlier source-only commit\n')
        self.git('commit', '-qam', 'Source-only predecessor', cwd=self.worktree)
        source_commit = self.commit_file()
        (self.worktree / 'unselected-new.txt').write_text('leave in worktree\n')
        (self.project / 'target-only.txt').write_text('independent target history\n')
        self.git('add', '--', 'target-only.txt')
        self.git('commit', '-qm', 'Independent target change')
        target_head = self.git('rev-parse', 'HEAD')
        preview = self.preview_transfer(source_commit)
        self.assertTrue(preview['can_apply'])
        self.assertEqual(preview['target_head'], target_head)
        self.assertEqual(self.git('rev-parse', 'HEAD'), target_head)
        result = self.store.delivery.apply(self.sid, {'preview_id': preview['preview_id']})
        self.assertTrue(result['ok'])
        self.assertEqual(result['branch'], 'main')
        self.assertEqual(self.git('rev-parse', 'HEAD'), result['commit'])
        self.assertEqual(self.git('rev-parse', 'HEAD^'), target_head)
        self.assertEqual((self.project / 'selected.txt').read_text(), 'delivered change\n')
        self.assertEqual((self.project / 'target-only.txt').read_text(), 'independent target history\n')
        self.assertEqual((self.project / 'unstaged.txt').read_text(), 'baseline\n')
        self.assertFalse((self.project / 'unselected-new.txt').exists())
        self.assertEqual(self.git('status', '--porcelain'), '')
        self.assertEqual(self.git('rev-parse', 'HEAD', cwd=self.worktree), source_commit)
        self.assertEqual((self.worktree / 'unselected-new.txt').read_text(), 'leave in worktree\n')

    def test_conflicting_transfer_leaves_target_head_index_and_files_unchanged(self):
        source_commit = self.commit_file('source changes the same line\n')
        (self.project / 'selected.txt').write_text('target changes the same line\n')
        self.git('commit', '-qam', 'Conflicting target change')
        before = self.project_state()
        preview = self.preview_transfer(source_commit)
        self.assertFalse(preview['can_apply'])
        self.assertIn('selected.txt', preview['conflicts'])
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.apply(self.sid, {'preview_id': preview['preview_id']})
        self.assertEqual(self.project_state(), before)
        self.assertFalse((self.project / '.git' / 'CHERRY_PICK_HEAD').exists())
        self.assertEqual(self.git('diff', '--name-only', '--diff-filter=U'), '')

    def test_target_head_change_after_preview_rejects_apply(self):
        source_commit = self.commit_file()
        preview = self.preview_transfer(source_commit)
        (self.project / 'target-only.txt').write_text('created after preview\n')
        self.git('add', '--', 'target-only.txt')
        self.git('commit', '-qm', 'Target moved after review')
        before = self.project_state()
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.apply(self.sid, {'preview_id': preview['preview_id']})
        self.assertEqual(self.project_state(), before)

    def test_dirty_target_after_preview_rejects_apply_and_keeps_edits(self):
        preview = self.preview_transfer(self.commit_file())
        (self.project / 'selected.txt').write_text('new uncommitted target edits\n')
        self.git('add', '--', 'selected.txt')
        before = self.project_state()
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.apply(self.sid, {'preview_id': preview['preview_id']})
        self.assertEqual(self.project_state(), before)

    def test_active_source_or_sibling_sessions_block_delivery_mutations(self):
        (self.worktree / 'selected.txt').write_text('ready to commit\n')
        preview = self.preview_commit()
        self.store._status(self.sid, 'running')
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.commit(self.sid, {'preview_id': preview['preview_id']})
        self.store._status(self.sid, 'completed')
        sibling = self.create()
        self.store._status(sibling, 'queued')
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.commit(self.sid, {'preview_id': preview['preview_id']})
        self.store._status(sibling, 'completed')
        source_commit = self.commit_file()
        preview = self.preview_transfer(source_commit)
        for active in ('queued', 'running'):
            self.store._status(sibling, active)
            with self.subTest(status=active), self.assertRaises(sessions.SessionError):
                self.store.delivery.apply(self.sid, {'preview_id': preview['preview_id']})
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.base)

    def test_current_project_and_creator_sessions_are_explicitly_unsupported(self):
        project_sid = self.create()
        legacy_sid = self.create(workspace='worktree')
        creator = {'run_id': 'a' * 32, 'nonce': 'b' * 32, 'phase': 'scan'}
        with self.store.lock:
            self.store.db.execute('UPDATE sessions SET result_base=NULL WHERE id=?', (legacy_sid,))
            self.store.db.execute('UPDATE sessions SET creator=?,workspace=? WHERE id=?',
                                  (json.dumps(creator), 'creator', self.sid))
            self.store.db.commit()
        for sid in (project_sid, self.sid, legacy_sid):
            with self.subTest(session=sid):
                state = self.store.delivery.get(sid)
                self.assertFalse(state['available'])
                self.assertTrue(state['reason'])
                with self.assertRaises(sessions.SessionError):
                    self.store.delivery.preview_commit(sid, {'snapshot_id': None,
                                                            'paths': ['selected.txt'], 'message': 'Unsupported'})
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.base)

    def test_invalid_paths_messages_and_foreign_commits_cannot_mutate_refs(self):
        (self.worktree / 'selected.txt').write_text('ready\n')
        snapshot = self.store.delivery.get(self.sid)['snapshot_id']
        for changes in ({'paths': []}, {'paths': ['../selected.txt']}, {'paths': ['/etc/passwd']},
                        {'paths': ['.git/config']}, {'paths': ['missing.txt']},
                        {'message': ''}, {'message': 'x' * 4001}, {'message': 'nul\0message'},
                        {'snapshot_id': 'stale'}, {'unexpected': True}):
            data = {'snapshot_id': snapshot, 'paths': ['selected.txt'], 'message': 'Valid fixture', **changes}
            with self.subTest(changes=changes), self.assertRaises(sessions.SessionError):
                self.store.delivery.preview_commit(self.sid, data)
        (self.project / 'target-only.txt').write_text('foreign commit\n')
        self.git('add', '--', 'target-only.txt')
        self.git('commit', '-qm', 'Outside session history')
        target = self.git('rev-parse', 'HEAD')
        for commit in (self.base, target):
            with self.subTest(commit=commit), self.assertRaises(sessions.SessionError):
                self.preview_transfer(commit)
        self.assertEqual(self.git('rev-parse', 'HEAD', cwd=self.worktree), self.base)
        self.assertEqual(self.git('rev-parse', 'HEAD'), target)

    def test_local_delivery_never_invokes_remote_commands(self):
        self.git('remote', 'add', 'origin', 'https://example.invalid/must-not-connect.git')
        original = subprocess.Popen
        commands = []

        def capture(command, *args, **kwargs):
            commands.append(command)
            self.assertFalse(set(command) & {'push', 'fetch', 'pull', 'ls-remote', 'gh'})
            return original(command, *args, **kwargs)

        with patch.object(subprocess, 'Popen', side_effect=capture):
            commit = self.commit_file()
            preview = self.preview_transfer(commit)
            self.store.delivery.apply(self.sid, {'preview_id': preview['preview_id']})
        self.assertTrue(commands)
        self.assertEqual(self.git('for-each-ref', '--format=%(refname)', 'refs/remotes'), '')

    def test_interrupted_commit_recovers_index_without_losing_unselected_staging(self):
        (self.worktree / 'selected.txt').write_text('reviewed before interruption\n')
        (self.worktree / 'staged.txt').write_text('preserved staging\n')
        (self.worktree / 'mixed.txt').write_text('preserved mixed index\n')
        self.git('add', '--', 'staged.txt', 'mixed.txt', cwd=self.worktree)
        (self.worktree / 'mixed.txt').write_text('preserved mixed worktree\n')
        unselected = ['staged.txt', 'mixed.txt', 'unstaged.txt']
        staged = self.git('ls-files', '--stage', '--', *unselected, cwd=self.worktree)
        contents = {name: (self.worktree / name).read_bytes() for name in unselected}
        primary = self.project_state()
        preview = self.preview_commit()
        index = Path(self.git('rev-parse', '--git-path', 'index', cwd=self.worktree))
        if not index.is_absolute():
            index = self.worktree / index
        lock = Path(str(index) + '.lock')
        with patch.object(self.store.delivery, '_recover', side_effect=OSError('Fixture interruption')) as recovery:
            with self.assertRaises(OSError):
                self.store.delivery.commit(self.sid, {'preview_id': preview['preview_id']})
            self.assertEqual(recovery.call_count, 2)
        commit = self.git('rev-parse', 'HEAD', cwd=self.worktree)
        self.assertNotEqual(commit, self.base)
        self.assertTrue(lock.exists())
        self.assertEqual(len(list(self.store.delivery.folder.glob('transaction-*.json'))), 1)
        self.store.delivery = delivery.Delivery(self.store)
        self.assertFalse(lock.exists())
        self.assertEqual(list(self.store.delivery.folder.glob('transaction-*.json')), [])
        self.assertEqual(self.git('rev-parse', 'HEAD', cwd=self.worktree), commit)
        self.assertEqual(self.git('status', '--porcelain', '--', 'selected.txt', cwd=self.worktree), '')
        self.assertEqual((self.worktree / 'selected.txt').read_text(), 'reviewed before interruption\n')
        self.assertEqual(self.git('ls-files', '--stage', '--', *unselected, cwd=self.worktree), staged)
        self.assertEqual({name: (self.worktree / name).read_bytes() for name in unselected}, contents)
        self.assertEqual(self.project_state(), primary)
        self.assertTrue(self.store.delivery.get(self.sid)['available'])

    def test_transfer_to_unoccupied_branch_preserves_primary_checkout(self):
        self.git('branch', 'release-fixture', 'main')
        self.git('tag', 'release-fixture', 'main')
        source_commit = self.commit_file()
        (self.project / 'selected.txt').write_text('primary local staging\n')
        self.git('add', '--', 'selected.txt')
        (self.project / 'private.txt').write_text('primary untracked data\n')
        primary = self.project_state()
        worktrees = self.git('worktree', 'list', '--porcelain')
        preview = self.store.delivery.preview_transfer(self.sid, {
            'commit': source_commit, 'target_branch': 'release-fixture',
        })
        self.assertTrue(preview['can_apply'])
        self.assertEqual(self.git('rev-parse', 'refs/heads/release-fixture'), self.base)
        result = self.store.delivery.apply(self.sid, {'preview_id': preview['preview_id']})
        self.assertTrue(result['ok'])
        self.assertEqual(result['branch'], 'release-fixture')
        self.assertEqual(self.git('rev-parse', 'refs/heads/release-fixture'), result['commit'])
        self.assertEqual(self.git('rev-parse', 'refs/heads/release-fixture^'), self.base)
        self.assertEqual(self.git('show', 'refs/heads/release-fixture:selected.txt'), 'delivered change')
        self.assertEqual(self.git('rev-parse', 'refs/tags/release-fixture'), self.base)
        self.assertEqual(self.project_state(), primary)
        self.assertEqual((self.project / 'private.txt').read_text(), 'primary untracked data\n')
        self.assertEqual(self.git('rev-parse', 'HEAD', cwd=self.worktree), source_commit)
        self.assertEqual(self.git('worktree', 'list', '--porcelain'), worktrees)

    def test_missing_index_commit_preserves_unselected_cached_deletions(self):
        index = Path(self.git('rev-parse', '--git-path', 'index', cwd=self.worktree))
        if not index.is_absolute():
            index = self.worktree / index
        index.unlink()
        self.assertFalse(index.exists())
        self.assertEqual(self.git('ls-files', '--stage', cwd=self.worktree), '')
        unselected = ['staged.txt', 'mixed.txt', 'unstaged.txt']
        deleted = self.git('diff', '--cached', '--name-only', '--diff-filter=D', '--',
                           *unselected, cwd=self.worktree)
        self.assertEqual(set(deleted.splitlines()), set(unselected))
        contents = {name: (self.worktree / name).read_bytes() for name in unselected}
        primary = self.project_state()
        commit = self.commit_file('selected survives missing index\n')
        self.assertEqual(self.git('ls-files', cwd=self.worktree), 'selected.txt')
        self.assertEqual(self.git('diff', '--cached', '--name-only', '--diff-filter=D', '--',
                                  *unselected, cwd=self.worktree), deleted)
        self.assertEqual(self.git('status', '--porcelain', '--', 'selected.txt', cwd=self.worktree), '')
        self.assertEqual({name: (self.worktree / name).read_bytes() for name in unselected}, contents)
        for name in unselected:
            self.assertEqual(self.git('show', commit + ':' + name), 'baseline')
        self.assertEqual(self.project_state(), primary)

    def target_preview(self, commit, code, timeout=30):
        return self.store.delivery.preview_transfer(self.sid, {
            'commit':commit, 'target_branch':'main',
            'check':{'command':shlex.join([sys.executable,'-c',code]),'timeout':timeout}})

    def run_target_check(self, preview):
        self.store.delivery.start_check(self.sid, {'preview_id':preview['preview_id']})
        sid,job,generation=self.store.jobs.get_nowait()
        try: self.store.results.run_check(sid,job['check_id'],generation)
        finally: self.store.jobs.task_done()
        return self.store.results.history(sid)['checks'][0]

    def test_target_check_runs_combined_tree_and_gates_apply_with_saved_evidence(self):
        commit=self.commit_file()
        (self.project/'target-only.txt').write_text('target content')
        self.git('add','target-only.txt'); self.git('commit','-qm','Target-only fixture')
        target=self.git('rev-parse','HEAD'); before=self.project_state()
        worktrees=self.git('worktree','list','--porcelain')
        preview=self.target_preview(commit,"from pathlib import Path; assert Path('target-only.txt').read_text()=='target content'; assert Path('selected.txt').read_text()=='delivered change\\n'; print('COMBINED_TREE_PASSED')")
        self.assertNotEqual(preview['candidate'],commit)
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.apply(self.sid,{'preview_id':preview['preview_id']})
        check=self.run_target_check(preview)
        self.assertEqual(check['status'],'passed',check['output'])
        self.assertIn('COMBINED_TREE_PASSED',check['output'])
        self.assertEqual(check['candidate'],preview['candidate'])
        self.assertEqual(check['target_head'],target)
        self.assertEqual(self.project_state(),before)
        self.assertEqual(self.git('worktree','list','--porcelain'),worktrees)
        self.assertEqual(self.store.delivery.get(self.sid)['preview']['target_check']['id'],check['id'])
        applied=self.store.delivery.apply(self.sid,{'preview_id':preview['preview_id']})
        self.assertEqual(applied['commit'],check['candidate'])
        self.assertIsNone(self.store.delivery.get(self.sid)['preview'])
        self.store.results=type(self.store.results)(self.store)
        self.assertEqual(self.store.results.history(self.sid)['checks'][0]['candidate'],applied['commit'])

    def test_failed_mutating_timed_out_and_stale_checks_cannot_authorize_apply(self):
        commit=self.commit_file(); before=self.project_state()
        for code,expected,timeout in [("print('FAILED_TEST'); raise SystemExit(7)",'failed',30),
                                      ("from pathlib import Path; Path('selected.txt').write_text('rewritten')",'workspace_changed',30),
                                      ("import time; time.sleep(10)",'timed_out',1)]:
            preview=self.target_preview(commit,code,timeout)
            check=self.run_target_check(preview)
            self.assertEqual(check['status'],expected,check['output'])
            with self.assertRaises(sessions.SessionError):
                self.store.delivery.apply(self.sid,{'preview_id':preview['preview_id']})
            self.assertEqual(self.project_state(),before)
            self.assertEqual(len(self.git('worktree','list','--porcelain').split('worktree '))-1,2)
        preview=self.target_preview(commit,"print('passed')")
        check=self.run_target_check(preview); self.assertEqual(check['status'],'passed',check['output'])
        (self.project/'staged.txt').write_text('new target change')
        self.git('commit','-qam','Target moved after check')
        changed=self.project_state()
        self.assertFalse(self.store.delivery.get(self.sid)['preview']['can_apply'])
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.apply(self.sid,{'preview_id':preview['preview_id']})
        self.assertEqual(self.project_state(),changed)
        replacement=self.target_preview(commit,"print('new preview needs its own check')")
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.apply(self.sid,{'preview_id':replacement['preview_id']})

    def test_target_check_cancellation_cleanup_and_restart_fail_closed(self):
        preview=self.target_preview(self.commit_file(),"import time; print('CHECK_RUNNING',flush=True); time.sleep(30)")
        self.store.delivery.start_check(self.sid,{'preview_id':preview['preview_id']})
        sid,job,generation=self.store.jobs.get_nowait()
        worker=threading.Thread(target=self.store.results.run_check,args=(sid,job['check_id'],generation))
        worker.start()
        try:
            deadline=time.monotonic()+30
            while time.monotonic()<deadline:
                if 'CHECK_RUNNING' in self.store.results.history(sid)['checks'][0]['output']: break
                time.sleep(.05)
            else: self.fail('Target command did not start')
            self.store.cancel(sid); worker.join(15)
            self.assertFalse(worker.is_alive())
        finally:
            self.store.cancel(sid); worker.join(15); self.store.jobs.task_done()
        check=self.store.results.history(sid)['checks'][0]
        self.assertEqual(check['status'],'cancelled',check['output'])
        self.assertEqual(len(self.git('worktree','list','--porcelain').split('worktree '))-1,2)
        self.assertEqual(self.git('rev-parse','HEAD'),self.base)
        self.store.delivery=delivery.Delivery(self.store)
        self.assertIsNone(self.store.delivery.get(sid)['preview'])
        with self.assertRaises(sessions.SessionError):
            self.store.delivery.apply(sid,{'preview_id':preview['preview_id']})


if __name__ == '__main__':
    unittest.main()
