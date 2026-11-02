"""Real opt-in completion intake, no providers, client data or model calls."""
import base64
import copy
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import memory_outcome_ab as ab
import memory_outcome_isolation as isolation
import memory_outcome_verify as verify
import memory_response_contract as fc
import memory_response_intake as intake


def public_contract(raw=b'public source\n'):
    return {'schema': 'memory-response-contract-v2', 'case_id': 'public-task',
        'marker': fc.MARKER, 'answer_fields': {'result': {'type': 'string', 'nullable': False}},
        'limits': {'max_messages': 8, 'max_message_bytes': 65536, 'max_total_bytes': 131072,
            'max_answer_string_chars': 1000, 'integer_abs_max': 10000,
            'min_citations': 1, 'max_citations': 2, 'max_citation_span_lines': 3,
            'max_citation_quote_chars': 128, 'max_citation_path_chars': 128,
            'min_explanation_chars': 1, 'max_explanation_chars': 1000},
        'citation_corpus': {'schema': 'memory-citation-corpus-v1', 'files': [
            {'path': 'Gold.php', 'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw)}]}}


def answer(value='42', path='Gold.php', start=1):
    return fc.MARKER + '\n' + json.dumps({'case_id': 'public-task', 'answer': {'result': value},
        'citations': [{'path': path, 'start_line': start, 'end_line': start, 'quote': 'source'}],
        'explanation': 'The immutable public source supports the result.'})


class IntakeFixture(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / 'source'; self.source.mkdir()
        (self.source / 'Gold.php').write_bytes(b'public source\n')
        self.task = {'id': 'public-task', 'prompt': 'Solve the public fixture.', 'task_mode': 'answer',
                     'verifier_id': 'public-answer', 'response_contract': public_contract()}
        self.case = {'mode': 'answer', 'equals': '42',
                     'response': {'reference': answer(), 'answer_field': 'result'}}
        self.target = self.root / 'citations'

    def prepare(self):
        return intake.prepare(self.task, self.case, self.source, self.target)

    def packet(self, messages=None):
        prepared = self.prepare()
        return prepared, {'contract': self.task['response_contract'],
            'capture': {'schema': 'memory-assistant-capture-v1', 'overflow': False,
                        'messages': [answer()] if messages is None else messages},
            'context': {'protocol_sha256': 'a' * 64, 'slot_id': 'slot-1',
                'verifier_sha256': verify.verifier_digest(self.case),
                'source_image_sha256': prepared['source_image_sha256']},
            'answer_field': self.case['response']['answer_field'],
            'sources': {'Gold.php': base64.b64encode(b'public source\n').decode()}}

    def require_namespace(self):
        probe = self.root / 'probe'; probe.mkdir()
        private = self.root / 'private-probe'; private.mkdir()
        sentinel = private / 'sentinel'; sentinel.write_text('private')
        try:
            view = isolation.prepare_agent_view(probe, private, credentials=False)
            if not isolation.probe_agent_view(view, sentinel):
                if sys.platform == 'linux' and os.environ.get('CI') == 'true':
                    self.fail('CI must provide the declared Linux read namespace prerequisite')
                self.skipTest('Linux read namespace unavailable')
        except isolation.IsolationError as error:
            if sys.platform == 'linux' and os.environ.get('CI') == 'true':
                self.fail('CI must provide the declared Linux read namespace prerequisite: ' + str(error))
            self.skipTest(str(error))


class CustodyTests(IntakeFixture):
    def test_prepare_discloses_exact_corpus_and_no_private_reference(self):
        prepared = self.prepare()
        sources, prompt = intake.validate_prepared(self.task, self.case, prepared)
        self.assertEqual({'Gold.php': b'public source\n'}, sources)
        self.assertIn(fc.render_contract(self.task['response_contract']).encode(), prompt)
        self.assertIn(intake.MOUNT_NAME.encode(), prompt)
        self.assertNotIn(b'"equals"', prompt)
        self.assertNotIn(answer().encode(), prompt)
        self.assertEqual(hashlib.sha256(prompt).hexdigest(), prepared['prompt_sha256'])
        self.assertEqual(0o444, (self.target / 'Gold.php').stat().st_mode & 0o777)

    def test_impossible_reference_is_refused_before_publication(self):
        for reference in (answer(start=2), answer(path='generated.txt'), answer(value='wrong')):
            with self.subTest(reference=reference):
                self.case['response']['reference'] = reference
                with self.assertRaises(ValueError): self.prepare()
                self.assertFalse(self.target.exists())

    def test_symlink_hardlink_fifo_and_private_source_are_refused(self):
        original = self.source / 'Gold.php'; original.unlink()
        outside = self.root / 'outside'; outside.write_bytes(b'public source\n')
        operations = [lambda: original.symlink_to(outside), lambda: os.link(outside, original)]
        if hasattr(os, 'mkfifo'):
            operations.append(lambda: os.mkfifo(original))
        for operation in operations:
            operation()
            with self.assertRaises((OSError, ValueError)): self.prepare()
            original.unlink()
        original.write_bytes(b'public source\n')
        self.task['response_contract']['citation_corpus']['files'][0]['path'] = '.env'
        with self.assertRaises(ValueError): self.prepare()

    def test_byte_mode_membership_and_prompt_drift_are_blockers(self):
        prepared = self.prepare(); path = self.target / 'Gold.php'
        path.chmod(0o644)
        with self.assertRaises(ValueError): intake.validate_prepared(self.task, self.case, prepared)
        path.chmod(0o444)
        extra = self.target / 'empty'; extra.mkdir()
        with self.assertRaises(ValueError): intake.validate_prepared(self.task, self.case, prepared)
        extra.rmdir()
        changed = copy.deepcopy(self.task); changed['prompt'] += 'changed'
        with self.assertRaises(ValueError): intake.validate_prepared(changed, self.case, prepared)
        path.chmod(0o644); path.write_bytes(b'changed source\n'); path.chmod(0o444)
        with self.assertRaises(ValueError): intake.validate_prepared(self.task, self.case, prepared)

    def test_empty_corpus_still_requires_existing_exact_snapshot(self):
        public = self.task['response_contract']
        public['citation_corpus']['files'] = []
        public['limits'].update(min_citations=0, max_citations=0)
        with self.assertRaises(ValueError): intake.source_map(self.target, public, exact=True)

    def test_last_marked_answer_survives_acknowledgment_but_never_bad_attempt(self):
        _, packet = self.packet(['progress', answer(), 'Saved.'])
        checked = intake.evidence(packet)
        self.assertTrue(checked['citation_check']['valid'])
        self.assertEqual(1, checked['artifact']['binding']['selected_message_index'])
        self.assertEqual('42', checked['answer_text'])
        packet['capture']['messages'].append(fc.MARKER + '\n{')
        checked = intake.evidence(packet)
        self.assertFalse(checked['citation_check']['valid'])
        self.assertEqual(3, checked['artifact']['binding']['selected_message_index'])

    def test_context_drift_precedes_bad_response_and_real_range_error_is_false(self):
        _, packet = self.packet([answer(start=2)])
        self.assertFalse(intake.evidence(packet)['citation_check']['valid'])
        packet['sources']['Gold.php'] = base64.b64encode(b'changed').decode()
        with self.assertRaises(ValueError): intake.evidence(packet)

    def test_overflow_cannot_reuse_a_successful_prefix(self):
        _, packet = self.packet()
        packet['capture']['overflow'] = True
        checked = intake.evidence(packet)
        self.assertFalse(checked['citation_check']['valid'])
        self.assertIsNone(checked['answer_text'])

    def test_capture_mismatch_blocks_an_otherwise_valid_answer(self):
        prepared, packet = self.packet()
        capture = self.root / 'capture.json'; capture.write_text(json.dumps(packet['capture']))
        checked = intake.grade(self.source, capture, self.case, self.task, prepared,
            self.target, packet['context'], capture_sha256='d' * 64)
        self.assertIsNone(checked['correct'])
        self.assertNotIn('response', checked)

    def test_isolated_evidence_and_semantics_use_pre_task_bytes(self):
        self.require_namespace()
        prepared, packet = self.packet(['progress', answer(), 'Saved.'])
        capture = self.root / 'capture.json'; capture.write_text(json.dumps(packet['capture']))
        (self.source / 'Gold.php').write_text('changed working source\n')
        result = intake.grade(self.source, capture, self.case, self.task, prepared,
                              self.target, packet['context'])
        self.assertTrue(result['correct'], result)
        self.assertEqual(verify.verifier_digest(self.case), result['verifier_digest'])
        self.assertEqual('slot-1', result['response']['artifact']['binding']['slot_id'])
        packet['context']['verifier_sha256'] = 'b' * 64
        self.assertIsNone(intake.grade(self.source, capture, self.case, self.task, prepared,
                                      self.target, packet['context'])['correct'])

    def test_patch_oracle_correct_false_and_unknown_remain_distinct(self):
        self.require_namespace()
        oracle_root = self.root / 'oracles'; oracle_root.mkdir()
        oracle = oracle_root / 'oracle.py'
        self.task['task_mode'] = 'patch'
        self.case = {'mode': 'patch', 'argv': [sys.executable, str(oracle)],
            'timeout': 3, 'read_roots': [str(oracle_root)],
            'response': {'reference': answer(), 'answer_field': None}}
        oracle.write_text('raise SystemExit(2)\n')
        prepared, packet = self.packet()
        capture = self.root / 'capture.json'; capture.write_text(json.dumps(packet['capture']))
        for expected in (True, False, None):
            program = ('raise SystemExit(2)\n' if expected is None else
                "import json\nprint(json.dumps({'schema':'task-outcome-verifier-v1',"
                "'completed':True,'correct':" + repr(expected) + '}))\n')
            oracle.write_text(program)
            packet['context']['verifier_sha256'] = verify.verifier_digest(self.case)
            with self.subTest(expected=expected):
                checked = intake.grade(self.source, capture, self.case, self.task,
                                       prepared, self.target, packet['context'])
                self.assertIs(expected, checked['correct'], checked)
                self.assertTrue(checked['response']['citation_check']['valid'])


class CompletionIntakeTests(IntakeFixture):
    def setUp(self):
        super().setUp()
        self.private = self.root / 'private'; self.private.mkdir()
        self.cache = self.root / 'cache'
        self.protocol = self.private / 'protocol.json'; self.result = self.private / 'result.json'
        self.set_path = self.private / 'tasks.json'; self.verifiers = self.private / 'verifiers.json'
        self.design = self.private / 'design.json'
        self.task.update(ts='2026-09-01T09:00:00Z', project='public', gold=['Gold.php'])
        self.design.write_text(json.dumps({'primary': 'correctness', 'quality_margin': 0,
            'minimum_tasks': 2, 'power_rationale': 'Mechanical integration only, not efficacy',
            'selection': 'public fixture', 'exclusion': 'none', 'secondary_benefit': 'elapsed',
            'stopping_rule': 'fixed-horizon', 'unknown_policy': 'insufficient'}))
        self.options = ['--set', str(self.set_path), '--verifiers', str(self.verifiers),
            '--cache', str(self.cache), '--arms', 'off,recall', '--model', 'public-fake',
            '--codex', sys.executable, '--timeout', '10']

    def build(self, task, base, *args):
        base.mkdir(parents=True)
        cfg = base / 'project-brain/config'; cfg.mkdir(parents=True)
        (cfg / 'runtime.json').write_text(json.dumps({'capsule_profile': 'recall'}))
        (base / 'Gold.php').write_bytes(b'public source\n')
        program = '''import hashlib,json,sys
from pathlib import Path
raw=sys.stdin.buffer.read()
assert hashlib.sha256(raw).hexdigest()==EXPECTED_PROMPT_HASH
public=json.loads(raw.decode().split('PUBLIC_CONTRACT\\n',1)[1])
evidence=Path('.memory-citation-corpus/Gold.php')
assert evidence.read_bytes()==b'public source\\n'
try:
 evidence.write_bytes(b'overwrite')
except OSError:
 pass
else:
 raise RuntimeError('citation snapshot was writable')
Path('Gold.php').write_bytes(b'changed working source\\n')
for text in MESSAGES:
 print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':text}}),flush=True)
print(json.dumps({'type':'turn.completed','usage':{'input_tokens':11,'output_tokens':3}}),flush=True)
'''.replace('MESSAGES', repr(getattr(self, 'messages', ['progress', answer(), 'Saved.']))).replace('EXPECTED_PROMPT_HASH',
    repr(hashlib.sha256(intake.effective_prompt(task, self.case)).hexdigest()))
        (base / 'agent.py').write_text(program + getattr(self, 'after_completion', ''))
        return {'commit': 'public-snapshot', 'edition': 'PHP Core', 'project': self.root}

    def seal(self):
        self.set_path.write_text(json.dumps({'tasks': [self.task]}))
        self.verifiers.write_text(json.dumps({'cases': {'public-answer': self.case}}))
        with mock.patch.object(ab, 'build_corpus', side_effect=self.build), \
                mock.patch.object(ab, 'seed_recent_files', return_value=0), \
                mock.patch.object(ab, 'plant_saves', return_value={}):
            return ab.main(['seal', *self.options, '--design', str(self.design), '--out', str(self.protocol)])

    def run_complete(self):
        def command(codex, corpus, arm, prompt, model):
            self.assertEqual('-', prompt)
            return [sys.executable, '-B', str(corpus / 'agent.py')]
        with mock.patch.object(ab, 'codex_command', side_effect=command), \
                mock.patch.dict(os.environ, {'CODEX_HOME': str(self.root / 'no-credentials')}):
            return ab.main(['run', *self.options, '--endpoint', 'task-complete',
                '--protocol', str(self.protocol), '--out', str(self.result)])

    def test_full_cli_with_large_stdin_prompt_readonly_snapshot_and_isolated_grading(self):
        self.require_namespace()
        self.task['prompt'] += 'x' * 150000  # Beyond Linux's one-argument transport limit.
        self.assertEqual(0, self.seal())
        self.assertEqual(0, self.run_complete())
        result = json.loads(self.result.read_text())
        prepared = result['protocol']['bases']['public-task']['response']
        for slot in result['slots']:
            attempt = slot['attempts'][0]
            self.assertTrue(attempt['completed'], attempt)
            self.assertTrue(attempt['verification']['correct'], attempt)
            self.assertEqual(prepared['prompt_sha256'], attempt['prompt_transport']['sha256'])
            self.assertGreater(attempt['prompt_transport']['bytes'], 131072)
            self.assertEqual(1, attempt['verification']['response']['artifact']['binding']['selected_message_index'])
            self.assertEqual('unknown', attempt['prompt_transport']['model_receipt'])
            self.assertEqual(['progress', answer(), 'Saved.'], attempt['response_capture']['messages'])
        self.assertFalse(list(self.cache.glob('completion-*')))
        self.assertEqual(1, ab.summarize_complete(result)['arms']['recall']['correct'])

    def test_corpus_tamper_and_missing_public_optin_prevent_dispatch(self):
        self.assertEqual(0, self.seal())
        metadata = json.loads(self.protocol.read_text())['bases']['public-task']['response']
        (Path(metadata['path']) / 'extra.txt').write_text('extra')
        with mock.patch.object(ab, 'run_arm') as run:
            self.assertEqual(2, self.run_complete())
        run.assert_not_called()
        del self.task['response_contract']
        self.set_path.write_text(json.dumps({'tasks': [self.task]}))
        with self.assertRaises(ab.OutcomeError): ab.complete_cases(self.verifiers, [self.task])

    def test_response_overflow_fails_without_losing_native_usage(self):
        self.require_namespace()
        self.task['response_contract']['limits']['max_messages'] = 2
        self.assertEqual(0, self.seal())
        self.assertEqual(0, self.run_complete())
        result = json.loads(self.result.read_text())
        for slot in result['slots']:
            attempt = slot['attempts'][0]
            self.assertTrue(attempt['completed'])
            self.assertIs(False, attempt['verification']['correct'])
            self.assertTrue(attempt['response_capture']['overflow'])
            self.assertEqual({'input_tokens': 11, 'output_tokens': 3}, attempt['known_usage'])

    def test_bad_later_marked_answer_is_not_rescued_by_valid_earlier_answer(self):
        self.require_namespace()
        self.messages = [answer(), fc.MARKER + '\n{', 'Saved.']
        self.assertEqual(0, self.seal())
        self.assertEqual(0, self.run_complete())
        result = json.loads(self.result.read_text())
        for slot in result['slots']:
            attempt = slot['attempts'][0]
            self.assertIs(False, attempt['verification']['correct'])
            self.assertEqual(1, attempt['verification']['response']['artifact']['binding']['selected_message_index'])

    def test_unfinished_continuation_preserves_capture_and_current_slot_binding(self):
        self.require_namespace()
        self.after_completion = ("print(json.dumps({'type':'turn.started'}),flush=True)\n"
            "print(json.dumps({'type':'item.completed','item':{'type':'agent_message',"
            "'text':'Unfinished continuation'}}),flush=True)\n")
        self.assertEqual(0, self.seal())
        self.assertEqual(0, self.run_complete())
        result = json.loads(self.result.read_text())
        for ordinal, slot in enumerate(result['slots']):
            attempt = slot['attempts'][0]
            self.assertFalse(attempt['completed'])
            self.assertIsNone(attempt['verification']['correct'])
            capture = attempt['response_capture']
            self.assertEqual(['progress', answer(), 'Saved.', 'Unfinished continuation'], capture['messages'])
            self.assertEqual(intake.digest(capture), attempt['response_capture_sha256'])
            self.assertEqual('slot-' + str(ordinal), attempt['response_artifact']['binding']['slot_id'])
            self.assertEqual(fc.make_artifact(self.task['response_contract'], capture['messages'], {
                'protocol_sha256': result['protocol']['seal_sha256'], 'slot_id': 'slot-' + str(ordinal),
                'verifier_sha256': result['protocol']['bindings']['verifier_artifacts']['public-answer'],
                'source_image_sha256': result['protocol']['bases']['public-task']['response']['source_image_sha256']}),
                attempt['response_artifact'])


if __name__ == '__main__':
    unittest.main()
