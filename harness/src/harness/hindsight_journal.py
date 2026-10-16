"""Private offline ingestion ownership and supplied document correspondence.

No transport, automatic retry, authentication, discovery or default wiring.
The caller owns a dedicated secure file-backed SQLite connection and sidecars.
All remote observations/settlements are caller supplied, never authority proof.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass, field
import hashlib
import json
import math
import re
import sqlite3
import threading
import uuid

from . import hindsight_export as export
from .hindsight_recall import Catalogue, Scope, SourceBinding

PROFILE = 'hindsight-ingestion-journal-v1'
QUIESCENT = 'caller-attested-operation-quiescent'
VISIBLE = 'caller-attested-full-visibility-and-quiescence'
MAX_GENERATIONS = 16
MAX_INTENTS = MAX_ATTEMPTS = 512
MAX_PAYLOAD = 8388608
MAX_METADATA = 65536
MAX_RESPONSE = 262144
MAX_UNION = 64
_DIGEST = re.compile(r'[a-f0-9]{64}')
_OUTCOMES = ('recorded', 'success', 'definite_no_write', 'settled_unknown')
_SCHEMA = (
    "CREATE TABLE journal (singleton INTEGER PRIMARY KEY CHECK(singleton=1), "
    "profile TEXT NOT NULL, journal_id TEXT NOT NULL, team TEXT NOT NULL, project TEXT NOT NULL, "
    "head INTEGER NOT NULL CHECK(head>=0 AND head<=16))",
    "CREATE TABLE generations (revision INTEGER PRIMARY KEY CHECK(revision>=1 AND revision<=16), "
    "generation TEXT NOT NULL UNIQUE, manifest BLOB NOT NULL CHECK(length(manifest)<=65536), "
    "digest TEXT NOT NULL CHECK(length(digest)=64))",
    "CREATE TABLE intents (identity TEXT PRIMARY KEY NOT NULL, revision INTEGER NOT NULL "
    "REFERENCES generations(revision), ordinal INTEGER NOT NULL CHECK(ordinal>=0 AND ordinal<32), "
    "binding BLOB NOT NULL CHECK(length(binding)<=2048), logical TEXT NOT NULL, "
    "body BLOB NOT NULL CHECK(length(body)>0 AND length(body)<=262144), "
    "digest TEXT NOT NULL CHECK(length(digest)=64), UNIQUE(revision,ordinal))",
    "CREATE TABLE attempts (nonce TEXT PRIMARY KEY NOT NULL, intent TEXT NOT NULL UNIQUE "
    "REFERENCES intents(identity), outcome TEXT NOT NULL "
    "CHECK(outcome IN ('recorded','success','definite_no_write','settled_unknown')))"
)
_TABLES = ('journal', 'generations', 'intents', 'attempts')


class JournalError(ValueError):
    """Fixed local codes only; exception/response/source prose is never retained."""
    def __init__(self, reason):
        reasons = ('profile', 'identity', 'schema', 'corrupt', 'capacity', 'conflict',
                   'pending', 'admitted', 'ticket', 'challenge', 'observation',
                   'settlement', 'storage', 'fenced', 'transaction', 'input')
        self.reason = reason if reason in reasons else 'input'
        super().__init__(self.reason)


def _json(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode('utf-8')


def _sha(raw):
    return hashlib.sha256(raw).hexdigest()


def _uuid(value):
    try:
        if type(value) is not str or str(uuid.UUID(value)) != value:
            raise JournalError('identity')
    except (ValueError, AttributeError):
        raise JournalError('identity') from None
    return value


def _integer(value, limit):
    if type(value) is not int or not 0 <= value <= limit:
        raise JournalError('input')


def _current_active(active):
    """Current cooperative admission only, outside history's corruption guard."""
    try:
        export._active(active)
    except ValueError:
        raise JournalError('input') from None


