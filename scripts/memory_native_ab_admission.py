"""Bounded known-credential admission for maintainer-only native experiments.

No rewriting or user approval switch. Exact canonical bytes are trusted code;
fresh Git/index binaries are declared derived state, not text-scanned data.
This is not a proof that arbitrary encoded secrets cannot exist.
"""
from __future__ import annotations

from functools import lru_cache
import configparser
import json
import os
from pathlib import Path
import re
import stat

import memory_native_ab_io as io
from memory_pilot_support import canonical_module

POLICY = 'native-snapshot-known-credentials-v1'
MARKER = '[REDACTED_PRIVATE_VALUE]'
_KEY = (r'[a-z0-9_.-]*'
        r'(?:(?:password|passwd|passphrase)(?:[_.-]?(?:hash|digest))?|secret(?:[_.-]?(?:access|key|token|passphrase))*'
        r'|api[_.-]?key|access[_.-]?(?:key|token)|auth[_.-]?token|refresh[_.-]?token'
        r'|private[_.-]?key|account[_.-]?key|shared[_.-]?access[_.-]?key'
        r'|master[_.-]?key|app[_.-]?key|s3[_.-](?:media[_.-])?key'
        r'|meili(?:search)?[_.-](?:master[_.-])?key|registry[_.-]?credentials)')
_CREDENTIAL_KEY = re.compile(_KEY, re.I)
# Tokenize a field once; do not retry a suffix regex at every underscore or
# copy the rest of a long line for every assignment on that line.
_ASSIGN = re.compile(r'(?<![A-Za-z0-9_.-])(?P<q>[\'"`]?)'
                     r'(?P<key>[A-Za-z_][A-Za-z0-9_.-]*)(?P=q)(?:[ \t\r\n]*\])?[ \t\r\n]*(?P<sep>:|=>|=)[ \t]*')
_OPAQUE = re.compile(r'\bEV[ \t\r\n]*\[')
_URL = re.compile(r'(?<![a-z0-9+.-])[a-z][a-z0-9+.-]*://[^\s/@:]*:[^\s/@]*@', re.I)
_SAFE_SCALAR = re.compile(
    r'(?:\[REDACTED_PRIVATE_VALUE\]|\[REDACTED\]|\$\{[A-Z][A-Z0-9_]*\}'
    r'|\$[A-Z][A-Z0-9_]*|%env\((?:[a-zA-Z_]\w*:)*[A-Z][A-Z0-9_]*\)%)')
_DERIVED_BINARY = {'memory-bank/local/context.db', 'project-brain/local/record-images.cache'}
_AUTH_STUB = 'memory-bank/local/native-ab/home/auth.json'
_DATA = ('memory-bank/local/', 'memory-bank/chunks/', 'project-brain/dynamic/',
         'project-brain/archive/', 'project-brain/control/', 'project-brain/indexes/',
         'project-brain/local/', '.context-handoff/')
_STATE_FILES = {'memory-bank/INDEX.md', 'tasks/.task-counter'}


def _fail(code):
    raise io.PilotError('snapshot-admission:' + code)


@lru_cache(maxsize=1)
def _canonical():
    import memory_eval as stand
    edition = stand.Edition('PHP Core')
    result = {}
    for _, name in edition.files:
        source = edition.source(name)
        raw, _ = io.read_file(source)
        source_name = edition.data['source_overrides'].get(name, name)
        executable = stand.installer.installs_executable(name, source, source_name, edition.bits)
        result[name] = (raw, 0o755 if executable else 0o644)
    return result


def _merged(name, raw, actual, canon):
    """Only the installer's intentional merge/keep rules may change input."""
    if actual is None or name not in canon:return False
    import memory_eval as stand
    try:
        old, new = raw.decode('utf-8'), canon[name][0].decode('utf-8')
        if name in stand.installer.ADDITIVE_FILES:
            expected = stand.installer.merge_additive_file(old, new).encode()
        elif name in stand.installer.MANAGED_POLICY_FILES:
            expected = stand.installer.merge_agents_file(old, new).encode()
        else:return False
        return actual == (expected, canon[name][1])
    except (ValueError, UnicodeError):return False


def _required_canonical(name):
    import memory_eval as stand
    if name in {'project-brain/config/runtime.json','project-brain/config/providers.json','project-brain/config/telemetry.json'}:
        return True
    return not (stand.project_owned(name) or name in stand.installer.ADDITIVE_FILES
                or name in stand.installer.MANAGED_POLICY_FILES or name == 'README.md')


def _safe_scalar(value):
    return isinstance(value,str) and len(value)<=1024 and _SAFE_SCALAR.fullmatch(value) is not None


