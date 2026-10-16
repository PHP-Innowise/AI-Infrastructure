"""Independent Tern lexical boundaries and native-source compatibility."""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_generator_boundary_public.json').read_text(encoding='utf-8'))


class GeneratorBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.observations = os.environ.get('TERN_OBSERVATIONS')
        self.native = Path(os.environ.get('TERN_NATIVE_ROOT', str(ROOT)))
        directory = Path(os.environ.get('TERN_VALIDATOR', str(ROOT / 'Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts')))
        environment = {key: os.environ[key] for key in ('PATH', 'HOME', 'LANG', 'LC_ALL', 'LC_CTYPE', 'SystemRoot', 'WINDIR', 'COMSPEC', 'PATHEXT') if key in os.environ}
        environment.update(GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1')
        patcher = mock.patch.dict(os.environ, environment, clear=True)
        patcher.start()
        self.addCleanup(patcher.stop)
        old_path = list(sys.path)
        names = [entry.stem for entry in directory.glob('*.py')]
        previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}
        spec = importlib.util.spec_from_file_location('tern_validator', directory / 'validate_generated.py')
        self.validator = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = self.validator

        def restore():
            sys.path[:] = old_path
            sys.modules.pop(spec.name, None)
            for name in names:
                sys.modules.pop(name, None)
            sys.modules.update(previous)

        self.addCleanup(restore)
        spec.loader.exec_module(self.validator)

    def observe(self, **values):
        if self.observations:
            with Path(self.observations).open('a', encoding='utf-8') as stream:
                stream.write(json.dumps({'method': self._testMethodName, **values}, ensure_ascii=False) + '\n')

    def check(self, role, source, expected, case):
        item = PUBLIC['roles'][role]
        actual = self.validator.memory_hook_identity_contract(source, item['host'], item['name'])
        self.observe(role=role, case=case, actual=actual, expected=expected)
        self.assertIs(actual, expected)

    def check_case(self, name):
        for role in PUBLIC['roles']:
            with self.subTest(role=role, case=name):
                self.check(role, PUBLIC['cases'][role][name], PUBLIC['expected_rejection'], name)

    def test_canonical_profiles(self):
        for role, source in PUBLIC['positive'].items():
            with self.subTest(role=role):
                self.check(role, source, PUBLIC['expected_positive'], 'canonical')

    def test_quoted_literal_data_profiles(self):
        for role, source in PUBLIC['positive_data'].items():
            with self.subTest(role=role):
                self.check(role, source, PUBLIC['expected_positive'], 'quoted tab-stripped data')

    def test_unsupported_heredoc_word_boundaries(self):
        for name in ('hyphen_data', 'concatenated_data'):
            self.check_case(name)

    def test_quote_word_adjacency(self):
        self.check_case('quote_adjacent')

    def test_explicit_selector_reassignment(self):
        self.check_case('selector_assign')

    def test_selector_mutators(self):
        for name in ('selector_unset', 'selector_read', 'selector_printf'):
            self.check_case(name)

    def test_arithmetic_shift_is_not_data(self):
        self.check_case('arith_shift')

    def test_quoted_data_preserves_continuation(self):
        self.check_case('literal_continuation')

    def test_only_lf_starts_shell_code(self):
        for name in ('nextline', 'paragraph', 'verticaltab'):
            self.check_case(name)

    def test_native_profiles_and_json_contract(self):
        for role, relative in PUBLIC['native_paths'].items():
            source = (self.native / relative).read_text(encoding='utf-8')
            with self.subTest(role=role, kind='native public source'):
                self.check(role, source, True, 'native source profile')
            item = PUBLIC['roles'][role]
            changed = source.replace('--expected-branch "$' + item['identity'] + '"', '--expected-branch "$FOREIGN_TASK"')
            with self.subTest(role=role, kind='native missing binding'):
                self.check(role, changed, False, 'native wrong marker')
        with tempfile.TemporaryDirectory(prefix='tern-native-source-') as temporary:
            target = Path(temporary)
            for host in ('claude', 'codex', 'cursor'):
                shutil.copytree(self.native / 'PHP Core' / ('.' + host) / 'hooks', target / ('.' + host) / 'hooks')
            # Input source buffers are private nonexecutable evidence. The
            # generated-hook fixture explicitly requires executable scripts.
            for script in target.rglob('*.sh'):
                script.chmod(0o755)
            files = {str(f.relative_to(target)): {} for f in target.rglob('*.sh')}
            errors = []
            self.validator.validate_hooks(target, ['claude', 'cursor', 'codex'], files, errors)
            self.observe(case='all native owned hook source', errors=errors)
            with self.subTest(kind='whole native hook structural validation'):
                self.assertEqual(errors, PUBLIC['native_validate_errors'])
        role = 'cursor-prompt'
        original = PUBLIC['positive'][role]
        line = 'ARGUMENTS+=(--json)\n'
        report = 'REPORT=$(run'
        missing = original.replace(line, '')
        for name, source in [('missing JSON', missing), ('late JSON', missing + line), ('duplicate JSON', original.replace(line, line * 2))]:
            with self.subTest(kind=name):
                self.check(role, source, False, name)
        for role in ('claude-prompt', 'codex-prompt'):
            with self.subTest(role=role, kind='wrong host JSON'):
                self.check(role, PUBLIC['positive'][role].replace(report, line + report), False, 'wrong host JSON')


if __name__ == '__main__':
    unittest.main()