def _decode(raw, limit):
    if type(raw) is not bytes or not 0 < len(raw) <= limit:
        raise JournalError('input')
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise JournalError('input')
            result[key] = value
        return result
    def nonfinite(_):
        raise JournalError('input')
    try:
        result = json.loads(raw.decode('utf-8', 'strict'), object_pairs_hook=pairs,
                            parse_constant=nonfinite)
        todo, nodes = [(result, 0)], 0
        while todo:
            item, depth = todo.pop()
            nodes += 1
            if depth > 16 or nodes > 4096:
                raise JournalError('input')
            if type(item) is dict:
                todo.extend((x, depth + 1) for x in item.values())
                todo.extend((x, depth + 1) for x in item.keys())
            elif type(item) is list:
                todo.extend((x, depth + 1) for x in item)
            elif type(item) is str:
                item.encode('utf-8', 'strict')
            elif type(item) is float and not math.isfinite(item):
                raise JournalError('input')
        return result
    except (ValueError, UnicodeError, RecursionError, OverflowError):
        raise JournalError('input') from None


def _binding_image(binding, scope):
    try:
        export._binding_integrity(binding, scope)
        binding.__post_init__()
        scope.__post_init__()
    except (ValueError, TypeError, AttributeError):
        raise JournalError('input') from None
    return {'document_id': binding.document_id, 'path': binding.path,
            'source_sha256': binding.source_sha256}


def _from_image(value, scope):
    if type(value) is not dict or set(value) != {'document_id', 'path', 'source_sha256'}:
        raise JournalError('corrupt')
    try:
        binding = SourceBinding(scope, value['document_id'], value['path'], value['source_sha256'])
        _binding_image(binding, scope)
        return binding
    except ValueError:
        raise JournalError('corrupt') from None


def _intent_id(journal_id, generation, draft):
    return 'intent-' + _sha(_json([PROFILE, journal_id, generation,
                                  draft.binding.document_id, _sha(draft.body)]))


def _descriptor(journal_id, generation, draft):
    return {**_binding_image(draft.binding, draft.binding.scope), 'source_id': draft.source_id,
            'body_sha256': _sha(draft.body), 'body_bytes': len(draft.body),
            'intent_id': _intent_id(journal_id, generation, draft)}


@dataclass(frozen=True)
class Prepared:
    revision: int
    generation: str
    manifest: bytes = field(repr=False)
    intent_ids: tuple[str, ...]

    def __post_init__(self):
        _integer(self.revision, MAX_GENERATIONS)
        _uuid(self.generation)
        if (type(self.manifest) is not bytes or not 0 < len(self.manifest) <= MAX_METADATA
                or type(self.intent_ids) is not tuple or len(self.intent_ids) > 32
                or any(type(x) is not str or not re.fullmatch(r'intent-[a-f0-9]{64}', x) for x in self.intent_ids)):
            raise JournalError('input')


@dataclass(frozen=True)
class Ticket:
    journal_id: str
    nonce: str
    intent_id: str
    revision: int
    body_sha256: str

    def __post_init__(self):
        _uuid(self.journal_id)
        _uuid(self.nonce)
        _integer(self.revision, MAX_GENERATIONS)
        if (type(self.intent_id) is not str or not re.fullmatch(r'intent-[a-f0-9]{64}', self.intent_id)
                or type(self.body_sha256) is not str or not _DIGEST.fullmatch(self.body_sha256)):
            raise JournalError('ticket')


@dataclass(frozen=True)
class Challenge:
    journal_id: str
    nonce: str
    instance_id: str
    revision: int
    generation: str
    manifest_sha256: str
    frontier_sha256: str
    expected: tuple[SourceBinding, ...] = field(repr=False)

    def __post_init__(self):
        _uuid(self.journal_id)
        _uuid(self.nonce)
        _uuid(self.generation)
        _uuid(self.instance_id)
        _integer(self.revision, MAX_GENERATIONS)
        if (any(type(x) is not str or not _DIGEST.fullmatch(x) for x in
                (self.manifest_sha256, self.frontier_sha256))
                or type(self.expected) is not tuple or len(self.expected) > MAX_UNION
                or any(type(x) is not SourceBinding for x in self.expected)
                or len({x.document_id for x in self.expected}) != len(self.expected)):
            raise JournalError('challenge')
        for binding in self.expected:
            _binding_image(binding, binding.scope)


