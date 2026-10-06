"""Knowledge › Memory use: what agent memory holds, how it moves and how often retrieval selects it.

Everything here is read from project files in this process, without the knowledge
lock and without running project code: a background read must never make a linked
launch's freshness check fail. Chunk identities, titles and dates leave the server;
Project Brain titles and bodies, retrieval queries and every non-chunk path do not.
"""
from __future__ import annotations

from collections import OrderedDict
from datetime import date, datetime, timedelta, timezone
import hashlib
import json
import os
import re
import stat
import threading

from .filesystem import fs
from .sessions import SessionError, open_project_path, read_context

CHUNK_LIMIT = 500
CHUNK_BYTES = 64 * 1024
RECORD_LIMIT = 5000
RECORD_TOTAL_BYTES = 8 * 1024 * 1024
RECORD_BYTES = 64 * 1024
PROMOTION_LIMIT = 500
RETRIEVAL_LIMIT = 200
# Each store keeps more manifests than are shown; Prepare and freshness pairs merge before the cut.
MANIFEST_SCAN = 400
MANIFEST_BYTES = 256 * 1024
HEALTH_BYTES = 2 * 1024 * 1024
SOURCE_BYTES = 32 * 1024 * 1024
RUNTIME_BYTES = 1024 * 1024

# What each retrieval category is, in the memory colours: static rules and docs, the bank, the live Brain.
KINDS = {'policy': 'rules', 'evidence': 'rules', 'durable': 'bank', 'handoff': 'brain', 'dynamic': 'brain'}
# Statuses still in progress. Terminal ones, an accepted decision and a recorded event are not open.
OPEN_STATES = {'task': {'active', 'blocked', 'verifying'}, 'finding': {'open', 'investigating'},
               'bug': {'reported', 'triaged', 'fixing', 'verifying'},
               'incident': {'open', 'contained', 'recovering', 'resolved'}, 'decision': {'proposed'}, 'event': set()}
# The runtime's PROMOTABLE_STATES: reaching one is what "resolved" means for the inflow.
RESOLVED_STATES = {'finding': 'resolved', 'bug': 'resolved', 'incident': 'closed', 'decision': 'accepted'}
CHUNK_ID = re.compile(r'MEM-(?:\d{8}-[0-9a-f]{8}|\d{4,})')
ROUTES = {'claude': 'Claude hook', 'codex': 'Codex hook', 'cursor': 'Cursor hook', 'cli': 'CLI hook'}
PROVIDER_HOSTS = ('claude', 'codex', 'cursor')

_digests = OrderedDict()
_runtime_checks = {}
_cache_lock = threading.Lock()


def _day(value):
    if not isinstance(value, str):
        return None
    try:
        return date.fromisoformat(value[:10]).isoformat() if len(value) == 10 else None
    except ValueError:
        return None


def _moment(value):
    if not isinstance(value, str) or len(value) > 64:
        return None
    try:
        parsed = datetime.fromisoformat(value.replace('Z', '+00:00'))
    except ValueError:
        return None
    # One clock for every store, so moments sort as text.
    return parsed.astimezone(timezone.utc).isoformat() if parsed.tzinfo else None


def _frontmatter(text):
    if not text.startswith('---\n'):
        return None
    end = text.find('\n---\n', 4)
    try:
        value = json.loads(text[4:end]) if end >= 0 else None
    except ValueError:
        return None
    return value if isinstance(value, dict) else None


def _json(text):
    try:
        value = json.loads(text)
    except ValueError:
        return None
    return value if isinstance(value, dict) else None


def _files(project, relative, suffix, limit):
    """Regular files directly inside a project directory, newest first, without following links.

    Returns None when the directory does not exist, and the newest `limit` names
    plus whether more were present.
    """
    try:
        descriptor = open_project_path(project, relative, directory=True)
    except OSError:
        return None
    found = []
    try:
        with fs.scandir(descriptor) as names:
            for item in names:
                if item.name.startswith('.') or not item.name.endswith(suffix):
                    continue
                try:
                    info = fs.stat(item.name, dir_fd=descriptor, follow_symlinks=False)
                except OSError:
                    continue
                if stat.S_ISREG(info.st_mode):
                    found.append((info.st_mtime_ns, item.name))
    finally:
        fs.close(descriptor)
    found.sort(reverse=True)
    return [name for _, name in found[:limit]], len(found) > limit


