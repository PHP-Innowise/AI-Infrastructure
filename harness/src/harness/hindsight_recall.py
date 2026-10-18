"""Experimental, source-only Hindsight recall boundary; not a native capsule.

No default Harness routing, retain, remote prose delivery or local ML dependency.
The caller owns authorization, the source catalogue and current local admission.
See docs/HINDSIGHT-PILOT.md before using this with a real service.
"""
from __future__ import annotations

from dataclasses import dataclass, field
import hashlib
import http.client
import json
import math
from pathlib import PurePosixPath
import re
import socket
import ssl
import time
from urllib.parse import urlsplit
import uuid

from .automatic_query import AutomaticQueryError, SECRET_PATTERNS, sanitize_automatic_query
from .sessions import capsule_envelope

REQUEST_BYTES = 8192
RESPONSE_BYTES = 65536
SOURCE_BYTES = 131072
MAX_REFERENCES = 32
QUERY_BYTES = 2000
PROFILE = 'hindsight-source-pilot-v1'
NATIVE_REFERENCES = 8
_IDENTIFIER = re.compile(r'[A-Za-z0-9][A-Za-z0-9._-]{0,127}')
_DIGEST = re.compile(r'[a-f0-9]{64}')
_REASONS = frozenset(('request_bound', 'redirect', 'authorization', 'http_failure',
                      'response_encoding', 'timeout', 'response_bound',
                      'incomplete_response', 'transport_failure', 'unavailable',
                      'cancelled', 'query_bound', 'query_rejected', 'query_empty',
                      'catalogue_empty', 'response_schema', 'admission_failure',
                      'no_admitted_sources', 'augmented'))


