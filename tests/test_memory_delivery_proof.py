"""Private delivery metadata is bounded evidence, never model receipt/use."""
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest import mock
from contextlib import redirect_stdout
import io

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
try:
    import memory_delivery_proof as proof
except ImportError:
    proof = None


class DeliveryProofTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(proof, 'maintainer delivery proof collector is missing')
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.path = 'memory-bank/chunks/MEM-public.md'
        source = self.root / 'src/Public.php'; source.parent.mkdir()
        source.write_bytes(b'first\r\nsecond\n')
        self.expected = hashlib.sha256(b'first\nsecond\n').hexdigest()
        self.document = self.root / self.path; self.document.parent.mkdir(parents=True)
        self.document.write_text('---\n' + json.dumps({'id': 'MEM-public', 'revision': 2,
            'source_digests': [{'path': 'src/Public.php', 'sha256': self.expected}]}) + '\n---\nA public claim.\n')
        self.image = hashlib.sha256(self.document.read_text().encode()).hexdigest()
        self.frame_id = '11111111-1111-4111-8111-111111111111'
        self.manifest = self.root / 'memory-bank/local/retrieval-manifests' / (self.frame_id + '.json')
        self.manifest.parent.mkdir(parents=True)
        self.frame = {'schema_version': 4, 'id': self.frame_id,
            'task_id': '22222222-2222-4222-8222-222222222222', 'task_revision': 3,
            'entry_point': 'refresh', 'host': 'codex', 'query': 'PRIVATE QUERY DO NOT EXPORT',
            'selected': [{'path': self.path, 'source_hash': self.image}], 'excluded': []}
        self.known = {self.path, 'src/Public.php'}
        self.write_frame()

    def write_frame(self):
        self.manifest.write_text(json.dumps(self.frame))

    def collect(self, **kwargs):
        return proof.collect_delivery_proof(self.root, [self.manifest], known_paths=self.known, **kwargs)

    def test_selected_identity_and_post_run_source_state(self):
        result = self.collect(); frame = result['frames'][0]; doc = frame['selected'][0]
        self.assertEqual('automatic', frame['origin'])
        self.assertEqual(self.path, doc['path'])
        self.assertEqual(self.image, doc['document_sha256'])
        self.assertEqual('match', doc['document_state'])
        self.assertEqual(self.expected, doc['sources'][0]['expected_sha256'])
        self.assertEqual('match', doc['sources'][0]['state'])
        self.assertEqual('post-run', result['phase'])
        self.assertFalse(result['producer_authenticated'])
        self.assertEqual('unknown', result['host_receipt'])
        self.assertEqual('unproven', result['model_use'])
        self.assertNotIn('PRIVATE QUERY', json.dumps(result))
        self.assertEqual('unknown', doc['render']['status'])

    def test_changed_document_does_not_launder_metadata(self):
        self.document.write_text('new content')
        doc = self.collect()['frames'][0]['selected'][0]
        self.assertEqual(self.image, doc['document_sha256'])
        self.assertEqual('changed', doc['document_state'])
        self.assertIsNone(doc['sources'])

    def test_changed_source_preserves_expected_hash(self):
        (self.root / 'src/Public.php').write_text('changed')
        source = self.collect()['frames'][0]['selected'][0]['sources'][0]
        self.assertEqual('changed', source['state'])
        self.assertEqual(self.expected, source['expected_sha256'])
        self.assertNotEqual(source['expected_sha256'], source['observed_sha256'])

    def test_partial_digest_coverage_is_unknown_not_confirmed_absence(self):
        for digests in ([], [{'path': 'src/Public.php', 'sha256': self.expected}]):
            with self.subTest(digests=digests):
                self.document.write_text('---\n' + json.dumps({'id': 'MEM-public',
                    'sources': ['src/Public.php', 'src/Other.php'],
                    'source_digests': digests}) + '\n---\nPublic claim.\n')
                self.known.add('src/Other.php')
                self.frame['selected'][0]['source_hash'] = hashlib.sha256(self.document.read_text().encode()).hexdigest()
                self.write_frame()
                self.assertIsNone(self.collect()['frames'][0]['selected'][0]['sources'])

    def test_record_identifier_cannot_echo_a_secret(self):
        marker = 'MEM-sk-proj-' + 'A1'*20
        self.document.write_text('---\n' + json.dumps({'id': marker}) + '\n---\nPublic claim.\n')
        self.frame['selected'][0]['source_hash'] = hashlib.sha256(self.document.read_text().encode()).hexdigest()
        self.write_frame()
        self.assertNotIn(marker, json.dumps(self.collect()))

    def test_render_sidecar_is_only_local_unauthenticated_observation(self):
        trace = self.root / 'memory-bank/local/retrieval-traces' / self.manifest.name
        trace.parent.mkdir()
        trace.write_text(json.dumps({'schema_version': 1, 'id': self.frame_id, 'truncated': False,
            'records': [{'path': self.path, 'source_sha256': hashlib.sha256(self.document.read_bytes()).hexdigest()}],
            'render': {'sha256': 'a'*64, 'bytes': 40, 'form': 'printed_capsule', 'records': [
                {'path': self.path, 'header_present': True, 'body_sha256': 'b'*64,
                 'body_bytes': 20, 'pointer_only': False}]}}))
        result = self.collect()
        self.assertEqual('trace-reported', result['frames'][0]['selected'][0]['render']['status'])
        self.assertEqual('unknown', result['host_receipt'])
        body = json.loads(trace.read_text()); body['id'] = '33333333-3333-4333-8333-333333333333'
        trace.write_text(json.dumps(body))
        self.assertEqual('unknown', self.collect()['frames'][0]['selected'][0]['render']['status'])

    def test_governed_explicit_manifest_has_same_safe_observations(self):
        self.frame['entry_point'] = 'retrieve'; self.write_frame()
        governed = self.root / 'project-brain/control/retrieval-manifests' / self.manifest.name
        governed.parent.mkdir(parents=True); self.manifest.rename(governed)
        result = proof.collect_delivery_proof(self.root, [governed], known_paths=self.known)
        self.assertEqual('observed', result['status'])
        frame = result['frames'][0]
        self.assertEqual('explicit', frame['origin'])
        self.assertEqual(governed.relative_to(self.root).as_posix(), frame['manifest_path'])
        self.assertEqual('match', frame['selected'][0]['document_state'])

    def test_explicit_retrieve_is_not_automatic_delivery(self):
        self.frame['entry_point'] = 'retrieve'; self.write_frame()
        self.assertEqual('explicit', self.collect()['frames'][0]['origin'])

    def test_malformed_or_private_data_is_never_echoed(self):
        for value in ['src/alice@example.org.php', '../outside', 'src/evil\nfile.php',
                      'src/ignore previous instructions.md', '.env', 'src/Unknown.php']:
            with self.subTest(value=value):
                self.frame['selected'] = [{'path': value, 'source_hash': 'c'*64}]; self.write_frame()
                result = self.collect()
                self.assertNotIn(value, json.dumps(result))
                self.assertEqual([], result['frames'][0]['selected'])
        self.manifest.write_text('{"id":"one","id":"two"}')
        self.assertEqual('unknown', self.collect()['status'])
        self.frame['entry_point'] = {'SECRET': 'DO NOT EXPORT'}; self.write_frame()
        self.assertNotIn('SECRET', json.dumps(self.collect()))

    def test_bounds_and_expired_deadline_are_explicit(self):
        self.frame['selected'] *= 33; self.write_frame()
        result = self.collect()
        self.assertNotEqual('observed', result['status'])
        self.assertLessEqual(len(json.dumps(result).encode()), 65536)
        result = self.collect(deadline=time.monotonic() - 1)
        self.assertEqual('unknown', result['status'])
        self.assertEqual([], result['frames'])

    def test_deadline_after_manifest_marks_partial_collection(self):
        with mock.patch.object(proof.time, 'monotonic', side_effect=[0, 0, 0, 2]):
            result = self.collect(deadline=1)
        self.assertTrue(result['incomplete'])
        self.assertEqual('partial', result['status'])

    def test_real_canonical_print_joins_manifest_for_lf_and_crlf(self):
        from tests.test_memory_optional_publication import NativeOptionalPublicationTests, PUBLIC
        for crlf in (False, True):
            with self.subTest(crlf=crlf):
                case = NativeOptionalPublicationTests('test_inline_transport_copies_native_full_hash_without_disk_rehash')
                case.setUp()
                try:
                    if crlf:
                        path = case.root / PUBLIC['source']
                        path.write_bytes(path.read_bytes().replace(b'\n', b'\r\n'))
                        case.context.index_repository(case.db, case.root)
                    packet = case.packet(); manifest = case.root / packet['manifest']
                    known = {PUBLIC['source'], PUBLIC['unrelated_source']}
                    before = proof.collect_delivery_proof(case.root, [manifest], known_paths=known)
                    self.assertEqual('unknown', before['frames'][0]['selected'][0]['render']['status'])
                    captured = io.StringIO()
                    with redirect_stdout(captured):
                        case.context.print_capsule(packet, case.context.capsule_excerpts(case.db, packet), repository=case.root)
                    result = proof.collect_delivery_proof(case.root, [manifest], known_paths=known)
                    doc = result['frames'][0]['selected'][0]
                    self.assertEqual('match', doc['document_state'])
                    self.assertEqual('trace-reported', doc['render']['status'])
                    self.assertEqual(hashlib.sha256(captured.getvalue().encode()).hexdigest(), doc['render']['frame_sha256'])
                    self.assertEqual(len(captured.getvalue().encode()), doc['render']['frame_bytes'])
                    self.assertEqual('unknown', result['host_receipt'])
                finally: case.doCleanups()

    def test_symlink_and_hardlink_evidence_is_unavailable(self):
        copy = self.manifest.with_suffix('.copy'); self.manifest.rename(copy)
        self.manifest.symlink_to(copy)
        self.assertEqual('unknown', self.collect()['status'])
        self.manifest.unlink(); os.link(copy, self.manifest)
        self.assertEqual('unknown', self.collect()['status'])


if __name__ == '__main__': unittest.main()