def _directories(project, relative):
    try:
        descriptor = open_project_path(project, relative, directory=True)
    except OSError:
        return []
    found = []
    try:
        with fs.scandir(descriptor) as names:
            for item in names:
                if item.name.startswith('.'):
                    continue
                try:
                    # Windows reports a link or junction as an error, POSIX as not a directory.
                    if stat.S_ISDIR(fs.stat(item.name, dir_fd=descriptor, follow_symlinks=False).st_mode):
                        found.append(item.name)
                except OSError:
                    continue
    finally:
        fs.close(descriptor)
    return sorted(found)


def _sha256(project, relative):
    """The runtime's fingerprint of a cited file, cached by its identity; None when it cannot be read here."""
    try:
        descriptor = open_project_path(project, relative)
    except OSError:
        return False
    try:
        info = fs.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode):
            return False
        if info.st_size > SOURCE_BYTES:
            return None
        key = (str(project), relative, info.st_dev, info.st_ino, info.st_mtime_ns, info.st_size)
        with _cache_lock:
            if key in _digests:
                _digests.move_to_end(key)
                return _digests[key]
        digest = hashlib.sha256()
        while True:
            block = os.read(descriptor, 1024 * 1024)
            if not block:
                break
            digest.update(block)
        value = digest.hexdigest()
        with _cache_lock:
            _digests[key] = value
            while len(_digests) > 4096:
                _digests.popitem(last=False)
        return value
    except OSError:
        return None
    finally:
        fs.close(descriptor)


def _sources_changed(project, prefix, metadata):
    """True when a cited file no longer matches its digest, as retrieval would judge it.

    A missing or redirected file counts as changed because the runtime skips the
    chunk for it. No recorded digests, or a file too large to hash here, is unknown.
    """
    digests = metadata.get('source_digests')
    if not isinstance(digests, list) or not digests:
        return None
    unknown = False
    for item in digests:
        if not isinstance(item, dict) or not isinstance(item.get('path'), str) or not isinstance(item.get('sha256'), str):
            return True
        actual = _sha256(project, prefix + item['path'])
        if actual is None:
            unknown = True
        elif actual is False or actual != item['sha256']:
            return True
    return None if unknown else False


def _chunks(project, prefix):
    listing = _files(project, prefix + 'memory-bank/chunks', '.md', CHUNK_LIMIT)
    if listing is None:
        return None
    names, truncated = listing
    items, total = [], 0
    for name in sorted(names):
        content = read_context(project, f'{prefix}memory-bank/chunks/{name}', CHUNK_BYTES)
        if content is None:
            continue
        size, text = content
        total += size
        metadata = _frontmatter(text) or {}
        tags = metadata.get('tags') if isinstance(metadata.get('tags'), list) else []
        identity = metadata.get('id') if isinstance(metadata.get('id'), str) and CHUNK_ID.fullmatch(metadata['id']) else None
        match = CHUNK_ID.match(name)
        items.append({
            'id': identity or (match.group(0) if match else name[:-3]),
            'title': metadata['title'][:200] if isinstance(metadata.get('title'), str) else None,
            'type': metadata['type'][:40] if isinstance(metadata.get('type'), str) else None,
            'status': metadata['status'][:40] if isinstance(metadata.get('status'), str) else None,
            'created': _day(metadata.get('created')), 'last_verified': _day(metadata.get('last_verified')),
            'review_after': _day(metadata.get('review_after')), 'valid_to': _day(metadata.get('valid_to')),
            'promoted': 'promoted' in tags, 'auto': 'auto-promoted' in tags, 'bytes': size,
            'sources': len(metadata['sources']) if isinstance(metadata.get('sources'), list) else 0,
            'sources_changed': _sources_changed(project, prefix, metadata),
            'path': 'chunks/' + name,
        })
    return {'items': items, 'truncated': truncated, 'bytes': total}


