"""Fresh Plover generated-hook structure and trusted argv transport controls."""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tests.hook_shell import BASH

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_generator_identity_public.json').read_text(encoding='utf-8'))
SAFE_ENV = ('PATH', 'HOME', 'LANG', 'LC_ALL', 'LC_CTYPE', 'SystemRoot', 'WINDIR', 'COMSPEC', 'PATHEXT')


def write_utf8(destination, source):
    with destination.open('w', encoding='utf-8', newline='\n') as stream:
        stream.write(source)


class GeneratorIdentityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='plover-generator-')
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name).resolve()
        self.project = self.directory / 'project'
        self.project.mkdir()
        self.env = {key: os.environ[key] for key in SAFE_ENV if key in os.environ}
        self.env.update(GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1',
                        XDG_CONFIG_HOME=str(self.directory / 'profile'), TMPDIR=str(self.directory))
        patcher = mock.patch.dict(os.environ, self.env, clear=True)
        # Keep the observation selector out of child environments.
        self.observations = os.environ.get('PLOVER_OBSERVATIONS')
        self.validator_directory = Path(os.environ.get('PLOVER_VALIDATOR', str(ROOT / 'Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts')))
        patcher.start()
        self.addCleanup(patcher.stop)
        old_path = list(sys.path)
        names = [entry.stem for entry in self.validator_directory.glob('*.py')]
        previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}
        spec = importlib.util.spec_from_file_location('plover_generated_validator', self.validator_directory / 'validate_generated.py')
        self.validator = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = self.validator
        spec.loader.exec_module(self.validator)

        def restore():
            sys.path[:] = old_path
            sys.modules.pop(spec.name, None)
            for name in names:
                sys.modules.pop(name, None)
            sys.modules.update(previous)

        self.addCleanup(restore)
        self.files = {}
        for host in ('claude', 'cursor', 'codex'):
            for name in self.validator.REQUIRED_HOOKS[host]:
                relative = '.' + host + '/hooks/' + name
                script = '#!/usr/bin/env bash\nexit 0\n'
                if name == 'context-continuity.sh':
                    script = '#!/usr/bin/env bash\n: memory-bank/scripts/context_continuity.py --event hook --host ' + host + '\n'
                self.write(relative, script)
        for role, item in PUBLIC['roles'].items():
            self.write(item['relative'], PUBLIC['scripts'][role])

    def write(self, relative, source):
        destination = self.project / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        write_utf8(destination, source)
        destination.chmod(0o755)
        self.files[relative] = {}

    def observe(self, **values):
        if self.observations:
            with Path(self.observations).open('a', encoding='utf-8') as stream:
                stream.write(json.dumps({'method': self._testMethodName, **values}, ensure_ascii=False) + '\n')

    def check_errors(self, expected, role=None, variant=None):
        errors = []
        self.validator.validate_hooks(self.project, ['claude', 'cursor', 'codex'], self.files, errors)
        self.observe(role=role, variant=variant, errors=errors)
        self.assertEqual(errors, expected)

    def replace_role(self, role, source):
        self.write(PUBLIC['roles'][role]['relative'], source)

    def each_role_mutation(self, mutation):
        for role, item in PUBLIC['roles'].items():
            with self.subTest(role=role):
                original = PUBLIC['scripts'][role]
                self.replace_role(role, mutation(original, item))
                self.check_errors([PUBLIC['diagnostics'][role]], role)
            self.replace_role(role, original)

    def test_valid_five_roles(self):
        self.check_errors(PUBLIC['valid_errors'])
        for role in ('claude-prompt', 'cursor-session'):
            self.replace_role(role, PUBLIC['scripts'][role] + "NOTE='public\nplover'\ncat <<'PLOVER_DATA' >/dev/null\npublic plover\nPLOVER_DATA\ncat <<< 'public plover' >/dev/null\n")
        relay = 'run() {\n  if command -v timeout >/dev/null 2>&1; then\n    timeout "${CONTEXT_HOOK_BUDGET:-5}" "$@"\n  else\n    "$@"\n  fi\n}'
        self.replace_role('codex-prompt', PUBLIC['scripts']['codex-prompt'].replace('run() { "$@"; }', relay))
        self.check_errors(PUBLIC['valid_errors'], variant='quoted programs/heredoc/here-string/full relay')

    def test_missing_marker_rejected_in_every_role(self):
        self.each_role_mutation(lambda source, item: source.replace(item['guard'], ':'))

    def test_wrong_marker_identity_rejected(self):
        self.each_role_mutation(lambda source, item: source.replace('--expected-branch "$' + item['identity'] + '"', '--expected-branch "$OTHER_TASK"'))

    def test_unconditional_marker_rejected(self):
        def mutation(source, item):
            if item['array'] == 'ARGUMENTS':
                return source.replace('[ -n "${CONTEXT_TASK_ID:-}" ] || ', '')
            return source.replace('if [ -z "${CONTEXT_TASK_ID:-}" ] && [ -n "$' + item['identity'] + '" ]; then', 'if true; then')
        self.each_role_mutation(mutation)

    def test_data_only_marker_spoofs_rejected(self):
        for role, item in PUBLIC['roles'].items():
            for kind in ('comment', 'quote', 'heredoc'):
                with self.subTest(role=role, kind=kind):
                    guard = item['guard']
                    data = {'comment': '# ' + guard + '\n', 'quote': "NOTE='public plover\n" + guard + "\nend'\n", 'heredoc': "cat <<'PLOVER_DATA'\n" + guard + '\nPLOVER_DATA\n'}[kind]
                    self.replace_role(role, PUBLIC['scripts'][role].replace(guard, ':') + data)
                    self.check_errors([PUBLIC['diagnostics'][role]], role, kind)
                    self.replace_role(role, PUBLIC['scripts'][role])

    def test_missing_array_forwarding_rejected(self):
        self.each_role_mutation(lambda source, item: source.replace('"${' + item['array'] + '[@]}"', '"${' + item['array'] + '[*]}"'))

    def test_missing_renderer_timeout_arm_rejected(self):
        for role in ('cursor-turn', 'cursor-session'):
            item = PUBLIC['roles'][role]
            for arm in ('timed', 'untimed'):
                with self.subTest(role=role, arm=arm):
                    self.replace_role(role, PUBLIC['scripts'][role].replace(item[arm], ':'))
                    self.check_errors([PUBLIC['diagnostics'][role]], role, arm)
                    self.replace_role(role, PUBLIC['scripts'][role])

    def test_late_mutations_and_broken_relay_rejected(self):
        for role in ('claude-prompt', 'cursor-turn'):
            item = PUBLIC['roles'][role]
            anchor = item.get('report', item.get('timed'))
            array = item['array']
            for mutation in (array + '=()', 'unset ' + array, 'read -a ' + array + ' </dev/null', 'printf -v ' + array + " ''"):
                with self.subTest(role=role, mutation=mutation):
                    self.replace_role(role, PUBLIC['scripts'][role].replace(anchor, mutation + '\n' + anchor))
                    self.check_errors([PUBLIC['diagnostics'][role]], role, mutation)
                    self.replace_role(role, PUBLIC['scripts'][role])
        role = 'claude-prompt'
        self.replace_role(role, PUBLIC['scripts'][role].replace('run() { "$@"; }', 'run() { :; }'))
        self.check_errors([PUBLIC['diagnostics'][role]], role, 'discarded relay arguments')
        source = PUBLIC['scripts'][role].replace('run() { "$@"; }\n', '') + 'run() { "$@"; }\n'
        self.replace_role(role, source)
        self.check_errors([PUBLIC['diagnostics'][role]], role, 'relay after invocation')
        source = PUBLIC['scripts'][role].replace('run() { "$@"; }', 'run() {\n  if command -v timeout >/dev/null 2>&1; then\n    timeout "${CONTEXT_HOOK_BUDGET:-5}"\n  else\n    "$@"\n  fi\n}')
        self.replace_role(role, source)
        self.check_errors([PUBLIC['diagnostics'][role]], role, 'asymmetric relay forwarding')

    def test_live_multiline_quote_tail_mutation_rejected(self):
        for role in ('claude-prompt', 'cursor-session'):
            item = PUBLIC['roles'][role]
            anchor = item.get('report', item.get('timed'))
            self.replace_role(role, PUBLIC['scripts'][role].replace(anchor, "NOTE='public plover\nend'; " + item['array'] + '=()\n' + anchor))
            self.check_errors([PUBLIC['diagnostics'][role]], role)
            self.replace_role(role, PUBLIC['scripts'][role])

    def test_split_name_continuation_mutation_rejected(self):
        for role in ('codex-prompt', 'cursor-turn'):
            item = PUBLIC['roles'][role]
            anchor = item.get('report', item.get('timed'))
            split = item['array'][:4] + '\\\n' + item['array'][4:] + '=()'
            self.replace_role(role, PUBLIC['scripts'][role].replace(anchor, split + '\n' + anchor))
            self.check_errors([PUBLIC['diagnostics'][role]], role)
            self.replace_role(role, PUBLIC['scripts'][role])

    def test_an_exit_every_run_reaches_before_the_memory_call_is_rejected(self):
        # The contract checked the memory call's shape, not that a run gets
        # there: an unguarded exit before it passed.
        def after_fallback(source, item, lines):
            fallback = next(line for line in source.splitlines()
                            if line.startswith(item['identity'] + '="${CONTEXT_TASK_ID:-'))
            return source.replace(fallback, fallback + '\n' + lines)
        self.each_role_mutation(lambda source, item: after_fallback(source, item, 'exit 0'))
        # Guarded exits, and exits inside a block or a function, stay allowed.
        guarded = ('[ -n "$PLOVER_SKIP" ] && exit 0\nif [ -n "$PLOVER_SKIP" ]; then\n  exit 0\nfi\n'
                   'case "$PLOVER_SKIP" in\n  stop) exit 0 ;;\nesac\nplover_stop() { exit 0; }')
        for role, item in PUBLIC['roles'].items():
            with self.subTest(role=role, variant='guarded exits'):
                self.replace_role(role, after_fallback(PUBLIC['scripts'][role], item, guarded))
                self.check_errors(PUBLIC['valid_errors'], role, 'guarded exits')
            self.replace_role(role, PUBLIC['scripts'][role])

    def test_unowned_hook_is_never_read(self):
        relative = PUBLIC['roles']['claude-prompt']['relative']
        del self.files[relative]
        unowned = self.project / relative
        original = Path.read_text

        def read_text(source, *arguments, **options):
            if source == unowned:
                raise AssertionError('unowned Plover hook read')
            return original(source, *arguments, **options)

        with mock.patch.object(Path, 'read_text', read_text):
            self.check_errors(PUBLIC['unowned_errors'])

    def test_trusted_fixture_native_argv_transport(self):
        bash = BASH
        git = shutil.which('git')
        self.assertIsNotNone(bash)
        self.assertIsNotNone(git)
        # The recorder stands in for context.py. The hooks reach it through the
        # loader memory-seed ships, then - the loader gone, as in an older
        # runtime - directly; it notes which (only the loader sets __spec__).
        # Bytes, as context.py writes them: its stdout keeps LF on Windows,
        # where a text stream would end the line with CRLF.
        helper = self.project / 'memory-bank/scripts/context.py'
        helper.parent.mkdir(parents=True)
        write_utf8(helper, 'import json,os,sys\nfrom pathlib import Path\nwith Path(os.environ["PLOVER_RECORD"]).open("a",encoding="utf-8") as stream: stream.write(json.dumps(sys.argv[1:])+"\\n")\nwith Path(os.environ["PLOVER_ROUTE"]).open("a",encoding="utf-8") as stream: stream.write(("loader" if __spec__ else "direct")+"\\n")\nsys.stdout.buffer.write(' + repr(PUBLIC['reader_stdout'].encode('utf-8')) + ')\n')
        loader = helper.with_name('context_cli.py')
        shutil.copyfile(ROOT / 'Infrastructure-Creator/.agents/skills/memory-seed/assets/scripts/context_cli.py', loader)
        launcher_directory = self.directory / 'bin'
        launcher_directory.mkdir()
        launcher = launcher_directory / 'python3'
        write_utf8(launcher, '#!/usr/bin/env bash\nexec ' + shlex.quote(Path(sys.executable).as_posix()) + ' "$@"\n')
        launcher.chmod(0o755)
        deadline_driver = self.directory / 'bounded_dispatch.py'
        write_utf8(deadline_driver, 'import subprocess,sys\ntry: result=subprocess.run(sys.argv[2:],timeout=float(sys.argv[1]))\nexcept subprocess.TimeoutExpired: raise SystemExit(124)\nraise SystemExit(result.returncode)\n')
        bounded_launcher = launcher_directory / 'timeout'
        write_utf8(bounded_launcher, '#!/usr/bin/env bash\nexec ' + shlex.quote(Path(sys.executable).as_posix()) + ' ' + shlex.quote(deadline_driver.as_posix()) + ' "$@"\n')
        bounded_launcher.chmod(0o755)
        self.check_errors([])
        for route in ('loader', 'direct'):
            if route == 'direct':
                loader.unlink()
            for origin in ('inferred', 'explicit', 'absent'):
                if origin == 'inferred':
                    subprocess.run([git, 'init', '-q', '-b', PUBLIC['branch'], str(self.project)], env=self.env, check=True, capture_output=True)
                if origin == 'absent':
                    shutil.rmtree(self.project / '.git')
                for role, item in PUBLIC['roles'].items():
                    for timed in ((0, 1) if not role.endswith('prompt') else (0,)):
                        with self.subTest(route=route, origin=origin, role=role, timed=timed):
                            name = route + '-' + origin + '-' + role + '-' + str(timed)
                            record = self.directory / (name + '.jsonl')
                            routes = self.directory / (name + '.route')
                            environment = dict(self.env, PLOVER_RECORD=str(record), PLOVER_ROUTE=str(routes), PLOVER_TIMEOUT=str(timed))
                            environment['PATH'] = str(launcher_directory) + os.pathsep + environment.get('PATH', '')
                            if origin == 'explicit':
                                environment['CONTEXT_TASK_ID'] = PUBLIC['explicit']
                            result = subprocess.run([bash, str(self.project / item['relative'])], env=environment, capture_output=True, timeout=10)
                            calls = [json.loads(line) for line in record.read_text(encoding='utf-8').splitlines()] if record.exists() else []
                            taken = routes.read_text(encoding='utf-8').splitlines() if routes.exists() else []
                            expected = PUBLIC['expected_argv'][role][origin]
                            expected_stdout = PUBLIC['reader_stdout'].encode('utf-8') if expected else b''
                            if role == 'cursor-prompt':
                                # Cursor's prompt hook answers on every exit (trap ... EXIT).
                                expected_stdout += b'{"continue": true}\n'
                            self.observe(role=role, origin=origin, timed=timed, route=route, returncode=result.returncode, calls=calls, stdout_hex=result.stdout.hex(), stderr_hex=result.stderr.hex())
                            self.assertEqual((result.returncode, calls, taken, result.stdout, result.stderr),
                                             (0, expected, [route] * len(expected), expected_stdout, b''))


if __name__ == '__main__':
    unittest.main()
