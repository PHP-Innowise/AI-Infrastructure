"""Fresh Marten source-ignore scope; actual Git plus finite transport controls."""
from __future__ import annotations

import importlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tests.hook_shell import BASH

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_git_source_scope_public.json').read_text(encoding='utf-8'))


class GitSourceScopeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='marten-git-')
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root = self.base / 'project'
        self.root.mkdir()
        self.foreign = self.base / 'foreign'
        self.foreign.mkdir()
        self.home = self.base / 'home'
        self.home.mkdir()
        environment = {k: v for k, v in os.environ.items()
                       if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_', 'PROJECT_BRAIN_'))}
        environment.update(HOME=str(self.home), XDG_CONFIG_HOME=str(self.home / 'xdg'),
                           GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1', PYTHONIOENCODING='utf-8')
        self.environment = environment
        patch = mock.patch.dict(os.environ, environment, clear=True)
        patch.start()
        self.addCleanup(patch.stop)
        self.init(self.root)
        self.init(self.foreign)
        (self.root / '.gitignore').write_text(PUBLIC['root_ignore'], encoding='utf-8')
        (self.foreign / '.gitignore').write_text(PUBLIC['foreign_ignore'], encoding='utf-8')
        self.excludes = self.home / 'public-ignore'
        self.excludes.write_text(PUBLIC['global_ignore'], encoding='utf-8')
        # A private ordinary global config, not environment command injection.
        (self.home / '.gitconfig').write_text('[core]\n\texcludesFile = ' + self.excludes.as_posix() + '\n', encoding='utf-8')
        directory = Path(os.environ.get('MARTEN_ORIGINAL_RUNTIME', str(ROOT / 'PHP Core/memory-bank/scripts')))
        self.directory = directory
        names = [item.stem for item in directory.glob('*.py')]
        previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}
        def restore():
            for name in names:
                sys.modules.pop(name, None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        sys.path.insert(0, str(directory))
        try:
            self.context = importlib.import_module('context')
        finally:
            sys.path.remove(str(directory))

    def init(self, root, *options):
        subprocess.run(['git', '-C', str(root), 'init', '-q', '-b', PUBLIC['native']['task_id'], *options],
                       env=self.environment, capture_output=True, timeout=5, check=True)

    def foreign_selectors(self):
        return {'GIT_DIR': str(self.foreign / '.git'), 'GIT_WORK_TREE': str(self.foreign)}

    def refused(self, function, *args):
        with self.assertRaises(self.context.ContextError) as caught:
            function(*args)
        self.assertEqual(PUBLIC['error'], str(caught.exception))

    def frame(self, root=None, prefix=''):
        root = self.root if root is None else root
        return PUBLIC['frame_template'].format(TOP=root, PREFIX=prefix, GITDIR=root / '.git').encode('utf-8')

    def result(self, status=0, output=b''):
        return subprocess.CompletedProcess([], status, output, b'')

    def test_inherited_foreign_selectors_still_read_the_projects_own_policy(self):
        # A GIT_DIR/GIT_WORK_TREE the shell exports is dropped from the probe:
        # `git -C <project>` finds the project's own repository, whose ignore
        # policy is the one that applies to the project's files.
        with mock.patch.dict(os.environ, self.foreign_selectors()):
            self.assertEqual({os.fsencode(p) for p in PUBLIC['expected_ignored']},
                             self.context.git_ignored_paths(self.root, PUBLIC['paths']))
            child = self.context._source_git_environment()
        self.assertFalse([key for key in child if key.startswith('GIT_')
                          and key not in ('GIT_OPTIONAL_LOCKS', 'GIT_TERMINAL_PROMPT')])

    def test_native_readonly_and_both_hooks_serve_the_task_under_foreign_selectors(self):
        native = PUBLIC['native']
        expected = native['expected']
        for relative in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/config',
                         'project-brain/schemas', 'project-brain/templates'):
            source = self.directory if relative == 'memory-bank/scripts' else ROOT / 'PHP Core' / relative
            shutil.copytree(source, self.root / relative, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        (self.root / 'memory-bank/chunks').mkdir()
        (self.root / 'project-brain/config/runtime.json').write_text(json.dumps(native['configuration']), encoding='utf-8')
        source = self.root / native['source']
        source.parent.mkdir()
        source.write_text(native['source_text'], encoding='utf-8')
        # The source is eligible while the actual native writer creates its task.
        (self.root / '.gitignore').write_text(PUBLIC['foreign_ignore'], encoding='utf-8')
        cli = self.root / 'memory-bank/scripts/context.py'
        def call(arguments, environment):
            return subprocess.run([sys.executable, '-B', str(cli), '--root', str(self.root), *arguments],
                                  cwd=self.root, env=environment, capture_output=True, timeout=12)
        start = call(['start', '--task-id', native['task_id'], '--goal', native['goal'], '--source', native['source'], '--json'], self.environment)
        self.assertEqual(0, start.returncode, start.stderr.decode('utf-8', 'replace'))
        self.assertEqual(native['start_revision'], json.loads(start.stdout)['revision'])
        update = call(['update', '--task-id', native['task_id'], '--progress', native['progress'], '--next-step', native['next_step'], '--revision', '1', '--json'], self.environment)
        self.assertEqual(0, update.returncode, update.stderr.decode('utf-8', 'replace'))
        self.assertEqual(native['update_revision'], json.loads(update.stdout)['revision'])
        record_id = json.loads(update.stdout)['task_uuid']
        get = call(['brain-get', '--record-id', record_id, '--json'], self.environment)
        self.assertEqual(0, get.returncode, get.stderr.decode('utf-8', 'replace'))
        persisted = json.loads(get.stdout)
        for key, value in PUBLIC['setup_supplement']['native_persisted_fields'].items():
            self.assertEqual(value, persisted[key], key)
        self.assertEqual(record_id, persisted['id'])
        (self.root / '.gitignore').write_text(PUBLIC['root_ignore'], encoding='utf-8')
        database = self.root / 'memory-bank/local/context.db'
        before_db = database.read_bytes()
        def shared():
            return {p.relative_to(self.root).as_posix(): p.read_bytes()
                    for p in (self.root / 'project-brain').rglob('*') if p.is_file()}
        before_shared = shared()
        environment = dict(self.environment, **self.foreign_selectors(), CONTEXT_TASK_ID=native['task_id'])
        reply = call(['refresh', '--query', native['query'], '--task-id', native['task_id'], '--read-only', '--json'], environment)
        # Read-only refresh is the working-state path: it answers 1 with the
        # task's own state. Neither the foreign selectors nor the task's
        # source now being ignored withholds that state.
        self.assertEqual(1, reply.returncode, reply.stderr.decode('utf-8', 'replace'))
        self.assertEqual(b'', reply.stderr)
        working = json.loads(reply.stdout)['capsule']['working']
        self.assertEqual(native['progress'], working['progress'])
        self.assertEqual(before_db, database.read_bytes())
        self.assertEqual(before_shared, shared())
        if BASH is None:
            self.skipTest('actual Bash hooks unavailable; native CLI assertions completed')
        binaries = self.base / 'bin'
        binaries.mkdir()
        try:
            (binaries / 'python3').symlink_to(sys.executable)
        except OSError as error:
            self.skipTest('native hook interpreter symlink unavailable: ' + type(error).__name__)
        scratch = self.base / 'tmp'
        scratch.mkdir()
        environment.update(PATH=str(binaries) + os.pathsep + environment['PATH'], TMPDIR=str(scratch))
        for host in ('.claude', '.codex'):
            hook = self.root / host / 'hooks/working-memory-read.sh'
            hook.parent.mkdir(parents=True)
            shutil.copy2(ROOT / 'PHP Core' / host / 'hooks/working-memory-read.sh', hook)
            observed = subprocess.run([BASH, str(hook)], cwd=self.root, env=environment,
                                      input=json.dumps({'prompt': native['query']}).encode('utf-8'), capture_output=True, timeout=12)
            self.assertEqual(expected['hook_returncode'], observed.returncode)
            self.assertIn('working: ' + native['task_id'], observed.stdout.decode('utf-8'))
            self.assertIn(native['progress'], observed.stdout.decode('utf-8'))
            self.assertNotIn('Memory refresh unavailable', observed.stdout.decode('utf-8'))
            self.assertEqual(b'', observed.stderr)

    def test_inherited_selectors_never_disable_the_probe(self):
        for key in PUBLIC['selectors']:
            for value in ('', 'public-selection'):
                with self.subTest(key=key, value=value), mock.patch.dict(os.environ, {key: value}):
                    self.assertEqual({os.fsencode(p) for p in PUBLIC['expected_ignored']},
                                     self.context.git_ignored_paths(self.root, PUBLIC['paths']))

    def test_benign_git_preferences_do_not_disable_or_escape_probe(self):
        with mock.patch.dict(os.environ, PUBLIC['benign']):
            before = dict(os.environ)
            self.assertEqual({os.fsencode(p) for p in PUBLIC['expected_ignored']}, self.context.git_ignored_paths(self.root, PUBLIC['paths']))
            self.assertEqual(before, dict(os.environ))
            child = self.context._source_git_environment()
            for key in PUBLIC['benign']:
                self.assertNotIn(key, child)
            self.assertEqual(str(self.home), child['HOME'])

    def test_real_global_and_repository_ignore_rules_are_retained(self):
        self.assertEqual({os.fsencode(p) for p in PUBLIC['expected_ignored']}, self.context.git_ignored_paths(self.root, PUBLIC['paths']))

    def test_command_environment_config_cannot_override_global_exclusions(self):
        empty = self.home / 'empty-ignore'
        empty.write_bytes(b'')
        injection = {k: v.replace('{EMPTY_EXCLUDES}', str(empty)) for k, v in PUBLIC['policy_injection'].items()}
        with mock.patch.dict(os.environ, injection):
            self.assertEqual({os.fsencode(p) for p in PUBLIC['expected_ignored']}, self.context.git_ignored_paths(self.root, PUBLIC['paths']))

    def test_plain_folder_and_empty_direct_candidates_have_zero_git_calls(self):
        plain = self.base / 'plain'
        plain.mkdir()
        with mock.patch.object(self.context.subprocess, 'run', side_effect=AssertionError('Git must not run')):
            self.assertEqual(set(), self.context.git_ignored_paths(plain, PUBLIC['paths']))
            self.assertEqual(set(), self.context.git_ignored_paths(self.root, []))

    def test_missing_git_inside_checkout_is_unknown(self):
        with mock.patch.object(self.context.subprocess, 'run', side_effect=FileNotFoundError('public missing git')):
            self.refused(self.context.git_ignored_paths, self.root, PUBLIC['paths'])

    @unittest.skipUnless(hasattr(os, 'mkfifo'), 'POSIX marker profile only')
    def test_dangling_and_special_git_markers_are_unknown(self):
        for label in ('dangling', 'fifo', 'inaccessible'):
            root = self.base / label
            root.mkdir()
            marker = root / '.git'
            if label == 'dangling':
                marker.symlink_to(self.base / 'absent-git')
            elif label == 'fifo':
                os.mkfifo(marker)
            else:
                original = Path.lstat
                def observed(path, *args, **kwargs):
                    if path == marker:
                        raise PermissionError('supplied inaccessible marker')
                    return original(path, *args, **kwargs)
                with mock.patch.object(Path, 'lstat', new=observed):
                    self.refused(self.context.git_ignored_paths, root, PUBLIC['paths'])
                continue
            self.refused(self.context.git_ignored_paths, root, PUBLIC['paths'])

    def test_nested_project_relative_paths_use_actual_top_and_prefix(self):
        nested = self.root / PUBLIC['nested_prefix']
        nested.mkdir(parents=True)
        (self.root / '.gitignore').write_text(PUBLIC['nested_root_ignore'], encoding='utf-8')
        self.assertEqual({os.fsencode(p) for p in PUBLIC['expected_ignored']}, self.context.git_ignored_paths(nested, PUBLIC['paths']))

    def test_separate_git_directory_file_marker_is_supported(self):
        root = self.base / 'separate'
        root.mkdir()
        self.init(root, '--separate-git-dir', str(self.base / 'separate-admin'))
        (root / '.gitignore').write_text(PUBLIC['root_ignore'], encoding='utf-8')
        self.assertEqual({os.fsencode(p) for p in PUBLIC['expected_ignored']}, self.context.git_ignored_paths(root, PUBLIC['paths']))

    def test_transport_whole_status_nul_subset_and_multiplicity(self):
        for row in PUBLIC['transport']['responses']:
            with self.subTest(name=row['name']):
                responses = [self.result(0, self.frame()), self.result(row['status'], bytes.fromhex(row['bytes_hex'])), self.result(0, self.frame())]
                with mock.patch.object(self.context.subprocess, 'run', side_effect=responses):
                    if row['expected'] is None:
                        self.refused(self.context.git_ignored_paths, self.root, PUBLIC['transport']['input'])
                    else:
                        self.assertEqual({os.fsencode(p) for p in row['expected']}, self.context.git_ignored_paths(self.root, PUBLIC['transport']['input']))

    def test_frame_requires_actual_absolute_top_prefix_and_admin(self):
        for raw in PUBLIC['malformed_frames']:
            with self.subTest(raw=raw):
                result = self.result(0, raw.format(TOP=self.root, GITDIR=self.root / '.git').encode('utf-8'))
                with mock.patch.object(self.context.subprocess, 'run', return_value=result):
                    self.refused(self.context.git_ignored_paths, self.root, PUBLIC['paths'])

    def test_deadline_is_shared_across_all_three_git_processes(self):
        # One operation-wide deadline, sized for a slow Git (2 s).
        with mock.patch.object(self.context.time, 'monotonic', side_effect=PUBLIC['deadlines']['mock_times']), mock.patch.object(self.context.subprocess, 'run', side_effect=[self.result(0, self.frame()), self.result(1), self.result(0, self.frame())]) as run:
            self.assertEqual(set(), self.context.git_ignored_paths(self.root, PUBLIC['paths']))
        self.assertEqual(3, run.call_count)
        for observed, expected in zip(run.call_args_list, PUBLIC['deadlines']['expected_timeouts']):
            self.assertAlmostEqual(expected, observed.kwargs['timeout'])
        self.assertIn('--git-dir=' + str(self.root / '.git'), run.call_args_list[1].args[0])
        self.assertNotIn('--git-dir=' + str(self.root / '.git'), run.call_args_list[2].args[0])

    @unittest.skipIf(os.name == 'nt', 'native POSIX link retarget only')
    def test_actual_git_marker_retarget_is_not_hidden_by_pinned_query(self):
        original = subprocess.run
        def observed(argv, *args, **kwargs):
            result = original(argv, *args, **kwargs)
            if 'check-ignore' in argv:
                (self.root / '.git').rename(self.base / 'original-admin')
                (self.root / '.git').symlink_to(self.foreign / '.git', target_is_directory=True)
            return result
        with mock.patch.object(self.context.subprocess, 'run', side_effect=observed):
            self.refused(self.context.git_ignored_paths, self.root, PUBLIC['paths'])

    def test_source_unknown_is_not_downgraded_to_index_warning(self):
        with mock.patch.object(self.context, 'index_repository', side_effect=self.context.SourceGitError(PUBLIC['error'])):
            self.refused(self.context.refresh_layers, None, self.root)

    def test_ignored_accelerator_documents_keep_their_exemption(self):
        with (self.root / '.gitignore').open('a', encoding='utf-8') as handle:
            handle.write(PUBLIC['working_extra_ignore'])
        with mock.patch.object(self.context.subprocess, 'run', side_effect=AssertionError('ordinary exemption must not probe')):
            self.assertEqual(set(), self.context.git_ignored_paths(self.root, [PUBLIC['accelerator_path']]))

    def test_post_query_failure_is_unknown_not_an_empty_ignore_set(self):
        with mock.patch.object(self.context.subprocess, 'run', side_effect=[self.result(0, self.frame()), self.result(1), self.result(128)]):
            self.refused(self.context.git_ignored_paths, self.root, PUBLIC['paths'])


if __name__ == '__main__':
    unittest.main()