def _records(project, prefix, promoted_ids):
    """Brain records as type, status and dates; private and restricted ones without type or status."""
    base = prefix + 'project-brain/'
    items, truncated, total = [], False, 0
    for folder in ('dynamic', 'archive'):
        for group in [''] + _directories(project, base + folder):
            if group in ('handoffs', 'messages'):
                continue
            relative = base + folder + ('/' + group if group else '')
            listing = _files(project, relative, '.md', RECORD_LIMIT)
            if listing is None:
                continue
            names, more = listing
            truncated = truncated or more
            for name in sorted(names):
                if len(items) >= RECORD_LIMIT or total >= RECORD_TOTAL_BYTES:
                    return items, True
                content = read_context(project, f'{relative}/{name}', RECORD_BYTES)
                if content is None:
                    continue
                total += min(content[0], RECORD_BYTES)
                record = _frontmatter(content[1])
                if not record or record.get('type') not in OPEN_STATES or not isinstance(record.get('status'), str):
                    continue
                kind, status = record['type'], record['status']
                resolved_at = None
                for transition in record.get('transitions') if isinstance(record.get('transitions'), list) else []:
                    if isinstance(transition, dict) and transition.get('to') == RESOLVED_STATES.get(kind):
                        resolved_at = _moment(transition.get('at')) or resolved_at
                # Resolved is a moment in the record's history; promotable is where it stands now.
                row = {'archived': folder == 'archive', 'open': status in OPEN_STATES[kind], 'resolved_at': resolved_at,
                       'promotable': status == RESOLVED_STATES.get(kind), 'promoted': record.get('id') in promoted_ids}
                if record.get('privacy') in ('public', 'team'):
                    row.update(type=kind, status=status[:40])
                else:
                    row['private'] = True
                items.append(row)
    return items, truncated


def _promotions(project, prefix):
    listing = _files(project, prefix + 'project-brain/control/promotions', '.json', PROMOTION_LIMIT)
    if listing is None:
        return None, set()
    names, truncated = listing
    items, promoted = [], set()
    for name in sorted(names):
        content = read_context(project, f'{prefix}project-brain/control/promotions/{name}', RECORD_BYTES)
        proposal = _json(content[1]) if content else None
        if not proposal or proposal.get('status') not in ('proposed', 'reviewed', 'rejected', 'applied'):
            continue
        sources = [item.get('id') for item in proposal.get('source_records') or [] if isinstance(item, dict)]
        if proposal['status'] != 'rejected':
            promoted.update(source for source in sources if isinstance(source, str))
        items.append({
            'status': proposal['status'], 'mode': 'automatic' if proposal.get('review_mode') == 'automatic' else 'human',
            'created_at': _moment(proposal.get('created_at')), 'reviewed_at': _moment(proposal.get('reviewed_at')),
            'applied_at': _moment(proposal.get('updated_at')) if proposal['status'] == 'applied' else None,
            'memory_id': proposal['destination_memory_id'] if isinstance(proposal.get('destination_memory_id'), str)
            and CHUNK_ID.fullmatch(proposal['destination_memory_id']) else None,
        })
    return {'items': items, 'truncated': truncated}, promoted


def _chunk_identity(path, chunk_paths):
    """A chunk ID for a retrieval path inside the bank's chunks folder; None for anything else."""
    if not isinstance(path, str) or not path.startswith('memory-bank/chunks/') or not path.endswith('.md'):
        return None
    name = path[len('memory-bank/chunks/'):]
    if '/' in name:
        return None
    match = CHUNK_ID.match(name)
    return chunk_paths.get('chunks/' + name) or (match.group(0) if match else None)


