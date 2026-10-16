"""Fresh Otter/Heron task provenance and delivery admission."""
from __future__ import annotations
import importlib
import hashlib
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
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_inferred_identity_public.json').read_text(encoding='utf-8'))


class InferredIdentityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='otter-identity-')
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root = self.base / 'project'
        self.root.mkdir()
        self.profile_directory = self.base / 'profile'
        self.profile_directory.mkdir()
        self.scratch = self.base / 'tmp'
        self.scratch.mkdir()
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_', 'PROJECT_BRAIN_'))}
        self.env.update(XDG_CONFIG_HOME=str(self.profile_directory / 'xdg'),
                        TMPDIR=str(self.scratch), GIT_OPTIONAL_LOCKS='0',
                        PYTHONDONTWRITEBYTECODE='1', PYTHONIOENCODING='utf-8')
        environment = mock.patch.dict(os.environ, self.env, clear=True)
        environment.start()
        self.addCleanup(environment.stop)
        self.git = shutil.which('git')
        self.assertIsNotNone(self.git)
        self.git_call('init', '-q', '-b', PUBLIC['branch'])
        (self.root / '.gitignore').write_text(PUBLIC['ignore'], encoding='utf-8')
        self.runtime = Path(os.environ.get('OTTER_ORIGINAL_RUNTIME', str(ROOT / 'PHP Core/memory-bank/scripts')))
        self.hooks = Path(os.environ.get('OTTER_ORIGINAL_EDITION', str(ROOT / 'PHP Core')))
        names = [path.stem for path in self.runtime.glob('*.py')]
        previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}
        def restore():
            for name in names:
                sys.modules.pop(name, None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        sys.path.insert(0, str(self.runtime))
        try:
            self.context = importlib.import_module('context')
            self.retrieval = importlib.import_module('context_retrieval')
        finally:
            sys.path.remove(str(self.runtime))
        self.observations = []

    def git_call(self, *arguments):
        return subprocess.run([self.git, '-C', str(self.root), *arguments], env=self.env,
                              capture_output=True, check=True, timeout=5)

    def switch(self, branch):
        self.git_call('symbolic-ref', 'HEAD', 'refs/heads/' + branch)

    def observe(self, label, result):
        capture = self.base / 'captured-branch'
        row = {'label': label, 'returncode': result.returncode,
               'stdout': result.stdout.decode('utf-8', 'strict'),
               'stderr': result.stderr.decode('utf-8', 'strict'),
               'branch_after': self.git_call('symbolic-ref', '--quiet', '--short', 'HEAD').stdout.decode('utf-8').rstrip('\n'),
               'branch_capture': capture.read_text(encoding='utf-8') if capture.exists() else None}
        if hasattr(self, 'before_guard_state'):
            row.update(branch_before=self.branch_before_guard, state_before=self.before_guard_state,
                       state_after=self.guard_state())
        self.observations.append(row)
        output = os.environ.get('OTTER_OBSERVATION_DIR')
        if output:
            directory = Path(output)
            directory.mkdir(parents=True, exist_ok=True)
            (directory / (self._testMethodName + '.json')).write_text(
                json.dumps(self.observations, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        return row

    def native(self):
        for relative in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/config',
                         'project-brain/schemas', 'project-brain/templates'):
            source = self.runtime if relative == 'memory-bank/scripts' else ROOT / 'PHP Core' / relative
            shutil.copytree(source, self.root / relative, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        (self.root / 'memory-bank/chunks').mkdir()
        (self.root / 'project-brain/config/runtime.json').write_text(json.dumps(PUBLIC['configuration']), encoding='utf-8')
        source = self.root / PUBLIC['source']
        source.parent.mkdir()
        source.write_text(PUBLIC['source_text'], encoding='utf-8')
        self.cli = self.root / 'memory-bank/scripts/context.py'
        first = self.call('start', '--task-id', PUBLIC['branch'], '--goal', PUBLIC['goal'], '--source', PUBLIC['source'], '--json')
        self.assertEqual(0, first.returncode, first.stderr)
        self.assertEqual(1, json.loads(first.stdout)['revision'])
        second = self.call('update', '--task-id', PUBLIC['branch'], '--progress', PUBLIC['progress'],
                           '--next-step', PUBLIC['next_steps'][0], '--revision', '1', '--json')
        self.assertEqual(0, second.returncode, second.stderr)
        self.assertEqual(2, json.loads(second.stdout)['revision'])
        record = self.call('brain-get', '--record-id', json.loads(second.stdout)['task_uuid'], '--json')
        self.assertEqual(0, record.returncode, record.stderr)
        actual = json.loads(record.stdout)
        for key, value in PUBLIC['persisted_fields'].items():
            self.assertEqual(value, actual[key], key)
        self.database = self.root / 'memory-bank/local/context.db'

    def call(self, *arguments):
        result = subprocess.run([sys.executable, '-B', str(self.cli), '--root', str(self.root), *arguments],
                                cwd=self.root, env=self.env, capture_output=True, timeout=15)
        self.observe(arguments[0], result)
        return result

    def guard_state(self):
        return {'database_sha256': hashlib.sha256(self.database.read_bytes()).hexdigest(),
                'shared_sha256': {name: hashlib.sha256(raw).hexdigest() for name, raw in self.shared().items()}}

    def shared(self):
        return {path.relative_to(self.root).as_posix(): path.read_bytes()
                for path in (self.root / 'project-brain').rglob('*') if path.is_file()}

    def refused(self, operation):
        with self.assertRaises(self.context.SourceGitError) as caught:
            operation()
        self.assertEqual(PUBLIC['branch_error'], str(caught.exception))

    def negative(self, result):
        expected = PUBLIC['required_negative']
        self.assertEqual(expected['returncode'], result.returncode)
        self.assertEqual(expected['stdout'], result.stdout.decode('utf-8'))
        self.assertEqual(expected['stderr'], result.stderr.decode('utf-8'))

    def working(self, result):
        self.assertEqual(0, result.returncode, result.stderr)
        packet = json.loads(result.stdout)['capsule']
        self.assertIsNotNone(packet)
        self.assertEqual(PUBLIC['branch'], packet['task_id'])
        for key, value in PUBLIC['positive_working_fields'].items():
            self.assertEqual(value, packet['working'][key], key)

    def race_hook(self, host):
        if BASH is None:
            self.skipTest('actual Bash hook profile unavailable')
        self.native()
        binaries = self.base / 'bin'
        binaries.mkdir()
        try:
            (binaries / 'python3').symlink_to(sys.executable)
        except OSError as error:
            self.skipTest('hook interpreter symlink unavailable: ' + type(error).__name__)
        shim = '''#!/usr/bin/env bash
if [ "$#" -eq 4 ] && [ "$1" = "-C" ] && [ "$3" = "branch" ] && [ "$4" = "--show-current" ]; then
  CAPTURED=$("$OTTER_REAL_GIT" "$@") || exit "$?"
  "$OTTER_REAL_GIT" -C "$2" symbolic-ref HEAD "refs/heads/$OTTER_OTHER_BRANCH" || exit "$?"
  printf '%s\\n' "$CAPTURED" > "$OTTER_BRANCH_CAPTURE"
  printf '%s\\n' "$CAPTURED"
else
  exec "$OTTER_REAL_GIT" "$@"
fi
'''
        # LF: Git Bash reads a carriage return as part of each command.
        (binaries / 'git').write_bytes(shim.encode('utf-8'))
        (binaries / 'git').chmod(0o755)
        hook = self.root / host / 'hooks/working-memory-read.sh'
        hook.parent.mkdir(parents=True)
        shutil.copy2(self.hooks / host / 'hooks/working-memory-read.sh', hook)
        environment = dict(self.env, PATH=str(binaries) + os.pathsep + self.env['PATH'],
                           OTTER_REAL_GIT=self.git, OTTER_OTHER_BRANCH=PUBLIC['other_branch'],
                           OTTER_BRANCH_CAPTURE=str(self.base / 'captured-branch'))
        before_db, before_shared = self.database.read_bytes(), self.shared()
        self.before_guard_state = self.guard_state()
        self.branch_before_guard = self.git_call('symbolic-ref', '--quiet', '--short', 'HEAD').stdout.decode('utf-8').rstrip('\n')
        result = subprocess.run([BASH, str(hook)], cwd=self.root, env=environment,
                                input=json.dumps({'prompt': PUBLIC['query']}).encode('utf-8'),
                                capture_output=True, timeout=15)
        observed = self.observe(host, result)
        self.assertEqual(PUBLIC['branch'] + '\n', observed['branch_capture'])
        self.assertEqual(PUBLIC['other_branch'], observed['branch_after'])
        expected = PUBLIC['required_negative']
        self.assertEqual(expected['hook_returncode'], result.returncode)
        self.assertEqual(expected['hook_stdout'], observed['stdout'])
        self.assertEqual(expected['hook_stderr'], observed['stderr'])
        self.assertNotIn(PUBLIC['progress'], observed['stdout'])
        self.assertEqual(before_db, self.database.read_bytes())
        self.assertEqual(before_shared, self.shared())

    def test_claude_inference_then_branch_move_refuses(self):
        self.race_hook('.claude')

    def test_codex_inference_then_branch_move_refuses(self):
        self.race_hook('.codex')

    def test_explicit_task_on_another_branch_still_returns_saved_working_state(self):
        self.native()
        self.switch(PUBLIC['other_branch'])
        self.working(self.call('refresh', '--query', PUBLIC['query'], '--task-id', PUBLIC['branch'], '--ephemeral', '--json'))

    def test_same_unborn_inferred_branch_returns_saved_working_state(self):
        self.native()
        self.working(self.call('refresh', '--query', PUBLIC['query'], '--task-id', PUBLIC['branch'],
                               '--expected-branch', PUBLIC['branch'], '--ephemeral', '--json'))

    def test_readonly_mismatch_refuses_without_state_change(self):
        self.native()
        self.switch(PUBLIC['other_branch'])
        before_db, before_shared = self.database.read_bytes(), self.shared()
        self.negative(self.call('refresh', '--query', PUBLIC['query'], '--task-id', PUBLIC['branch'],
                                '--expected-branch', PUBLIC['branch'], '--read-only', '--json'))
        self.assertEqual(before_db, self.database.read_bytes())
        self.assertEqual(before_shared, self.shared())

    def main_call(self, arguments):
        with mock.patch.object(sys, 'argv', ['context.py', '--root', str(self.root), *arguments]), \
             mock.patch.object(self.context, 'utf8_output'), \
             mock.patch.object(sys, 'stdout', new_callable=io.StringIO) as output, \
             mock.patch.object(sys, 'stderr', new_callable=io.StringIO) as errors:
            code = self.context.main()
        result = subprocess.CompletedProcess([], code, output.getvalue().encode('utf-8'), errors.getvalue().encode('utf-8'))
        self.observe('in-process-main', result)
        return result

    def test_branch_move_during_index_cannot_publish_manifest_or_receipt(self):
        self.native()
        original = self.context.index_repository
        def index(*arguments, **options):
            result = original(*arguments, **options)
            self.switch(PUBLIC['other_branch'])
            return result
        with mock.patch.object(self.context, 'index_repository', side_effect=index), \
             mock.patch.object(self.retrieval, 'atomic_json', wraps=self.retrieval.atomic_json) as publication, \
             mock.patch.object(self.retrieval, '_commit_ordinary_receipts', wraps=self.retrieval._commit_ordinary_receipts) as receipt:
            self.negative(self.main_call(['refresh', '--query', PUBLIC['query'], '--task-id', PUBLIC['branch'],
                                          '--expected-branch', PUBLIC['branch'], '--ephemeral', '--json']))
        self.assertEqual(0, publication.call_count)
        self.assertEqual(0, receipt.call_count)

    def test_branch_move_during_pack_precedes_manifest_and_receipt_publication(self):
        self.native()
        connection = self.context.connect(self.database)
        self.addCleanup(connection.close)
        original = self.context.enforce_governed_capsule_contract
        def pack(*arguments, **options):
            result = original(*arguments, **options)
            self.switch(PUBLIC['other_branch'])
            return result
        with mock.patch.object(self.context, 'enforce_governed_capsule_contract', side_effect=pack), \
             mock.patch.object(self.retrieval, 'atomic_json', wraps=self.retrieval.atomic_json) as publication, \
             mock.patch.object(self.retrieval, '_commit_ordinary_receipts', wraps=self.retrieval._commit_ordinary_receipts) as receipt:
            self.refused(lambda: self.context.assemble_capsule(
                connection, self.root, mode='governed', query=PUBLIC['query'], task_id=PUBLIC['branch'],
                limit=3, ephemeral=True, delivery_admission=lambda: self.context._admit_inferred_task_branch(self.root, PUBLIC['branch'])))
        self.assertEqual(0, publication.call_count)
        self.assertEqual(0, receipt.call_count)

    def test_readonly_checks_branch_after_working_projection(self):
        self.native()
        original = self.context.project_working_task
        def project(*arguments, **options):
            result = original(*arguments, **options)
            self.switch(PUBLIC['other_branch'])
            return result
        before_db, before_shared = self.database.read_bytes(), self.shared()
        with mock.patch.object(self.context, 'project_working_task', side_effect=project):
            self.negative(self.main_call(['refresh', '--query', PUBLIC['query'], '--task-id', PUBLIC['branch'],
                                          '--expected-branch', PUBLIC['branch'], '--read-only', '--json']))
        self.assertEqual(before_db, self.database.read_bytes())
        self.assertEqual(before_shared, self.shared())

    def test_readonly_render_branch_move_is_refused_before_stdout(self):
        self.native()
        original = self.context.render_capsule_lines
        def render(*arguments, **options):
            result = original(*arguments, **options)
            self.switch(PUBLIC['other_branch'])
            return result
        before_db, before_shared = self.database.read_bytes(), self.shared()
        with mock.patch.object(self.context, 'render_capsule_lines', side_effect=render):
            self.negative(self.main_call(['refresh', '--query', PUBLIC['query'], '--task-id', PUBLIC['branch'],
                                          '--expected-branch', PUBLIC['branch'], '--read-only', '--json']))
        self.assertEqual(before_db, self.database.read_bytes())
        self.assertEqual(before_shared, self.shared())

    def test_lightweight_and_warning_packets_require_admission(self):
        self.native()
        connection = self.context.connect(self.database)
        self.addCleanup(connection.close)
        self.switch(PUBLIC['other_branch'])
        for mode, warnings in [('lightweight', None), ('governed', ['supplied unavailable index'])]:
            with self.subTest(mode=mode):
                self.refused(lambda: self.context.assemble_capsule(
                    connection, self.root, mode=mode, query=PUBLIC['query'], task_id=PUBLIC['branch'],
                    limit=3, ephemeral=True, refresh_index=False, index_warnings=warnings,
                    delivery_admission=lambda: self.context._admit_inferred_task_branch(self.root, PUBLIC['branch'])))

    def test_hook_none_and_skip_do_not_masquerade_as_valid_branch_outcome(self):
        self.native()
        for value in [None, {'gate': {'mode': 'enforce', 'decision': 'skip', 'reason': 'already-delivered'}}]:
            self.switch(PUBLIC['branch'])
            def assembled(*arguments, **options):
                self.switch(PUBLIC['other_branch'])
                return value
            with self.subTest(value=value), mock.patch.object(self.context, 'assemble_hook_context', side_effect=assembled):
                self.negative(self.main_call(['hook-context', '--task-id', PUBLIC['branch'],
                                              '--expected-branch', PUBLIC['branch'], '--json']))

    def test_hook_warming_assembler_requires_admission(self):
        connection = self.context.connect(self.base / 'warming.db')
        self.addCleanup(connection.close)
        def changed(*arguments, **options):
            self.switch(PUBLIC['other_branch'])
            return [PUBLIC['source']], []
        with mock.patch.object(self.context, 'changed_paths', side_effect=changed):
            self.refused(lambda: self.context.assemble_hook_context(
                connection, self.root, mode='governed', task_id=PUBLIC['branch'],
                delivery_admission=lambda: self.context._admit_inferred_task_branch(self.root, PUBLIC['branch'])))

    def test_hook_governed_pack_guard_precedes_manifest_and_receipt(self):
        self.native()
        connection = self.context.connect(self.database)
        self.addCleanup(connection.close)
        original = self.context.enforce_governed_capsule_contract
        def pack(*arguments, **options):
            result = original(*arguments, **options)
            self.switch(PUBLIC['other_branch'])
            return result
        with mock.patch.object(self.context, 'enforce_governed_capsule_contract', side_effect=pack), mock.patch.object(self.retrieval, 'atomic_json', wraps=self.retrieval.atomic_json) as publication, mock.patch.object(self.retrieval, '_commit_ordinary_receipts', wraps=self.retrieval._commit_ordinary_receipts) as receipt:
            self.refused(lambda: self.context.assemble_hook_context(
                connection, self.root, mode='governed', task_id=PUBLIC['branch'],
                delivery_admission=lambda: self.context._admit_inferred_task_branch(self.root, PUBLIC['branch'])))
        self.assertEqual(0, publication.call_count)
        self.assertEqual(0, receipt.call_count)

    def test_invalid_or_mismatched_raw_marker_precedes_state_creation(self):
        for expected in [*PUBLIC['invalid_expected'], PUBLIC['other_branch']]:
            with self.subTest(expected=expected), \
                 mock.patch.object(self.context, 'ensure_attached_state', side_effect=AssertionError('state must not create')) as state, \
                 mock.patch.object(self.context, 'connect', side_effect=AssertionError('database must not create')) as connection:
                self.negative(self.main_call(['refresh', '--task-id', PUBLIC['branch'], '--expected-branch', expected, '--json']))
                self.assertEqual(0, state.call_count)
                self.assertEqual(0, connection.call_count)

    def frame(self):
        return (str(self.root) + '\n\n' + str(self.root / '.git') + '\n').encode('utf-8')

    def test_symbolic_transport_requires_exact_full_head_ref(self):
        for row in PUBLIC['symbolic_cases']:
            # One symbolic-ref probe: the branch question needs no pinned frame.
            replies = [subprocess.CompletedProcess([], row['status'], row['output'].encode('utf-8'))]
            with self.subTest(name=row['name']), mock.patch.object(self.context.subprocess, 'run', side_effect=replies):
                operation = lambda: self.context._admit_inferred_task_branch(self.root, PUBLIC['branch'])
                if row['allow']:
                    self.assertIsNone(operation())
                else:
                    self.refused(operation)

    def test_branch_admission_is_one_unpinned_probe_with_its_own_deadline(self):
        # It runs up to three times a prompt; one Git start each, found from
        # the project itself, with room for a slow Git.
        replies = [subprocess.CompletedProcess([], 0, ('refs/heads/' + PUBLIC['branch'] + '\n').encode())]
        with mock.patch.object(self.context.time, 'monotonic', return_value=20.0), \
             mock.patch.object(self.context.subprocess, 'run', side_effect=replies) as run:
            self.context._admit_inferred_task_branch(self.root, PUBLIC['branch'])
        self.assertEqual(1, run.call_count)
        self.assertAlmostEqual(2.0, run.call_args.kwargs['timeout'])
        self.assertIn('symbolic-ref', run.call_args.args[0])
        self.assertNotIn('--git-dir=' + str(self.root / '.git'), run.call_args.args[0])

    def test_unencodable_expected_identity_has_fixed_refusal(self):
        for codepoint in PUBLIC['encoding_supplement']['invalid_utf8_expected_codepoints']:
            self.refused(lambda: self.context._admit_inferred_task_branch(self.root, chr(int(codepoint, 16))))

    def test_unmarked_explicit_request_adds_zero_branch_probes(self):
        with mock.patch.object(self.context, '_source_git_environment', side_effect=AssertionError('no inferred probe')):
            self.assertIsNone(self.context._admit_inferred_task_branch(self.root, None))

    def test_generated_cursor_session_and_stop_literals_preserve_marker_origin(self):
        for name, task in [('_WM_DELIVERY_SESSION_CURSOR', 'CAPSULE_TASK_ID'), ('_WM_DELIVERY_STOP_CURSOR', 'TASK_ID')]:
            text = getattr(self.retrieval, name)
            self.assertIn('CONTEXT_TASK_ID', text)
            self.assertIn('--expected-branch', text)
            self.assertEqual(2, text.count('"' + chr(36) + '{CAPSULE_IDENTITY_ARGUMENTS[@]}"'))
            self.assertIn('--task-id "$' + task + '"', text)

    def test_prompt_hook_marker_transport_inferred_vs_explicit(self):
        if BASH is None:
            self.skipTest('Bash transport unavailable')
        scripts = self.root / 'memory-bank/scripts'
        scripts.mkdir(parents=True)
        fake = '''import json,os,sys
from pathlib import Path
p=Path(os.environ['OTTER_ARGV_LOG'])
with p.open('a',encoding='utf-8') as f:f.write(json.dumps(sys.argv[1:])+'\\n')
if any(x in sys.argv for x in ['cursor-rule','cursor-rule-retire']):
 print(json.dumps({'state':'absent'}));raise SystemExit(0)
print('context: Inferred task branch changed or unavailable',file=sys.stderr)
raise SystemExit(1)
'''
        (scripts / 'context.py').write_text(fake, encoding='utf-8')
        binaries = self.base / 'bin'
        binaries.mkdir()
        try:
            (binaries / 'python3').symlink_to(sys.executable)
        except OSError as error:
            self.skipTest('interpreter symlink unavailable: ' + type(error).__name__)
        log = self.base / 'argv.ndjson'
        for row in PUBLIC['transport_supplement']['transport_cases']:
            host = row['host']
            hook = self.root / host / 'hooks/working-memory-read.sh'
            hook.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / 'PHP Core' / host / 'hooks/working-memory-read.sh', hook)
            log.write_bytes(b'')
            environment = dict(self.env, PATH=str(binaries) + os.pathsep + self.env['PATH'], OTTER_ARGV_LOG=str(log))
            if row['explicit']:
                environment['CONTEXT_TASK_ID'] = PUBLIC['branch']
                self.switch(PUBLIC['other_branch'])
            else:
                self.switch(PUBLIC['branch'])
            result = subprocess.run([BASH, str(hook)], cwd=self.root, env=environment,
                                    input=json.dumps({'prompt': PUBLIC['query']}).encode(),
                                    capture_output=True, timeout=15)
            self.observe('transport-' + host + '-' + str(row['explicit']), result)
            calls = [json.loads(line) for line in log.read_text(encoding='utf-8').splitlines()]
            refresh = [arguments for arguments in calls if arguments and arguments[0] == 'refresh']
            self.assertEqual(1, len(refresh))
            arguments = refresh[0]
            self.assertEqual(PUBLIC['branch'], arguments[arguments.index('--task-id') + 1])
            self.assertEqual(row['expected_marker'], '--expected-branch' in arguments)
            if row['expected_marker']:
                self.assertEqual(PUBLIC['branch'], arguments[arguments.index('--expected-branch') + 1])


if __name__ == '__main__':
    unittest.main()