@dataclass(frozen=True)
class DocumentObservation:
    binding: SourceBinding
    state: str

    def __post_init__(self):
        if (type(self.binding) is not SourceBinding or type(self.state) is not str
                or self.state not in ('source_match', 'observed_not_found', 'unknown', 'conflict')):
            raise JournalError('observation')
        _binding_image(self.binding, self.binding.scope)


def observe_document(binding, status, body, *, eof):
    """Parse a supplied GET image, retaining no body and trusting no server hash.

    Even a complete HTTP404 may hide a document outside the credential tag scope.
    This projection compares source bytes and our metadata/tags only, not every
    retain option, server setting, extraction effect or native source eligibility.
    """
    if type(binding) is not SourceBinding:
        raise JournalError('input')
    _binding_image(binding, binding.scope)
    def result(state):
        return DocumentObservation(binding, state)
    if (type(status) is not int or type(body) is not bytes or len(body) > MAX_RESPONSE
            or eof is not True):
        return result('unknown')
    if status == 404:
        return result('observed_not_found')
    if status != 200:
        return result('unknown')
    try:
        value = _decode(body, MAX_RESPONSE)
        if type(value) is not dict:
            return result('unknown')
        required = ('id', 'bank_id', 'original_text', 'document_metadata', 'retain_params', 'tags')
        if any(value.get(k) is None for k in required):
            return result('unknown')
        if (any(type(value[k]) is not str for k in ('id', 'bank_id', 'original_text'))
                or type(value['document_metadata']) is not dict
                or type(value['retain_params']) is not dict or type(value['tags']) is not list
                or any(type(x) is not str for x in value['tags'])):
            return result('unknown')
        meta = value['document_metadata']
        params = value['retain_params'].get('metadata')
        wanted = {'harness_scope': binding.scope.key, 'source_sha256': binding.source_sha256,
                  'source_id': 'logical-' + _sha(_json([binding.scope.key, binding.path])),
                  'export_profile': export.PROFILE}
        if type(params) is not dict or any(k not in meta or k not in params for k in wanted):
            return result('unknown')
        if (value['id'] != binding.document_id or value['bank_id'] != binding.scope.bank_id
                or _sha(value['original_text'].encode('utf-8')) != binding.source_sha256
                or value['tags'] != [binding.scope.tag]
                or any(meta[k] != v or params[k] != v for k, v in wanted.items())):
            return result('conflict')
        return result('source_match')
    except (ValueError, UnicodeError):
        return result('unknown')