def _retrievals(project, prefix, chunk_paths, harness_tasks):
    stores = (('governed', prefix + 'project-brain/control/retrieval-manifests'),
              ('local', prefix + 'memory-bank/local/retrieval-manifests'))
    manifests = []
    for store, relative in stores:
        listing = _files(project, relative, '.json', MANIFEST_SCAN)
        for name in listing[0] if listing else []:
            content = read_context(project, f'{relative}/{name}', MANIFEST_BYTES)
            manifest = _json(content[1]) if content and content[0] <= MANIFEST_BYTES else None
            at = _moment(manifest.get('created_at')) if manifest else None
            if at and isinstance(manifest.get('selected'), list):
                manifests.append((at, store, manifest, f'{store}:{name}'))
    manifests.sort(key=lambda item: item[0])
    tasks, prepared, rows, merged = {}, {}, [], 0
    for at, store, manifest, source in manifests:
        selected = [item for item in manifest['selected'] if isinstance(item, dict)]
        key = (manifest.get('task_id'), manifest.get('task_revision'),
               frozenset((item.get('path'), item.get('source_hash')) for item in selected))
        # The Harness retrieves with the repeat gate forced off: reviewed context through
        # `retrieve` as the CLI, unattended memory through `refresh` as the provider it is
        # for. A person's retrieval and the provider hooks run under the configured gate.
        # Older manifests record no gate at all.
        gate = manifest.get('gate')
        host, entry = manifest.get('host'), manifest.get('entry_point')
        ungated = not isinstance(gate, dict) or gate.get('mode') == 'off'
        reviewed_shape = ungated and (host, entry) == ('cli', 'retrieve')
        harness_shape = reviewed_shape or ungated and host in PROVIDER_HOSTS and entry == 'refresh'
        # Reviewed context is checked for freshness by retrieving again into the local store.
        # That second manifest repeats the prepared selection; it is not another retrieval.
        if store == 'local' and reviewed_shape and key in prepared:
            prepared[key]['merged'] += 1
            merged += 1
            continue
        counts = {'brain': 0, 'bank': 0, 'rules': 0}
        chunks = []
        for item in selected:
            kind = KINDS.get(item.get('category'))
            if kind:
                counts[kind] += 1
            identity = _chunk_identity(item.get('path'), chunk_paths) if item.get('category') == 'durable' else None
            if identity:
                chunks.append(identity)
        if type(manifest.get('local_episode_count')) is int and manifest['local_episode_count'] > 0:
            counts['brain'] += manifest['local_episode_count']
        cuts = []
        for item in manifest.get('excluded') if isinstance(manifest.get('excluded'), list) else []:
            identity = _chunk_identity(item.get('path'), chunk_paths) if isinstance(item, dict) else None
            if identity and isinstance(item.get('reason'), str):
                cuts.append([identity, item['reason'][:40]])
        if host not in ROUTES:
            route = 'not recorded'
        elif entry == 'hook-context':
            route = ROUTES[host]
        elif harness_shape and manifest.get('task_id') in harness_tasks:
            route = 'Harness'
        elif host in PROVIDER_HOSTS and entry == 'refresh':
            # The Claude and Codex read hooks call `refresh` with their own host.
            route = ROUTES[host]
        else:
            route = 'CLI'
        task = tasks.setdefault(manifest.get('task_id'), len(tasks))
        row = {'key': source, 'at': at, 'version': manifest.get('schema_version') if type(manifest.get('schema_version')) is int else None,
               'route': route, 'task': task, **counts, 'chunks': chunks, 'cuts': cuts, 'merged': 0}
        if store == 'governed' and reviewed_shape:
            prepared[key] = row
        rows.append(row)
    return rows, merged


HISTORY_DAYS = 120
HISTORY_SCHEMA = '''
    CREATE TABLE IF NOT EXISTS retrieval_days (project_id TEXT NOT NULL, bank TEXT NOT NULL, day TEXT NOT NULL,
        data TEXT NOT NULL, PRIMARY KEY (project_id, bank, day));
    CREATE TABLE IF NOT EXISTS retrieval_seen (project_id TEXT NOT NULL, bank TEXT NOT NULL, manifest TEXT NOT NULL,
        day TEXT NOT NULL, PRIMARY KEY (project_id, bank, manifest));
'''


def _local_day(moment):
    return datetime.fromisoformat(moment).astimezone().date().isoformat()


