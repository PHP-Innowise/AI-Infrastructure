"""Bounded private observations of retrieval metadata; never proof of use."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import stat
import time
import uuid
from itertools import islice

from memory_pilot_support import PilotError, canonical_module, public_text, relative_path

MAX_BYTES = 65536
MAX_FILE = 1024 * 1024
MAX_READ = 8 * MAX_FILE
POINTS = {'refresh', 'hook-context', 'retrieve', 'context'}
HEX = re.compile(r'^[0-9a-f]{64}$')


def _sha(raw):
    return hashlib.sha256(raw).hexdigest()


def _json(raw):
    def integer(value):
        if len(value) > 20: raise PilotError('integer limit')
        return int(value)
    def bad(_): raise PilotError('nonfinite JSON')
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result: raise PilotError('duplicate JSON key')
            result[key] = value
        return result
    result = json.loads(raw.decode('utf-8'), parse_int=integer, parse_constant=bad,
                        object_pairs_hook=pairs)
    json.dumps(result, allow_nan=False)
    return result


def _uuid(value):
    try: return isinstance(value, str) and str(uuid.UUID(value)) == value
    except (ValueError, AttributeError): return False


def _number(value):
    return type(value) is int and 0 <= value <= 2**53 - 1


def _path(value, known):
    if not isinstance(value, str) or len(value) > 512 or value not in known:
        raise PilotError('unrecognized path')
    if any(not (c.isalnum() or c in '-_./ ') for c in value):
        raise PilotError('path syntax')
    public_text(value)
    if canonical_module('brain_runtime').instruction_like('', value):
        raise PilotError('instruction-like path')
    return relative_path(value, memory=True).as_posix()


class _Reads:
    def __init__(self, root, deadline):
        self.root, self.deadline, self.remaining = Path(root), deadline, MAX_READ
        self.exhausted = False

    def read(self, name):
        if self.deadline is not None and time.monotonic() >= self.deadline:
            self.exhausted = True
            raise PilotError('collection deadline')
        parts = relative_path(name, memory=True).parts
        descriptors = []
        try:
            if self.root.is_symlink(): raise PilotError('linked root')
            if os.name == 'posix':
                parent = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
                descriptors.append(parent)
                for part in parts[:-1]:
                    parent = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
                    descriptors.append(parent)
                fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
            else:
                path = self.root
                for part in parts:
                    path = path / part
                    if path.is_symlink() or getattr(path.lstat(), 'st_file_attributes', 0) & 0x400:
                        raise PilotError('linked path')
                fd = os.open(path, os.O_RDONLY | getattr(os, 'O_BINARY', 0))
            descriptors.append(fd)
            before = os.fstat(fd)
            if before.st_size > self.remaining: self.exhausted = True
            if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1
                    or before.st_size > min(MAX_FILE, self.remaining)):
                raise PilotError('file bound')
            raw = bytearray()
            while True:
                if self.deadline is not None and time.monotonic() >= self.deadline:
                    self.exhausted = True
                    raise PilotError('collection deadline')
                if self.remaining <= 0:
                    self.exhausted = True
                    raise PilotError('read budget')
                block = os.read(fd, min(65536, self.remaining, MAX_FILE + 1 - len(raw)))
                self.remaining -= len(block)
                if not block: break
                raw.extend(block)
                if len(raw) > MAX_FILE: raise PilotError('file bound')
            after = os.fstat(fd)
            named = (os.stat(parts[-1], dir_fd=parent, follow_symlinks=False) if os.name == 'posix'
                     else path.lstat())
            identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns, s.st_nlink)
            if identity(before) != identity(after) or identity(after) != identity(named):
                raise PilotError('changing artifact')
            return bytes(raw)
        finally:
            for fd in reversed(descriptors): os.close(fd)


def _render(reads, manifest, selected, source_hash):
    unknown = {'status': 'unknown'}
    try:
        trace = _json(reads.read('memory-bank/local/retrieval-traces/' + manifest['id'] + '.json'))
        if (trace.get('id') != manifest['id'] or trace.get('schema_version') != 1
                or trace.get('truncated') is not False): return unknown
        render = trace.get('render')
        if (not isinstance(render, dict) or render.get('form') not in ('printed_capsule', 'capsule_text')
                or not HEX.fullmatch(str(render.get('sha256', ''))) or not _number(render.get('bytes'))
                or render['bytes'] > MAX_FILE): return unknown
        images = [r for r in trace.get('records', []) if isinstance(r, dict) and r.get('path') == selected['path']]
        rows = [r for r in render.get('records', []) if isinstance(r, dict) and r.get('path') == selected['path']]
        if len(images) != 1 or len(rows) != 1 or images[0].get('source_sha256') != source_hash:
            return unknown
        row = rows[0]
        if (row.get('header_present') is not True or not _number(row.get('body_bytes'))
                or row['body_bytes'] > MAX_FILE or type(row.get('pointer_only')) is not bool
                or not HEX.fullmatch(str(row.get('body_sha256', '')))): return unknown
        return {'status': 'trace-reported', 'frame_sha256': render['sha256'],
                'frame_bytes': render['bytes'], 'form': render['form'],
                'body_sha256': row['body_sha256'], 'body_bytes': row['body_bytes'],
                'pointer_only': row['pointer_only']}
    except (OSError, ValueError, UnicodeError, RecursionError, AttributeError, TypeError): return unknown


def _document(reads, item, manifest, known):
    path = _path(item.get('path'), known)
    image = item.get('source_hash')
    if not isinstance(image, str) or not HEX.fullmatch(image): raise PilotError('digest')
    result = {'document_id': _sha(path.encode()), 'path': path, 'document_sha256': image,
              'document_state': 'unknown', 'record_id': None, 'revision': None,
              'sources': None, 'render': {'status': 'unknown'}}
    try:
        raw = reads.read(path)
        text = raw.decode('utf-8').replace('\r\n', '\n').replace('\r', '\n')
        result['document_state'] = 'match' if _sha(text.encode()) == image else 'changed'
        if result['document_state'] != 'match': return result
        result['render'] = _render(reads, manifest, item, image)
        match = re.match(r'\A---\n(.*?)\n---(?:\n|$)', text, re.S)
        if match is None: return result
        metadata = _json(match.group(1).encode())
        identifier = metadata.get('id')
        if _uuid(identifier) or (isinstance(identifier, str) and re.fullmatch(r'MEM-[A-Za-z0-9-]{1,120}', identifier)):
            try:
                public_text(identifier)
                if not canonical_module('brain_runtime').instruction_like('', identifier):
                    result['record_id'] = identifier
            except PilotError: pass
        if _number(metadata.get('revision')): result['revision'] = metadata['revision']
        entries = metadata.get('source_fingerprints', metadata.get('source_digests'))
        if not isinstance(entries, list) or len(entries) > 16: return result
        refs, seen = [], set()
        for entry in entries:
            name = _path(entry.get('path'), known)
            expected = entry.get('sha256')
            if name in seen or not isinstance(expected, str) or not HEX.fullmatch(expected):
                return result
            seen.add(name)
            ref = {'path': name, 'expected_sha256': expected, 'observed_sha256': None, 'state': 'unknown'}
            try:
                data = reads.read(name)
                normalized = data if b'\0' in data[:8192] else data.replace(b'\r\n', b'\n')
                ref['observed_sha256'] = _sha(normalized)
                ref['state'] = 'match' if ref['observed_sha256'] == expected else 'changed'
            except FileNotFoundError: ref['state'] = 'missing'
            except (OSError, ValueError): pass
            refs.append(ref)
        declared = metadata.get('sources')
        if declared is not None:
            if not isinstance(declared, list) or len(declared) > 16: return result
            names = [_path(name, known) for name in declared]
            if len(set(names)) != len(names) or set(names) != seen: return result
        result['sources'] = refs
    except FileNotFoundError: result['document_state'] = 'missing'
    except (OSError, ValueError, UnicodeError, RecursionError, AttributeError, TypeError): pass
    return result


def snapshot_known_paths(corpus):
    """Maintainer-owned pre-run path vocabulary, not agent supplied labels."""
    result, visited = set(), 0
    for folder, directories, names in os.walk(corpus, followlinks=False):
        visited += len(directories) + len(names) + 1
        if visited > 100000: return result
        directories[:] = [n for n in directories if n not in
            ('.git', 'local', '.context-handoff', 'node_modules', 'vendor', '__pycache__')
            and not (Path(folder) / n).is_symlink()]
        for name in names:
            path = Path(folder) / name
            if not path.is_symlink(): result.add(path.relative_to(corpus).as_posix())
            if len(result) >= 100000: return result
    return result


def collect_delivery_proof(corpus, manifests, *, known_paths, deadline=None):
    """Collect pre-run-known identities; current hashes are post-run observations.

    Input manifests and traces remain unauthenticated and agent-writable.
    Bounds limit bytes/work; regular filesystem reads are not preemptible.
    """
    result = {'schema': 'memory-delivery-proof-v1', 'status': 'unknown', 'phase': 'post-run',
              'producer_authenticated': False, 'host_receipt': 'unknown', 'model_use': 'unproven',
              'semantic_verification': 'unproven', 'frames': [], 'incomplete': False}
    reads = _Reads(corpus, deadline)
    paths = list(islice(iter(manifests), 33))
    if len(paths) > 32: result['incomplete'] = True
    count = 0
    for path in sorted(paths, key=str)[:32]:
        try:
            relative = Path(path).relative_to(corpus).as_posix()
            if Path(relative).parent.as_posix() not in ('memory-bank/local/retrieval-manifests',
                                                       'project-brain/control/retrieval-manifests'):
                raise PilotError('manifest directory')
            raw = reads.read(relative); manifest = _json(raw)
            if (not isinstance(manifest, dict) or manifest.get('schema_version') != 4
                    or not _uuid(manifest.get('id')) or Path(relative).stem != manifest['id']
                    or manifest.get('entry_point') not in POINTS
                    or not (_uuid(manifest.get('task_id')) or manifest.get('task_id') is None)
                    or not (_number(manifest.get('task_revision')) or manifest.get('task_revision') is None)):
                raise PilotError('manifest shape')
            selected = manifest.get('selected')
            if not isinstance(selected, list) or len(selected) > 32: raise PilotError('document bound')
            frame = {'manifest_id': manifest['id'], 'manifest_path': relative, 'manifest_sha256': _sha(raw),
                     'task_uuid': manifest.get('task_id'), 'task_revision': manifest.get('task_revision'),
                     'entry_point': manifest['entry_point'],
                     'origin': 'automatic' if manifest['entry_point'] in ('refresh', 'hook-context') else 'explicit',
                     'selected': []}
            seen = set()
            for item in selected:
                count += 1
                if count > 32: raise PilotError('total document bound')
                try:
                    doc = _document(reads, item, manifest, known_paths)
                    if doc['path'] in seen: raise PilotError('duplicate document')
                    seen.add(doc['path']); frame['selected'].append(doc)
                except (OSError, ValueError, UnicodeError, RecursionError, AttributeError, TypeError):
                    result['incomplete'] = True
            result['frames'].append(frame)
            if len(json.dumps(result, ensure_ascii=False).encode()) > MAX_BYTES:
                result['frames'].pop(); result['incomplete'] = True; break
        except (OSError, ValueError, UnicodeError, RecursionError, AttributeError, TypeError):
            result['incomplete'] = True
    result['incomplete'] |= reads.exhausted
    if result['frames']: result['status'] = 'partial' if result['incomplete'] else 'observed'
    while len(json.dumps(result, ensure_ascii=False).encode()) > MAX_BYTES and result['frames']:
        result['frames'].pop(); result['incomplete'] = True
        result['status'] = 'partial' if result['frames'] else 'unknown'
    return result
