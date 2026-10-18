"""New public, offline source/transport controls; no native provider calls."""
import contextlib
from dataclasses import replace
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import ssl
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import hindsight_recall as hr
from harness.sessions import BRAIN_CONTEXT_HEADER, read_context

PUBLIC = json.loads((Path(__file__).parent / 'fixtures/hindsight_recall_public.json').read_text())


class SuppliedClient:
    def __init__(self, raw):
        self.raw, self.requests = raw, []

    def post(self, scope, request, ledger):
        self.requests.append((scope, request))
        ledger.update(attempts=1, request_body_bytes=len(hr._json(request).encode()),
                      response_body_bytes=len(self.raw), response_eof=True)
        return self.raw


@contextlib.contextmanager
def server(status=200, body=b'{"results":[]}', *, encoding=None, length=None, delay=0):
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            raw = self.rfile.read(int(self.headers['Content-Length']))
            requests.append((self.path, dict(self.headers), raw))
            if delay:
                time.sleep(delay)
            try:
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body) if length is None else length))
                if 300 <= status < 400:
                    self.send_header('Location', '/forbidden-redirect-target')
                if encoding:
                    self.send_header('Content-Encoding', encoding)
                self.end_headers()
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

    listener = HTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=listener.serve_forever, kwargs={'poll_interval': .01})
    thread.start()
    try:
        yield f'http://127.0.0.1:{listener.server_port}', requests
    finally:
        listener.shutdown()
        listener.server_close()
        thread.join(timeout=2)
        if thread.is_alive():
            raise AssertionError('Fixture thread did not terminate.')


