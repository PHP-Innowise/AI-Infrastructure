"""Offline, operator-attested public source export and ingestion drafts.

No service executor, credential handling, repository discovery or default wiring.
Approvals/registered IDs come from trusted code outside cloned configuration.
Matching bytes are not proof of native eligibility or remote authorization.
"""
from __future__ import annotations

from dataclasses import InitVar, dataclass, field
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import stat
import unicodedata

from .automatic_query import (_NOT_A_LITERAL, PRIVATE_PATTERNS, RAW_TEXT_PATTERN, SECRET_PATTERNS,
                              source_path_problem)
from .hindsight_recall import Catalogue, Scope, SourceBinding, SOURCE_BYTES, MAX_REFERENCES

PROFILE = 'hindsight-public-export-v1'
TOTAL_SOURCE_BYTES = 524288
RETAIN_BYTES = 262144
PACKET_BYTES = 1048576
_DIGEST = re.compile(r'[a-f0-9]{64}')
_SUFFIXES = frozenset(('.md', '.txt', '.py', '.php', '.ts', '.js', '.json',
                       '.yaml', '.yml', '.toml', '.sql'))
_PRIVATE_PARTS = frozenset(('task', 'tasks', 'memory-bank', 'project-brain', 'private',
                            'restricted', 'local', 'logs', 'log', 'artifacts',
                            'archive', 'tmp', 'temp'))
_REASONS = frozenset(('approval', 'source_path', 'source_bound', 'source_text',
                      'source_changed', 'private_content', 'file_kind', 'unsupported',
                      'unavailable', 'cancelled', 'request_bound', 'packet_bound',
                      'previous_catalogue'))
_IO_KEYS = ('source_read_bytes', 'source_read_calls', 'opened_descriptors',
            'closed_descriptors', 'close_errors')
# Export is more conservative than an ordinary automatic query. Quoted PHP/JSON
# keys and WordPress constants are literal credential carriers even when their
# value has no recognizable provider prefix. Do not alter the approved bytes.
# A key is any name ending in a credential word, whatever its prefix: snake
# (DB_PASSWORD), camel (dbPassword) or kebab (x-api-key).
_EXPORT_WORDS = (r'(?:password|passwd|passphrase|pass|pwd|secret(?:_access)?(?:_key|_token)?'
                 r'|api[_-]?(?:key|token)|access[_-]?(?:token|key)|auth[_-]?token|refresh[_-]?token'
                 r'|private[_-]?key|account[_-]?key|shared[_-]?access[_-]?key|client[_-]?secret'
                 r'|encryption[_-]?key|app[_-]?key|token|salt)')
_EXPORT_KEY = (r'(?:[A-Za-z0-9_-]*?' + _EXPORT_WORDS
               + r'|(?:SECURE_AUTH|LOGGED_IN|AUTH|NONCE)_(?:KEY|SALT))')
_COMPOSER_AUTH = r'(?:http-basic|github-oauth|gitlab-oauth|gitlab-token|bitbucket-oauth|bearer)'
_SENSITIVE_NAME = re.compile(r'(?:' + _EXPORT_KEY + '|' + _COMPOSER_AUTH + ')', re.IGNORECASE)
_QUOTED_CREDENTIAL = re.compile(
    r'(?P<quote>[\'"`])' + _EXPORT_KEY + r'(?P=quote)\s*(?:=>|:|\]\s*=)\s*[\'"`]',
    re.IGNORECASE)
_CONSTANT_CREDENTIAL = re.compile(
    r'\bdefine\s*\(\s*(?P<quote>[\'"])' + _EXPORT_KEY + r'(?P=quote)\s*,\s*[\'"]',
    re.IGNORECASE)
_LITERAL_CREDENTIAL = re.compile(
    r'(?<![A-Za-z0-9_])' + _EXPORT_KEY + r'\s*=\s*[\'"`]', re.IGNORECASE)
# An unquoted value in .env, ini or YAML form (API_TOKEN=..., pass: ...). A
# value is a run of four or more characters with a digit or symbol in it, or
# of eight or more, and not a placeholder or a reference. A shell `PWD=` is a
# directory, so `pwd` counts here only through the quoted forms above.
_ASSIGNED_CREDENTIAL = re.compile(
    r'(?<![A-Za-z0-9_$])(?:[A-Za-z0-9_-]*?(?!pwd\b)' + _EXPORT_WORDS + r')[ \t]*[:=][ \t]*' + _NOT_A_LITERAL
    + r'(?=[^\s\'"`,;]*[^A-Za-z\s\'"`,;]|[^\s\'"`,;]{8})[^\s\'"`,;]{4,}', re.IGNORECASE)
