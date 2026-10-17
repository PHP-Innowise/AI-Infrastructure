"""Fresh public credential syntax controls; no provider or service dispatch."""
from dataclasses import replace
import hashlib
import json
import socket
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import hindsight_export as he
from harness.hindsight_recall import Scope, SourceBinding

PUBLIC = json.loads((ROOT / 'tests/fixtures/hindsight_export_security_public.json').read_text())


class ExportSecurityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='crane-export-security-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.path = PUBLIC['path']
        self.source = self.root / self.path
        self.source.parent.mkdir()
        self.scope = Scope(PUBLIC['scope']['team'], PUBLIC['scope']['project'])
        blocker = patch.object(socket, 'create_connection', side_effect=AssertionError('No transport'))
        self.network = blocker.start(); self.addCleanup(blocker.stop)

    def export(self, text):
        raw = text.encode('utf-8')
        self.source.write_bytes(raw)
        approval = he.PublicApproval(self.scope, self.path, hashlib.sha256(raw).hexdigest())
        return he.build_public_export(self.root, self.scope, (approval,))

    def test_quoted_php_wordpress_and_composer_credentials_refuse_whole_capture(self):
        for case in PUBLIC['denied']:
            with self.subTest(case=case['name']), self.assertRaises(he.ExportError) as caught:
                self.export(case['text'])
            self.assertEqual(case['reason'], caught.exception.reason)
            self.assertEqual(case['reason'], str(caught.exception))
            io = caught.exception.observation
            self.assertEqual(io['opened_descriptors'], io['closed_descriptors'])
            self.assertEqual(0, io['close_errors'])
            self.assertNotIn(PUBLIC['opaque_secret_marker'], repr(caught.exception))
        self.network.assert_not_called()

    def test_public_prose_runtime_references_and_empty_auth_maps_preserve_bytes(self):
        for case in PUBLIC['accepted']:
            with self.subTest(case=case['name']):
                result = self.export(case['text'])
                self.assertEqual(1, len(result.retain_drafts))
                retained = json.loads(result.retain_drafts[0].body)['items'][0]['content'].encode('utf-8')
                self.assertEqual(case['expected_raw_utf8_hex'], retained.hex())
                self.assertEqual(case['expected_sha256'], hashlib.sha256(retained).hexdigest())
                result.__post_init__()
        self.network.assert_not_called()

    def test_malformed_auth_maps_and_escaped_provider_values_refuse_capture(self):
        supplement = json.loads((ROOT / 'tests/fixtures/hindsight_export_security_supplement.json').read_text())
        for case in supplement['denied']:
            with self.subTest(case=case['name']), self.assertRaises(he.ExportError) as caught:
                self.export(case['text'])
            self.assertEqual(case['reason'], caught.exception.reason)
            io = caught.exception.observation
            self.assertEqual(io['opened_descriptors'], io['closed_descriptors'])
        self.network.assert_not_called()

    def test_nested_json_environment_strings_and_decoder_admission_fail_closed(self):
        supplement = json.loads((ROOT / 'tests/fixtures/hindsight_export_encoded_public.json').read_text())
        for case in supplement['denied']:
            with self.subTest(case=case['name']), self.assertRaises(he.ExportError) as caught:
                self.export(case['text'])
            self.assertEqual(case['reason'], caught.exception.reason)
            io = caught.exception.observation
            self.assertEqual(io['opened_descriptors'], io['closed_descriptors'])
        self.network.assert_not_called()

    def test_decoded_json_cr_views_are_normalized_without_rewriting_source(self):
        case = json.loads((ROOT / 'tests/fixtures/hindsight_export_cr_public.json').read_text())
        pattern = he.RAW_TEXT_PATTERN
        from types import SimpleNamespace
        from unittest.mock import Mock
        search = Mock(wraps=pattern.search)
        with patch.object(he, 'RAW_TEXT_PATTERN', SimpleNamespace(search=search)):
            result = self.export(case['text'])
        self.assertIn(case['expected_decoded_view'], [call.args[0] for call in search.call_args_list])
        self.assertEqual(case['text'], json.loads(result.retain_drafts[0].body)['items'][0]['content'])
        self.network.assert_not_called()

    def forged(self, case):
        # Supplied unchecked objects cannot bypass the same export screen.
        good = self.export(PUBLIC['accepted'][0]['text'])
        old = good.retain_drafts[0]
        raw = case['text'].encode('utf-8'); digest = hashlib.sha256(raw).hexdigest()
        identity = json.dumps([self.scope.key, self.path, digest], ensure_ascii=False, separators=(',', ':'))
        binding = SourceBinding(self.scope, 'source-' + hashlib.sha256(identity.encode()).hexdigest(), self.path, digest)
        body = json.loads(old.body)
        body['items'][0]['content'] = case['text']
        body['items'][0]['document_id'] = binding.document_id
        body['items'][0]['metadata']['source_sha256'] = digest
        draft = object.__new__(he.RetainDraft)
        for name, value in {'binding': binding, 'source_id': old.source_id,
                            'body': json.dumps(body, ensure_ascii=False, separators=(',', ':')).encode()}.items():
            object.__setattr__(draft, name, value)
        return good, draft

    def test_direct_draft_revalidation_rejects_opaque_secrets_with_existing_reason(self):
        for case in PUBLIC['denied']:
            with self.subTest(case=case['name']):
                _good, draft = self.forged(case)
                with self.assertRaises(he.ExportError) as caught:
                    draft.__post_init__()
                self.assertEqual('approval', caught.exception.reason)
        self.network.assert_not_called()

    def test_packet_and_planner_revalidate_unchecked_drafts_before_dispatch(self):
        good, draft = self.forged(PUBLIC['denied'][5])
        catalogue = replace(good.catalogue, sources=(draft.binding,))
        packet = json.loads(good.serialized)
        packet['sources'][0]['document_id'] = draft.binding.document_id
        packet['sources'][0]['source_sha256'] = draft.binding.source_sha256
        packet['sources'][0]['retain_request'] = json.loads(draft.body)
        raw = json.loads(draft.body)['items'][0]['content'].encode('utf-8')
        io = dict(good._io)
        io['source_read_bytes'] = len(raw)
        io['source_read_calls'] = (len(raw) + 8191) // 8192 + 1
        export = object.__new__(he.PublicExport)
        for name, value in {'catalogue': catalogue, 'retain_drafts': (draft,),
                            'serialized': json.dumps(packet, ensure_ascii=False, separators=(',', ':')).encode(),
                            '_io': tuple(io.items())}.items():
            object.__setattr__(export, name, value)
        # Prove that screening is the refusal, rather than a mismatched packet
        # or IO subtotal. This is an isolated synthetic mutation control.
        with patch.object(he, '_screen', return_value=None):
            export.__post_init__()
        with self.assertRaises(he.ExportError):
            export.__post_init__()
        with self.assertRaises(he.ExportError):
            he.plan_public_ingestion(export)
        self.network.assert_not_called()


if __name__ == '__main__':
    unittest.main()
