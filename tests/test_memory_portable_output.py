"""Fresh public exact-byte controls for memory CLI stream startup."""
from __future__ import annotations

import importlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_portable_output_public.json').read_text(encoding='utf-8'))


class PortableOutputTests(unittest.TestCase):
    def setUp(self):
        self.runtime = Path(os.environ.get('CAPYBARA_RUNTIME', str(ROOT / 'PHP Core/memory-bank/scripts')))
        names = [entry.stem for entry in self.runtime.glob('*.py')]
        previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}

        def restore():
            for name in names:
                sys.modules.pop(name, None)
            sys.modules.update(previous)

        self.addCleanup(restore)
        sys.path.insert(0, str(self.runtime))
        try:
            self.context = importlib.import_module('context')
        finally:
            sys.path.remove(str(self.runtime))
        temporary = tempfile.TemporaryDirectory(prefix='capybara-streams-')
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name).resolve()
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith(('GIT_', 'CONTEXT_', 'PROJECT_BRAIN_', 'ACCELERATOR_'))}
        environment.update(XDG_CONFIG_HOME=str(self.directory / 'profile'),
                           TMPDIR=str(self.directory), GIT_OPTIONAL_LOCKS='0')
        patcher = mock.patch.dict(os.environ, environment, clear=True)
        patcher.start()
        self.addCleanup(patcher.stop)

    def observe(self, **values):
        destination = os.environ.get('CAPYBARA_OBSERVATIONS')
        if destination:
            with Path(destination).open('a', encoding='utf-8') as stream:
                stream.write(json.dumps({'method': self._testMethodName, **values}, ensure_ascii=False) + '\n')

    def wrapper(self, encoding='cp1252', newline='\r\n', **options):
        buffer = io.BytesIO()
        stream = io.TextIOWrapper(buffer, encoding=encoding, newline=newline, **options)
        self.addCleanup(stream.close)
        return stream, buffer

    def emit_pair(self, encoding, newline):
        stdout, output = self.wrapper(encoding, newline, line_buffering=True)
        stderr, errors = self.wrapper(encoding, newline, write_through=True)
        stdin = io.StringIO('capybara input\n')
        with mock.patch.object(sys, 'stdout', stdout), mock.patch.object(sys, 'stderr', stderr), mock.patch.object(sys, 'stdin', stdin):
            self.context.utf8_output()
            stdout.write(PUBLIC['stdout_text'])
            stderr.write(PUBLIC['stderr_text'])
            stdout.flush()
            stderr.flush()
            self.observe(stdout_hex=output.getvalue().hex(), stderr_hex=errors.getvalue().hex())
            self.assertEqual(output.getvalue().hex(), PUBLIC['stdout_hex'])
            self.assertEqual(errors.getvalue().hex(), PUBLIC['stderr_hex'])
            self.assertIs(stdout.buffer, output)
            self.assertIs(stderr.buffer, errors)
            self.assertTrue(stdout.line_buffering)
            self.assertTrue(stderr.write_through)
            self.assertFalse(stdout.closed)
            self.assertFalse(stderr.closed)
            self.assertIs(sys.stdin, stdin)
            self.assertEqual(stdin.read(), 'capybara input\n')

    def test_crlf_streams_emit_exact_utf8_lf(self):
        self.emit_pair('cp1252', '\r\n')

    def test_cr_streams_emit_exact_utf8_lf(self):
        self.emit_pair('utf-8', '\r')

    def test_lf_streams_keep_exact_bytes_and_properties(self):
        self.emit_pair('utf-8', '\n')

    def test_explicit_cr_content_is_not_normalized(self):
        stdout, output = self.wrapper()
        with mock.patch.object(sys, 'stdout', stdout), mock.patch.object(sys, 'stderr', io.StringIO()):
            self.context.utf8_output()
            stdout.write(PUBLIC['explicit_cr_text'])
            stdout.flush()
            self.observe(stdout_hex=output.getvalue().hex())
            self.assertEqual(output.getvalue().hex(), PUBLIC['explicit_cr_hex'])

    def test_prior_buffered_bytes_keep_original_encoding(self):
        stdout, output = self.wrapper(newline='\n')
        stdout.write(PUBLIC['prefix_text'])
        with mock.patch.object(sys, 'stdout', stdout), mock.patch.object(sys, 'stderr', io.StringIO()):
            self.context.utf8_output()
            stdout.write(PUBLIC['stdout_text'])
            stdout.flush()
            self.observe(stdout_hex=output.getvalue().hex())
            self.assertEqual(output.getvalue().hex(), PUBLIC['prefix_hex'] + PUBLIC['stdout_hex'])

    def test_repeated_startup_is_idempotent(self):
        stdout, output = self.wrapper()
        with mock.patch.object(sys, 'stdout', stdout), mock.patch.object(sys, 'stderr', io.StringIO()):
            for _ in range(2):
                self.context.utf8_output()
                stdout.write(PUBLIC['stdout_text'])
            stdout.flush()
            self.observe(stdout_hex=output.getvalue().hex())
            self.assertEqual(output.getvalue().hex(), PUBLIC['stdout_hex'] * 2)

    def test_custom_legacy_api_receives_one_encoding_only_call(self):
        class Legacy:
            def __init__(self):
                self.calls = []

            def reconfigure(self, *, encoding, errors):
                self.calls.append((encoding, errors))

        stdout, stderr = Legacy(), Legacy()
        with mock.patch.object(sys, 'stdout', stdout), mock.patch.object(sys, 'stderr', stderr):
            self.context.utf8_output()
        self.observe(stdout_calls=stdout.calls, stderr_calls=stderr.calls)
        self.assertEqual(stdout.calls, [('utf-8', 'replace')])
        self.assertEqual(stderr.calls, [('utf-8', 'replace')])

    def test_subclass_legacy_api_does_not_receive_new_keyword(self):
        class LegacyWrapper(io.TextIOWrapper):
            def reconfigure(self, *, encoding, errors):
                self.calls.append((encoding, errors))
                super().reconfigure(encoding=encoding, errors=errors)

        stdout = LegacyWrapper(io.BytesIO(), encoding='cp1252', newline='\r\n')
        stdout.calls = []
        self.addCleanup(stdout.close)
        with mock.patch.object(sys, 'stdout', stdout), mock.patch.object(sys, 'stderr', io.StringIO()):
            self.context.utf8_output()
        self.observe(calls=stdout.calls)
        self.assertEqual(stdout.calls, [('utf-8', 'replace')])

    def test_unsupported_streams_do_not_block_other_stream(self):
        class Broken:
            def __init__(self, exception):
                self.exception = exception
                self.calls = 0

            def reconfigure(self, **options):
                self.calls += 1
                raise self.exception

        for exception in (AttributeError('capybara missing'), TypeError('capybara signature'), ValueError('capybara closed'), OSError('capybara unavailable')):
            stdout = Broken(exception)
            stderr, errors = self.wrapper()
            with self.subTest(exception=type(exception).__name__), mock.patch.object(sys, 'stdout', stdout), mock.patch.object(sys, 'stderr', stderr):
                self.context.utf8_output()
                stderr.write(PUBLIC['stderr_text'])
                stderr.flush()
                self.observe(exception=type(exception).__name__, calls=stdout.calls, stderr_hex=errors.getvalue().hex())
                self.assertEqual(stdout.calls, 1)
                self.assertEqual(errors.getvalue().hex(), PUBLIC['stderr_hex'])
        with mock.patch.object(sys, 'stdout', object()), mock.patch.object(sys, 'stderr', io.StringIO()):
            self.context.utf8_output()

    def test_closed_standard_stream_does_not_block_other_stream(self):
        stdout, _ = self.wrapper()
        stdout.close()
        stderr, errors = self.wrapper()
        with mock.patch.object(sys, 'stdout', stdout), mock.patch.object(sys, 'stderr', stderr):
            self.context.utf8_output()
            stderr.write(PUBLIC['stderr_text'])
            stderr.flush()
            self.observe(stderr_hex=errors.getvalue().hex())
            self.assertEqual(errors.getvalue().hex(), PUBLIC['stderr_hex'])

    def test_cancellation_identity_is_preserved(self):
        class Cancelled:
            def reconfigure(self, **options):
                raise cancellation

        for cancellation in (KeyboardInterrupt('capybara interrupt'), SystemExit('capybara exit')):
            stderr = mock.Mock()
            with self.subTest(exception=type(cancellation).__name__), mock.patch.object(sys, 'stdout', Cancelled()), mock.patch.object(sys, 'stderr', stderr):
                with self.assertRaises(type(cancellation)) as caught:
                    self.context.utf8_output()
                self.assertIs(caught.exception, cancellation)
                stderr.reconfigure.assert_not_called()
                self.observe(exception=type(cancellation).__name__, identity=True, next_stream_calls=0)

    def test_actual_main_error_has_exact_bytes(self):
        project = self.directory / 'project'
        project.mkdir()
        stdout, output = self.wrapper()
        stderr, errors = self.wrapper()
        arguments = ['context.py', '--root', str(project), 'search', PUBLIC['query'], '--limit', '0']
        with mock.patch.object(sys, 'argv', arguments), mock.patch.object(sys, 'stdout', stdout), mock.patch.object(sys, 'stderr', stderr):
            result = self.context.main()
            stdout.flush()
            stderr.flush()
            self.observe(returncode=result, stdout_hex=output.getvalue().hex(), stderr_hex=errors.getvalue().hex())
            self.assertEqual(result, 1)
            self.assertEqual(output.getvalue(), b'')
            self.assertEqual(errors.getvalue().hex(), PUBLIC['main_error_hex'])


if __name__ == '__main__':
    unittest.main()