def _json(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'), allow_nan=False)


def _uuid(value):
    if type(value) is not str or str(uuid.UUID(value)) != value:
        raise ValueError('A registered canonical UUID is required.')
    return value


@dataclass(frozen=True)
class Scope:
    """Caller-authorized registry IDs; neither paths nor repo-supplied config.

    IDs/tags express routing consistency. Service credentials must independently
    authorize this bank; constructing this object does not grant access.
    """
    team_id: str
    project_id: str

    def __post_init__(self):
        _uuid(self.team_id)
        _uuid(self.project_id)

    @property
    def key(self):
        return hashlib.sha256(_json([self.team_id, self.project_id]).encode()).hexdigest()

    @property
    def bank_id(self):
        return 'harness-' + self.key

    @property
    def tag(self):
        return 'harness-scope:' + self.key


@dataclass(frozen=True)
class SourceBinding:
    """Locally registered document identity, never a server-selected path."""
    scope: Scope
    document_id: str
    path: str
    source_sha256: str

    def __post_init__(self):
        if (type(self.scope) is not Scope or type(self.document_id) is not str
                or not _IDENTIFIER.fullmatch(self.document_id)
                or type(self.path) is not str or not 1 <= len(self.path.encode('utf-8')) <= 512
                or '\\' in self.path or any(ord(c) < 32 or ord(c) == 127 for c in self.path)
                or PurePosixPath(self.path).is_absolute()
                or any(part in ('', '.', '..') for part in self.path.split('/'))
                or type(self.source_sha256) is not str or not _DIGEST.fullmatch(self.source_sha256)):
            raise ValueError('Invalid local source binding.')

    @classmethod
    def from_bytes(cls, scope, path, raw):
        if type(raw) is not bytes or len(raw) > SOURCE_BYTES:
            raise ValueError('Invalid bounded source image.')
        digest = hashlib.sha256(raw).hexdigest()
        identity = hashlib.sha256(_json([scope.key, path, digest]).encode()).hexdigest()
        return cls(scope, 'source-' + identity, path, digest)


@dataclass(frozen=True)
class Catalogue:
    scope: Scope
    sources: tuple[SourceBinding, ...]

    def __post_init__(self):
        if (type(self.scope) is not Scope or type(self.sources) is not tuple
                or len(self.sources) > MAX_REFERENCES
                or any(type(item) is not SourceBinding or item.scope != self.scope for item in self.sources)
                or len({item.document_id for item in self.sources}) != len(self.sources)):
            raise ValueError('Invalid bounded source catalogue.')


@dataclass(frozen=True)
class Evidence:
    """Admission callback's current source image and exact byte range.

    The callback must also validate current policy, privacy, lifecycle and
    ownership. Matching bytes alone do not certify that native eligibility.
    """
    binding: SourceBinding
    raw: bytes = field(repr=False)
    start: int = 0
    end: int | None = None

    def passage(self, expected):
        if (self.binding != expected or type(self.raw) is not bytes or len(self.raw) > SOURCE_BYTES
                or hashlib.sha256(self.raw).hexdigest() != expected.source_sha256):
            raise ValueError('Source image changed.')
        end = len(self.raw) if self.end is None else self.end
        if type(self.start) is not int or type(end) is not int or not 0 <= self.start < end <= len(self.raw):
            raise ValueError('Invalid source range.')
        text = self.raw[self.start:end].decode('utf-8', 'strict')
        if any(ord(c) < 32 and c not in '\n\t' or ord(c) == 127 for c in text):
            raise ValueError('Invalid source text.')
        return Passage(expected.document_id, expected.path, expected.source_sha256, text)


@dataclass(frozen=True)
class Passage:
    document_id: str
    path: str
    source_sha256: str
    text: str


@dataclass(frozen=True)
class PilotContext:
    """Supplied source passages, NOT a task-bound native TaskContext capsule."""
    sources: tuple[Passage, ...] = ()

    @property
    def envelope(self):
        body = {'profile': PROFILE, 'sources': [
            {'document_id': item.document_id, 'path': item.path,
             'source_sha256': item.source_sha256, 'text': item.text}
            for item in self.sources]}
        return capsule_envelope(body, compact=True)


class RemoteFailure(Exception):
    def __init__(self, reason):
        self.reason = reason if type(reason) is str and reason in _REASONS and reason != 'augmented' else 'unavailable'
        super().__init__(self.reason)


def _strict_json(raw):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('Duplicate JSON key.')
            result[key] = value
        return result

    def invalid_constant(_value):
        raise ValueError('Invalid JSON constant.')

    value = json.loads(raw.decode('utf-8', 'strict'), object_pairs_hook=unique,
                       parse_constant=invalid_constant)
    pending, count = [(value, 0)], 0
    while pending:
        item, depth = pending.pop()
        count += 1
        if count > 4096 or depth > 16:
            raise ValueError('JSON admission bound.')
        if type(item) is dict:
            pending.extend((child, depth + 1) for child in item.values())
        elif type(item) is list:
            pending.extend((child, depth + 1) for child in item)
        elif type(item) is float and not math.isfinite(item):
            raise ValueError('Invalid JSON number.')
    return value


class RecallClient:
    """One POST, direct connection, verified TLS, no redirects/proxy/retries.

    Socket timeouts and monotonic I/O boundary checks are not a hard wall limit
    for DNS/header processing. This is not ready for automatic native routing.
    """
    def __init__(self, endpoint, *, bearer=None, timeout=2.0, loopback_fixture=False):
        try:
            if type(endpoint) is not str or not 1 <= len(endpoint) <= 2048 or any(ord(c) <= 32 or ord(c) == 127 for c in endpoint):
                raise ValueError
            parsed = urlsplit(endpoint)
            if (parsed.username is not None or parsed.password is not None
                    or parsed.path not in ('', '/') or parsed.query or parsed.fragment
                    or not parsed.hostname or parsed.scheme not in ('http', 'https')):
                raise ValueError
            port = parsed.port if parsed.port is not None else (443 if parsed.scheme == 'https' else 80)
            if type(loopback_fixture) is not bool:
                raise ValueError
            if parsed.scheme == 'http':
                if not loopback_fixture or parsed.hostname not in ('127.0.0.1', '::1') or bearer is not None:
                    raise ValueError
            elif loopback_fixture or type(bearer) is not str or not 1 <= len(bearer) <= 4096 or any(not 33 <= ord(c) <= 126 for c in bearer):
                raise ValueError
            if type(timeout) not in (int, float) or not math.isfinite(timeout) or not .05 <= timeout <= 10:
                raise ValueError
            if not 1 <= port <= 65535:
                raise ValueError
        except (ValueError, TypeError):
            raise ValueError('Invalid remote endpoint or credential configuration.') from None
        self._scheme, self._host, self._port = parsed.scheme, parsed.hostname, port
        self._bearer, self.timeout = bearer, float(timeout)

    def post(self, scope, request, ledger):
        raw = _json(request).encode('utf-8')
        if len(raw) > REQUEST_BYTES:
            raise RemoteFailure('request_bound')
        ledger['attempts'] = 1
        ledger['request_body_bytes'] = len(raw)
        deadline = time.monotonic() + self.timeout
        connection = None
        response = None
        try:
            if self._scheme == 'https':
                connection = http.client.HTTPSConnection(self._host, self._port, timeout=self.timeout,
                                                         context=ssl.create_default_context())
            else:
                connection = http.client.HTTPConnection(self._host, self._port, timeout=self.timeout)
            headers = {'Content-Type': 'application/json', 'Accept': 'application/json',
                       'Accept-Encoding': 'identity'}
            if self._bearer is not None:
                headers['Authorization'] = 'Bearer ' + self._bearer
            connection.request('POST', '/v1/default/banks/' + scope.bank_id + '/memories/recall', raw, headers)
            response = connection.getresponse()
            if 300 <= response.status < 400:
                raise RemoteFailure('redirect')
            if response.status in (401, 403):
                raise RemoteFailure('authorization')
            if response.status != 200:
                raise RemoteFailure('http_failure')
            if (response.getheader('Content-Encoding', 'identity').lower() != 'identity'
                    or response.getheader('Content-Type', '').split(';')[0].strip().lower() != 'application/json'):
                raise RemoteFailure('response_encoding')
            data = bytearray()
            ledger['response_body_bytes'] = 0
            while True:
                left = deadline - time.monotonic()
                if left <= 0:
                    raise RemoteFailure('timeout')
                if connection.sock is not None:
                    connection.sock.settimeout(left)
                chunk = response.read1(min(4096, RESPONSE_BYTES + 1 - len(data)))
                ledger['response_body_bytes'] += len(chunk)
                if time.monotonic() > deadline:
                    raise RemoteFailure('timeout')
                if not chunk:
                    if response.length not in (None, 0):
                        raise RemoteFailure('incomplete_response')
                    ledger['response_eof'] = True
                    break
                data.extend(chunk)
                if len(data) > RESPONSE_BYTES:
                    raise RemoteFailure('response_bound')
            return bytes(data)
        except (TimeoutError, socket.timeout):
            # socket.timeout is TimeoutError only from Python 3.10.
            raise RemoteFailure('timeout') from None
        except (OSError, http.client.HTTPException):
            raise RemoteFailure('transport_failure') from None
        finally:
            try:
                if response is not None:
                    response.close()
            finally:
                if connection is not None:
                    connection.close()


@dataclass(frozen=True)
class PilotOutcome:
    context: PilotContext
    diagnostics: dict


@dataclass(frozen=True)
class ReferenceOutcome:
    """Catalogue-resolved discovery references, never native admission/delivery."""
    references: tuple[SourceBinding, ...]
    diagnostics: dict

    def native_payload(self):
        if (type(self.references) is not tuple or len(self.references) > NATIVE_REFERENCES
                or any(type(item) is not SourceBinding for item in self.references)):
            raise ValueError('Invalid reference outcome.')
        return {'schema_version': 1, 'references': [
            {'path': item.path, 'source_sha256': item.source_sha256} for item in self.references]}


def recall_references(query, client, catalogue, *, active=lambda: True):
    """One optional recall, resolved only through an operator-owned catalogue.

    Local source admission and final delivery are performed by native retrieval.
    The synchronous trusted callback/HTTP transport is not a hard deadline.
    """
    if type(catalogue) is not Catalogue or not callable(active):
        raise ValueError('Invalid reference integration boundary.')
    started = time.monotonic()
    transport = {'request_body_bytes': None, 'response_body_bytes': None, 'response_eof': False}
    attempts, rejected = 0, 0

    def finish(references=(), outcome='unavailable'):
        if type(outcome) is not str or outcome not in (_REASONS - {'augmented'} | {'resolved'}):
            outcome = 'unavailable'
        diagnostics = {'profile': PROFILE, 'attempts': attempts, 'resolved': len(references),
                       'rejected': rejected, 'outcome': outcome, 'elapsed_seconds': time.monotonic() - started,
                       'remote_tokens': None, 'remote_usd': None, 'provider_tokens': None,
                       'total_cost_complete': False}
        for key, maximum in (('request_body_bytes', REQUEST_BYTES), ('response_body_bytes', RESPONSE_BYTES + 1)):
            value = transport.get(key)
            diagnostics[key] = value if type(value) is int and 0 <= value <= maximum else None
        eof = transport.get('response_eof')
        diagnostics['response_eof'] = eof if type(eof) is bool else None
        return ReferenceOutcome(tuple(references), diagnostics)

    try:
        if not active():
            return finish(outcome='cancelled')
        if type(query) is not str or len(query.encode('utf-8')) > QUERY_BYTES:
            return finish(outcome='query_bound')
        if any(pattern.search(query) for pattern in SECRET_PATTERNS.values()):
            return finish(outcome='query_rejected')
        query = sanitize_automatic_query(query).strip()
        if not query:
            return finish(outcome='query_empty')
        if not catalogue.sources:
            return finish(outcome='catalogue_empty')
        request = {'query': query, 'types': ['world'], 'budget': 'low', 'max_tokens': 512,
                   'trace': False, 'include': {'entities': None},
                   'tags': [catalogue.scope.tag], 'tags_match': 'all_strict'}
        attempts = 1  # client method invocation, not server receipt or wire count
        raw = client.post(catalogue.scope, request, transport)
        if type(raw) is not bytes or len(raw) > RESPONSE_BYTES:
            return finish(outcome='response_bound')
        value = _strict_json(raw)
        if type(value) is not dict or type(value.get('results')) is not list or len(value['results']) > MAX_REFERENCES:
            return finish(outcome='response_schema')
        bindings = {item.document_id: item for item in catalogue.sources}
        references, seen = [], set()
        for item in value['results']:
            if not active():
                return finish(outcome='cancelled')
            if type(item) is not dict:
                return finish(outcome='response_schema')
            document, metadata = item.get('document_id'), item.get('metadata')
            binding = bindings.get(document) if type(document) is str else None
            if (binding is None or type(metadata) is not dict or metadata.get('harness_scope') != catalogue.scope.key
                    or metadata.get('source_sha256') != binding.source_sha256 or item.get('type') != 'world'):
                rejected += 1
                continue
            if document not in seen:
                seen.add(document)
                if len(references) < NATIVE_REFERENCES:
                    references.append(binding)
                else:
                    rejected += 1
        if not active():
            return finish(outcome='cancelled')
        return finish(references, 'resolved' if references else 'no_admitted_sources')
    except RemoteFailure as error:
        return finish(outcome=error.reason)
    except Exception:
        return finish()


def source_hint_provider(client, catalogue, *, project_id, bank, workspace):
    """Operator-bound local route to registered remote UUIDs, never path-derived IDs.

    A worktree/clone needs its own explicit local route and current catalogue.
    The fixed local route is checked before any query leaves the process.
    """
    if (type(catalogue) is not Catalogue or any(type(value) is not str or not value
                                              for value in (project_id, bank, workspace))):
        raise ValueError('Invalid source catalogue.')
    route = (project_id, bank, workspace)
    def provider(query, *, active, project_id, bank, workspace):
        if (project_id, bank, workspace) != route:
            return ReferenceOutcome((), {'attempts': 0, 'outcome': 'unavailable',
                                         'request_body_bytes': None, 'response_body_bytes': None, 'response_eof': None})
        return recall_references(query, client, catalogue, active=active)
    return provider


def _fits(context, byte_limit, char_limit):
    if type(context) is not PilotContext or type(context.sources) is not tuple:
        raise ValueError('Invalid local context.')
    envelope = context.envelope
    return len(envelope) <= char_limit and len(envelope.encode('utf-8')) <= byte_limit


def recall_context(query, client, catalogue, *, admit, local, active=lambda: True,
                   byte_limit=8000, char_limit=8000):
    """Augment a once-computed local result with locally admitted source text.

    `admit(binding)` must revalidate current source and local eligibility, then
    return Evidence or None. All failures preserve the exact baseline object.
    No remote fact text, exceptions or IDs are retained in diagnostics.
    """
    if type(catalogue) is not Catalogue or not callable(admit) or not callable(local) or not callable(active):
        raise ValueError('Invalid pilot integration boundary.')
    if (type(byte_limit) is not int or type(char_limit) is not int
            or not 256 <= byte_limit <= 8000 or not 256 <= char_limit <= 8000):
        raise ValueError('Invalid whole-context budget.')
    started = time.monotonic()
    baseline = local()
    if not _fits(baseline, byte_limit, char_limit):
        raise ValueError('Local baseline exceeds the whole-context budget.')
    transport = {'request_body_bytes': None, 'response_body_bytes': None, 'response_eof': False}
    ledger = {'profile': PROFILE, 'attempts': 0,
              'accepted': 0, 'rejected': 0, 'outcome': 'unavailable', 'fallback': True,
              'elapsed_seconds': None, 'remote_tokens': None, 'remote_usd': None,
              'provider_tokens': None, 'total_cost_complete': False}

    def finish(context, reason):
        reason = reason if type(reason) is str and reason in _REASONS else 'unavailable'
        if context is baseline and reason == 'augmented':
            reason = 'unavailable'
        ledger.update(outcome=reason, fallback=context is baseline,
                      elapsed_seconds=round(time.monotonic() - started, 6))
        if context is baseline:
            ledger['accepted'] = 0
        # A replaceable transport cannot insert prose, claim cost completeness,
        # overwrite selection counts, or supply unbounded metrics.
        diagnostics = dict(ledger)
        for name, maximum in (('request_body_bytes', REQUEST_BYTES),
                              ('response_body_bytes', RESPONSE_BYTES + 1)):
            value = transport.get(name)
            diagnostics[name] = value if type(value) is int and 0 <= value <= maximum else None
        eof = transport.get('response_eof')
        diagnostics['response_eof'] = eof if type(eof) is bool else None
        diagnostics['transport_metadata_complete'] = (
            ledger['attempts'] == 0 or (diagnostics['request_body_bytes'] is not None
            and diagnostics['response_body_bytes'] is not None and type(eof) is bool))
        return PilotOutcome(context, diagnostics)

    try:
        if not active():
            return finish(baseline, 'cancelled')
        if type(query) is not str or len(query.encode('utf-8')) > QUERY_BYTES:
            return finish(baseline, 'query_bound')
        if any(pattern.search(query) for pattern in SECRET_PATTERNS.values()):
            return finish(baseline, 'query_rejected')
        try:
            query = sanitize_automatic_query(query).strip()
        except AutomaticQueryError:
            return finish(baseline, 'query_rejected')
        if not query:
            return finish(baseline, 'query_empty')
        if not catalogue.sources:
            return finish(baseline, 'catalogue_empty')
        request = {'query': query, 'types': ['world'], 'budget': 'low', 'max_tokens': 512,
                   'trace': False, 'include': {'entities': None},
                   'tags': [catalogue.scope.tag], 'tags_match': 'all_strict'}
        ledger['attempts'] = 1  # one logical method call; not server receipt proof
        raw = client.post(catalogue.scope, request, transport)
        if type(raw) is not bytes or len(raw) > RESPONSE_BYTES:
            return finish(baseline, 'response_bound')
        value = _strict_json(raw)
        if type(value) is not dict or type(value.get('results')) is not list or len(value['results']) > MAX_REFERENCES:
            return finish(baseline, 'response_schema')
        bindings = {item.document_id: item for item in catalogue.sources}
        sources, seen = list(baseline.sources), {item.document_id for item in baseline.sources}
        for item in value['results']:
            if not active():
                return finish(baseline, 'cancelled')
            if type(item) is not dict:
                return finish(baseline, 'response_schema')
            document = item.get('document_id')
            metadata = item.get('metadata')
            binding = bindings.get(document) if type(document) is str else None
            if (binding is None or type(metadata) is not dict
                    or metadata.get('harness_scope') != catalogue.scope.key
                    or metadata.get('source_sha256') != binding.source_sha256
                    or item.get('type') != 'world'):
                ledger['rejected'] += 1
                continue
            if document in seen:
                continue
            seen.add(document)
            evidence = admit(binding)
            if evidence is None:
                ledger['rejected'] += 1
                continue
            if type(evidence) is not Evidence:
                return finish(baseline, 'admission_failure')
            try:
                passage = evidence.passage(binding)
            except (ValueError, UnicodeError):
                ledger['rejected'] += 1
                continue
            proposed = PilotContext(tuple([*sources, passage]))
            if not _fits(proposed, byte_limit, char_limit):
                ledger['rejected'] += 1
                continue
            sources.append(passage)
            ledger['accepted'] += 1
        if not active():
            return finish(baseline, 'cancelled')
        if not ledger['accepted']:
            return finish(baseline, 'no_admitted_sources')
        return finish(PilotContext(tuple(sources)), 'augmented')
    except RemoteFailure as error:
        return finish(baseline, error.reason)
    except Exception:
        # Optional backend/callback failures cannot abort valid local memory.
        # BaseException cancellation is not swallowed. No exception prose kept.
        return finish(baseline, 'unavailable')
