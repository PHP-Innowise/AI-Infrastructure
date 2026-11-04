import copy
import hashlib
import json
import unittest
from unittest import mock
import tempfile

from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'scripts'))
import memory_response_contract as fc


def contract():
    return {
        'schema': 'memory-response-contract-v1',
        'case_id': 'public-case',
        'marker': 'MEMORY_TASK_ANSWER',
        'answer_fields': {
            'count': {'type': 'integer', 'nullable': False},
            'flag': {'type': 'boolean', 'nullable': False},
            'note': {'type': 'string', 'nullable': True},
        },
        'limits': {
            'max_messages': 8, 'max_message_bytes': 4096, 'max_total_bytes': 16384,
            'max_answer_string_chars': 16, 'integer_abs_max': 100,
            'min_citations': 1, 'max_citations': 2, 'max_citation_span_lines': 3,
            'max_citation_quote_chars': 8, 'max_citation_path_chars': 32,
            'min_explanation_chars': 4, 'max_explanation_chars': 20,
        },
    }


def response():
    return {
        'case_id': 'public-case',
        'answer': {'count': 1, 'flag': True, 'note': None},
        'citations': [{'path': 'src/File.php', 'start_line': 1, 'end_line': 3, 'quote': 'source'}],
        'explanation': 'A clear explanation.',
    }


def message(value=None, marker='MEMORY_TASK_ANSWER', ending='\n'):
    return marker + ending + json.dumps(response() if value is None else value, ensure_ascii=False)


def context():
    return {'protocol_sha256': 'a' * 64, 'slot_id': 'slot-1',
            'verifier_sha256': 'b' * 64, 'source_image_sha256': 'c' * 64}


def receipt(artifact, verdict='consistent'):
    return json.dumps({'schema': 'independent-answer-review-v1', 'binding': artifact['binding'],
                       'reviewer_id': 'independent-agent', 'reviewer_kind': 'agent',
                       'verdict': verdict}, ensure_ascii=False)


def canonical_digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True,
                                    separators=(',', ':'), allow_nan=False).encode('utf-8')).hexdigest()