# A literal default for a credential variable: env('DB_PASSWORD', '...'),
# os.getenv('API_TOKEN', '...'). An empty default carries nothing.
_ENVIRONMENT_DEFAULT = re.compile(
    r'(?:\benv|getenv|environ\.get)\s*\(\s*(?P<name>[\'"])[A-Za-z0-9_]*?' + _EXPORT_WORDS
    + r'[A-Za-z0-9_]*(?P=name)\s*,\s*(?P<value>[\'"])(?!(?P=value))', re.IGNORECASE)
# SQL that sets a password: IDENTIFIED BY '...', PASSWORD '...'.
_SQL_CREDENTIAL = re.compile(
    r'\b(?:IDENTIFIED\s+(?:WITH\s+\w+\s+)?BY(?:\s+PASSWORD)?|PASSWORD)\s+[\'"]' + _NOT_A_LITERAL
    + r'[^\'"\s]', re.IGNORECASE)
_COMPOSER_BLOCK = re.compile(r'"' + _COMPOSER_AUTH + r'"\s*:\s*(?P<block>\S)', re.IGNORECASE)


def _always_active():
    return True


def _json(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'), allow_nan=False)


def _sha(raw):
    return hashlib.sha256(raw).hexdigest()


class ExportError(ValueError):
    """Fixed codes and count-only local observation; never source/error prose."""
    def __init__(self, reason, observation=None):
        self.reason = reason if reason in _REASONS else 'unavailable'
        supplied = observation if type(observation) is dict else {}
        self.observation = {key: supplied[key] for key in _IO_KEYS
                            if type(supplied.get(key)) is int and 0 <= supplied[key] <= 11000 * SOURCE_BYTES}
        super().__init__(self.reason)


def _openapi_object_members(view, start, end, *, active):
    """Fixed-path member spans after whole strict validation; character offsets."""
    decoder = json.JSONDecoder()
    cursor = start + 1
    while cursor < end:
        _active(active)
        while cursor < end and view[cursor] in ' \t\r\n':
            cursor += 1
        if cursor >= end or view[cursor] == '}':
            return
        key_start = cursor
        key, cursor = decoder.raw_decode(view, cursor)
        while cursor < end and view[cursor] in ' \t\r\n':
            cursor += 1
        if type(key) is not str or cursor >= end or view[cursor] != ':':
            raise ValueError('Invalid member position')
        cursor += 1
        while cursor < end and view[cursor] in ' \t\r\n':
            cursor += 1
        value_start = cursor
        _active(active)
        value, cursor = decoder.raw_decode(view, cursor)
        _active(active)
        yield key, key_start, value_start, cursor, value
        while cursor < end and view[cursor] in ' \t\r\n':
            cursor += 1
        if cursor < end and view[cursor] == ',':
            cursor += 1
        elif cursor >= end or view[cursor] == '}':
            return
        else:
            raise ValueError('Invalid member position')


def _openapi_bearer_spans(view, *, active=_always_active):
    """Finite original-JSON declaration subset; no source/authenticity claim."""
    _active(active)
    if not view.lstrip().startswith('{'):
        return set()
    try:
        if len(view.encode('utf-8', 'strict')) > SOURCE_BYTES:
            return set()
        # Bound structural work before the recursive stdlib decoder. The
        # existing forward scanner supplies string spans; only outside-string
        # delimiters affect depth. Tokens are strings plus nonspace characters.
        depth = tokens = inspected = end = 0
        spans = _json_string_spans(view, active=active)
        for start, stop, _escaped, opaque in (*spans, (len(view), len(view), False, False)):
            if opaque:
                return set()
            for character in view[end:start]:
                inspected += 1
                if inspected % 2048 == 0:
                    _active(active)
                if not character.isspace():
                    tokens += 1
                if character in '{[':
                    depth += 1
                elif character in '}]':
                    depth -= 1
                if not 0 <= depth <= 16 or tokens > 4096:
                    return set()
            if start < stop:
                tokens += 1
                if tokens > 4096:
                    return set()
            end = stop
        if depth:
            return set()
        def unique(pairs):
            value = {}
            for key, item in pairs:
                _active(active)
                if key in value:
                    raise ValueError('Duplicate member')
                value[key] = item
            return value
        def nonfinite(_value):
            raise ValueError('Nonfinite value')
        _active(active)
        root = json.loads(view, object_pairs_hook=unique, parse_constant=nonfinite)
        _active(active)
        if type(root) is not dict or root.get('openapi') != '3.1.1':
            return set()
        pending, nodes = [(root, 0)], 0
        while pending:
            _active(active)
            value, level = pending.pop()
            nodes += 1
            if nodes > 4096 or level > 16:
                return set()
            if type(value) is dict:
                pending.extend((child, level + 1) for child in value.values())
            elif type(value) is list:
                pending.extend((child, level + 1) for child in value)
            elif type(value) is float and not math.isfinite(value):
                return set()
            elif type(value) is str:
                value.encode('utf-8', 'strict')
        start = len(view) - len(view.lstrip())
        _value, end = json.JSONDecoder().raw_decode(view, start)
        approved = set()
        for key, _key_start, begin, stop, value in _openapi_object_members(view, start, end, active=active):
            if key != 'components' or type(value) is not dict:
                continue
            for key2, _key_start2, begin2, stop2, value2 in _openapi_object_members(view, begin, stop, active=active):
                if key2 != 'securitySchemes' or type(value2) is not dict:
                    continue
                for name, key_start3, begin3, stop3, leaf in _openapi_object_members(view, begin2, stop2, active=active):
                    if (name.casefold() == 'bearer' and type(leaf) is dict
                            and set(leaf) in ({'type', 'scheme'}, {'type', 'scheme', 'bearerFormat'})
                            and leaf.get('type') == 'http' and leaf.get('scheme') == 'bearer'
                            and ('bearerFormat' not in leaf or leaf['bearerFormat'] == 'JWT')):
                        approved.add((key_start3, begin3, stop3))
        _active(active)
        return approved
    except ExportError as error:
        if error.reason in ('cancelled', 'unavailable'):
            raise
        return set()
    except (ValueError, TypeError, UnicodeError, RecursionError):
        return set()


def _export_credentials(view, *, active=_always_active, original_json=False):
    for pattern in (_QUOTED_CREDENTIAL, _CONSTANT_CREDENTIAL, _LITERAL_CREDENTIAL,
                    _ASSIGNED_CREDENTIAL, _ENVIRONMENT_DEFAULT, _SQL_CREDENTIAL):
        _active(active)
        if pattern.search(view):
            raise ExportError('private_content')
    _active(active)
    approved = None
    for match in _COMPOSER_BLOCK.finditer(view):
        _active(active)
        try:
            block, _end = json.JSONDecoder().raw_decode(view, match.start('block'))
        except (ValueError, RecursionError):
            raise ExportError('private_content') from None
        if type(block) is not dict:
            raise ExportError('private_content')
        if block:
            if original_json:
                if approved is None:
                    approved = _openapi_bearer_spans(view, active=active)
                if (match.start(), match.start('block'), _end) in approved:
                    continue
            raise ExportError('private_content')
    _active(active)


# Sources whose double-quoted strings keep an unknown `\u` as literal text: a
# PHP namespace ("App\utils"), a Windows path in prose ("C:\users").
_LITERAL_BACKSLASH_U = frozenset(('.php', '.md', '.txt', '.sql'))
_CODE_POINT = re.compile(r'\\(?:(\\)|u\{([0-9A-Fa-f]{1,6})\})')
_HEX = frozenset('0123456789abcdefABCDEF')


def _literal_backslash_u(path):
    return PurePosixPath(path).suffix.casefold() in _LITERAL_BACKSLASH_U


def _json_span(span):
    """A string span as JSON: JavaScript/PHP `\\u{...}` code points become
    JSON escapes, so what they spell is screened like any other text."""
    if '\\u{' not in span:
        return span
    return _CODE_POINT.sub(lambda match: match.group(0) if match.group(1)
                           else json.dumps(chr(int(match.group(2), 16)))[1:-1], span)


def _json_string_spans(view, *, active=_always_active, lenient=False):
    """Forward span discovery only; regex/decoder work remains separate.

    A raw line break ends a quote that opened no string: neither JSON nor most
    languages put one inside a string, and an unmatched prose quote must not
    pair with the next line's. `lenient` keeps an unknown `\\u` opaque for
    the sources in _LITERAL_BACKSLASH_U and lets an unterminated quote end the
    text there.
    """
    inspected, next_poll = 0, 2048
    def character(index):
        nonlocal inspected, next_poll
        value = view[index]
        inspected += 1
        if inspected >= next_poll:
            _active(active)
            next_poll += 2048
        return value

    _active(active)
    index, size = 0, len(view)
    while index < size:
        if character(index) != '"':
            index += 1
            continue
        start = index
        index += 1
        escaped = opaque = known_escape = False
        high_surrogate = False
        while index < size:
            current = character(index)
            if current == '\n':
                break
            if high_surrogate and current != '\\':
                if not lenient:
                    raise ExportError('source_text')
                opaque, high_surrogate = True, False
            if current == '"':
                _active(active)
                yield start, index + 1, escaped, opaque
                index += 1
                break
            if current != '\\':
                index += 1
                continue
            escaped = True
            if index + 1 >= size:
                if lenient:
                    return
                raise ExportError('source_text')
            code = character(index + 1)
            if high_surrogate and code != 'u':
                if not lenient:
                    raise ExportError('source_text')
                opaque, high_surrogate = True, False
            if code == 'u' and index + 2 < size and character(index + 2) == '{' and not high_surrogate:
                # A JavaScript/PHP code point escape, decoded by _json_span.
                digits = ''
                for n in range(index + 3, min(index + 10, size)):
                    current = character(n)
                    if current == '}':
                        break
                    digits += current
                else:
                    n = -1
                point = int(digits, 16) if 0 < len(digits) <= 6 and set(digits) <= _HEX else -1
                if n != -1 and 0 <= point <= 0x10FFFF and not 0xD800 <= point <= 0xDFFF:
                    known_escape = True
                    index = n + 1
                    continue
                if not lenient:
                    raise ExportError('source_text')
                opaque = True
                index += 2
            elif code == 'u':
                digits = ''.join(character(n) for n in range(index + 2, min(index + 6, size)))
                unit = int(digits, 16) if len(digits) == 4 and set(digits) <= _HEX else -1
                if (unit < 0 or (high_surrogate and not 0xDC00 <= unit <= 0xDFFF)
                        or (not high_surrogate and 0xDC00 <= unit <= 0xDFFF)):
                    if not lenient:
                        raise ExportError('source_text')
                    opaque, high_surrogate = True, False
                    index += 2
                    continue
                known_escape = True
                high_surrogate = False if high_surrogate else 0xD800 <= unit <= 0xDBFF
                index += 6
            else:
                if code in '"\\/bfnrt':
                    known_escape = True
                else:
                    opaque = True
                index += 2
        else:
            _active(active)
            if (known_escape or high_surrogate) and not lenient:
                raise ExportError('source_text')
            return
    _active(active)


def _screen(text, *, active=_always_active, lenient=False):
    # Iterate decoded JSON string views, including COMPOSER_AUTH strings,
    # without changing a byte of the retained source. Admission is finite;
    # unknown excessive encoding/structure is not a successful public screen.
    pending = [(text, 0)]
    views, total = 0, 0
    while pending:
        _active(active)
        view, depth = pending.pop()
        view = view.replace('\r\n', '\n').replace('\r', '\n')
        views += 1
        try:
            total += len(view.encode('utf-8', 'strict'))
        except UnicodeError:
            raise ExportError('source_text') from None
        if views > 4096 or depth > 16 or total > 8 * SOURCE_BYTES:
            raise ExportError('source_text')
        for pattern in (*SECRET_PATTERNS.values(), *PRIVATE_PATTERNS.values(), RAW_TEXT_PATTERN):
            _active(active)
            if pattern.search(view):
                raise ExportError('private_content')
        # Raw credential refusal precedes any ambiguous encoded-span failure.
        _export_credentials(view, active=active, original_json=depth == 0)
        replacements, end = [], 0
        for start, stop, escaped, opaque in _json_string_spans(view, active=active,
                                                                lenient=lenient and depth == 0):
            if not escaped or opaque:
                continue
            _active(active)
            try:
                value = json.loads(_json_span(view[start:stop]))
                value.encode('utf-8', 'strict')
            except (ValueError, UnicodeError):
                raise ExportError('source_text') from None
            _active(active)
            if value != view[start + 1:stop - 1]:
                if len(pending) + views >= 4096:
                    raise ExportError('source_text')
                pending.append((value, depth + 1))
            if _SENSITIVE_NAME.fullmatch(value):
                replacements.extend((view[end:start], json.dumps(value)))
                end = stop
        if replacements:
            view = ''.join((*replacements, view[end:]))
        _export_credentials(view, active=active, original_json=depth == 0)
    _active(active)


def _source_path(name, *, active=_always_active):
    _active(active)
    if (type(name) is not str or not name or len(name) > 512
            or any(unicodedata.category(c).startswith('C') for c in name)):
        raise ExportError('source_path')
    if (len(name.encode('utf-8')) > 512 or '\\' in name or ':' in name or '#' in name
            or PurePosixPath(name).is_absolute() or str(PurePosixPath(name)) != name
            or any(part in ('', '.', '..') for part in name.split('/'))
            or any(part.startswith('.') or part.casefold() in _PRIVATE_PARTS for part in name.split('/'))
            or PurePosixPath(name).suffix.casefold() not in _SUFFIXES
            or source_path_problem(name)):
        raise ExportError('source_path')
    _screen(name, active=active)


@dataclass(frozen=True)
class PublicApproval:
    """Explicit approval of exact public bytes; not read from repo metadata."""
    scope: Scope
    path: str
    source_sha256: str
    classification: str = 'public'

    def __post_init__(self):
        if (type(self.scope) is not Scope or type(self.source_sha256) is not str
                or not _DIGEST.fullmatch(self.source_sha256)
                or type(self.classification) is not str or self.classification != 'public'):
            raise ExportError('approval')
        _source_path(self.path)


def _active(callback):
    try:
        ready = callback()
    except Exception:
        raise ExportError('unavailable') from None
    if ready is not True:
        raise ExportError('cancelled')


def _identity(info):
    return (info.st_dev, info.st_ino, info.st_mode, info.st_nlink,
            info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def _directory_identity(info):
    return (info.st_dev, info.st_ino, info.st_mode)


def _binding_integrity(binding, scope):
    """Historical correspondence, independent of today's export admission."""
    if type(binding) is not SourceBinding or type(scope) is not Scope or binding.scope != scope:
        raise ExportError('approval')
    try:
        scope.__post_init__()
        binding.__post_init__()
    except (ValueError, TypeError, AttributeError):
        raise ExportError('approval') from None
    expected = 'source-' + _sha(_json([scope.key, binding.path, binding.source_sha256]).encode())
    if binding.document_id != expected:
        raise ExportError('approval')


def _binding(binding, scope, *, active=_always_active):
    _active(active)
    _binding_integrity(binding, scope)
    _source_path(binding.path, active=active)


def _text_integrity(raw):
    if type(raw) is not bytes or len(raw) > SOURCE_BYTES:
        raise ExportError('source_bound')
    try:
        text = raw.decode('utf-8', 'strict')
    except UnicodeError:
        raise ExportError('source_text') from None
    if (not text.strip() or any(unicodedata.category(c).startswith('C')
                               and c not in '\r\n\t' for c in text)):
        raise ExportError('source_text')
    return text


def _text(raw, *, active=_always_active, lenient=False):
    _active(active)
    text = _text_integrity(raw)
    _screen(text, active=active, lenient=lenient)
    return text


def _request(binding, source_id, text):
    scope = binding.scope
    return {'items': [{'content': text, 'document_id': binding.document_id,
        'timestamp': 'unset', 'context': 'public canonical source',
        'metadata': {'harness_scope': scope.key, 'source_sha256': binding.source_sha256,
                     'source_id': source_id, 'export_profile': PROFILE},
        'tags': [scope.tag], 'update_mode': 'replace'}],
        'async': False, 'document_tags': [scope.tag]}


def _packet(catalogue, drafts):
    return _json({'schema_version': 1, 'profile': PROFILE, 'bank_id': catalogue.scope.bank_id,
                  'scope': catalogue.scope.key, 'sources': [
                      {'source_id': x.source_id, 'path': x.binding.path,
                       'document_id': x.binding.document_id, 'source_sha256': x.binding.source_sha256,
                       'classification': 'operator-attested-public', 'retain_request': json.loads(x.body)}
                      for x in drafts]}).encode('utf-8')


def _drafts(catalogue, drafts, *, integrity_only=False, active=_always_active):
    if (type(catalogue) is not Catalogue or type(drafts) is not tuple
            or len(drafts) != len(catalogue.sources) or len(drafts) > MAX_REFERENCES
            or any(type(x) is not RetainDraft for x in drafts)
            or tuple(x.binding for x in drafts) != catalogue.sources
            or len({x.path for x in catalogue.sources}) != len(catalogue.sources)):
        raise ExportError('approval')
    total = 0
    for draft in drafts:
        if not integrity_only:
            _active(active)
            draft.__post_init__(active=active)
        total += len(_draft_text(draft).encode('utf-8'))
    if total > TOTAL_SOURCE_BYTES:
        raise ExportError('source_bound')
    if not integrity_only:
        _active(active)
    return total


def _read(root, approval, active, io):
    if (os.name != 'posix' or not hasattr(os, 'O_NOFOLLOW') or not hasattr(os, 'O_DIRECTORY')
            or not hasattr(os, 'O_NONBLOCK') or os.open not in os.supports_dir_fd
            or os.stat not in os.supports_dir_fd or os.stat not in os.supports_follow_symlinks):
        raise ExportError('unsupported')
    descriptors, directories = [], []
    close_failed = False
    try:
        flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
        parent = os.open('/', flags)
        descriptors.append(parent)
        io['opened_descriptors'] += 1
        # Walk and retain *all* absolute parents; root parents may not be links.
        for part in root.parts[1:] + tuple(approval.path.split('/')[:-1]):
            _active(active)
            child = os.open(part, flags, dir_fd=parent)
            descriptors.append(child)
            io['opened_descriptors'] += 1
            directories.append((parent, part, child))
            parent = child
        leaf = approval.path.split('/')[-1]
        fd = os.open(leaf, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        descriptors.append(fd)
        io['opened_descriptors'] += 1
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
            raise ExportError('file_kind')
        if not 0 <= before.st_size <= SOURCE_BYTES:
            raise ExportError('source_bound')
        raw = bytearray()
        while len(raw) <= SOURCE_BYTES:
            _active(active)
            chunk = os.read(fd, min(8192, SOURCE_BYTES + 1 - len(raw)))
            io['source_read_calls'] += 1
            io['source_read_bytes'] += len(chunk)
            if not chunk:
                break
            raw.extend(chunk)
        after = os.fstat(fd)
        named = os.stat(leaf, dir_fd=parent, follow_symlinks=False)
        if (len(raw) > SOURCE_BYTES or _identity(before) != _identity(after)
                or _identity(after) != _identity(named) or _sha(raw) != approval.source_sha256):
            raise ExportError('source_changed')
        for owner, name, child in directories:
            if (_directory_identity(os.fstat(child))
                    != _directory_identity(os.stat(name, dir_fd=owner, follow_symlinks=False))):
                raise ExportError('source_changed')
        return bytes(raw)
    except OSError:
        raise ExportError('unavailable') from None
    finally:
        for fd in reversed(descriptors):
            try:
                os.close(fd)
                io['closed_descriptors'] += 1
            except OSError:
                io['close_errors'] += 1
                close_failed = True
        if close_failed:
            raise ExportError('unavailable') from None


def _draft_text(draft):
    """Validate the frozen request envelope without mutable privacy screening."""
    if type(draft) is not RetainDraft or type(draft.binding) is not SourceBinding:
        raise ExportError('approval')
    _binding_integrity(draft.binding, draft.binding.scope)
    expected_id = 'logical-' + _sha(_json([draft.binding.scope.key, draft.binding.path]).encode())
    if (type(draft.source_id) is not str or draft.source_id != expected_id
            or type(draft.body) is not bytes or not 0 < len(draft.body) <= RETAIN_BYTES):
        raise ExportError('approval')
    try:
        value = json.loads(draft.body.decode('utf-8', 'strict'))
        text = value['items'][0]['content']
        if type(text) is not str:
            raise ExportError('approval')
        raw = text.encode('utf-8', 'strict')
        _text_integrity(raw)
        if (_sha(raw) != draft.binding.source_sha256
                or draft.body != _json(_request(draft.binding, draft.source_id, text)).encode('utf-8')):
            raise ExportError('approval')
        return text
    except (KeyError, IndexError, TypeError, ValueError, UnicodeError, RecursionError):
        raise ExportError('approval') from None


def _historical_draft(binding, source_id, body):
    """Journal-only integrity image; public export/dispatch must re-admit it."""
    draft = object.__new__(RetainDraft)
    object.__setattr__(draft, 'binding', binding)
    object.__setattr__(draft, 'source_id', source_id)
    object.__setattr__(draft, 'body', body)
    _draft_text(draft)
    return draft


@dataclass(frozen=True)
class RetainDraft:
    binding: SourceBinding
    source_id: str
    body: bytes = field(repr=False)
    active: InitVar[object] = None

    def __post_init__(self, active=None):
        if active is None:
            active = _always_active
        if type(self.binding) is not SourceBinding:
            raise ExportError('approval')
        _binding(self.binding, self.binding.scope, active=active)
        text = _draft_text(self)
        try:
            _screen(text, active=active, lenient=_literal_backslash_u(self.binding.path))
        except ExportError as error:
            if error.reason in ('cancelled', 'unavailable'):
                raise
            raise ExportError('approval') from None
        except (ValueError, TypeError, UnicodeError, RecursionError):
            # Keep the public draft's original fixed text-refusal contract.
            raise ExportError('approval') from None

    @property
    def method(self):
        return 'POST'

    @property
    def resource(self):
        return '/v1/default/banks/' + self.binding.scope.bank_id + '/memories'


@dataclass(frozen=True)
class PublicExport:
    catalogue: Catalogue
    retain_drafts: tuple[RetainDraft, ...] = field(repr=False)
    serialized: bytes = field(repr=False)
    _io: tuple[tuple[str, int], ...] = field(repr=False)
    active: InitVar[object] = None

    def __post_init__(self, active=None):
        if active is None:
            active = _always_active
        _active(active)
        total = _drafts(self.catalogue, self.retain_drafts, active=active)
        if (type(self.serialized) is not bytes or len(self.serialized) > PACKET_BYTES
                or self.serialized != _packet(self.catalogue, self.retain_drafts)
                or type(self._io) is not tuple or len(self._io) != len(_IO_KEYS)
                or any(type(x) is not tuple or len(x) != 2 or type(x[0]) is not str
                       or type(x[1]) is not int or not 0 <= x[1] <= 11000 * SOURCE_BYTES for x in self._io)
                or tuple(x[0] for x in self._io) != _IO_KEYS):
            raise ExportError('approval')
        io = dict(self._io)
        minimum_reads = sum((len(json.loads(x.body)['items'][0]['content'].encode('utf-8')) + 8191) // 8192 + 1
                            for x in self.retain_drafts)
        if (io['source_read_bytes'] != total
                or not minimum_reads <= io['source_read_calls'] <= TOTAL_SOURCE_BYTES + MAX_REFERENCES
                or not 2 * len(self.retain_drafts) <= io['opened_descriptors'] <= 11000
                or io['closed_descriptors'] != io['opened_descriptors'] or io['close_errors'] != 0):
            raise ExportError('approval')
        _active(active)

    @property
    def diagnostics(self):
        return {'schema_version': 1, 'profile': PROFILE,
                'classification': 'operator-attested-public',
                'source_count': len(self.catalogue.sources), **dict(self._io),
                'export_packet_bytes': len(self.serialized),
                'retain_draft_body_bytes': sum(len(x.body) for x in self.retain_drafts),
                'executed_transport_calls': 0, 'remote_tokens': None,
                'provider_tokens': None, 'remote_usd': None, 'total_cost_complete': False,
                'remote_cost_stages': {stage: None for stage in ('retain', 'extraction',
                    'embeddings', 'consolidation', 'storage', 'delete', 'reconciliation', 'retries')}}


def build_public_export(workspace, scope, approvals, *, active=lambda: True):
    """All-or-nothing capture of a supplied selection; never discover/classify files.

    Returns held bytes in memory only. No source is reread during serialization.
    A successful capture is point-in-time; caller must reapprove later revisions.
    """
    io = {'source_read_bytes': 0, 'source_read_calls': 0, 'opened_descriptors': 0,
          'closed_descriptors': 0, 'close_errors': 0}
    try:
        if (type(scope) is not Scope or type(approvals) is not tuple or len(approvals) > MAX_REFERENCES
                or any(type(x) is not PublicApproval or x.scope != scope for x in approvals)
                or len({x.path for x in approvals}) != len(approvals)):
            raise ExportError('approval')
        root = Path(workspace)
        if (not root.is_absolute() or '..' in root.parts or len(root.parts) > 64
                or len(str(root)) > 4096 or any(ord(c) < 32 for c in str(root))):
            raise ExportError('source_path')
        _active(active)
        drafts, total = [], 0
        for approval in approvals:
            raw = _read(root, approval, active, io)
            total += len(raw)
            if total > TOTAL_SOURCE_BYTES:
                raise ExportError('source_bound')
            text = _text(raw, active=active, lenient=_literal_backslash_u(approval.path))
            binding = SourceBinding.from_bytes(scope, approval.path, raw)
            source_id = 'logical-' + _sha(_json([scope.key, approval.path]).encode())
            request = _request(binding, source_id, text)
            body = _json(request).encode('utf-8')
            if len(body) > RETAIN_BYTES:
                raise ExportError('request_bound')
            drafts.append(RetainDraft(binding, source_id, body, active=active))
        catalogue = Catalogue(scope, tuple(x.binding for x in drafts))
        serialized = _packet(catalogue, drafts)
        if len(serialized) > PACKET_BYTES:
            raise ExportError('packet_bound')
        _active(active)
        result = PublicExport(catalogue, tuple(drafts),
                              serialized, tuple(io.items()), active=active)
        _active(active)
        return result
    except ExportError as error:
        error.observation = dict(io)
        raise
    except (TypeError, ValueError, OSError):
        raise ExportError('unavailable', io) from None


@dataclass(frozen=True)
class IngestionPlan:
    """Desired-input delta only. No mutation executor or remote-state evidence."""
    current: Catalogue
    current_retain_drafts: tuple[RetainDraft, ...] = field(repr=False)
    retain_candidates: tuple[RetainDraft, ...] = field(repr=False)
    retire_candidates: tuple[SourceBinding, ...]
    reconciliation: tuple[SourceBinding, ...]
    input_match_count: int
    active: InitVar[object] = None

    def __post_init__(self, active=None):
        if active is None:
            active = _always_active
        _active(active)
        _drafts(self.current, self.current_retain_drafts, active=active)
        if (type(self.retain_candidates) is not tuple
                or any(type(x) is not RetainDraft or x not in self.current_retain_drafts for x in self.retain_candidates)
                or len(set(x.binding.document_id for x in self.retain_candidates)) != len(self.retain_candidates)
                or type(self.retire_candidates) is not tuple or len(self.retire_candidates) > MAX_REFERENCES
                or type(self.reconciliation) is not tuple or len(self.reconciliation) > 2 * MAX_REFERENCES
                or type(self.input_match_count) is not int
                or self.input_match_count != len(self.current.sources) - len(self.retain_candidates)):
            raise ExportError('previous_catalogue')
        for item in self.retire_candidates + self.reconciliation:
            _active(active)
            _binding_integrity(item, self.current.scope)
        old = {x.document_id for x in self.retire_candidates}
        new = {x.document_id for x in self.current.sources}
        union = {x.document_id for x in self.reconciliation}
        if (old & new or len(old) != len(self.retire_candidates) or len(union) != len(self.reconciliation)
                or old | new != union):
            raise ExportError('previous_catalogue')
        _active(active)

    @property
    def diagnostics(self):
        return {'schema_version': 1, 'profile': PROFILE, 'state': 'reconciliation_required',
                'input_match_count': self.input_match_count,
                'retain_candidate_count': len(self.retain_candidates),
                'retire_candidate_count': len(self.retire_candidates),
                'reconciliation_draft_count': len(self.reconciliation),
                'authorized_mutations': 0, 'remote_noop': None,
                'executed_transport_calls': 0, 'remote_cost_complete': False}

    @property
    def reconciliation_drafts(self):
        return tuple(('GET', '/v1/default/banks/' + item.scope.bank_id + '/documents/' + item.document_id)
                     for item in self.reconciliation)


def plan_public_ingestion(export, *, previous_owned=None, active=_always_active):
    """A prior owned catalogue is not a receipt, public approval or remote response.

    Matching input never authorizes a remote no-op. Deletion candidates name only
    supplied, canonical prior versions. A future authenticated executor must
    independently reconcile ownership/current remote state and in-flight work.
    """
    if type(export) is not PublicExport:
        raise ExportError('previous_catalogue')
    _active(active)
    export.__post_init__(active=active)
    current = export.catalogue
    if previous_owned is None:
        previous_owned = Catalogue(current.scope, ())
    if type(previous_owned) is not Catalogue or previous_owned.scope != current.scope:
        raise ExportError('previous_catalogue')
    previous_owned.__post_init__()
    for item in previous_owned.sources:
        _active(active)
        _binding_integrity(item, current.scope)
    old_ids = {x.document_id for x in previous_owned.sources}
    new_ids = {x.document_id for x in current.sources}
    union = previous_owned.sources + tuple(x for x in current.sources if x.document_id not in old_ids)
    return IngestionPlan(current, export.retain_drafts,
                         tuple(x for x in export.retain_drafts if x.binding.document_id not in old_ids),
                         tuple(x for x in previous_owned.sources if x.document_id not in new_ids),
                         union, len(new_ids & old_ids), active=active)
