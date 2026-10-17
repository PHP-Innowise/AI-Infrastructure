"""NEW Juniper ordinary-source isolation; policy/authority remain strict."""
import contextlib
import hashlib
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

ROOT = Path(__file__).resolve().parents[1]
P = json.loads((ROOT/'tests/fixtures/memory_utf8_sources_public.json').read_text(encoding='utf-8'))


class Utf8SourceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='juniper-utf8-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)/'project'
        self.root.mkdir()
        env = {k: os.environ[k] for k in ('PATH', 'HOME', 'LANG', 'LC_ALL', 'LC_CTYPE',
               'SystemRoot', 'WINDIR', 'COMSPEC', 'PATHEXT') if k in os.environ}
        env.update(GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1',
                   TMPDIR=temporary.name, XDG_CONFIG_HOME=str(self.root/'xdg'))
        patch = mock.patch.dict(os.environ, env, clear=True)
        patch.start()
        self.addCleanup(patch.stop)
        subprocess.run(['git', '-C', str(self.root), 'init', '-q', '-b', 'juniper-utf8'],
                       env=env, check=True, capture_output=True, timeout=8)
        runtime = ROOT/'PHP Core/memory-bank/scripts'
        names = [p.stem for p in runtime.glob('*.py')]
        previous = {n: sys.modules.pop(n) for n in names if n in sys.modules}
        def restore():
            for n in names:
                sys.modules.pop(n, None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        sys.path.insert(0, str(runtime))
        try:
            self.c = importlib.import_module('context')
            self.r = importlib.import_module('context_retrieval')
            self.v = importlib.import_module('validate')
        finally:
            sys.path.remove(str(runtime))
        for rel in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/config',
                    'project-brain/schemas', 'project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel, self.root/rel,
                            ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        cfg = json.loads((self.root/'project-brain/config/runtime.json').read_text(encoding='utf-8'))
        cfg.update(automatic_promotion=False, automatic_compaction=False)
        self.write('project-brain/config/runtime.json', json.dumps(cfg).encode('utf-8'))
        self.write(P['source'], b'<?php // Juniper public source\n')
        self.write(P['valid'], P['text'].encode('utf-8'))
        self.valid_image = ((self.root/P['valid']).read_bytes(), (self.root/P['valid']).stat().st_mtime_ns)
        self.db = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(self.db.close)

    def write(self, name, raw):
        source_file = self.root/name
        source_file.parent.mkdir(parents=True, exist_ok=True)
        source_file.write_bytes(raw)
        return source_file

    def bad_name(self):
        raw = os.fsencode(self.root)+b'/docs/'+bytes.fromhex(P['invalid_basename_hex'])
        with open(raw, 'wb') as stream:
            stream.write(b'# Juniper omitted filename\n\nPublic filename-only fixture.\n')
        self.raw_name = raw
        return Path(os.fsdecode(raw))

    def bad_body(self):
        return self.write(P['invalid_body_path'], bytes.fromhex(P['invalid_body_hex']))

    def cli(self, args):
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.object(self.c, 'default_root', return_value=self.root), \
                mock.patch.object(sys, 'argv', ['context.py', *args]), \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = self.c.main()
        out.getvalue().encode('utf-8')
        err.getvalue().encode('utf-8')
        return status, out.getvalue(), err.getvalue()

    def refresh(self):
        status, output, error = self.cli(['refresh', '--task-id', P['task'], '--query', P['query'], '--json'])
        self.assertEqual(0, status, error)
        return json.loads(output)

    def check_fact(self, result, reason, notice):
        self.assertEqual([], result['warnings'])
        self.assertEqual({reason: 1}, result['source_notices'])
        self.assertEqual({reason: 1}, result['capsule']['source_notices'])
        self.assertEqual({P['valid']}, {row['path'] for row in result['capsule']['semantic']})
        self.assertIn(P['fact'], result['capsule_text'])
        self.assertIn(notice, result['capsule_text'])
        self.assertTrue(result['capsule_text'].startswith('working:'))
        self.assertNotIn('JUNIPER_PUBLIC_DECODE_CANARY', json.dumps(result))
        self.assertEqual(self.valid_image, ((self.root/P['valid']).read_bytes(), (self.root/P['valid']).stat().st_mtime_ns))
        rows = [json.loads(line) for line in (self.root/'memory-bank/local/refresh-health.ndjson').read_text(encoding='utf-8').splitlines()]
        self.assertEqual({reason: 1}, rows[-1]['source_notices'])
        self.assertTrue(rows[-1]['layers_updated'])
        self.assertEqual(0, rows[-1]['warnings'])

    @unittest.skipUnless(os.name == 'posix', 'Raw POSIX filename bytes unavailable')
    def test_bad_basename_native_refresh_keeps_unicode_neighbor(self):
        self.bad_name()
        self.check_fact(self.refresh(), 'invalid-utf8-path', P['path_notice'])
        all_rows = [tuple(row) for row in self.db.execute('SELECT path,layer,kind,title,summary,content FROM documents')]
        self.assertEqual([(P['valid'], 'semantic', 'spec', P['title'], P['summary'], P['text'])], all_rows)
        metadata = [tuple(row) for row in self.db.execute('SELECT * FROM document_metadata')]
        self.assertEqual([(P['valid'], 'evidence', 'public', '*', 'verified', 'active',
                          hashlib.sha256(P['text'].encode('utf-8')).hexdigest(), None, '[]', '[]', '', 1.0, '')], metadata)
        state = [tuple(row) for row in self.db.execute('SELECT * FROM document_source_state')]
        self.assertEqual([(P['valid'], self.valid_image[1], len(self.valid_image[0]), None)], state)
        self.assertEqual([], self.db.execute('SELECT * FROM document_links').fetchall())

    @unittest.skipUnless(os.name == 'posix', 'Raw POSIX filename bytes unavailable')
    def test_supplied_discovery_candidates_are_filtered_before_git(self):
        bad = self.bad_name()
        valid = self.root/P['valid']
        candidates = [('semantic', 'spec', f, f.relative_to(self.root).as_posix(),
                       (f.stat().st_mtime_ns, f.stat().st_size), 'docs/**/*.md') for f in (bad, valid)]
        original = self.c.git_ignored_paths
        seen = []
        def checked(repository, names):
            seen.extend(names)
            for name in names:
                name.encode('utf-8')
            return original(repository, names)
        with mock.patch.object(self.c, 'git_ignored_paths', side_effect=checked):
            documents, retained, state, excluded, masked = self.c.discover_documents(self.root, candidates=candidates)
        self.assertEqual([P['valid']], seen)
        self.assertEqual([(P['valid'], 'semantic', 'spec', P['title'], P['summary'], P['text'])], documents)
        self.assertEqual({}, retained)
        self.assertEqual({P['valid']}, set(state))
        self.assertEqual({}, masked)
        self.assertEqual(['invalid-utf8-path'], [row['reason'] for row in excluded])
        self.assertTrue(excluded[0]['path'].isascii())
        self.assertNotIn('juniper', excluded[0]['path'])

    @unittest.skipUnless(os.name == 'posix', 'Raw POSIX filename bytes unavailable')
    def test_incremental_repeat_reports_and_repair_removes_path_notice(self):
        self.bad_name()
        self.check_fact(self.refresh(), 'invalid-utf8-path', P['path_notice'])
        self.check_fact(self.refresh(), 'invalid-utf8-path', P['path_notice'])
        os.unlink(self.raw_name)
        repaired = self.refresh()
        self.assertNotIn('source_notices', repaired)
        self.assertNotIn('sources omitted:', repaired['capsule_text'])
        self.assertIn(P['fact'], repaired['capsule_text'])

    @unittest.skipUnless(os.name == 'posix', 'Raw POSIX filename bytes unavailable')
    def test_rename_fresh_indexed_file_to_bad_name_removes_old_cache(self):
        self.write(P['old_valid'], b'# Juniper old row\n\nPublic pre-rename source.\n')
        self.c.index_repository(self.db, self.root)
        bad = os.fsencode(self.root)+b'/docs/'+bytes.fromhex(P['invalid_basename_hex'])
        os.rename(os.fsencode(self.root/P['old_valid']), bad)
        result = self.c.index_repository(self.db, self.root, incremental=True)
        self.assertIn('invalid-utf8-path', [row['reason'] for row in result['excluded']])
        for table in ('documents', 'document_metadata', 'document_source_state'):
            self.assertEqual({P['valid']}, {row[0] for row in self.db.execute('SELECT path FROM '+table)})

    def test_invalid_body_notice_is_nonfatal_and_private(self):
        self.bad_body()
        self.check_fact(self.refresh(), 'invalid-utf8', P['body_notice'])

    def test_direct_assemble_refresh_carries_informational_counts(self):
        self.bad_body()
        result = self.c.assemble_capsule(self.db, self.root, mode='governed', query=P['query'],
            task_id=P['task'], limit=10, ephemeral=True, allow_unprovisioned=True, gate_mode='off', render=True)
        self.assertEqual([], result['warnings'])
        self.assertEqual({'invalid-utf8': 1}, result['source_notices'])
        self.assertEqual({P['valid']}, {row['path'] for row in result['semantic']})
        text = '\n'.join(self.c.render_capsule_lines(result, self.c.capsule_excerpts(self.db, result)))
        self.assertIn(P['body_notice'], text)
        self.assertIn(P['fact'], text)

    def test_operator_refresh_prints_counts_without_names(self):
        self.bad_body()
        status, output, error = self.cli(['refresh'])
        self.assertEqual(0, status, error)
        self.assertIn(P['body_notice'], output)
        self.assertNotIn(P['invalid_body_path'], output)
        self.assertNotIn('JUNIPER_PUBLIC_DECODE_CANARY', output+error)

    def test_empty_query_refresh_keeps_exclusion_diagnostic(self):
        self.bad_body()
        status, output, error = self.cli(['refresh', '--task-id', P['task'], '--query', '   ', '--sanitize', '--json'])
        self.assertEqual(0, status, error)
        result = json.loads(output)
        self.assertEqual({'invalid-utf8': 1}, result['source_notices'])
        self.assertEqual([], result['warnings'])

    def test_printed_refresh_stays_complete_with_notice(self):
        # The prompt hook prints this form into the turn: the omitted
        # neighbour is counted, and the valid fact still arrives.
        self.bad_body()
        status, output, error = self.cli(['refresh', '--task-id', P['task'], '--query', P['query']])
        self.assertEqual(0, status, error)
        self.assertIn(P['body_notice'], output)
        self.assertIn(P['fact'], output)

    def bank(self):
        self.write('memory-bank/README.md', b'# Juniper public bank\n')
        self.write('memory-bank/INDEX.md', b'# Juniper public bank index\n')
        return self.root/'memory-bank'

    def test_validator_bad_index_is_path_labelled_fixed_error(self):
        bank = self.bank()
        self.write('memory-bank/INDEX.md', bytes.fromhex(P['invalid_body_hex']))
        errors, warnings = self.v.validate_bank_report(bank, self.root)
        self.assertEqual([], warnings)
        self.assertEqual(['memory-bank/INDEX.md: invalid UTF-8 index'], errors)

    def test_validator_bad_chunk_is_path_labelled_fixed_error(self):
        bank = self.bank()
        self.write(P['chunk_path'], bytes.fromhex(P['invalid_body_hex']))
        errors, warnings = self.v.validate_bank_report(bank, self.root)
        self.assertEqual([], warnings)
        self.assertEqual(['memory-bank/chunks/MEM-1684-juniper-wheel.md: invalid UTF-8 chunk'], errors)

    def test_validator_decode_race_in_second_secret_read_is_fixed_error(self):
        bank = self.bank()
        chunk = self.write(P['chunk_path'], ('---\n'+json.dumps(P['chunk_metadata'])+'\n---\n# Juniper public wheel\n').encode('utf-8'))
        original = self.v.validate_secret_patterns
        def corrupt(source_file):
            source_file.write_bytes(bytes.fromhex(P['invalid_body_hex']))
            return original(source_file)
        with mock.patch.object(self.v, 'validate_secret_patterns', side_effect=corrupt):
            errors, warnings = self.v.validate_bank_report(bank, self.root)
        self.assertEqual([], warnings)
        self.assertEqual(['memory-bank/chunks/MEM-1684-juniper-wheel.md: invalid UTF-8 chunk'], errors)
        self.assertEqual(bytes.fromhex(P['invalid_body_hex']), chunk.read_bytes())

    def test_validator_cli_returns_failure_without_traceback_or_body(self):
        bank = self.bank()
        self.write(P['chunk_path'], bytes.fromhex(P['invalid_body_hex']))
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.object(sys, 'argv', ['validate.py', str(bank)]), \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = self.v.main()
        self.assertEqual(1, status)
        self.assertEqual('', out.getvalue())
        self.assertIn('chunks/MEM-1684-juniper-wheel.md: invalid UTF-8 chunk', err.getvalue())
        self.assertNotIn('Traceback', err.getvalue())
        self.assertNotIn('JUNIPER_PUBLIC_DECODE_CANARY', err.getvalue())

    def test_bank_summary_does_not_crash_on_invalid_chunk(self):
        bank = self.bank()
        self.write(P['chunk_path'], bytes.fromhex(P['invalid_body_hex']))
        self.assertEqual('Memory bank: 1 chunks (0 active, 0 needs review).', self.v.summarize_bank(bank))


if __name__ == '__main__':
    unittest.main()