def _credential_field(key,value, *, deadline=None):
    # JSON separates the header name from its value; whole-text header
    # patterns cannot bridge the closing quote of the key.
    if _CREDENTIAL_KEY.fullmatch(key):return True
    if key.casefold() not in {'authorization','proxy-authorization'}:return False
    # Headers normalizes leading HTTP whitespace and stringifies header
    # arrays. Inspect their literal values before that coercion hides context.
    pending,count=[value],0
    while pending:
        io.check_deadline(deadline);item=pending.pop();count+=1
        if count>20000:_fail('field-limit')
        if isinstance(item,list):pending.extend(item)
        elif isinstance(item,str) and re.match(r'''^["']?(?:Basic|Bearer)[ \t]+\S''',
                item.lstrip(' \t\r\n'),re.I):return True
    return False


def _line_value(text, start, separator):
    # An artificial slice end must never establish a scalar's real end.
    tail = text[start:start+1025]
    end = re.search(r'[\r\n]',tail)
    if end:tail = tail[:end.start()]
    elif len(tail)>1024:return None
    tail = tail.strip(' \t')
    if separator==':' and tail.startswith(('$','%env(')):
        # An unquoted YAML reference may fold later indented lines into the
        # value. Only explicit quoted references are supported in this form.
        return None
    if len(tail)>=2 and tail[0] in '\'"' and tail[-1]==tail[0]:tail=tail[1:-1]
    return tail


def _known(text):
    if _OPAQUE.search(text) or _URL.search(text):_fail('known-credential')
    auto=canonical_module('automatic_query')
    if any(pattern.search(text) for label,pattern in auto.SECRET_PATTERNS.items()
           if label not in {'assigned credential','credential in URL'}):_fail('known-credential')


def _json_fields(value, *, deadline=None):
    pending,count=[value],0
    while pending:
        io.check_deadline(deadline);node=pending.pop();count+=1
        if count>20000:_fail('field-limit')
        if isinstance(node,dict):
            secret=False
            for key,val in node.items():
                if len(key)>256:_fail('field-limit')
                _known(key);_plain_fields(key,deadline=deadline)
                if _credential_field(key,val,deadline=deadline) and not _safe_scalar(val):_fail('credential-assignment')
                if key.casefold() in {'key','name'} and isinstance(val,str) and _CREDENTIAL_KEY.fullmatch(val):secret=True
                if key.casefold()=='type' and isinstance(val,str) and val.casefold()=='secret':secret=True
                pending.append(val)
            if secret and not _safe_scalar(node.get('value')):_fail('credential-container')
        elif isinstance(node,list):pending.extend(node)
        elif isinstance(node,str):_known(node);_plain_fields(node,deadline=deadline)


def _text(raw, *, deadline=None):
    try:
        text = raw.decode('utf-8')
    except UnicodeError:
        _fail('unscannable-text')
    if any(ord(c) < 32 and c not in '\t\r\n' for c in text) or '\x7f' in text:
        _fail('unscannable-text')
    _known(text)
    normalized=text.lstrip('\ufeff \t\r\n')
    if normalized.startswith(('{','[')):
        try:parsed=io._json(normalized.encode())
        except (ValueError,UnicodeError,RecursionError):
            # Simple INI is the one supported bracket-led alternative (fresh
            # Git config and PHP-FPM sections). Never reinterpret malformed
            # JSON, duplicate escaped keys or partial containers as safe prose.
            if not re.match(r'^\[[A-Za-z0-9_. *?-]+(?: "[A-Za-z0-9_. -]+")?\][ \t]*(?:\r?\n|$)',normalized):
                _fail('invalid-structured-text')
            parser=configparser.RawConfigParser(strict=True,interpolation=None)
            try:parser.read_string(normalized)
            except configparser.Error:_fail('invalid-structured-text')
            if any(not re.fullmatch(r'[A-Za-z0-9_. *?-]+(?: "[A-Za-z0-9_. -]+")?',s) for s in parser.sections()):
                _fail('invalid-structured-text')
            rows=[dict(parser.defaults()),*(dict(parser.items(s,raw=True)) for s in parser.sections())]
            if any(not re.fullmatch(r'[A-Za-z0-9_.-]+',key) for row in rows for key in row):
                _fail('invalid-structured-text')
            parsed=[{k:v[1:-1] if len(v)>=2 and v[0] in '\'"' and v[-1]==v[0] else v
                     for k,v in row.items()} for row in rows]
            _json_fields(parsed,deadline=deadline);io.check_deadline(deadline);return
        else:
            _json_fields(parsed,deadline=deadline);io.check_deadline(deadline);return
    _plain_fields(text,deadline=deadline)