def corpus_contract(sources=None):
    sources = {'src/File.php': b'source\nsecond\nthird\n'} if sources is None else sources
    public = contract()
    public['schema'] = 'memory-response-contract-v2'
    public['citation_corpus'] = {'schema': 'memory-citation-corpus-v1', 'files': [
        {'path': path, 'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw)}
        for path, raw in sorted(sources.items())]}
    if not sources:
        public['limits'].update(min_citations=0, max_citations=0)
    return public


class V1CompatibilityTests(unittest.TestCase):
    def test_pre_edit_golden_outputs_remain_exact(self):
        # Recorded from helper fbb3508405a82eee77cf48ebc664d8e52ac7b67b9a69da9ae2b30a8689f591bb
        # before the v2 edit. These are not recomputed expected values.
        public = contract(); texts = ['progress', message(), 'Saved.']
        artifact = fc.make_artifact(public, texts, context())
        projection = {
            'validated': fc.validate_contract(public), 'rendered': fc.render_contract(public),
            'contract_digest': fc.contract_digest(public),
            'checks': [fc.check_response(public, text) for text in
                       [message(), message(ending='\r\n'), 'MEMORY_TASK_ANSWER\n{', 'unmarked']],
            'selections': [fc.select_answer(public, items) for items in
                           [texts, [message(), 'MEMORY_TASK_ANSWER\n{', 'Saved.'], ['unmarked'], []]],
            'artifact': artifact,
            'reviews': [fc.review_decision(public, artifact, machine, review)
                        for machine in [True, False, None]
                        for review in [None, receipt(artifact), receipt(artifact, 'contradiction'),
                                       receipt(artifact, 'uncertain')]],
        }
        self.assertEqual('199a78eb7875ff67c28a8500b0ce7f74c23b0c34bf4ef516a3428e99ffb49a96',
                         hashlib.sha256(projection['rendered'].encode('utf-8')).hexdigest())
        self.assertEqual(1949, len(projection['rendered'].encode('utf-8')))
        self.assertEqual('160539ffeb996e91e4248fa1c2f1f45080f21d52f7fccc181ed59ea9f4a9f849',
                         fc.contract_digest(public))
        self.assertEqual('5d72c2d229577b9dd11b750a1052792541952640af92f1d98dce2bdc25a34159',
                         canonical_digest(artifact))
        self.assertEqual('741bc238526a069d4068ac25a52a9ad25e6fdee4eff6db3d03880528bb3ac885',
                         canonical_digest(projection))

    def test_v1_remains_shape_only_and_cannot_silently_adopt_a_corpus(self):
        candidate = response(); candidate['citations'][0]['path'] = '../readable/file'
        self.assertTrue(fc.check_response(contract(), message(candidate))['valid'])
        public = contract(); public['citation_corpus'] = corpus_contract()['citation_corpus']
        with self.assertRaises(ValueError): fc.validate_contract(public)
        with self.assertRaises(ValueError): fc.check_citations(contract(), message(), {})


class CitationCorpusTests(unittest.TestCase):
    def setUp(self):
        self.sources = {'src/File.php': b'source\nsecond\nthird\n'}
        self.public = corpus_contract(self.sources)

    def check(self, value=None, public=None, sources=None):
        return fc.check_citations(self.public if public is None else public, message(value),
                                  self.sources if sources is None else sources)

    def test_render_discloses_complete_exact_corpus_and_literal_rules(self):
        sources = {'p%03d.txt' % index: b'x' for index in range(256)}
        public = corpus_contract(sources)
        rendered = fc.render_contract(public)
        self.assertEqual(public, json.loads(rendered.split('\nPUBLIC_CONTRACT\n', 1)[1]))
        for entry in public['citation_corpus']['files']:
            self.assertIn(entry['path'], rendered); self.assertIn(entry['sha256'], rendered)
        for rule in ('not eligible', 'LF-delimited', 'CR in CRLF', 'lone CR', 'exact contiguous',
                     'not factual truth', 'complete prompt transport'):
            self.assertIn(rule, rendered)
        self.assertEqual(rendered, fc.render_contract(copy.deepcopy(public)))

    def test_installed_readable_but_unlisted_source_is_publicly_ineligible(self):
        with tempfile.TemporaryDirectory() as folder:
            catalog = Path(folder) / 'catalog.txt'; catalog.write_bytes(b'source\n')
            self.assertEqual(b'source\n', catalog.read_bytes())
            candidate = response(); candidate['citations'][0].update(path='runtime/catalog.txt', end_line=1)
            self.assertFalse(fc.check_response(self.public, message(candidate))['valid'])
            self.assertFalse(self.check(candidate)['valid'])
            included = {**self.sources, 'runtime/catalog.txt': catalog.read_bytes()}
            public = corpus_contract(included)
            self.assertTrue(self.check(candidate, public, included)['valid'])

    def test_empty_corpus_and_empty_source_have_distinct_line_semantics(self):
        public = corpus_contract({}); candidate = response(); candidate['citations'] = []
        self.assertTrue(self.check(candidate, public, {})['valid'])
        self.assertFalse(self.check(response(), public, {})['valid'])
        with self.assertRaises(ValueError): self.check(candidate, public, {'extra': b''})
        for field in ('min_citations', 'max_citations'):
            changed = copy.deepcopy(public); changed['limits'][field] = 1
            with self.subTest(field=field), self.assertRaises(ValueError):fc.validate_contract(changed)
        empty = {'src/File.php': b''}; public = corpus_contract(empty)
        self.assertFalse(self.check(response(), public, empty)['valid'])
        public['limits']['min_citations'] = 0
        self.assertTrue(self.check(candidate, public, empty)['valid'])

    def test_context_drift_is_an_exception_before_reader_response_errors(self):
        contexts = [{}, {**self.sources, 'extra': b'x'}, {'src/File.php': b'changed'},
                    {'src/File.php': bytearray(self.sources['src/File.php'])},
                    {'src/File.php': self.sources['src/File.php'].decode('utf-8')}]
        for sources in contexts:
            with self.subTest(sources=sources), self.assertRaises(ValueError):self.check(sources=sources)
        with self.assertRaises(ValueError):fc.check_citations(self.public, 'bad reader answer', {})
        self.assertFalse(fc.check_citations(self.public, 'bad reader answer', self.sources)['valid'])
        for field, value in [('bytes', 1), ('sha256', '0' * 64)]:
            public = copy.deepcopy(self.public); public['citation_corpus']['files'][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):self.check(public=public)
        bad_utf8 = {'src/File.php': b'\xff'}
        with self.assertRaises(ValueError):self.check(public=corpus_contract(bad_utf8), sources=bad_utf8)
        class DictSubclass(dict):pass
        class BytesSubclass(bytes):pass
        class StringSubclass(str):pass
        for sources in (DictSubclass(self.sources), {'src/File.php': BytesSubclass(b'x')},
                        {StringSubclass('src/File.php'): self.sources['src/File.php']}):
            with self.assertRaises(ValueError):self.check(sources=sources)

    def test_exact_quote_and_existing_range_are_independent_of_shape(self):
        self.assertTrue(self.check()['valid'])
        for start, end, quote in [(1, 1, 'second'), (4, 4, 'source'), (1, 4, 'source'),
                                  (1, 1, 'Source'), (1, 1, ' source'), (1, 1, 'sour...')]:
            candidate = response(); candidate['citations'][0].update(start_line=start, end_line=end, quote=quote)
            # Use a large enough disclosed span so out-of-file differs from shape failure.
            public = copy.deepcopy(self.public); public['limits']['max_citation_span_lines'] = 4
            self.assertTrue(fc.check_response(public, message(candidate))['valid'])
            self.assertIsNone(fc.preflight_reference(public, message(candidate)))
            with self.subTest(start=start, end=end, quote=quote):self.assertFalse(self.check(candidate, public)['valid'])
        candidate = response(); candidate['citations'][0].update(start_line=2, end_line=2, quote='second')
        self.assertTrue(self.check(candidate)['valid'])

    def test_literal_crlf_unicode_bom_and_lone_cr_are_not_normalized(self):
        public = copy.deepcopy(self.public); public['limits']['max_citation_quote_chars'] = 100
        for raw, quote, end in [(b'one\r\ntwo\r\n', 'one\r\ntwo', 2),
                                ('α\nβ\n'.encode('utf-8'), 'α\nβ', 2),
                                ('\ufeffone\n'.encode('utf-8'), '\ufeffone', 1),
                                ('one\rtwo\u2028three'.encode('utf-8'), 'one\rtwo\u2028three', 1)]:
            sources = {'src/File.php': raw}; public['citation_corpus'] = corpus_contract(sources)['citation_corpus']
            candidate = response(); candidate['citations'][0].update(quote=quote, end_line=end)
            with self.subTest(raw=raw):self.assertTrue(self.check(candidate, public, sources)['valid'])
        sources = {'src/File.php': b'one\r\ntwo\r\n'}
        public['citation_corpus'] = corpus_contract(sources)['citation_corpus']
        candidate['citations'][0].update(quote='one\ntwo', end_line=2)
        self.assertFalse(self.check(candidate, public, sources)['valid'])
        candidate['citations'][0].update(quote='two', start_line=3, end_line=3)
        self.assertFalse(self.check(candidate, public, sources)['valid'])

    def test_line_windows_match_an_independent_small_text_reference(self):
        for text in ('', '\n', '\n\n', 'a', 'a\n', 'α\r\n\nβ\r\n', '\ufeffa\rb\u2028c\n'):
            sources = {'src/File.php': text.encode('utf-8')}; public = corpus_contract(sources)
            public['limits'].update(max_citation_quote_chars=100, max_citation_span_lines=6)
            lines = text.split('\n') if text else []
            if text.endswith('\n'):lines.pop()
            for start in range(1, len(lines) + 2):
                for end in range(start, len(lines) + 2):
                    for quote in ('a', 'α', '\n', '\r', 'β\r', 'absent'):
                        candidate = response(); candidate['citations'][0].update(
                            start_line=start, end_line=end, quote=quote)
                        expected = end <= len(lines) and quote in '\n'.join(lines[start - 1:end])
                        with self.subTest(text=repr(text), start=start, end=end, quote=repr(quote)):
                            self.assertEqual(expected, self.check(candidate, public, sources)['valid'])

    def test_multiple_ranges_and_sources_share_the_same_frozen_context(self):
        sources = {**self.sources, 'src/Other.php': b'other\nsource\n'}; public = corpus_contract(sources)
        candidate = response(); first = candidate['citations'][0]; first.update(start_line=2, end_line=2, quote='second')
        candidate['citations'].append({'path': 'src/Other.php', 'start_line': 1, 'end_line': 1, 'quote': 'other'})
        self.assertTrue(self.check(candidate, public, sources)['valid'])
        candidate['citations'][1].update(path='src/File.php', start_line=3, end_line=3, quote='third')
        self.assertTrue(self.check(candidate, public, sources)['valid'])
        candidate['citations'][1]['quote'] = 'second'
        self.assertFalse(self.check(candidate, public, sources)['valid'])

    def test_sources_are_snapshotted_at_entry_and_inputs_are_not_mutated(self):
        original = copy.deepcopy((self.public, self.sources, response()))
        self.assertTrue(self.check()['valid'])
        self.assertEqual(original, (self.public, self.sources, response()))
        real_validate = fc.validate_contract
        def replace_sources(public):
            self.sources.clear(); self.sources['wrong'] = b'changed'
            return real_validate(public)
        with mock.patch.object(fc, 'validate_contract', side_effect=replace_sources):
            self.assertTrue(self.check()['valid'])

    def test_no_filesystem_network_or_process_work_is_delegated(self):
        import socket
        import subprocess
        with mock.patch('builtins.open', side_effect=AssertionError('filesystem')), \
             mock.patch.object(Path, 'open', side_effect=AssertionError('filesystem')), \
             mock.patch.object(socket, 'socket', side_effect=AssertionError('network')), \
             mock.patch.object(subprocess, 'Popen', side_effect=AssertionError('process')):
            fc.render_contract(self.public)
            self.assertTrue(self.check()['valid'])

    def test_paths_are_canonical_literals_and_membership_has_no_aliases(self):
        for path in ('/absolute', 'C:/file', '//server/file', 'https://host/file', 'a\\b', './a',
                     'a/../b', 'a//b', 'a/', '', 'a\0b', 'a\nb', 'a\x7fb', '\ud800'):
            public = copy.deepcopy(self.public); public['citation_corpus']['files'][0]['path'] = path
            with self.subTest(path=repr(path)), self.assertRaises(ValueError):fc.validate_contract(public)
            candidate = response(); candidate['citations'][0]['path'] = path
            self.assertFalse(fc.check_response(self.public, message(candidate))['valid'])
        sources = {'src/é.txt': b'source', 'src/%2e%2e.txt': b'source'}
        public = corpus_contract(sources)
        for path, valid in [('src/é.txt', True), ('src/e\u0301.txt', False), ('SRC/é.txt', False),
                            ('src/%2e%2e.txt', True), ('src/...txt', False)]:
            candidate = response(); candidate['citations'][0].update(path=path, end_line=1)
            with self.subTest(path=path):self.assertEqual(valid, self.check(candidate, public, sources)['valid'])

    def test_descriptor_keys_hashes_types_duplicates_and_order_are_strict(self):
        class UnsupportedSchema:
            def __eq__(self, other):raise AssertionError('unsupported schema equality must not execute')
        public = copy.deepcopy(self.public); public['schema'] = UnsupportedSchema()
        with self.assertRaises(ValueError):fc.validate_contract(public)
        for change in ({'schema': 'unknown'}, {'schema': True}, {'files': {}}, {'extra': 'scope prose'}):
            public = copy.deepcopy(self.public); public['citation_corpus'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):fc.validate_contract(public)
        for change in ({'bytes': True}, {'bytes': -1}, {'bytes': 1.0}, {'sha256': 'A' * 64},
                       {'sha256': 'f' * 63}, {'sha256': True}, {'extra': 1}):
            public = copy.deepcopy(self.public); public['citation_corpus']['files'][0].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):fc.validate_contract(public)
        for files in ([self.public['citation_corpus']['files'][0]] * 2,
                      corpus_contract({'a': b'x', 'b': b'x'})['citation_corpus']['files'][::-1]):
            public = copy.deepcopy(self.public); public['citation_corpus']['files'] = files
            with self.assertRaises(ValueError):fc.validate_contract(public)

    def test_file_count_path_and_byte_boundaries(self):
        sources = {'p%03d' % index: b'x' for index in range(fc.CORPUS_MAX_FILES)}
        public = corpus_contract(sources); fc.validate_contract(public)
        public['citation_corpus']['files'].append({'path': 'z', 'sha256': '0' * 64, 'bytes': 0})
        with self.assertRaises(ValueError):fc.validate_contract(public)
        with self.assertRaises(ValueError):self.check(sources={str(index): b'' for index in range(257)})
        public = corpus_contract({'p' * 1024: b'x'}); public['limits']['max_citation_path_chars'] = 1024
        fc.validate_contract(public)
        public['citation_corpus']['files'][0]['path'] += 'p'
        with self.assertRaises(ValueError):fc.validate_contract(public)
        raw = b'source' + b'x' * (fc.CORPUS_MAX_FILE_BYTES - 6)
        sources = {'src/File.php': raw}; public = corpus_contract(sources)
        candidate = response(); candidate['citations'][0]['end_line'] = 1
        self.assertTrue(self.check(candidate, public, sources)['valid'])
        public['citation_corpus']['files'][0]['bytes'] += 1
        with self.assertRaises(ValueError):fc.validate_contract(public)
        sources = {'p%02d' % index: raw for index in range(16)}
        public = corpus_contract(sources); candidate['citations'][0]['path'] = 'p00'
        self.assertTrue(self.check(candidate, public, sources)['valid'])
        public['citation_corpus']['files'].append({'path': 'z', 'sha256': hashlib.sha256(b'x').hexdigest(), 'bytes': 1})
        with self.assertRaises(ValueError):fc.validate_contract(public)

    def test_complete_descriptor_byte_boundary_and_render_refusal_never_clip(self):
        public = corpus_contract({'p%03d' % index: b'' for index in range(100)})
        public['limits']['max_citation_path_chars'] = 1024
        corpus = public['citation_corpus']
        size = lambda:len(json.dumps(corpus, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode('utf-8'))
        remaining = fc.CORPUS_MAX_DESCRIPTOR_BYTES - size()
        for entry in corpus['files']:
            extra = min(remaining, 1024 - len(entry['path']))
            entry['path'] += 'x' * extra; remaining -= extra
        self.assertEqual(0, remaining); self.assertEqual(fc.CORPUS_MAX_DESCRIPTOR_BYTES, size())
        fc.validate_contract(public)
        rendered = fc.render_contract(public)
        self.assertEqual(public, json.loads(rendered.split('\nPUBLIC_CONTRACT\n', 1)[1]))
        public['citation_corpus']['files'][-1]['path'] += 'x'
        with self.assertRaises(ValueError):fc.validate_contract(public)
        # The render cap is defense in depth above the other admission bounds.
        # Tightening it to this exact valid rendering exercises both sides without clipping.
        public = self.public; length = len(fc.render_contract(public).encode('utf-8'))
        with mock.patch.object(fc, 'V2_MAX_RENDERED_BYTES', length):fc.validate_contract(public)
        with mock.patch.object(fc, 'V2_MAX_RENDERED_BYTES', length - 1):
            with self.assertRaises(ValueError):fc.render_contract(public)

    def test_descriptor_byte_cap_counts_unicode_utf8_not_code_points(self):
        for suffix, valid in [('x' * 1020, True), ('Ω' * 1020, False)]:
            public = corpus_contract({'p%03d' % index + suffix: b'' for index in range(32)})
            public['limits']['max_citation_path_chars'] = 1024
            if valid:fc.validate_contract(public)
            else:
                with self.assertRaises(ValueError):fc.validate_contract(public)

    def test_changed_public_corpus_invalidates_a_stale_review_binding(self):
        first = fc.make_artifact(self.public, [message()], context())
        for change in ({'sha256': '0' * 64}, {'bytes': 1}, {'path': 'src/Other.php'}):
            public = copy.deepcopy(self.public); public['citation_corpus']['files'][0].update(change)
            candidate = response(); candidate['citations'][0]['path'] = public['citation_corpus']['files'][0]['path']
            second = fc.make_artifact(public, [message(candidate)], context())
            self.assertNotEqual(first['binding']['public_contract_sha256'], second['binding']['public_contract_sha256'])
            result = fc.review_decision(public, second, True, receipt(first))
            self.assertIsNone(result['task_correct']); self.assertEqual('mismatch', result['review_status'])


class PublicContractTests(unittest.TestCase):
    def setUp(self):
        self.contract = contract()

    def test_rendered_contract_is_the_exact_checked_public_object(self):
        rendered = fc.render_contract(self.contract)
        public = json.loads(rendered.split('\nPUBLIC_CONTRACT\n', 1)[1])
        self.assertEqual(self.contract, public)
        self.assertIn('first line', rendered)
        self.assertIn('MEMORY_TASK_ANSWER', rendered)
        self.assertIn('UTF-8 bytes', rendered)
        self.assertEqual(rendered, fc.render_contract(copy.deepcopy(self.contract)))
        self.assertNotIn('expected_answer', rendered)

    def test_contract_rejects_gold_unknown_keys_and_unsupported_types(self):
        for change in ({'expected_answer': 42}, {'marker': 'OTHER_MARKER'}, {'case_id': ''},
                       {'schema': True}, {'marker': True}):
            candidate = copy.deepcopy(self.contract); candidate.update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                fc.validate_contract(candidate)
        candidate = copy.deepcopy(self.contract)
        candidate['answer_fields']['count']['type'] = 'array'
        with self.assertRaises(ValueError): fc.validate_contract(candidate)

    def test_contract_is_bounded_and_booleans_are_not_numeric_limits(self):
        for name, value in [('max_messages', True), ('max_messages', 65),
                            ('max_message_bytes', 2**30), ('min_citations', -1),
                            ('max_citation_span_lines', 0), ('integer_abs_max', False)]:
            candidate = copy.deepcopy(self.contract); candidate['limits'][name] = value
            with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                fc.validate_contract(candidate)
        candidate = copy.deepcopy(self.contract); candidate['limits']['min_citations'] = 3
        with self.assertRaises(ValueError): fc.validate_contract(candidate)

    def test_reference_preflight_checks_shape_without_claiming_truth(self):
        self.assertIsNone(fc.preflight_reference(self.contract, message()))
        candidate = response(); candidate['answer']['count'] = 'one'
        with self.assertRaises(ValueError): fc.preflight_reference(self.contract, message(candidate))

    def test_public_data_and_messages_are_not_mutated(self):
        before = copy.deepcopy(self.contract)
        messages = ['progress', message(), 'Saved.']; held = list(messages)
        artifact = fc.make_artifact(self.contract, messages, context())
        fc.review_decision(self.contract, artifact, True, receipt(artifact))
        self.assertEqual(before, self.contract)
        self.assertEqual(held, messages)

    def test_integer_boolean_and_nullable_string_fields_are_explicit(self):
        for field, value in [('count', True), ('count', 1.0), ('flag', 1), ('count', None),
                             ('note', []), ('count', 101), ('count', -101)]:
            candidate = response(); candidate['answer'][field] = value
            with self.subTest(field=field, value=value):
                self.assertFalse(fc.check_response(self.contract, message(candidate))['valid'])
        for value in (-100, 100):
            candidate = response(); candidate['answer']['count'] = value
            self.assertTrue(fc.check_response(self.contract, message(candidate))['valid'])
        for value in (None, '', 'Ω' * 16):
            candidate = response(); candidate['answer']['note'] = value
            self.assertTrue(fc.check_response(self.contract, message(candidate))['valid'])
        candidate['answer']['note'] = 'Ω' * 17
        self.assertFalse(fc.check_response(self.contract, message(candidate))['valid'])

    def test_exact_answer_envelope_and_fields_are_enforced(self):
        candidates = []
        candidate = response(); candidate['extra'] = True; candidates.append(candidate)
        candidate = response(); del candidate['answer']['flag']; candidates.append(candidate)
        candidate = response(); candidate['answer']['extra'] = True; candidates.append(candidate)
        candidate = response(); candidate['case_id'] = 'wrong'; candidates.append(candidate)
        for candidate in candidates:
            self.assertFalse(fc.check_response(self.contract, message(candidate))['valid'])
        self.assertFalse(fc.check_response(self.contract, json.dumps(response()))['valid'])
        self.assertFalse(fc.check_response(self.contract, 'MEMORY_TASK_ANSWER\n```json\n{}\n```')['valid'])

    def test_disclosed_citation_count_boundaries(self):
        candidate = response()
        self.assertTrue(fc.check_response(self.contract, message(candidate))['valid'])
        candidate['citations'] *= 2
        self.assertTrue(fc.check_response(self.contract, message(candidate))['valid'])
        candidate['citations'].append(dict(candidate['citations'][0]))
        self.assertFalse(fc.check_response(self.contract, message(candidate))['valid'])
        candidate['citations'] = []
        self.assertFalse(fc.check_response(self.contract, message(candidate))['valid'])

    def test_disclosed_span_quote_path_and_explanation_boundaries(self):
        candidate = response(); cite = candidate['citations'][0]
        cite.update(path='p' * 32, quote='Ω' * 8, start_line=4, end_line=6)
        candidate['explanation'] = 'x' * 20
        self.assertTrue(fc.check_response(self.contract, message(candidate))['valid'])
        for key, value in [('path', 'p' * 33), ('quote', 'Ω' * 9), ('quote', ''),
                           ('start_line', True), ('start_line', 0), ('end_line', 7),
                           ('end_line', 3)]:
            changed = copy.deepcopy(candidate); changed['citations'][0][key] = value
            with self.subTest(key=key, value=value):
                self.assertFalse(fc.check_response(self.contract, message(changed))['valid'])
        for text in ('x' * 4, 'Ω' * 20):
            candidate['explanation'] = text
            self.assertTrue(fc.check_response(self.contract, message(candidate))['valid'])
        for text in ('x' * 3, 'x' * 21):
            candidate['explanation'] = text
            self.assertFalse(fc.check_response(self.contract, message(candidate))['valid'])

    def test_message_limit_counts_utf8_bytes_not_unicode_characters(self):
        candidate = response(); candidate['explanation'] = 'Ω' * 10
        text = message(candidate); exact_bytes = len(text.encode('utf-8'))
        self.assertGreater(exact_bytes, len(text))
        self.contract['limits']['max_message_bytes'] = exact_bytes
        self.assertTrue(fc.check_response(self.contract, text)['valid'])
        self.contract['limits']['max_message_bytes'] -= 1
        self.assertFalse(fc.check_response(self.contract, text)['valid'])

    def test_strict_json_rejects_duplicate_keys_nonfinite_and_invalid_utf8(self):
        bad = ['MEMORY_TASK_ANSWER\n{"case_id":"public-case","case_id":"wrong"}',
               'MEMORY_TASK_ANSWER\n{"case_id":"public-case","answer":{"count":NaN}}',
               'MEMORY_TASK_ANSWER\n{"case_id":"public-case","answer":{"count":Infinity}}',
               'MEMORY_TASK_ANSWER\n{"case_id":"public-case","answer":{"count":1e999}}',
               'MEMORY_TASK_ANSWER\n\ud800']
        for text in bad:
            with self.subTest(text=repr(text)):
                self.assertFalse(fc.check_response(self.contract, text)['valid'])


class SelectionTests(unittest.TestCase):
    def test_last_marked_message_retains_exact_unicode_hash_and_index(self):
        text = message(); selected = fc.select_answer(contract(), ['progress', text, 'Native save complete.'])
        self.assertEqual('selected', selected['status'])
        self.assertEqual(1, selected['index'])
        self.assertEqual(text, selected['text'])
        self.assertEqual(hashlib.sha256(text.encode('utf-8')).hexdigest(), selected['message_sha256'])
        self.assertTrue(selected['check']['valid'])

    def test_malformed_late_attempt_without_case_id_never_falls_back(self):
        for last in ('MEMORY_TASK_ANSWER\n{', 'MEMORY_TASK_ANSWER',
                     'MEMORY_TASK_ANSWER\n{"answer":', 'MEMORY_TASK_ANSWER\n{}'):
            with self.subTest(last=last):
                selected = fc.select_answer(contract(), [message(), last, 'Saved.'])
                self.assertEqual(1, selected['index'])
                self.assertFalse(selected['check']['valid'])

    def test_wrong_case_late_attempt_never_falls_back(self):
        wrong = response(); wrong['case_id'] = 'different-case'
        selected = fc.select_answer(contract(), [message(), message(wrong)])
        self.assertEqual(1, selected['index'])
        self.assertFalse(selected['check']['valid'])

    def test_unmarked_intent_is_never_inferred(self):
        selected = fc.select_answer(contract(), [message(), '{"case_id":"wrong"}', 'Saved.'])
        self.assertEqual(0, selected['index'])
        absent = fc.select_answer(contract(), ['No marked answer'])
        self.assertEqual('unmarked', absent['status'])
        self.assertIsNone(absent['index'])

    def test_crlf_marker_is_recognized_but_hash_is_not_normalized(self):
        lf = message(); crlf = message(ending='\r\n')
        selected = fc.select_answer(contract(), [lf, crlf])
        self.assertEqual(1, selected['index'])
        self.assertTrue(selected['check']['valid'])
        self.assertNotEqual(hashlib.sha256(lf.encode()).hexdigest(), selected['message_sha256'])

    def test_stream_limits_fail_closed_without_selecting_an_earlier_answer(self):
        public = contract(); public['limits']['max_messages'] = 1
        selected = fc.select_answer(public, [message(), 'ack'])
        self.assertEqual('unavailable', selected['status'])
        self.assertIsNone(selected['index'])
        public = contract(); public['limits']['max_total_bytes'] = public['limits']['max_message_bytes']
        selected = fc.select_answer(public, [message(), 'x' * 4096])
        self.assertEqual('unavailable', selected['status'])


class ReviewTests(unittest.TestCase):
    def setUp(self):
        self.public = contract()
        self.artifact = fc.make_artifact(self.public, ['progress', message(), 'Saved.'], context())

    def decide(self, machine=True, review=None):
        return fc.review_decision(self.public, self.artifact, machine, review)

    def test_binding_contains_all_required_identities_and_exact_message(self):
        bound = self.artifact['binding']
        for key, value in context().items(): self.assertEqual(value, bound[key])
        self.assertEqual('public-case', bound['case_id'])
        self.assertEqual(fc.contract_digest(self.public), bound['public_contract_sha256'])
        self.assertEqual(1, bound['selected_message_index'])
        self.assertEqual(hashlib.sha256(message().encode()).hexdigest(), bound['selected_message_sha256'])
        self.assertEqual(64, len(bound['messages_sha256']))

    def test_consistent_bound_review_can_gate_machine_true(self):
        result = self.decide(review=receipt(self.artifact))
        self.assertIs(True, result['task_correct'])
        self.assertEqual('consistent', result['review_status'])
        self.assertFalse(result['authenticated'])
        self.assertFalse(result['human_confirmation'])
        self.assertFalse(result['semantic_truth_proven'])
        claimed_human = json.loads(receipt(self.artifact)); claimed_human['reviewer_kind'] = 'human'
        claimed = self.decide(review=json.dumps(claimed_human))
        self.assertIs(True, claimed['task_correct'])
        self.assertFalse(claimed['human_confirmation'])

    def test_machine_failure_stays_false_for_every_review_diagnostic(self):
        bad = json.loads(receipt(self.artifact)); bad['binding']['slot_id'] = 'other-slot'
        for review in (None, json.dumps(bad), receipt(self.artifact, 'uncertain'),
                       receipt(self.artifact), receipt(self.artifact, 'contradiction')):
            self.assertIs(False, self.decide(False, review)['task_correct'])

    def test_machine_unknown_always_stays_unknown(self):
        for review in (None, receipt(self.artifact), receipt(self.artifact, 'contradiction')):
            self.assertIsNone(self.decide(None, review)['task_correct'])

    def test_missing_uncertain_and_contradictory_review_remain_distinct(self):
        self.assertEqual('missing', self.decide()['review_status'])
        self.assertIsNone(self.decide()['task_correct'])
        self.assertIsNone(self.decide(review=receipt(self.artifact, 'uncertain'))['task_correct'])
        self.assertIs(False, self.decide(review=receipt(self.artifact, 'contradiction'))['task_correct'])

    def test_stale_review_cannot_be_reused_for_any_changed_binding(self):
        for field in self.artifact['binding']:
            changed = json.loads(receipt(self.artifact))
            value = changed['binding'][field]
            changed['binding'][field] = 0 if type(value) is int else 'd' * 64
            with self.subTest(field=field):
                result = self.decide(review=json.dumps(changed))
                self.assertIsNone(result['task_correct'])
                self.assertEqual('mismatch', result['review_status'])

    def test_wrong_slot_reuse_against_a_new_artifact_is_unknown(self):
        other = context(); other['slot_id'] = 'slot-2'
        second = fc.make_artifact(self.public, ['progress', message(), 'Saved.'], other)
        result = fc.review_decision(self.public, second, True, receipt(self.artifact))
        self.assertIsNone(result['task_correct'])

    def test_boolean_binding_index_and_check_flag_are_invalid(self):
        for field in ('binding', 'check'):
            changed = copy.deepcopy(self.artifact)
            if field == 'binding': changed['binding']['selected_message_index'] = True
            else: changed['selection']['check']['valid'] = 1
            result = fc.review_decision(self.public, changed, True, receipt(changed))
            self.assertIsNone(result['task_correct'])
            self.assertEqual('artifact-invalid', result['review_status'])

    def test_boolean_index_in_review_is_not_integer_index_equivalent(self):
        changed = json.loads(receipt(self.artifact)); changed['binding']['selected_message_index'] = True
        result = self.decide(review=json.dumps(changed))
        self.assertIsNone(result['task_correct'])
        self.assertEqual('mismatch', result['review_status'])

    def test_receipt_requires_reviewer_identity_and_strict_json(self):
        missing = json.loads(receipt(self.artifact)); missing['reviewer_id'] = ''
        valid = receipt(self.artifact)
        duplicate = valid[:-1] + ',"verdict":"consistent"}'
        nonfinite = valid.replace('"reviewer_id": "independent-agent"', '"reviewer_id": NaN')
        for review in (json.dumps(missing), duplicate, nonfinite, '[]'):
            result = self.decide(review=review)
            self.assertIsNone(result['task_correct'])
            self.assertEqual('invalid', result['review_status'])

    def test_review_cannot_rescue_shape_failure_or_tampered_artifact(self):
        bad = fc.make_artifact(self.public, [message(), 'MEMORY_TASK_ANSWER\n{'], context())
        self.assertIs(False, fc.review_decision(self.public, bad, True, receipt(bad))['task_correct'])
        changed = copy.deepcopy(self.artifact); changed['selection']['text'] += ' '
        result = fc.review_decision(self.public, changed, True, receipt(self.artifact))
        self.assertIsNone(result['task_correct'])
        self.assertEqual('artifact-invalid', result['review_status'])

    def test_contract_change_invalidates_review_and_context_is_validated(self):
        changed = copy.deepcopy(self.public); changed['limits']['max_citations'] = 3
        self.assertIsNone(fc.review_decision(changed, self.artifact, True, receipt(self.artifact))['task_correct'])
        invalid = context(); invalid['protocol_sha256'] = 'not-a-hash'
        with self.assertRaises(ValueError): fc.make_artifact(self.public, [message()], invalid)
        with self.assertRaises(ValueError): self.decide(1, receipt(self.artifact))


if __name__ == '__main__':
    unittest.main()