class HindsightRecallTests(unittest.TestCase):
    def setUp(self):
        self.scope = hr.Scope(PUBLIC['team_id'], PUBLIC['project_id'])
        self.images = {item['path']: item['text'].encode() for item in PUBLIC['sources']}
        self.bindings = tuple(hr.SourceBinding.from_bytes(self.scope, path, raw)
                              for path, raw in self.images.items())
        self.catalogue = hr.Catalogue(self.scope, self.bindings)
        self.baseline = hr.PilotContext()
        self.local_calls = 0

    def local(self):
        self.local_calls += 1
        return self.baseline

    def admit(self, binding):
        return hr.Evidence(binding, self.images[binding.path])

    def fact(self, binding=None, **changes):
        binding = binding or self.bindings[0]
        return {'id': 'remote-fact-id', 'text': PUBLIC['remote_prose'], 'type': 'world',
                'document_id': binding.document_id,
                'metadata': {'harness_scope': self.scope.key, 'source_sha256': binding.source_sha256},
                **changes}

    def run_recall(self, facts=None, *, client=None, **kwargs):
        client = client or SuppliedClient(hr._json({'results': [self.fact()] if facts is None else facts}).encode())
        return hr.recall_context(PUBLIC['query'], client, self.catalogue,
                                 admit=kwargs.pop('admit', self.admit), local=self.local, **kwargs)

    def fallback(self, outcome):
        self.assertIs(outcome.context, self.baseline)
        self.assertTrue(outcome.diagnostics['fallback'])
        self.assertEqual(0, outcome.diagnostics['accepted'])
        self.assertEqual(1, self.local_calls)

    def test_registered_identity_is_path_independent_and_team_scoped(self):
        with tempfile.TemporaryDirectory() as temporary:
            paths = [Path(temporary) / value / 'same-name' for value in ('clone-a', 'clone-b', 'worktree')]
            for path in paths:
                path.mkdir(parents=True)
            # Registry IDs, deliberately not filesystem names, select the bank.
            self.assertEqual(1, len({hr.Scope(PUBLIC['team_id'], PUBLIC['project_id']).bank_id for _ in paths}))
        self.assertNotEqual(self.scope.bank_id, hr.Scope(PUBLIC['other_team_id'], PUBLIC['project_id']).bank_id)
        self.assertNotEqual(self.scope.bank_id, hr.Scope(PUBLIC['team_id'], PUBLIC['other_project_id']).bank_id)
        with self.assertRaises(ValueError):
            hr.Scope('repo-basename', PUBLIC['project_id'])

    def test_endpoint_and_secret_configuration_refuses_unsafe_forms(self):
        for endpoint in ('http://example.com', 'http://localhost', 'https://user:pass@host',
                         'https://host/base', 'https://host?secret=x', 'https://host/#x', 'https://host\n'):
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError):
                hr.RecallClient(endpoint, bearer='fixture-secret')
        with self.assertRaises(ValueError):
            hr.RecallClient('http://127.0.0.1', bearer='fixture-secret', loopback_fixture=True)
        client = hr.RecallClient('https://memory.example', bearer='fixture-secret')
        self.assertNotIn('fixture-secret', repr(client))
        with self.assertRaises(ValueError):
            hr.RecallClient('https://memory.example', bearer='bad\r\nheader')

    def test_request_has_minimal_world_profile_and_nonempty_strict_scope(self):
        client = SuppliedClient(b'{"results":[]}')
        self.fallback(self.run_recall(client=client))
        scope, request = client.requests[0]
        self.assertEqual(self.scope, scope)
        self.assertEqual({'query': PUBLIC['query'], 'types': ['world'], 'budget': 'low', 'max_tokens': 512,
                          'trace': False, 'include': {'entities': None},
                          'tags': [self.scope.tag], 'tags_match': 'all_strict'}, request)

    def test_explicit_zero_ports_are_rejected_instead_of_redirected(self):
        with self.assertRaises(ValueError):
            hr.RecallClient('https://memory.example:0', bearer='fixture-secret')
        with self.assertRaises(ValueError):
            hr.RecallClient('http://127.0.0.1:0', loopback_fixture=True)

    def test_only_exact_local_source_prose_is_delivered(self):
        result = self.run_recall()
        self.assertFalse(result.diagnostics['fallback'])
        self.assertEqual(1, result.diagnostics['accepted'])
        self.assertIn(PUBLIC['expected']['canonical_retry_phrase'], result.context.envelope)
        self.assertNotIn(PUBLIC['expected']['prohibited_remote_phrase'], result.context.envelope)
        self.assertTrue(result.context.envelope.startswith(PUBLIC['whole_warning']))
        self.assertEqual(PUBLIC['whole_warning'], BRAIN_CONTEXT_HEADER)
        self.assertEqual(self.bindings[0].source_sha256, result.context.sources[0].source_sha256)

    def test_changed_and_deleted_sources_refuse_stale_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binding = self.bindings[0]
            target = root / binding.path
            target.parent.mkdir(parents=True)
            target.write_bytes(self.images[binding.path])

            def current(item):
                read = read_context(root, item.path, hr.SOURCE_BYTES + 1)
                return None if read is None else hr.Evidence(item, read[1].encode())

            target.write_text(PUBLIC['sources'][0]['corrected'])
            self.fallback(self.run_recall(admit=current))
            self.local_calls = 0
            target.unlink()
            self.fallback(self.run_recall(admit=current))

    def test_corrected_source_gets_new_identity_and_can_be_admitted(self):
        old = self.bindings[0]
        raw = PUBLIC['sources'][0]['corrected'].encode()
        new = hr.SourceBinding.from_bytes(self.scope, old.path, raw)
        self.assertNotEqual(old.document_id, new.document_id)
        catalogue = hr.Catalogue(self.scope, (new,))
        fact = self.fact(new)
        result = hr.recall_context(PUBLIC['query'], SuppliedClient(hr._json({'results': [fact]}).encode()),
                                   catalogue, admit=lambda binding: hr.Evidence(binding, raw), local=self.local)
        self.assertIn(PUBLIC['expected']['correction_phrase'], result.context.envelope)
        self.assertEqual(1, result.diagnostics['accepted'])

    def test_unknown_foreign_and_hash_mismatch_hints_are_omitted(self):
        for fact in (self.fact(document_id='../outside'), self.fact(type='observation'),
                     self.fact(metadata={'harness_scope': 'foreign', 'source_sha256': self.bindings[0].source_sha256}),
                     self.fact(metadata={'harness_scope': self.scope.key, 'source_sha256': '0' * 64})):
            with self.subTest(fact=fact):
                self.local_calls = 0
                result = self.run_recall([fact])
                self.fallback(result)
                self.assertEqual(1, result.diagnostics['rejected'])

    def test_duplicates_preserve_baseline_and_order(self):
        self.baseline = hr.PilotContext((hr.Evidence(self.bindings[1], self.images[self.bindings[1].path]).passage(self.bindings[1]),))
        before = self.baseline.envelope
        result = self.run_recall([self.fact(self.bindings[1]), self.fact(), self.fact()])
        self.assertEqual([self.bindings[1].path, self.bindings[0].path], [item.path for item in result.context.sources])
        self.assertEqual(1, result.diagnostics['accepted'])
        self.assertEqual(before, self.baseline.envelope)

    def test_strict_json_and_response_cardinality_fail_to_exact_baseline(self):
        for raw in (b'{"results":[],"results":[]}', b'{"results":[],"x":NaN}',
                    b'{"results":[],"x":1e999}', b'\xff', b'{"results":', b'[]',
                    hr._json({'results': [self.fact()] * 33}).encode(),
                    b'{"results":[],"x":' + b'[' * 18 + b'0' + b']' * 18 + b'}'):
            with self.subTest(raw=raw[:40]):
                self.local_calls = 0
                self.fallback(self.run_recall(client=SuppliedClient(raw)))

    def test_whole_envelope_budget_charges_metadata_escaping_and_multibyte(self):
        binding = self.bindings[0]
        raw = ('\\"界' * 900).encode()
        binding = hr.SourceBinding.from_bytes(self.scope, binding.path, raw)
        catalogue = hr.Catalogue(self.scope, (binding,))
        client = SuppliedClient(hr._json({'results': [self.fact(binding)]}).encode())
        outcome = hr.recall_context(PUBLIC['query'], client, catalogue,
                                   admit=lambda item: hr.Evidence(item, raw), local=self.local,
                                   byte_limit=1000, char_limit=1000)
        self.fallback(outcome)
        self.assertEqual(1, outcome.diagnostics['rejected'])
        self.assertLess(len(outcome.context.envelope.encode()), 1000)

    def test_overbudget_baseline_is_a_local_refusal_without_network(self):
        self.baseline = hr.PilotContext((hr.Passage('local', 'docs/local.md', '0' * 64, 'x' * 9000),))
        client = SuppliedClient(b'{"results":[]}')
        with self.assertRaises(ValueError):
            self.run_recall(client=client)
        self.assertEqual([], client.requests)
        self.assertEqual(1, self.local_calls)

    def test_callback_denial_and_oversize_images_preserve_exact_baseline(self):
        for callback in (lambda binding: None, lambda binding: hr.Evidence(binding, b'x' * (hr.SOURCE_BYTES + 1)),
                         lambda binding: hr.Evidence(binding, self.images[binding.path], True, 10)):
            self.local_calls = 0
            self.fallback(self.run_recall(admit=callback))

    def test_callback_failure_after_first_source_discards_tentative_evidence(self):
        def callback(binding):
            if binding == self.bindings[1]:
                raise RuntimeError('private-source-error-prose')
            return self.admit(binding)
        result = self.run_recall([self.fact(), self.fact(self.bindings[1])], admit=callback)
        self.fallback(result)
        self.assertNotIn('private-source-error-prose', json.dumps(result.diagnostics))

    def test_final_callback_cancellation_does_not_publish(self):
        live = [True]
        def callback(binding):
            live[0] = False
            return self.admit(binding)
        result = self.run_recall(admit=callback, active=lambda: live[0])
        self.fallback(result)
        self.assertEqual('cancelled', result.diagnostics['outcome'])

    def test_empty_private_and_secret_queries_do_not_reach_transport(self):
        for query in ('', 'alice@example.com', 'API_KEY=sk-proj-' + 'A1' * 20):
            client = SuppliedClient(b'{"results":[]}')
            self.local_calls = 0
            result = hr.recall_context(query, client, self.catalogue, admit=self.admit, local=self.local)
            self.fallback(result)
            self.assertEqual([], client.requests)

    def test_unknown_cost_and_private_identifiers_stay_out_of_diagnostics(self):
        result = self.run_recall()
        for key in ('remote_tokens', 'remote_usd', 'provider_tokens'):
            self.assertIsNone(result.diagnostics[key])
        self.assertFalse(result.diagnostics['total_cost_complete'])
        encoded = json.dumps(result.diagnostics)
        for secret in (PUBLIC['query'], PUBLIC['remote_prose'], self.scope.key, self.bindings[0].path):
            self.assertNotIn(secret, encoded)

    def test_unknown_remote_failure_prose_is_replaced_with_fixed_code(self):
        for reason in ('fixture-private-error-path-and-body', 'augmented', {'private': 'error'}):
            class FailingClient:
                def post(inner, scope, request, metrics):
                    raise hr.RemoteFailure(reason)
            self.local_calls = 0
            result = self.run_recall(client=FailingClient())
            self.fallback(result)
            self.assertEqual('unavailable', result.diagnostics['outcome'])
            self.assertNotIn('fixture-private-error', json.dumps(result.diagnostics))
            self.assertEqual(1, result.diagnostics['attempts'])
            self.assertFalse(result.diagnostics['transport_metadata_complete'])

    def test_supplied_transport_metrics_cannot_inject_diagnostics_or_cost(self):
        raw = hr._json({'results': [self.fact()]}).encode()
        class HostileClient:
            def post(inner, scope, request, metrics):
                metrics.update(raw_body='fixture-private-body', outcome='forged', accepted=999,
                               remote_tokens=0, remote_usd=0, provider_tokens=0, total_cost_complete=True,
                               request_body_bytes=True, response_body_bytes=10**100, response_eof='yes')
                return raw
        result = self.run_recall(client=HostileClient())
        self.assertEqual('augmented', result.diagnostics['outcome'])
        self.assertEqual(1, result.diagnostics['accepted'])
        self.assertEqual(1, result.diagnostics['attempts'])
        self.assertNotIn('raw_body', result.diagnostics)
        for name in ('request_body_bytes', 'response_body_bytes', 'response_eof',
                     'remote_tokens', 'remote_usd', 'provider_tokens'):
            self.assertIsNone(result.diagnostics[name])
        self.assertFalse(result.diagnostics['total_cost_complete'])
        self.assertFalse(result.diagnostics['transport_metadata_complete'])

    def test_mixed_secret_query_is_refused_but_useful_pii_query_is_redacted(self):
        secret = 'API_KEY=sk-proj-' + 'A1' * 20
        client = SuppliedClient(b'{"results":[]}')
        result = hr.recall_context('Explain worker retries; ' + secret, client,
                                   self.catalogue, admit=self.admit, local=self.local)
        self.fallback(result)
        self.assertEqual('query_rejected', result.diagnostics['outcome'])
        self.assertEqual([], client.requests)
        self.local_calls = 0
        client = SuppliedClient(b'{"results":[]}')
        result = hr.recall_context('Explain worker retries for alice@example.com', client,
                                   self.catalogue, admit=self.admit, local=self.local)
        self.fallback(result)
        self.assertEqual(1, len(client.requests))
        self.assertNotIn('alice@example.com', client.requests[0][1]['query'])
        self.assertIn('worker retries', client.requests[0][1]['query'])

    def test_real_loopback_client_posts_once_and_ignores_ambient_proxy(self):
        body = hr._json({'results': [self.fact()]}).encode()
        with server(body=body) as (endpoint, requests), patch.dict(os.environ, {'http_proxy': 'http://127.0.0.1:1', 'HTTP_PROXY': 'http://127.0.0.1:1', 'NO_PROXY': ''}):
            result = self.run_recall(client=hr.RecallClient(endpoint, loopback_fixture=True))
        self.assertEqual(1, result.diagnostics['accepted'])
        self.assertEqual(1, len(requests))
        path, headers, raw = requests[0]
        self.assertEqual('/v1/default/banks/' + self.scope.bank_id + '/memories/recall', path)
        self.assertEqual('identity', headers['Accept-Encoding'])
        self.assertNotIn('Authorization', headers)
        self.assertEqual(len(raw), result.diagnostics['request_body_bytes'])
        self.assertEqual(len(body), result.diagnostics['response_body_bytes'])
        self.assertTrue(result.diagnostics['response_eof'])

    def test_redirect_and_auth_failures_have_no_retry_or_body_retention(self):
        for status in (302, 401, 403, 429, 500):
            self.local_calls = 0
            with server(status=status, body=b'private-error-body') as (endpoint, requests):
                result = self.run_recall(client=hr.RecallClient(endpoint, loopback_fixture=True))
            self.fallback(result)
            self.assertEqual(1, len(requests))
            self.assertEqual(1, result.diagnostics['attempts'])
            self.assertNotIn('private-error-body', json.dumps(result.diagnostics))

    def test_response_bound_compression_and_premature_eof_fail_closed(self):
        for parameters, reason in (({'body': b'x' * (hr.RESPONSE_BYTES + 10)}, 'response_bound'),
                                   ({'encoding': 'gzip'}, 'response_encoding'),
                                   ({'body': b'{"results":[]}', 'length': 200}, 'incomplete_response')):
            self.local_calls = 0
            with server(**parameters) as (endpoint, requests):
                result = self.run_recall(client=hr.RecallClient(endpoint, loopback_fixture=True))
            self.fallback(result)
            self.assertEqual(reason, result.diagnostics['outcome'])
            self.assertFalse(result.diagnostics['response_eof'])
            self.assertEqual(1, len(requests))
            if reason == 'response_bound':
                self.assertEqual(hr.RESPONSE_BYTES + 1, result.diagnostics['response_body_bytes'])

    def test_socket_timeout_closes_fixture_and_preserves_local(self):
        with server(delay=.12) as (endpoint, requests):
            result = self.run_recall(client=hr.RecallClient(endpoint, loopback_fixture=True, timeout=.05))
        self.fallback(result)
        self.assertEqual('timeout', result.diagnostics['outcome'])
        self.assertEqual(1, len(requests))

    def test_https_uses_verified_context_and_does_not_keep_failure_prose(self):
        with patch.object(hr.http.client, 'HTTPSConnection', side_effect=ssl.SSLError('fixture-private-tls-error')) as connection:
            result = self.run_recall(client=hr.RecallClient('https://memory.example', bearer='fixture-secret'))
        self.fallback(result)
        context = connection.call_args.kwargs['context']
        self.assertTrue(context.check_hostname)
        self.assertEqual(ssl.CERT_REQUIRED, context.verify_mode)
        self.assertNotIn('fixture-private-tls-error', json.dumps(result.diagnostics))

    def test_catalogue_refuses_duplicates_scope_mixture_and_unsafe_paths(self):
        with self.assertRaises(ValueError):
            hr.Catalogue(self.scope, (self.bindings[0], self.bindings[0]))
        with self.assertRaises(ValueError):
            hr.Catalogue(hr.Scope(PUBLIC['other_team_id'], PUBLIC['project_id']), self.bindings)
        for path in ('../secret', '/etc/passwd', 'docs/../secret', 'docs\\secret', 'docs//secret'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                replace(self.bindings[0], path=path)


if __name__ == '__main__':
    unittest.main()