def _plain_fields(text, *, deadline=None):
    # Unlike prose-query sanitization, defaults/numbers/example-looking values
    # do not excuse credential assignments. Block unknown/container forms too.
    secret_container, values, count = False, [], 0
    for match in _ASSIGN.finditer(text):
        io.check_deadline(deadline);count += 1
        if count > 20000 or len(match['key']) > 256:
            _fail('field-limit')
        key = match['key']
        rhs = _line_value(text,match.end(),match['sep'])
        if _credential_field(key,rhs,deadline=deadline) and not _safe_scalar(rhs):
            _fail('credential-assignment')
        if key.casefold() == 'value':values.append(rhs)
        if key.casefold() in {'key', 'name', 'type'}:
            scalar = re.match(r'''^["']?([A-Za-z_][A-Za-z0-9_.-]*)["']?(?=[ \t]*(?:[,;}\]#]|\r|\n|$))''', text[match.end():match.end()+1024])
            if scalar and ((key.casefold() == 'type' and scalar[1].casefold() == 'secret')
                           or (key.casefold() != 'type' and _CREDENTIAL_KEY.fullmatch(scalar[1]))):
                secret_container = True
    # A split key/type/value carrier is ambiguous without a full YAML parser.
    # Conservatively require every value field in such a file to be screened.
    if secret_container and (not values or any(not _safe_scalar(v) for v in values)):
        _fail('credential-container')
    io.check_deadline(deadline)


def _path(name, *, generated=False):
    if generated and isinstance(name, str) and (name == '.git' or name.startswith('.git/') or name in _DERIVED_BINARY or name == _AUTH_STUB):
        if '\\' in name or ':' in name or '\0' in name or any(p in ('', '.', '..') for p in name.split('/')):
            _fail('unsafe-path')
        return
    try:
        relative = io.relative_path(name, memory=True)
    except (ValueError, TypeError):
        _fail('unsafe-path')
    parts = relative.parts
    if any(p.casefold().startswith('.env') for p in parts) or any(
            p.casefold() in {'auth.json', 'credentials.json', 'settings.local.json'} for p in parts):
        _fail('private-path')
    if any(ord(c)<32 or ord(c)==127 for c in name):_fail('unsafe-path')
    _known(name);_plain_fields(name)


def _opaque(name):
    return name in _DERIVED_BINARY or name == '.git/index' or name.startswith('.git/objects/')


def _admit(image, *, generated=False, deadline=None):
    seen, total, canonical, derived = set(), 0, 0, 0
    canon = _canonical()
    for name, raw, mode in image:
        io.check_deadline(deadline)
        _path(name, generated=generated)
        if name in seen or not isinstance(raw, bytes) or type(mode) is not int or not 0 <= mode <= 0o777:
            _fail('invalid-image')
        seen.add(name); total += len(raw)
        if len(raw) > io.MAX_FILE or total > io.MAX_TREE or len(seen) > 20000:
            _fail('image-limit')
        if generated and name == _AUTH_STUB:
            if raw or mode != 0o600:
                _fail('invalid-transport-stub')
            derived += 1
        elif generated and _opaque(name):
            derived += 1
        elif canon.get(name) == (raw, mode):
            canonical += 1
        else:
            if name in canon and name.startswith(('memory-bank/scripts/','project-brain/scripts/')):
                _fail('modified-runtime')
            _text(raw, deadline=deadline)
    return {'policy': POLICY, 'image_sha256': io.image_digest(image), 'files': len(seen),
            'bytes': total, 'exact_canonical_files': canonical, 'fresh_derived_files': derived}


def admit_image(image, *, deadline=None):
    """Admit the held selected bytes, before they enter Git or an index."""
    try:
        return _admit(image, deadline=deadline)
    except io.PilotError:
        raise
    except (OSError, ValueError, TypeError, UnicodeError):
        _fail('invalid-image')


def physical_image(root, *, deadline=None):
    """Read every physical file and directory, with no snapshot() exclusions."""
    image, folders, total, visited = [], [], 0, 0
    try:
        root = io.ordinary(root)
        if not root.is_dir():
            _fail('unsafe-tree')
        root_mode = stat.S_IMODE(root.stat().st_mode)
        def onerror(_):
            _fail('unreadable-tree')
        for base, dirs, files in os.walk(root, followlinks=False, onerror=onerror):
            io.check_deadline(deadline)
            current = Path(base)
            visited += len(dirs) + len(files) + 1
            if visited > 50000:
                _fail('image-limit')
            for name in sorted(dirs):
                directory = current / name
                info = directory.lstat()
                if not stat.S_ISDIR(info.st_mode) or directory.is_symlink():
                    _fail('unsafe-tree')
                relative = directory.relative_to(root).as_posix()
                _path(relative, generated=True)
                folders.append((relative, stat.S_IMODE(info.st_mode)))
            for name in sorted(files):
                file = current / name
                raw, mode = io.read_file(file)
                total += len(raw)
                if total > io.MAX_TREE or len(image) >= 20000:
                    _fail('image-limit')
                image.append((file.relative_to(root).as_posix(), raw, mode))
        return sorted(image), {'root_mode': root_mode, 'directories': sorted(folders)}
    except io.PilotError:
        raise
    except (OSError, ValueError, TypeError, UnicodeError):
        _fail('unsafe-tree')