class Journal:
    """Use create_new/open_existing; no auto-create/reset or background execution.

    Connection and filesystem custody are trusted caller prerequisites. All
    methods serialize their own calls; the caller must not concurrently use the
    same connection. Other Journal connections coordinate with BEGIN IMMEDIATE.
    """
    def __init__(self, connection, scope, journal_id):
        if type(scope) is not Scope:
            raise JournalError('identity')
        try:
            scope.__post_init__()
        except ValueError:
            raise JournalError('identity') from None
        self._db, self.scope, self.journal_id = connection, scope, _uuid(journal_id)
        self._fenced = False
        self._lock = threading.RLock()
        self._issued, self._used = {}, set()
        self._epoch = str(uuid.uuid4())

    def _profile(self):
        db = self._db
        if self._fenced:
            raise JournalError('fenced')
        if type(db) is not sqlite3.Connection:
            raise JournalError('profile')
        if db.in_transaction:
            raise JournalError('transaction')
        if db.row_factory is not None or db.text_factory is not str:
            raise JournalError('profile')
        images = db.execute('PRAGMA database_list').fetchall()
        mains = [x for x in images if x[1] == 'main']
        if (len(mains) != 1 or not mains[0][2] or any(x[1] not in ('main', 'temp') for x in images)
                or db.execute('SELECT COUNT(*) FROM sqlite_temp_master').fetchone()[0] != 0):
            raise JournalError('profile')
        mode = db.execute('PRAGMA journal_mode').fetchone()[0]
        sync = db.execute('PRAGMA synchronous').fetchone()[0]
        wait = db.execute('PRAGMA busy_timeout').fetchone()[0]
        if (not ((mode == 'wal' and sync in (2, 3)) or (mode == 'delete' and sync == 3))
                or not 0 <= wait <= 1000
                or db.execute('PRAGMA ignore_check_constraints').fetchone()[0] != 0):
            raise JournalError('profile')

    @contextmanager
    def _transaction(self):
        with self._lock:
            begun = False
            try:
                self._profile()
                self._db.execute('BEGIN IMMEDIATE')
                begun = True
                yield
                # Explicit SQL also works on Python 3.14 autocommit=True.
                try:
                    self._db.execute('COMMIT')
                except BaseException as error:
                    self._fenced = True
                    if isinstance(error, sqlite3.Error):
                        raise JournalError('storage') from None
                    raise
                begun = False
            except sqlite3.Error:
                raise JournalError('storage') from None
            finally:
                if begun and self._db.in_transaction:
                    try:
                        self._db.execute('ROLLBACK')
                    except sqlite3.Error:
                        self._fenced = True

    def _schema(self):
        if self._db.execute("SELECT COUNT(*) FROM sqlite_master WHERE substr(name,1,7)!='sqlite_'").fetchone()[0] != 4:
            raise JournalError('schema')
        if self._db.execute("SELECT COUNT(*) FROM sqlite_master WHERE substr(name,1,7)!='sqlite_' AND (typeof(name)!='text' OR typeof(sql)!='text' OR length(CAST(name AS BLOB))>32 OR length(CAST(sql AS BLOB))>2048)").fetchone()[0]:
            raise JournalError('schema')
        actual = self._db.execute("SELECT name,sql FROM sqlite_master WHERE substr(name,1,7)!='sqlite_' ORDER BY name").fetchall()
        expected = sorted(zip(_TABLES, _SCHEMA))
        if actual != expected:
            raise JournalError('schema')

    def _load(self):
        """Validate bounded whole history before using any row as ownership."""
        self._schema()
        if self._db.execute('SELECT COUNT(*) FROM journal').fetchone()[0] != 1:
            raise JournalError('identity')
        if self._db.execute("SELECT COUNT(*) FROM journal WHERE typeof(head)!='integer' OR typeof(profile)!='text' OR typeof(journal_id)!='text' OR typeof(team)!='text' OR typeof(project)!='text' OR length(CAST(profile AS BLOB))>64 OR length(CAST(journal_id AS BLOB))!=36 OR length(CAST(team AS BLOB))!=36 OR length(CAST(project AS BLOB))!=36").fetchone()[0]:
            raise JournalError('identity')
        meta = self._db.execute('SELECT * FROM journal').fetchall()
        if len(meta) != 1 or meta[0][:5] != (1, PROFILE, self.journal_id, self.scope.team_id, self.scope.project_id):
            raise JournalError('identity')
        head = meta[0][5]
        if type(head) is not int or not 0 <= head <= MAX_GENERATIONS:
            raise JournalError('corrupt')
        for table, maximum in (('generations', MAX_GENERATIONS), ('intents', MAX_INTENTS), ('attempts', MAX_ATTEMPTS)):
            if self._db.execute('SELECT COUNT(*) FROM ' + table).fetchone()[0] > maximum:
                raise JournalError('capacity')
        # Admission before materializing untrusted stored BLOBs or TEXT fields.
        if self._db.execute("SELECT COUNT(*) FROM generations WHERE typeof(manifest)!='blob' OR length(manifest)>65536 OR typeof(generation)!='text' OR typeof(digest)!='text' OR length(CAST(generation AS BLOB))!=36 OR length(CAST(digest AS BLOB))!=64").fetchone()[0]:
            raise JournalError('corrupt')
        if self._db.execute("SELECT COUNT(*) FROM intents WHERE typeof(revision)!='integer' OR typeof(ordinal)!='integer' OR typeof(body)!='blob' OR typeof(binding)!='blob' OR length(body)>262144 OR length(binding)>2048 OR typeof(identity)!='text' OR typeof(logical)!='text' OR typeof(digest)!='text' OR length(CAST(identity AS BLOB))!=71 OR length(CAST(logical AS BLOB))!=72 OR length(CAST(digest AS BLOB))!=64").fetchone()[0]:
            raise JournalError('corrupt')
        if self._db.execute("SELECT COUNT(*) FROM attempts WHERE typeof(nonce)!='text' OR typeof(intent)!='text' OR typeof(outcome)!='text' OR length(CAST(nonce AS BLOB))!=36 OR length(CAST(intent AS BLOB))!=71 OR length(CAST(outcome AS BLOB))>17").fetchone()[0]:
            raise JournalError('corrupt')
        size = self._db.execute('SELECT COALESCE(SUM(length(body)+length(binding)),0) FROM intents').fetchone()[0]
        size += self._db.execute('SELECT COALESCE(SUM(length(manifest)),0) FROM generations').fetchone()[0]
        if size > MAX_PAYLOAD:
            raise JournalError('capacity')
        generations = self._db.execute('SELECT * FROM generations ORDER BY revision').fetchall()
        intents = self._db.execute('SELECT * FROM intents ORDER BY revision,ordinal').fetchall()
        attempts = self._db.execute('SELECT * FROM attempts ORDER BY nonce').fetchall()
        if [row[0] for row in generations] != list(range(1, head + 1)):
            raise JournalError('corrupt')
        parsed, owned = {}, {}
        try:
            for rev, generation, raw, digest in generations:
                _uuid(generation)
                manifest = _decode(raw, MAX_METADATA)
                keys = {'profile', 'journal_id', 'scope', 'generation', 'revision', 'export_sha256', 'current', 'previous'}
                if (type(manifest) is not dict or set(manifest) != keys or _json(manifest) != raw
                        or _sha(raw) != digest or manifest['profile'] != PROFILE
                        or manifest['journal_id'] != self.journal_id or manifest['scope'] != self.scope.key
                        or type(manifest['revision']) is not int or manifest['revision'] != rev
                        or manifest['generation'] != generation
                        or type(manifest['export_sha256']) is not str or not _DIGEST.fullmatch(manifest['export_sha256'])
                        or type(manifest['current']) is not list or len(manifest['current']) > 32
                        or type(manifest['previous']) is not list or len(manifest['previous']) > 32):
                    raise JournalError('corrupt')
                prior = tuple(_from_image(x, self.scope) for x in manifest['previous'])
                Catalogue(self.scope, prior)
                if len({x.path for x in prior}) != len(prior):
                    raise JournalError('corrupt')
                parsed[rev] = (generation, raw, manifest)
            by_revision = {rev: [] for rev in parsed}
            for identity, rev, ordinal, raw_binding, logical, body, digest in intents:
                if rev not in parsed or type(ordinal) is not int or ordinal != len(by_revision[rev]):
                    raise JournalError('corrupt')
                binding = _from_image(_decode(raw_binding, 2048), self.scope)
                if _json(_binding_image(binding, self.scope)) != raw_binding or digest != _sha(body):
                    raise JournalError('corrupt')
                draft = export._historical_draft(binding, logical, body)
                desc = _descriptor(self.journal_id, parsed[rev][0], draft)
                if identity != desc['intent_id']:
                    raise JournalError('corrupt')
                owned[identity] = (rev, draft)
                by_revision[rev].append(desc)
            for rev, (_, _, manifest) in parsed.items():
                if manifest['current'] != by_revision[rev]:
                    raise JournalError('corrupt')
                drafts = tuple(draft for row_rev, draft in owned.values() if row_rev == rev)
                catalogue = Catalogue(self.scope, tuple(d.binding for d in drafts))
                export._drafts(catalogue, drafts, integrity_only=True)
                if _sha(export._packet(catalogue, drafts)) != manifest['export_sha256']:
                    raise JournalError('corrupt')
            seen = set()
            for nonce, intent, outcome in attempts:
                _uuid(nonce)
                if intent not in owned or intent in seen or type(outcome) is not str or outcome not in _OUTCOMES:
                    raise JournalError('corrupt')
                seen.add(intent)
        except (ValueError, TypeError, KeyError, UnicodeError, AttributeError):
            raise JournalError('corrupt') from None
        return head, parsed, owned, attempts, size

    def prepare(self, selection, generation, expected_revision, *, previous_owned=None,
                active=export._always_active):
        """Persist exact bodies/desired head atomically; input replay is not upload."""
        _uuid(generation)
        _integer(expected_revision, MAX_GENERATIONS)
        try:
            if type(selection) is not export.PublicExport or selection.catalogue.scope != self.scope:
                raise JournalError('input')
            _current_active(active)
            selection.__post_init__(active=active)
            prior = ()
            if previous_owned is not None:
                if type(previous_owned) is not Catalogue or previous_owned.scope != self.scope:
                    raise JournalError('input')
                previous_owned.__post_init__()
                prior = previous_owned.sources
            previous = [_binding_image(b, self.scope) for b in prior]
            if len({b.path for b in prior}) != len(prior):
                raise JournalError('input')
        except ValueError:
            raise JournalError('input') from None
        with self._transaction():
            _current_active(active)
            head, rows, owned, attempts, size = self._load()
            _current_active(active)
            replay = head and rows[head][0] == generation
            revision = head if replay else head + 1
            manifest = _json({'profile': PROFILE, 'journal_id': self.journal_id, 'scope': self.scope.key,
                              'generation': generation, 'revision': revision,
                              'export_sha256': _sha(selection.serialized),
                              'current': [_descriptor(self.journal_id, generation, d) for d in selection.retain_drafts],
                              'previous': previous})
            if len(manifest) > MAX_METADATA:
                raise JournalError('capacity')
            if replay:
                if expected_revision != head or manifest != rows[head][1]:
                    raise JournalError('conflict')
            else:
                if expected_revision != head or any(row[0] == generation for row in rows.values()):
                    raise JournalError('conflict')
                added = len(manifest) + sum(len(d.body) + len(_json(_binding_image(d.binding, self.scope))) for d in selection.retain_drafts)
                if revision > MAX_GENERATIONS or len(owned) + len(selection.retain_drafts) > MAX_INTENTS or size + added > MAX_PAYLOAD:
                    raise JournalError('capacity')
                self._db.execute('INSERT INTO generations VALUES (?,?,?,?)',
                                 (revision, generation, manifest, _sha(manifest)))
                for ordinal, draft in enumerate(selection.retain_drafts):
                    self._db.execute('INSERT INTO intents VALUES (?,?,?,?,?,?,?)',
                                     (_intent_id(self.journal_id, generation, draft), revision, ordinal,
                                      _json(_binding_image(draft.binding, self.scope)), draft.source_id,
                                      draft.body, _sha(draft.body)))
                self._db.execute('UPDATE journal SET head=? WHERE singleton=1', (revision,))
            result = Prepared(revision, generation, manifest,
                              tuple(_intent_id(self.journal_id, generation, d) for d in selection.retain_drafts))
            _current_active(active)
        return result

    def current(self):
        with self._transaction():
            head, rows, _, _, _ = self._load()
            if not head:
                return None
            generation, raw, manifest = rows[head]
            return Prepared(head, generation, raw, tuple(x['intent_id'] for x in manifest['current']))

    def request_body(self, intent_id, *, active=export._always_active):
        """Return only a revalidated frozen body; this does not admit transport."""
        if type(intent_id) is not str or len(intent_id) != 71:
            raise JournalError('input')
        with self._transaction():
            _current_active(active)
            _, _, owned, _, _ = self._load()
            _current_active(active)
            if intent_id not in owned:
                raise JournalError('input')
            draft = owned[intent_id][1]
            try:
                draft.__post_init__(active=active)
            except ValueError:
                raise JournalError('input') from None
            _current_active(active)
            return draft.body

    def reserve_attempt(self, intent_id, nonce, *, expected_revision,
                        active=export._always_active):
        _uuid(nonce)
        _integer(expected_revision, MAX_GENERATIONS)
        if type(intent_id) is not str or len(intent_id) != 71:
            raise JournalError('input')
        with self._transaction():
            _current_active(active)
            head, _, owned, attempts, _ = self._load()
            _current_active(active)
            if expected_revision != head or intent_id not in owned or owned[intent_id][0] != head:
                raise JournalError('conflict')
            if any(n == nonce or identity == intent_id for n, identity, _ in attempts):
                raise JournalError('admitted')
            rev, draft = owned[intent_id]
            if any(outcome == 'recorded' and owned[identity][1].source_id == draft.source_id
                   for _, identity, outcome in attempts):
                raise JournalError('pending')
            if len(attempts) >= MAX_ATTEMPTS:
                raise JournalError('capacity')
            try:
                draft.__post_init__(active=active)
            except ValueError:
                raise JournalError('input') from None
            self._db.execute('INSERT INTO attempts VALUES (?,?,?)', (nonce, intent_id, 'recorded'))
            ticket = Ticket(self.journal_id, nonce, intent_id, rev, _sha(draft.body))
            _current_active(active)
        return ticket  # Only after explicit COMMIT succeeds.

    def attest_terminal(self, ticket, outcome, *, profile):
        if type(profile) is not str or profile != QUIESCENT or type(outcome) is not str or outcome not in _OUTCOMES[1:]:
            raise JournalError('settlement')
        if type(ticket) is not Ticket or ticket.journal_id != self.journal_id:
            raise JournalError('ticket')
        ticket.__post_init__()
        with self._transaction():
            _, _, owned, attempts, _ = self._load()
            found = next((row for row in attempts if row[0] == ticket.nonce), None)
            if found is None or ticket.intent_id not in owned:
                raise JournalError('ticket')
            rev, draft = owned[ticket.intent_id]
            if ticket != Ticket(self.journal_id, found[0], found[1], rev, _sha(draft.body)):
                raise JournalError('ticket')
            if found[2] not in ('recorded', outcome):
                raise JournalError('conflict')
            self._db.execute('UPDATE attempts SET outcome=? WHERE nonce=?', (outcome, ticket.nonce))
        # Does not modify desired head or a recall Catalogue.

    def _challenge(self, nonce, state):
        head, rows, owned, attempts, _ = state
        if not head:
            raise JournalError('challenge')
        generation, raw, manifest = rows[head]
        expected = {}
        images = manifest['current'] + manifest['previous']
        for image in images:
            binding = _from_image({k: image[k] for k in ('document_id', 'path', 'source_sha256')}, self.scope)
            expected[binding.document_id] = binding
        for _, identity, _ in attempts:
            binding = owned[identity][1].binding
            expected[binding.document_id] = binding
        if len(expected) > MAX_UNION:
            raise JournalError('capacity')
        frontier = _json([{'nonce': n, 'intent_id': i, 'outcome': o} for n, i, o in attempts])
        return Challenge(self.journal_id, nonce, self._epoch, head, generation, _sha(raw), _sha(frontier),
                         tuple(expected[k] for k in sorted(expected)))

    def challenge(self, nonce):
        _uuid(nonce)
        with self._lock:
            with self._transaction():
                state = self._load()
                if nonce in self._issued or nonce in self._used or len(self._issued) + len(self._used) >= 32:
                    raise JournalError('challenge')
                challenge = self._challenge(nonce, state)
            self._issued[nonce] = challenge
        return challenge

    def evaluate(self, challenge, observations, *, profile=None):
        """Consume a local challenge and return suggestions authorizing no work."""
        if type(challenge) is not Challenge or type(challenge.nonce) is not str:
            raise JournalError('challenge')
        challenge.__post_init__()
        with self._lock:
            if self._issued.get(challenge.nonce) != challenge:
                raise JournalError('challenge')
            del self._issued[challenge.nonce]
            self._used.add(challenge.nonce)
            with self._transaction():
                state = self._load()
                if challenge != self._challenge(challenge.nonce, state):
                    raise JournalError('challenge')
                if (type(observations) is not tuple or len(observations) != len(challenge.expected)
                        or any(type(x) is not DocumentObservation for x in observations)):
                    raise JournalError('observation')
                for observation in observations:
                    observation.__post_init__()
                supplied = {x.binding.document_id: x for x in observations}
                if (len(supplied) != len(observations) or set(supplied) != {b.document_id for b in challenge.expected}
                        or any(supplied[b.document_id].binding != b for b in challenge.expected)):
                    raise JournalError('observation')
                head, rows, _, attempts, _ = state
                current = {x['document_id'] for x in rows[head][2]['current']}
                missing = sum(x.state == 'observed_not_found' and k in current for k, x in supplied.items())
                obsolete = sum(x.state == 'source_match' and k not in current for k, x in supplied.items())
                unknown = sum(x.state == 'unknown' for x in supplied.values())
                conflict = sum(x.state == 'conflict' for x in supplied.values())
                pending = sum(outcome == 'recorded' for _, _, outcome in attempts)
                decision = 'incomplete'
                if not (unknown or conflict or pending) and type(profile) is str and profile == VISIBLE:
                    decision = 'cleanup_required' if obsolete else ('missing_source_images' if missing else 'supplied_selection_current')
                return {'state': decision, 'expected_count': len(supplied), 'missing_current': missing,
                        'obsolete_present': obsolete, 'unknown': unknown, 'conflict': conflict,
                        'pending_attempts': pending, 'authorized_mutations': 0, 'actual_remote_noop': None,
                        'executed_transport_calls': 0, 'physical_sqlite_io_bytes': None,
                        'provider_tokens': None, 'remote_usd': None, 'total_cost_complete': False}

    def diagnostics(self):
        with self._transaction():
            head, _, owned, attempts, size = self._load()
            return {'profile': PROFILE, 'revision': head, 'generations': head, 'intents': len(owned),
                    'recorded_attempts': len(attempts), 'pending_attempts': sum(o == 'recorded' for _, _, o in attempts),
                    'persisted_payload_bytes': size, 'physical_sqlite_io_bytes': None,
                    'executed_transport_calls': 0, 'provider_tokens': None, 'remote_usd': None,
                    'total_cost_complete': False}


def create_new(connection, scope, journal_id):
    journal = Journal(connection, scope, journal_id)
    with journal._transaction():
        if connection.execute('SELECT COUNT(*) FROM sqlite_master').fetchone()[0]:
            raise JournalError('schema')
        for sql in _SCHEMA:
            connection.execute(sql)
        connection.execute('INSERT INTO journal VALUES (1,?,?,?,?,0)',
                           (PROFILE, journal.journal_id, scope.team_id, scope.project_id))
        journal._load()
    return journal


def open_existing(connection, scope, journal_id):
    journal = Journal(connection, scope, journal_id)
    with journal._transaction():
        journal._load()
    return journal
