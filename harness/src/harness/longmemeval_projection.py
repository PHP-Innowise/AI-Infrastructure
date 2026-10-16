"""Private, bounded LongMemEval-V2 states-to-text/native-FTS5 experiment.

This standalone insert/query interface does not import/register upstream backends,
ingest images, evaluate answers or authorize publication. The operator prepares a
dedicated private project with the pinned native runtime, config and schemas. Run
each corpus in a separate process: native sibling imports have bare module names.
The embedded refresh lane retains native guards/packing, but omits CLI health and
telemetry wrappers. Native local manifests may contain sanitized request text.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import stat
import threading
import time
import unicodedata

PROFILE = 'native-fts5/public-states-text-projection-v1'
INPUT_BYTES = 262144
DOCUMENT_BYTES = 100000
TOTAL_BYTES = 33554432
MAX_TRAJECTORIES = 500
MAX_STATES = 128
MAX_QUERIES = 4096
QUERY_CHARS = 8192
CONTEXT_BYTES = 262144
CUSTODY_FAILURE_PROFILE = 'longmemeval/custody-failure-stage-v1'
CUSTODY_FAILURE_STAGES = frozenset((
    'directory_stat', 'directory_guard', 'expected_entries', 'entry_scan',
    'missing_entries_guard', 'file_open', 'file_stat_before',
    'file_shape_guard', 'file_identity_guard', 'file_read',
    'file_stat_after', 'file_image_guard', 'file_close'))


class ProjectionError(ValueError):
    """Fixed refusal code only; never interpolates untrusted values/errors."""


def _refuse(code):
    raise ProjectionError(code)


def _text(value, maximum, *, nonempty=False, multiline=False, nullable=False):
    if nullable and value is None:
        return None
    if type(value) is not str or len(value) > maximum or (nonempty and not value.strip()):
        _refuse('input_schema')
    for char in value:
        if unicodedata.category(char) in ('Cc', 'Cs') and not (multiline and char in '\r\n\t'):
            _refuse('input_encoding')
    return value


def _admit_tree(value):
    """Admit size/type before canonical serialization (also for direct dicts)."""
    stack = [(value, 0)]
    count = size = 0
    while stack:
        item, depth = stack.pop()
        count += 1
        if count > 16000 or depth > 8:
            _refuse('input_bound')
        if type(item) is str:
            if len(item) > 65536:
                _refuse('input_bound')
            try:
                size += len(item.encode('utf-8'))
            except UnicodeError:
                _refuse('input_encoding')
        elif type(item) is dict:
            if len(item) > 16 or any(type(key) is not str for key in item):
                _refuse('input_schema')
            for key, child in item.items():
                stack.extend(((key, depth + 1), (child, depth + 1)))
            size += 2 + len(item) * 4
        elif type(item) is list:
            if len(item) > MAX_STATES:
                _refuse('input_bound')
            stack.extend((child, depth + 1) for child in item)
            size += 2 + len(item)
        elif item is None:
            size += 4
        elif type(item) is int and 0 <= item <= 2147483647:
            size += 10
        else:
            _refuse('input_schema')
        if size > INPUT_BYTES:
            _refuse('input_bound')


def _json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True,
                      separators=(',', ':'), allow_nan=False)


def decode_trajectory(raw):
    """Strict, bounded transport; direct insert() is validated independently."""
    if type(raw) is not bytes or len(raw) > INPUT_BYTES:
        _refuse('input_bound')

    def pairs(values):
        result = {}
        for key, value in values:
            if key in result:
                _refuse('duplicate_key')
            result[key] = value
        return result

    def constant(_):
        _refuse('input_schema')

    try:
        value = json.loads(raw.decode('utf-8'), object_pairs_hook=pairs, parse_constant=constant)
    except (UnicodeError, json.JSONDecodeError, RecursionError, ValueError) as error:
        if isinstance(error, ProjectionError):
            raise
        _refuse('input_encoding')
    _admit_tree(value)
    return value


def _keys(value, allowed, required):
    if type(value) is not dict or not required <= value.keys() or value.keys() - allowed:
        _refuse('input_schema')


def _reference(value):
    value = _text(value, 1024, nonempty=True)
    node = PurePosixPath(value)
    if (value != node.as_posix() or node.is_absolute() or ':' in value or '\\' in value
            or any(part in ('..', '.') for part in value.split('/'))):
        _refuse('screenshot_reference')
    return value


@dataclass(frozen=True)
class Projection:
    trajectory_id: str
    relative: str
    body: bytes
    sha256: str
    input_sha256: str
    states: int


def project_trajectory(trajectory):
    """Only the named public states subset, no evaluator records or coercions."""
    _admit_tree(trajectory)
    # Render and identity derive from the same captured input image, never from
    # two reads of a caller-owned mutable dictionary.
    raw = _json(trajectory).encode('utf-8')
    if len(raw) > INPUT_BYTES:
        _refuse('input_bound')
    trajectory = decode_trajectory(raw)
    _keys(trajectory, {'id', 'goal', 'start_url', 'outcome', 'states'},
          {'id', 'goal', 'start_url', 'states'})
    identity = _text(trajectory['id'], 256, nonempty=True)
    goal = _text(trajectory['goal'], 8192, multiline=True)
    start = _text(trajectory['start_url'], 4096, nonempty=True)
    outcome = _text(trajectory.get('outcome'), 8192, multiline=True, nullable=True)
    states = trajectory['states']
    if type(states) is not list or not 1 <= len(states) <= MAX_STATES:
        _refuse('input_schema')
    # Native item titles are not excerpt-sanitized. Keep untrusted identifiers
    # in the private mapping, never in the rendered document title.
    opaque_id = hashlib.sha256(identity.encode('utf-8')).hexdigest()
    rows = ['# Recorded trajectory ' + opaque_id, '', 'Goal: ' + _json(goal),
            'Start URL: ' + _json(start), 'Outcome: ' + _json(outcome)]
    for ordinal, state in enumerate(states):
        _keys(state, {'url', 'action', 'thought', 'thoughts', 'accessibility_tree',
                      'text', 'screenshot', 'step', 'state_index'}, {'url', 'screenshot'})
        if ('thought' in state and 'thoughts' in state) or ('accessibility_tree' in state and 'text' in state):
            _refuse('ambiguous_alias')
        if 'step' in state and 'state_index' in state:
            _refuse('ambiguous_alias')
        if 'accessibility_tree' not in state and 'text' not in state:
            _refuse('input_schema')
        step = state.get('step', state.get('state_index', ordinal))
        if type(step) is not int or not 0 <= step <= 2147483647:
            _refuse('input_schema')
        url = _text(state['url'], 4096, nonempty=True)
        action = _text(state.get('action'), 8192, multiline=True, nullable=True)
        thought = _text(state.get('thought', state.get('thoughts')), 8192, multiline=True, nullable=True)
        recorded = _text(state.get('accessibility_tree', state.get('text')), 65536, multiline=True)
        reference = _reference(state['screenshot'])
        rows.extend(['', '## Recorded state %d (step %d)' % (ordinal, step),
                     'URL: ' + _json(url), 'Arrival action: ' + _json(action),
                     'Thought: ' + _json(thought),
                     'Screenshot reference (not ingested): ' + _json(reference),
                     '', 'Recorded text:', '', recorded])
    body = ('\n'.join(rows) + '\n').encode('utf-8')
    if len(body) > DOCUMENT_BYTES or len(raw) > INPUT_BYTES:
        _refuse('input_bound')
    name = 'trajectory-' + opaque_id + '.md'
    return Projection(identity, 'specs/longmemeval/' + name, body,
                      hashlib.sha256(body).hexdigest(), hashlib.sha256(raw).hexdigest(), len(states))


def _identity(fd):
    item = os.fstat(fd)
    return item.st_dev, item.st_ino


def _stable_stat(item):
    # Reading may legitimately change atime; it does not change source bytes.
    return (item.st_dev, item.st_ino, item.st_mode, item.st_nlink,
            item.st_size, item.st_mtime_ns, item.st_ctime_ns)


class NativeProjectionMemory:
    """Operator-isolated corpus; unsupported persistence, no historical adoption.

    An exclusive source directory and descriptor checks protect accidental reuse
    and supplied-file changes. The trusted private root/ancestors/runtime and
    absence of concurrent foreign writers remain prerequisites, not an OS sandbox.
    Query retains native local metadata writes. No-match omits the native warming
    head and returns []; this is an explicit benchmark interface projection.
    """

    def __init__(self, repository, native):
        self.repository = Path(repository).absolute()
        self.native = native
        self._lock = threading.RLock()
        self._fds = []
        self._documents = {}
        self._file_ids = {}
        self._bytes = self._queries = 0
        self._closed = self._fenced = False
        self._last = None
        try:
            self._custody_failure = ('unobserved', None)
        except BaseException:
            pass  # Optional diagnostic storage cannot block initialization.
        try:
            for name in ('connect', 'refresh_layers', 'sanitize_automatic_query',
                         'distill_capsule_query', 'assemble_capsule', 'capsule_excerpts',
                         'render_capsule_lines', 'record_render_trace', 'configured_mode',
                         'reject_secrets', 'reject_capsule_privacy', 'configured_retrieval_gate'):
                if not callable(getattr(native, name, None)):
                    _refuse('native_profile')
            if not (self.repository / 'project-brain/config/runtime.json').is_file():
                _refuse('native_configuration')
            schemas = self.repository / 'project-brain/schemas'
            if not schemas.is_dir() or not tuple(schemas.glob('*.schema.json')):
                _refuse('native_configuration')
            flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
            self._root_fd = os.open(self.repository, flags)
            self._fds.append(self._root_fd)
            self._specs_fd = os.open('specs', flags, dir_fd=self._root_fd)
            self._fds.append(self._specs_fd)
            os.mkdir('longmemeval', mode=0o700, dir_fd=self._specs_fd)
            self._source_fd = os.open('longmemeval', flags, dir_fd=self._specs_fd)
            self._fds.append(self._source_fd)
            os.fsync(self._specs_fd)
        except BaseException:
            try:
                self._drain_owned_fds()
            except BaseException:
                # Keep the construction primary even if helper bookkeeping
                # aborts. Any remaining cleanup is unknown, not successful.
                pass
            raise

    def _drain_owned_fds(self):
        """Consume registered attempts; errors do not certify kernel closure.

        Current-runtime private ownership is trusted. Retire before one close
        so an uncertain descriptor is never retried. Keep errors transient.
        """
        first_error = None
        while self._fds:
            fd = self._fds.pop()
            try:
                os.close(fd)
            except BaseException as error:
                if first_error is None:
                    first_error = error
        return first_error

    def _ready(self):
        if self._closed or self._fenced:
            _refuse('instance_unavailable')

    def _custody(self):
        self._ready()
        stage = 'directory_stat'
        try:
            for node, fd in ((self.repository, self._root_fd),
                             (self.repository / 'specs', self._specs_fd),
                             (self.repository / 'specs/longmemeval', self._source_fd)):
                stage = 'directory_stat'
                current = node.lstat()
                stage = 'directory_guard'
                if not stat.S_ISDIR(current.st_mode) or (current.st_dev, current.st_ino) != _identity(fd):
                    _refuse('source_changed')
            stage = 'expected_entries'
            expected = {PurePosixPath(item.relative).name for item in self._documents.values()}
            # An unexpected population must not be materialized before refusal.
            stage = 'entry_scan'
            # Rewind first. btrfs refreshes a directory's last index only on
            # open and on a seek, and the scan below rewinds only when it
            # closes - so without this, a file inserted since the previous
            # scan reads as missing and the instance fences itself.
            try:
                os.lseek(self._source_fd, 0, os.SEEK_SET)
            except OSError:
                pass
            with os.scandir(self._source_fd) as entries:
                for entry in entries:
                    if entry.name not in expected:
                        _refuse('source_changed')
                    expected.remove(entry.name)
            stage = 'missing_entries_guard'
            if expected:
                _refuse('source_changed')
            for item in self._documents.values():
                stage = 'file_open'
                fd = os.open(PurePosixPath(item.relative).name,
                             os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                             dir_fd=self._source_fd)
                try:
                    stage = 'file_stat_before'
                    before = os.fstat(fd)
                    stage = 'file_shape_guard'
                    if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or before.st_size != len(item.body):
                        _refuse('source_changed')
                    stage = 'file_identity_guard'
                    if (before.st_dev, before.st_ino) != self._file_ids[item.trajectory_id]:
                        _refuse('source_changed')
                    stage = 'file_read'
                    raw = bytearray()
                    while len(raw) <= DOCUMENT_BYTES:
                        chunk = os.read(fd, min(65536, DOCUMENT_BYTES + 1 - len(raw)))
                        if not chunk:
                            break
                        raw.extend(chunk)
                    stage = 'file_stat_after'
                    after = os.fstat(fd)
                    stage = 'file_image_guard'
                    if (_stable_stat(before) != _stable_stat(after) or len(raw) != len(item.body)
                            or hashlib.sha256(raw).hexdigest() != item.sha256):
                        _refuse('source_changed')
                finally:
                    pending_stage = stage
                    stage = 'file_close'
                    os.close(fd)
                    stage = pending_stage
        except (OSError, ProjectionError):
            self._fenced = True
            try:
                self._custody_failure = ('unmeasured', None)
                self._capture_custody_failure(stage)
            except BaseException:
                # Only diagnostic failure is suppressed; retain normal refusal.
                try:
                    self._custody_failure = ('unmeasured', None)
                except BaseException:
                    pass  # Storage unavailable: existing metadata is not fresh.
            _refuse('source_changed')

    def _capture_custody_failure(self, stage):
        with self._lock:
            self._custody_failure = (('stage_reported', stage)
                if type(stage) is str and len(stage) <= 32
                and stage in CUSTODY_FAILURE_STAGES else ('unmeasured', None))

    def custody_failure_observation(self):
        """Last caught body failure site; no subpredicate or root-cause proof.

        Pre-body availability and uncaught aborts are outside this profile.
        Successful calls retain the last failure; absence does not mean no errors.
        Rejected diagnostic writes can retain prior/unobserved data; this record
        does not attest freshness for a particular call.
        Bookkeeping is inside operation time; getter serialization is separate.
        """
        with self._lock:
            value = getattr(self, '_custody_failure', ('unobserved', None))
            state, stage = 'unmeasured', None
            if type(value) is tuple and len(value) == 2:
                candidate, site = value
                if type(candidate) is str and len(candidate) <= 32:
                    if candidate in ('unobserved', 'unmeasured') and site is None:
                        state = candidate
                    elif (candidate == 'stage_reported' and type(site) is str
                          and len(site) <= 32 and site in CUSTODY_FAILURE_STAGES):
                        state, stage = candidate, site
            return {'profile': CUSTODY_FAILURE_PROFILE,
                    'scope': 'last_caught_custody_body_failure',
                    'observation_state': state, 'stage': stage,
                    'root_cause': None, 'provider_tokens': None, 'usd_micros': None}

    def insert(self, trajectory):
        item = project_trajectory(trajectory)
        with self._lock:
            self._custody()
            old = self._documents.get(item.trajectory_id)
            if old is not None:
                if old.input_sha256 == item.input_sha256:
                    return
                _refuse('conflicting_identity')
            if len(self._documents) >= MAX_TRAJECTORIES or self._bytes + len(item.body) > TOTAL_BYTES:
                _refuse('corpus_bound')
            fd = None
            try:
                fd = os.open(PurePosixPath(item.relative).name,
                             os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                             0o600, dir_fd=self._source_fd)
                pending = memoryview(item.body)
                while pending:
                    written = os.write(fd, pending)
                    if written <= 0:
                        raise OSError('write_failed')
                    pending = pending[written:]
                os.fsync(fd)
                os.fsync(self._source_fd)
                self._documents[item.trajectory_id] = item
                self._file_ids[item.trajectory_id] = _identity(fd)
                self._bytes += len(item.body)
            except BaseException:
                self._fenced = True
                raise
            finally:
                if fd is not None:
                    try:
                        os.close(fd)
                    except BaseException:
                        # Closure is uncertain; retain publication and never
                        # retry this descriptor or permit more insert/query work.
                        self._fenced = True
                        raise

    def query(self, query, query_image=None):
        """Native embedded automatic refresh, no raw-question diagnostics."""
        _text(query, QUERY_CHARS, nonempty=True, multiline=True)
        if query_image is not None:
            _text(query_image, 4096, nonempty=True)
        with self._lock:
            self._ready()
            if self._queries >= MAX_QUERIES:
                _refuse('query_bound')
            self._queries += 1
            started = time.monotonic()
            self._last = {'profile': PROFILE, 'state': 'failed', 'queries': self._queries,
                          'trajectories': len(self._documents), 'projection_bytes': self._bytes,
                          'context_bytes': 0, 'context_chars': 0, 'images_ingested': 0,
                          'query_image_ignored': query_image is not None,
                          'provider_tokens': None, 'provider_tokens_complete': False}
            conn, primary_error = None, None
            try:
                self._custody()
                prompt = self.native.sanitize_automatic_query(query)
                if not prompt:
                    self._last['state'] = 'sanitized_empty'
                    return []
                self.native.reject_secrets('Task Capsule', [prompt])
                self.native.reject_capsule_privacy('Task Capsule request', [prompt])
                mode = self.native.configured_mode(self.repository)
                gate = self.native.configured_retrieval_gate(self.repository)
                conn = self.native.connect(self.repository / 'memory-bank/local/context.db')
                _, indexed, warnings = self.native.refresh_layers(conn, self.repository)
                capsule = self.native.assemble_capsule(
                    conn, self.repository, mode=mode,
                    query=prompt if warnings else self.native.distill_capsule_query(conn, prompt),
                    task_id='BENCH-%04d' % self._queries, limit=3, ephemeral=True,
                    refresh_index=False, query_source='prompt',
                    phase_seconds=indexed.get('phase_seconds'), index_warnings=warnings,
                    gate_mode=gate,
                    host='cli', entry_point='refresh', allow_unprovisioned=True,
                    render=True, prompt=prompt, automatic_links=True)
                trace = {}
                body = '\n'.join(self.native.render_capsule_lines(
                    capsule, self.native.capsule_excerpts(conn, capsule), trace=trace))
                self.native.record_render_trace(self.repository, capsule, trace)
                raw = body.encode('utf-8')
                if len(raw) > CONTEXT_BYTES:
                    _refuse('context_bound')
                evidence = bool(capsule.get('semantic') or capsule.get('episodic'))
                self._last['state'] = 'refresh_failed' if warnings else 'delivered' if evidence else 'no_evidence'
                if warnings or not evidence:
                    return []
                self._last.update(context_bytes=len(raw), context_chars=len(body))
                return [{'type': 'text', 'value': body}]
            except BaseException as error:
                primary_error = error
                raise
            finally:
                try:
                    if conn is not None:
                        conn.close()
                except BaseException as error:
                    primary_error = error
                    # A return interrupted by cleanup did not deliver context.
                    self._last.update(state='cleanup_failed', context_bytes=0, context_chars=0)
                    raise
                finally:
                    elapsed, clock_error = None, None
                    try:
                        ended = time.monotonic()
                        if ((type(started) is int or type(started) is float)
                                and (type(ended) is int or type(ended) is float)
                                and math.isfinite(started) and math.isfinite(ended)
                                and started >= 0 and ended >= 0):
                            value = ended - started
                            if math.isfinite(value) and value >= 0:
                                elapsed = value
                    except BaseException as error:
                        clock_error = error
                    self._last['elapsed_seconds'] = elapsed
                    # Diagnostic failures never replace an existing task/cleanup
                    # primary. Propagate a new abort only outside its except, so
                    # no diagnostic error becomes the primary's context.
                    if (primary_error is None and clock_error is not None
                            and not issubclass(type(clock_error), Exception)):
                        self._last.update(state='failed', context_bytes=0, context_chars=0)
                        raise clock_error

    def post_query_hook(self, **_):
        """No callback dictionaries retained; own fixed numeric metadata only."""
        with self._lock:
            return dict(self._last) if self._last is not None else {'profile': PROFILE, 'state': 'not_queried'}

    def close(self):
        with self._lock:
            if not self._closed:
                self._closed = True
                error = self._drain_owned_fds()
                if error is not None:
                    raise error