def _state(name):
    return name in _STATE_FILES or name.startswith(_DATA) or any(name == p.rstrip('/') for p in _DATA)


def _manifest(image, folders):
    return io.digest({'files': [(n, io.sha(raw), mode) for n, raw, mode in image], 'folders': folders})


class MaterializedGate:
    """Controller-owned lifecycle: clean initial tree -> prepare -> exact verify.

    Opaque derived paths are allowed only after trusted controller preparation
    of this initially Git/local-state-free copy. No receipt or caller flag can
    admit a preexisting history/database or grant trust to later user content.
    """
    def __init__(self, root, source_image, *, deadline=None):
        self.root, self.deadline, self.expected, self.derived = root, deadline, None, None
        image, folders = physical_image(root, deadline=deadline)
        if folders['root_mode'] != 0o700:
            _fail('nonprivate-root')
        allowed = {n for n, _, _ in source_image} | set(_canonical())
        parents={p.as_posix() for name in allowed for p in Path(name).parents if p.as_posix()!='.'}
        if any(n not in parents and n!='.context-handoff' for n,_ in folders['directories']):
            _fail('unexpected-initial-directory')
        for name, _, _ in image:
            if name not in allowed or name.startswith('.git/') or name in _DERIVED_BINARY:
                _fail('nonfresh-initial-tree')
        if any(n == '.git' or n.startswith('.git/') for n, _ in folders['directories']):
            _fail('nonfresh-initial-tree')
        actual = {n: (raw, mode) for n, raw, mode in image}
        canon = _canonical()
        for name, expected in canon.items():
            if _required_canonical(name) and actual.get(name) != expected:
                _fail('canonical-overlay-changed')
        for name, raw, mode in source_image:
            value = actual.get(name)
            if name in canon and value == canon[name]:continue
            import memory_eval as stand
            if value == (raw, mode) and (name not in canon or stand.project_owned(name) or name == 'README.md'):
                continue
            if not _merged(name, raw, value, canon):
                _fail('held-source-changed')
        self.initial = admit_image(image, deadline=deadline)
        self.protected = {n: (raw, mode) for n, raw, mode in image if not _state(n)}
        self.initial_names = {n for n, _, _ in image}
        self.initial_dirs = {n for n, _ in folders['directories']}

    def _prepared(self):
        image, folders = physical_image(self.root, deadline=self.deadline)
        if folders['root_mode'] != 0o700:
            _fail('nonprivate-root')
        current = {n: (raw, mode) for n, raw, mode in image}
        if any(current.get(n) != value for n, value in self.protected.items()):
            _fail('protected-image-changed')
        for name in current:
            if name not in self.initial_names and not _state(name) and not name.startswith('.git/'):
                _fail('unexpected-file')
        for name, _ in folders['directories']:
            if name not in self.initial_dirs and name != '.git' and not name.startswith('.git/') and not _state(name):
                _fail('unexpected-directory')
        return image, folders

    def seal_generated(self):
        """Called only after trusted fresh Git/task setup, before mount setup."""
        if self.derived is not None:
            _fail('derived-already-fixed')
        image, folders = self._prepared()
        _admit(image, generated=True, deadline=self.deadline)
        self.derived = {n: (raw, mode) for n, raw, mode in image if _opaque(n)}
        self.derived_dirs = [(n, mode) for n, mode in folders['directories'] if n == '.git' or n.startswith('.git/')]

    def checkpoint(self):
        if self.expected is not None or self.derived is None:
            _fail('invalid-checkpoint-state')
        image, folders = self._prepared()
        if {n: (raw, mode) for n, raw, mode in image if _opaque(n)} != self.derived:
            _fail('derived-image-changed')
        if [(n, mode) for n, mode in folders['directories'] if n == '.git' or n.startswith('.git/')] != self.derived_dirs:
            _fail('derived-image-changed')
        report = _admit(image, generated=True, deadline=self.deadline)
        self.expected = _manifest(image, folders)
        return {**report, 'materialized_sha256': self.expected,
                'derived_origin': 'fresh-controller-git-and-index-from-admitted-bytes'}

    def verify(self):
        if self.expected is None:
            _fail('checkpoint-required')
        image, folders = physical_image(self.root, deadline=self.deadline)
        if _manifest(image, folders) != self.expected:
            _fail('materialized-image-changed')
        return {'policy': POLICY, 'materialized_sha256': self.expected, 'verified': True}