def fold(store, project_id, bank_id, rows):
    """Fold retrievals into this Harness's numbers-only daily rollup and return the kept days, oldest first.

    A project keeps only its newest manifests, so history would end with them. Each
    manifest is folded once: its name is remembered until it is older than the
    horizon, and a manifest older than that is never folded.
    """
    horizon = (date.today() - timedelta(days=HISTORY_DAYS)).isoformat()
    with store.lock:
        store.db.executescript(HISTORY_SCHEMA)
        seen = {row[0] for row in store.db.execute('SELECT manifest FROM retrieval_seen WHERE project_id=? AND bank=?',
                                                   (project_id, bank_id))}
        days, fresh = {}, []
        for row in rows:
            day = _local_day(row['at'])
            if row['key'] in seen or day < horizon:
                continue
            fresh.append((project_id, bank_id, row['key'], day))
            bucket = days.setdefault(day, {'retrievals': 0, 'with_chunk': 0, 'brain': 0, 'bank': 0, 'rules': 0, 'cuts': 0,
                                           'routes': {}, 'chunks': {}})
            bucket['retrievals'] += 1
            bucket['with_chunk'] += bool(row['chunks'])
            for kind in ('brain', 'bank', 'rules'):
                bucket[kind] += row[kind]
            bucket['cuts'] += len(row['cuts'])
            bucket['routes'][row['route']] = bucket['routes'].get(row['route'], 0) + 1
            for identity in set(row['chunks']):
                bucket['chunks'][identity] = bucket['chunks'].get(identity, 0) + 1
        for day, bucket in days.items():
            existing = store.db.execute('SELECT data FROM retrieval_days WHERE project_id=? AND bank=? AND day=?',
                                        (project_id, bank_id, day)).fetchone()
            total = json.loads(existing[0]) if existing else {}
            for key, value in bucket.items():
                if isinstance(value, dict):
                    merged = total.get(key) if isinstance(total.get(key), dict) else {}
                    for name, count in value.items():
                        merged[name] = merged.get(name, 0) + count
                    total[key] = merged
                else:
                    total[key] = total.get(key, 0) + value
            store.db.execute('INSERT OR REPLACE INTO retrieval_days VALUES (?,?,?,?)', (project_id, bank_id, day, json.dumps(total)))
        store.db.executemany('INSERT OR IGNORE INTO retrieval_seen VALUES (?,?,?,?)', fresh)
        store.db.execute('DELETE FROM retrieval_seen WHERE project_id=? AND bank=? AND day<?', (project_id, bank_id, horizon))
        store.db.commit()
        kept = store.db.execute('SELECT day,data FROM retrieval_days WHERE project_id=? AND bank=? ORDER BY day DESC LIMIT 366',
                                (project_id, bank_id)).fetchall()
    return [{'day': day, **json.loads(data)} for day, data in reversed(kept)]


def fold_project(knowledge, project_id):
    """Fold every bank of a project; a session launch calls this so its retrievals outlive pruning."""
    info = knowledge.info(project_id)
    project = knowledge.sessions.project(project_id)['path']
    tasks = knowledge.sessions.linked_tasks(project_id)
    for bank in info['banks']:
        bank_info = knowledge.info(project_id, bank['id'])
        prefix = bank_info['root'] + '/' if bank_info['root'] else ''
        chunks = _chunk_names(project, prefix)
        rows, _ = _retrievals(project, prefix, chunks, tasks)
        fold(knowledge.sessions, project_id, bank['id'], rows)


def _chunk_names(project, prefix):
    """Chunk IDs by their listing path, from names alone, for folding without reading every chunk."""
    listing = _files(project, prefix + 'memory-bank/chunks', '.md', CHUNK_LIMIT)
    return {'chunks/' + name: match.group(0) for name in (listing[0] if listing else []) if (match := CHUNK_ID.match(name))}


def _health(project, prefix):
    content = read_context(project, prefix + 'memory-bank/local/refresh-health.ndjson', HEALTH_BYTES)
    if content is None:
        return None
    lines = content[1].splitlines()
    if content[0] > HEALTH_BYTES:
        lines = lines[:-1]
    items = []
    for line in lines:
        record = _json(line)
        at = _moment(record.get('at')) if record else None
        if not at:
            continue
        omitted = record.get('omitted')
        counts = [value for value in omitted.values() if type(value) is int] if isinstance(omitted, dict) else []
        items.append({'at': at, 'dropped': any(value > 0 for value in counts) if counts else None})
    return {'items': items, 'truncated': content[0] > HEALTH_BYTES}


def _runtime_check_available(project, prefix):
    """Whether the installed runtime has the policy helpers Check eligibility asks for, read as text."""
    found = True
    for relative, needle in (('memory-bank/scripts/brain_runtime.py', 'def promotable_records('),
                             ('memory-bank/scripts/context.py', 'def memory_eligibility(')):
        try:
            descriptor = open_project_path(project, prefix + relative)
        except OSError:
            return False
        try:
            info = fs.fstat(descriptor)
            key = (str(project), prefix + relative, info.st_ino, info.st_mtime_ns, info.st_size)
            with _cache_lock:
                cached = _runtime_checks.get(key)
            if cached is None:
                text = os.read(descriptor, RUNTIME_BYTES).decode('utf-8', errors='replace') if stat.S_ISREG(info.st_mode) else ''
                cached = needle in text
                with _cache_lock:
                    if len(_runtime_checks) > 64:
                        _runtime_checks.clear()
                    _runtime_checks[key] = cached
            found = found and cached
        finally:
            fs.close(descriptor)
    return found


def read(knowledge, project_id, bank=None, harness_tasks=frozenset()):
    """The Memory use payload for one knowledge root of a registered project."""
    info = knowledge.info(project_id, bank)
    project = knowledge.sessions.project(project_id)['path']
    payload = {'project_id': project_id, 'banks': info['banks'], 'bank_id': info['bank_id'], 'root': info['root'],
               'mode': info['mode'], 'brain_available': info['brain_available'], 'check_available': False,
               # Review state follows the local date, the clock the runtime's eligibility uses.
               'today': date.today().isoformat(),
               'chunks': None, 'brain': None, 'promotions': None, 'retrievals': None, 'health': None, 'history': None}
    if info['bank_id'] is None:
        return payload
    prefix = info['root'] + '/' if info['root'] else ''
    chunks = _chunks(project, prefix)
    payload['chunks'] = chunks
    chunk_paths = {item['path']: item['id'] for item in chunks['items']} if chunks else {}
    if info['brain_available'] and info['mode'] == 'governed':
        promotions, promoted_ids = _promotions(project, prefix)
        records, truncated = _records(project, prefix, promoted_ids)
        payload['brain'] = {'items': records, 'truncated': truncated}
        payload['promotions'] = promotions or {'items': [], 'truncated': False}
    rows, merged = _retrievals(project, prefix, chunk_paths, harness_tasks)
    history = fold(knowledge.sessions, project_id, info['bank_id'], rows)
    payload['retrievals'] = {'items': [{key: value for key, value in row.items() if key != 'key'} for row in rows[-RETRIEVAL_LIMIT:]],
                             'found': len(rows), 'merged': merged, 'limit': RETRIEVAL_LIMIT}
    payload['history'] = {'days': history, 'horizon': HISTORY_DAYS}
    payload['health'] = _health(project, prefix)
    payload['check_available'] = bool(info['runtime_available'] and chunks is not None
                                      and _runtime_check_available(project, prefix))
    return payload


def check(knowledge, project_id, bank=None):
    """Check eligibility: the runtime's verdicts, shaped for the page without paths or record IDs."""
    raw = knowledge.memory_use_check(project_id, bank)
    project = knowledge.sessions.project(project_id)['path']
    chunks = _chunks(project, raw['root'] + '/' if raw['root'] else '')
    paths = {item['path']: item['id'] for item in chunks['items']} if chunks else {}
    return {'project_id': project_id, 'bank_id': raw['bank_id'], **check_result(raw, paths)}


def held_back_reason(text):
    """The rule a held-back record failed, without the paths or values the runtime's sentence names."""
    if not isinstance(text, str):
        return 'other'
    for prefix, label in (('authority is', 'authority'), ('privacy', 'privacy'),
                          ('cited source changed', 'cited source changed'),
                          ('record carries no content', 'no content'), ('status', 'status')):
        if text.startswith(prefix) or (prefix == 'status' and ' status ' in text):
            return label
    return 'type' if 'not promotable' in text else 'other'


def check_result(raw, chunk_paths):
    """Shape the guarded runtime check for the page: counts of held-back reasons and skips per chunk ID."""
    if not isinstance(raw, dict):
        raise SessionError('The installed runtime could not check eligibility.')
    held = raw.get('held_back')
    skips = raw.get('skips') if isinstance(raw.get('skips'), dict) else {}
    result = {}
    for name, reason in skips.items():
        identity = _chunk_identity('memory-bank/chunks/' + name, chunk_paths) if isinstance(name, str) else None
        if identity and isinstance(reason, str):
            result[identity] = reason[:40]
    counts = None
    if isinstance(held, list):
        counts = {}
        for reason in held:
            label = held_back_reason(reason)
            counts[label] = counts.get(label, 0) + 1
    return {'checked_at': datetime.now().astimezone().isoformat(timespec='seconds'), 'held_back': counts,
            'eligible': raw['eligible'] if type(raw.get('eligible')) is int else None, 'skips': result}
