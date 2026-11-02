#!/usr/bin/env python3
"""Read-only maintenance packets over the canonical bank audit, off prompt path."""
from __future__ import annotations

import argparse
from datetime import date, datetime, timedelta, timezone
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

from memory_pilot_support import (ROOT, CANON, PilotError, canonical_module, child_env, copy_snapshot,
                                  digest, public_text, relative_path, safe_read, snapshot_digest, strict_json)

EVIDENCE_PROFILE = 'memory-maintenance-evidence-v1'
MAX_SELECTED = 50
SOURCE_BYTES = 262144
SOURCE_TOTAL = 524288
PACKET_BYTES = 1048576
ARCHIVED_ORIGIN = re.compile(r'project-brain/archive/finding/([0-9a-f-]{36})\.md\Z')


def build_review_packet(audit: dict, *, snapshot_digest: str, limit: int = 50) -> dict:
    if type(limit) is not int or limit < 1 or limit > 10000: raise PilotError('invalid review limit')
    if not isinstance(audit, dict): raise PilotError('invalid audit')
    entries, sources, dates, identifiers = {}, {}, {}, {}
    def add(path, reason):
        relative_path(path, memory=True)
        if not path.startswith('memory-bank/chunks/') or not path.endswith('.md'):
            raise PilotError('invalid audit chunk')
        public_text(path)
        entries.setdefault(path, set()).add(reason)
    for row in audit.get('source_changed', []):
        if row.get('state') not in ('changed', 'missing'): raise PilotError('invalid source state')
        add(row['chunk'], row['state'])
        detail = {'state': row['state'], 'source_withheld': True}
        try:
            relative_path(row.get('source')); public_text(row['source'])
            detail.update(source=row['source'], source_withheld=False)
        except (TypeError, ValueError): pass
        for key in ('expected_sha256', 'current_sha256'):
            value = row.get(key)
            if isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value): detail[key] = value
        sources.setdefault(row['chunk'], []).append(detail)
        if isinstance(row.get('id'), str) and re.fullmatch(r'MEM-[A-Za-z0-9-]+', row['id']):
            identifiers[row['chunk']] = row['id']
    for row in audit.get('overdue_review', []):
        add(row['chunk'], 'overdue')
        value = row.get('review_after')
        if isinstance(value, str) and re.fullmatch(r'\d{4}-\d{2}-\d{2}', value):
            dates.setdefault(row['chunk'], set()).add(value)
    for path in audit.get('undigested', []): add(path, 'undigested')
    for path in audit.get('provenance', {}).get('instruction_like', []): add(path, 'instruction-like')
    duplicates = audit.get('duplicates', [])
    priority = {'instruction-like': 0, 'missing': 1, 'changed': 2, 'overdue': 3, 'undigested': 4, 'duplicate': 5}
    # Duplicate ids are not paths; keep a separate unresolved pair, never choose a survivor.
    for row in duplicates:
        pair = sorted([row['left'], row['right']])
        if any(not isinstance(value, str) or not re.fullmatch(r'MEM-[A-Za-z0-9-]+', value) for value in pair):
            raise PilotError('invalid duplicate identifiers')
        public_text(json.dumps(pair))
        entries.setdefault('pair:' + '|'.join(pair), set()).add('duplicate')
    queue = []
    for path, reasons in sorted(entries.items(), key=lambda item: (min(priority[r] for r in item[1]), item[0])):
        queue.append({'chunk': path if not path.startswith('pair:') else None,
            'id': identifiers.get(path),
            'duplicate_ids': path[5:].split('|') if path.startswith('pair:') else [],
            'sources': sorted(sources.get(path, []), key=lambda row: (row.get('source', ''), row['state'])),
            'review_dates': sorted(dates.get(path, [])),
            'reasons': sorted(reasons, key=lambda reason: priority[reason]), 'review_required': True,
            'required_evidence': 'Read the claim and current canonical sources; verify applicability before mutation.',
            'allowed_actions': ['investigate', 'quarantine'] if 'instruction-like' in reasons else
                ['investigate', 'reverify-after-check', 'replace', 'retire']})
    return {'schema': 'memory-freshness-plan-v1', 'snapshot_digest': snapshot_digest,
            'queue': queue[:limit], 'limit': limit, 'total_candidates': len(queue),
            'returned': min(limit, len(queue)), 'truncated': len(queue) > limit,
            'remaining': max(0, len(queue) - limit), 'read_only': True, 'attestation': 'not-performed',
            'audit_limit_applies': False, 'audit_chunks': audit.get('chunks', 0)}


def _audit_copy(root: Path) -> dict:
    from memory_pilot_support import canonical_module
    brain = canonical_module('brain_runtime')
    audit = brain.audit_bank(root, provenance=True)
    for row in audit['source_changed']:
        try:
            relative_path(row['chunk'], memory=True)
            metadata, _ = brain._parse_chunk(root / row['chunk'])
            row['expected_sha256'] = next((entry['sha256'] for entry in metadata.get('source_digests', [])
                                           if entry.get('path') == row['source']), None)
            relative_path(row['source'])
            row['current_sha256'] = brain.fingerprint(root, row['source'])['sha256']
        except (OSError, ValueError, brain.BrainError):
            pass
    return audit


def plan_reviews(root: Path, *, limit: int = 50) -> dict:
    with tempfile.TemporaryDirectory(prefix='memory-freshness-') as temporary:
        copy = Path(temporary) / 'snapshot'
        image = copy_snapshot(Path(root), copy)
        code = ('import sys; from pathlib import Path; sys.path.insert(0,sys.argv[1]); '
                'from memory_freshness_plan import _audit_copy; import json; '
                'print(json.dumps(_audit_copy(Path(sys.argv[2]))))')
        done = subprocess.run([sys.executable, '-I', '-B', '-c', code, str(ROOT / 'scripts'), str(copy)],
            cwd=copy, env=child_env(), capture_output=True, timeout=120)
        if done.returncode: raise PilotError('canonical audit unavailable')
        if snapshot_digest(Path(root)) != image: raise PilotError('input changed during audit')
        return build_review_packet(json.loads(done.stdout), snapshot_digest=image, limit=limit)


def _seal(value):
    return {**value, 'sha256': digest(value)}


def _check_seal(value):
    if (type(value) is not dict or not isinstance(value.get('sha256'), str)
            or value['sha256'] != digest({key: item for key, item in value.items() if key != 'sha256'})):
        raise PilotError('evidence image changed')


def _selection(selected_chunks, limit):
    if type(limit) is not int or not 1 <= limit <= MAX_SELECTED:
        raise PilotError('invalid evidence limit')
    if selected_chunks is not None:
        if (type(selected_chunks) is not list or not 1 <= len(selected_chunks) <= limit
                or any(type(name) is not str for name in selected_chunks)
                or len(set(selected_chunks)) != len(selected_chunks)):
            raise PilotError('invalid explicit selection')
        for name in selected_chunks:
            relative_path(name, memory=True); public_text(name)
            if not re.fullmatch(r'memory-bank/chunks/[^/]+\.md', name):
                raise PilotError('invalid selected chunk')


def _runtime_results():
    # Only trusted tooling is executable. No imports are resolved from the copy.
    sys.path.insert(0, str(CANON))
    try:
        import memory_results
        return memory_results
    finally:
        sys.path.pop(0)


def _applicable_now(metadata):
    """Replacement applicability, using one inclusive calendar observation."""
    today = date.today()
    for key in ('valid_from', 'valid_to', 'review_after'):
        if key not in metadata:
            if key == 'review_after': return False
            continue
        value = metadata[key]
        if key == 'valid_to' and value is None: continue
        if not isinstance(value, str) or re.fullmatch(r'\d{4}-\d{2}-\d{2}', value) is None:
            return False
        try: boundary = date.fromisoformat(value)
        except ValueError: return False
        if (key == 'valid_from' and boundary > today) or (key != 'valid_from' and boundary < today):
            return False
    return True


def _successor_applicable_now(root, row):
    raw = safe_read(root, row['chunk'], memory=True, limit=65536)
    if hashlib.sha256(raw).hexdigest() != row['chunk_sha256']: return False
    metadata, _ = canonical_module('brain_runtime').parse_markdown_record(
        Path(row['chunk']), text=raw.decode('utf-8'))
    return _applicable_now(metadata)


def _frontier_matches(root, row, results, *, chunk=True):
    return ((not chunk or hashlib.sha256(safe_read(root, row['chunk'], memory=True)).hexdigest()
             == row['chunk_sha256'])
            and hashlib.sha256(safe_read(root, row['record_path'], memory=True)).hexdigest()
            == row['record_sha256']
            and results.checked_fingerprints(root, row['record_sources']) == row['source_fingerprints'])


def _evidence_copy(root, selected_chunks, limit, owner):
    _selection(selected_chunks, limit)
    public_text(owner)
    brain = canonical_module('brain_runtime')
    results = _runtime_results()
    config = brain.load_config(root)
    if selected_chunks is None:
        plan = build_review_packet(_audit_copy(root), snapshot_digest=snapshot_digest(root), limit=limit)
        offered = [{'chunk': row['chunk'], 'reasons': row['reasons']} for row in plan['queue']]
        total, truncated = plan['total_candidates'], plan['truncated']
    else:
        offered = [{'chunk': name, 'reasons': ['explicit-selection']} for name in selected_chunks]
        total, truncated = len(offered), False
    items, total_source_bytes = [], 0
    for candidate in offered:
        row = {**candidate, 'state': 'held', 'reason': 'unsupported', 'sources': []}
        try:
            if candidate['chunk'] is None:
                raise PilotError('duplicate-unresolved')
            raw = safe_read(root, candidate['chunk'], memory=True, limit=65536)
            text = raw.decode('utf-8'); public_text(text)
            metadata, body = brain.parse_markdown_record(Path(candidate['chunk']), text=text)
            if (metadata.get('status') != 'active'
                    or metadata.get('privacy', 'team') not in set(config['allowed_privacy']) & {'public', 'team'}
                    or metadata.get('authority', 'observed') not in config['allowed_authority']
                    or brain.instruction_like(metadata.get('title'), body)):
                raise PilotError('chunk-not-admitted')
            links = [pin for pin in metadata.get('source_digests', []) if isinstance(pin, dict)
                     and isinstance(pin.get('path'), str) and (re.fullmatch(
                         r'project-brain/dynamic/(?:findings|decisions)/[^/]+\.md', pin['path'])
                         or ARCHIVED_ORIGIN.fullmatch(pin['path']))]
            if len(links) != 1:
                raise PilotError('unsupported-provenance')
            record_path = links[0]['path']
            record_raw = safe_read(root, record_path, memory=True, limit=65536)
            public_text(record_raw.decode('utf-8'))
            if brain.fingerprint(root, record_path)['sha256'] != links[0].get('sha256'):
                raise PilotError('record-image-changed')
            record, _ = brain.parse_markdown_record(Path(record_path), text=record_raw.decode('utf-8'))
            archived = ARCHIVED_ORIGIN.fullmatch(record_path)
            brain.validate_record(record, archived=bool(archived))
            if archived:
                found, unique, _ = brain.find_record(root, record['id'], include_archive=True)
                if (record['type'] != 'finding' or archived[1] != record['id']
                        or found.relative_to(root).as_posix() != record_path or unique != record):
                    raise PilotError('archive-origin-identity-changed')
            if (record['type'] not in results.CLOSED_STATE or record['status'] != results.CLOSED_STATE[record['type']]
                    or record['authority'] != 'verified' or brain.record_attestation(record) != 'agent'
                    or record['superseded_by'] or owner not in record['authorized_owners']
                    or not (set(config['owners']) & {'*', record['owner']})
                    or record['privacy'] not in set(config['allowed_privacy']) & {'public', 'team'}
                    or record['authority'] not in config['allowed_authority']):
                raise PilotError('record-not-admitted')
            pins = results.checked_fingerprints(root, record['sources'])
            expected_paths = {pin['path'] for pin in pins} | {record_path}
            chunk_pins = brain.canonical_source_image(metadata.get('source_digests'))
            if (type(metadata.get('sources')) is not list
                    or any(type(source) is not str for source in metadata['sources'])
                    or {source.split('#', 1)[0] for source in metadata['sources']} != expected_paths
                    or {pin['path'] for pin in chunk_pins} != expected_paths):
                raise PilotError('unsupported chunk source coverage')
            sources, source_bytes = [], 0
            for pin in pins:
                relative_path(pin['path'])
                source = safe_read(root, pin['path'], limit=SOURCE_BYTES)
                source_text = source.decode('utf-8'); public_text(source_text)
                if '\0' in source_text:
                    raise PilotError('source-unavailable')
                if brain.fingerprint(root, pin['path']) != pin:
                    raise PilotError('source-image-changed')
                source_bytes += len(source)
                if total_source_bytes + source_bytes > SOURCE_TOTAL:
                    raise PilotError('source-budget')
                sources.append({**pin, 'raw_sha256': hashlib.sha256(source).hexdigest(), 'text': source_text})
            consequence = record.get('progress') or record['goal']
            if not consequence or len(consequence) > 1000 or consequence != ' '.join(consequence.split()):
                raise PilotError('unsupported-claim-rendering')
            total_source_bytes += source_bytes
            chunk_fresh = bool(metadata.get('source_digests')) and all(
                brain.fingerprint(root, pin['path'])['sha256'] == pin['sha256']
                for pin in metadata['source_digests'])
            row.update(state='ready', reason=None, memory_id=metadata['id'], chunk_sha256=hashlib.sha256(raw).hexdigest(),
                body=body, title=metadata['title'], record_path=record_path,
                record_sha256=hashlib.sha256(record_raw).hexdigest(), record_id=record['id'],
                expected_revision=record['revision'], type=record['type'], record_title=record['title'],
                consequence=consequence, record_sources=record['sources'], sources=sources,
                source_fingerprints=pins, previous_source_fingerprints=record['source_fingerprints'],
                source_image_matches_record=pins == brain.canonical_source_image(record['source_fingerprints']),
                chunk_sources_fresh=chunk_fresh, currently_applicable=_applicable_now(metadata))
            if archived:
                row.update(origin_placement='archive', maintenance_capabilities={
                    'confirm': 'unchanged-evidence-only',
                    'correct': 'unchanged-consequence-and-evidence-only',
                    'retire': 'selected-fresh-successor', 'defer': 'supported',
                    'origin_record_mutable': False})
        except (OSError, ValueError, TypeError, KeyError, brain.BrainError, results.BrainError):
            # Neither private content nor caller exception prose crosses this boundary.
            row = {**candidate, 'state': 'held', 'reason': 'evidence-unavailable-or-unsupported', 'sources': []}
        items.append(_seal(row))
    packet = _seal({'schema': EVIDENCE_PROFILE, 'snapshot_digest': snapshot_digest(root), 'owner': owner,
        'selection': selected_chunks, 'limit': limit, 'total_candidates': total,
        'truncated': truncated, 'remaining': max(0, total-len(items)), 'items': items,
        'read_only': True, 'attestation': 'not-performed', 'source_evidence': 'captured-current-bytes',
        'semantic_verification': 'not-performed'})
    if len(json.dumps(packet, ensure_ascii=False).encode()) > PACKET_BYTES:
        raise PilotError('evidence packet exceeds byte limit')
    return packet


def _worker(root, operation, data):
    if operation == 'evidence':
        return _evidence_copy(root, data['selected_chunks'], data['limit'], data['owner'])
    if operation != 'replay':
        raise PilotError('unknown maintenance operation')
    packet, decisions = data['packet'], data['decisions']
    current = _evidence_copy(root, packet['selection'], packet['limit'], packet['owner'])
    if current != packet:
        raise PilotError('evidence image changed before replay')
    brain = canonical_module('brain_runtime'); results = _runtime_results()
    # Copy-only overrides prevent record_result's broad turn-boundary promotion
    # and all unrelated consolidation/compaction. Targeted promotion is explicit.
    schemas = root/'project-brain/schemas'
    if schemas.exists(): shutil.rmtree(schemas)
    shutil.copytree(ROOT/'PHP Core/project-brain/schemas', schemas)
    configuration = root/'project-brain/config/runtime.json'
    flags = strict_json(safe_read(root, 'project-brain/config/runtime.json')) if configuration.exists() else {}
    overrides = {name: {'input': flags.get(name, brain.load_config(root).get(name)), 'copy': False}
                 for name in ('automatic_promotion', 'automatic_consolidation',
                              'automatic_consolidation_verification', 'automatic_compaction')}
    for name in overrides: flags[name] = False
    overrides['automatic_promotion']['targeted_selected_records_only'] = True
    configuration.parent.mkdir(parents=True, exist_ok=True)
    configuration.write_text(json.dumps(flags), encoding='utf-8')
    owner = packet['owner']; task = None; actions = []
    by_chunk = {}
    for decision in decisions:
        if type(decision) is not dict or type(decision.get('chunk')) is not str:
            raise PilotError('invalid maintenance decision')
        by_chunk.setdefault(decision['chunk'], []).append(decision)
    offered = {row['chunk']: row for row in packet['items'] if row['chunk'] is not None}
    if set(by_chunk) - set(offered):
        raise PilotError('decision outside selected evidence')
    for row in packet['items']:
        action = {'chunk': row['chunk'], 'state': 'held', 'reason': 'decision-or-evidence-unavailable'}
        effect_started = False
        try:
            chosen = by_chunk.get(row['chunk'], [])
            if len(chosen) != 1 or row['state'] != 'ready':
                actions.append(action); continue
            decision = chosen[0]
            if decision.get('action') == 'defer' and set(decision) == {'chunk','row_sha256','action','reason'}:
                if (decision['row_sha256'] != row['sha256'] or type(decision['reason']) is not str
                        or not 1 <= len(decision['reason']) <= 1000):
                    raise PilotError('invalid defer')
                public_text(decision['reason'])
                actions.append({**action, 'state': 'deferred', 'reason': 'explicit-defer'}); continue
            fields = {'chunk','row_sha256','action','attestation','reason','evidence'}
            kind = decision.get('action')
            extra = {'consequence'} if kind == 'correct' else {'successor','valid_to'} if kind == 'retire' else set()
            if (kind not in ('confirm','correct','retire') or set(decision) != fields | extra
                    or decision['row_sha256'] != row['sha256'] or decision['attestation'] != 'agent'
                    or type(decision['reason']) is not str or not 1 <= len(decision['reason']) <= 1000):
                raise PilotError('invalid checked decision')
            public_text(decision['reason'])
            citations = decision['evidence']; sources = {source['path']:source for source in row['sources']}
            if type(citations) is not list or not 1 <= len(citations) <= 20:
                raise PilotError('missing checked evidence')
            covered = set()
            for citation in citations:
                if type(citation) is not dict or set(citation) != {'path','sha256','quote'}:
                    raise PilotError('invalid checked evidence')
                source = sources.get(citation['path'])
                if (source is None or citation['sha256'] != source['sha256']
                        or type(citation['quote']) is not str or not citation['quote'].strip()
                        or len(citation['quote']) > 2000 or citation['quote'] not in source['text']):
                    raise PilotError('unchecked evidence')
                covered.add(citation['path'])
            if covered != set(sources):
                raise PilotError('incomplete checked evidence')
            if (hashlib.sha256(safe_read(root,row['chunk'],memory=True)).hexdigest()!=row['chunk_sha256']
                    or hashlib.sha256(safe_read(root,row['record_path'],memory=True)).hexdigest()!=row['record_sha256']
                    or results.checked_fingerprints(root,row['record_sources'])!=row['source_fingerprints']):
                raise PilotError('selected frontier changed')
            if kind == 'retire':
                successor = next((item for item in packet['items'] if item.get('memory_id')==decision['successor']),None)
                if (successor is None or successor['state']!='ready' or successor is row
                        or not successor['chunk_sources_fresh'] or not successor['source_image_matches_record']
                        or not successor['currently_applicable']):
                    raise PilotError('fresh selected successor required')
                with brain.mutation_lock(root):
                    if (not all(_frontier_matches(root, item, results) for item in (row, successor))
                            or not _successor_applicable_now(root, successor)):
                        raise PilotError('retirement evidence image changed')
                    images = [brain.parse_markdown_record(Path(item['chunk']), text=safe_read(
                        root, item['chunk'], memory=True, limit=65536).decode('utf-8'))
                        for item in (row, successor)]
                    effect_started = True
                    outcome=brain.retire_chunk(root,row['memory_id'],valid_to=decision['valid_to'],
                        superseded_by=successor['memory_id'],reason=decision['reason'])
                    expected_old, old_body = images[0]
                    expected_new, new_body = images[1]
                    expected_old.update(status='superseded', superseded_by=successor['memory_id'],
                                        valid_to=date.fromisoformat(decision['valid_to']).isoformat())
                    expected_new['supersedes'] = list(expected_new.get('supersedes') or [])
                    if row['memory_id'] not in expected_new['supersedes']:
                        expected_new['supersedes'].append(row['memory_id'])
                    after = [brain.parse_markdown_record(Path(item['chunk']), text=safe_read(
                        root, item['chunk'], memory=True, limit=65536).decode('utf-8'))
                        for item in (row, successor)]
                    if (after != [(expected_old, old_body), (expected_new, new_body)]
                            or not _applicable_now(after[1][0])
                            or not all(_frontier_matches(root, item, results, chunk=False)
                                       for item in (row, successor))):
                        raise PilotError('retirement evidence changed during operation')
                action.update(state='applied',reason=None,result=outcome,attestation='agent')
            else:
                consequence = decision['consequence'] if kind=='correct' else row['consequence']
                if (type(consequence) is not str or consequence!=' '.join(consequence.split())
                        or not 1<=len(consequence)<=1000):
                    raise PilotError('unsupported claim rendering')
                public_text(consequence)
                archived = row.get('origin_placement') == 'archive'
                if archived and (consequence != row['consequence'] or not row['chunk_sources_fresh']
                                 or not row['source_image_matches_record']):
                    action.update(reason='archived-origin-revision-required')
                    actions.append(action); continue
                if (consequence == row['consequence'] and row['chunk_sources_fresh']
                        and row['source_image_matches_record']):
                    with brain.mutation_lock(root):
                        raw = safe_read(root, row['chunk'], memory=True, limit=65536)
                        if hashlib.sha256(raw).hexdigest() != row['chunk_sha256']:
                            raise PilotError('renewal chunk image changed')
                        metadata, body = brain._publication_metadata(raw)
                        review = metadata.get('review_after')
                        if isinstance(review, str) and date.fromisoformat(review) < date.today():
                            expected = brain.canonical_source_image(metadata.get('source_digests'))
                            renewal_pins = brain.chunk_source_digests(root, metadata['sources'])
                            if (brain.canonical_source_image(renewal_pins) != expected
                                    or results.checked_fingerprints(root, row['record_sources']) != row['source_fingerprints']
                                    or hashlib.sha256(safe_read(root, row['record_path'], memory=True)).hexdigest() != row['record_sha256']):
                                raise PilotError('renewal source image changed')
                            effect_started = True
                            started = datetime.now(timezone.utc).date()
                            renewal = brain.reverify_chunk(root, row['memory_id'])
                            finished = datetime.now(timezone.utc).date()
                            after_raw = safe_read(root, row['chunk'], memory=True, limit=65536)
                            after_metadata, after_body = brain._publication_metadata(after_raw)
                            # Canonical renewal changes only these three fields. Its
                            # dates use UTC; crossing midnight admits either observed day.
                            legal_metadata = (started <= finished <= started+timedelta(days=1)
                                and any(after_metadata == dict(metadata,
                                    last_verified=day.isoformat(),
                                    review_after=(day+timedelta(days=365)).isoformat(),
                                    source_digests=renewal_pins) for day in {started, finished}))
                            if (after_body != body or not legal_metadata or type(renewal) is not dict
                                    or renewal.get('id') != row['memory_id'] or renewal.get('chunk') != row['chunk']
                                    or any(renewal.get(field) != after_metadata.get(field)
                                           for field in ('last_verified', 'review_after', 'source_digests'))
                                    or brain.canonical_source_image(renewal['source_digests']) != expected
                                    or results.checked_fingerprints(root, row['record_sources']) != row['source_fingerprints']
                                    or hashlib.sha256(safe_read(root, row['record_path'], memory=True)).hexdigest() != row['record_sha256']
                                    or safe_read(root, row['chunk'], memory=True, limit=65536) != after_raw):
                                raise PilotError('renewal evidence changed during check')
                            action.update(state='applied', reason=None, renewal=renewal, attestation='agent')
                            actions.append(action)
                            continue
                        if archived:
                            if not _frontier_matches(root, row, results):
                                raise PilotError('archive confirmation evidence changed')
                            action.update(state='applied', reason=None, unchanged=True,
                                          record_id=row['record_id'], attestation='agent')
                            actions.append(action)
                            continue
                effect_started = True
                if task is None:
                    task=brain.create_task(root,'memory-maintenance/'+packet['sha256'][:16],
                        'Apply explicitly checked memory maintenance on a private copy',[],[],owner=owner)
                payload={'verified':True,'learnings':[{'type':row['type'],'title':row['record_title'],
                    'consequence':consequence,'sources':row['record_sources'],'record_id':row['record_id'],
                    'expected_revision':row['expected_revision'],'source_fingerprints':row['source_fingerprints']}]}
                outcome=results.record_result(root,task['id'],'review-'+row['sha256'][:48],'auto',payload,
                    owner=owner,reason=decision['reason'],attestation='agent')
                if any(item['state'] not in ('updated','existing') for item in outcome['records']):
                    raise PilotError('canonical checked save held')
                flags['automatic_promotion']=True
                configuration.write_text(json.dumps(flags),encoding='utf-8')
                try:
                    promotion=brain.auto_promote(root,owner=owner,only={row['record_id']},limit=1,max_attempts=1)
                finally:
                    flags['automatic_promotion']=False
                    configuration.write_text(json.dumps(flags),encoding='utf-8')
                if promotion['failed'] or promotion['blocked'] or promotion.get('retry_unavailable'):
                    action.update(state='failed',reason='targeted-promotion-incomplete',record_saved=True)
                else:
                    action.update(state='applied',reason=None,record_id=row['record_id'],
                                  promotion=promotion,attestation='agent')
        except (OSError,ValueError,TypeError,KeyError,brain.BrainError,results.BrainError):
            action.update(state='failed' if effect_started else 'held',reason='checked-operation-refused',
                          copy_effects_may_have_applied=effect_started)
        actions.append(action)
    audit=build_review_packet(_audit_copy(root),snapshot_digest=snapshot_digest(root),limit=packet['limit'])
    errors=brain.validate_bank(root/'memory-bank')
    return {'schema':'memory-maintenance-replay-v1','input_digest':packet['snapshot_digest'],
        'packet_sha256':packet['sha256'],'actions':actions,'audit_after':audit,
        'validation_errors':len(errors),'copy_overrides':overrides,
        'tooling_overrides':{'schemas':'trusted-canonical-copy',
            'runtime_sha256':digest({path.name:hashlib.sha256(path.read_bytes()).hexdigest()
                                    for path in sorted(CANON.glob('*.py'))}),
            'schema_sha256':digest({path.name:hashlib.sha256(path.read_bytes()).hexdigest()
                                   for path in sorted(schemas.glob('*.json'))})},
        'targeted_promotion_only':True,'attestation':'agent-supplied-decisions',
        'semantic_verification':'not-independently-established','remaining':packet['remaining'],
        'success':bool(actions) and all(item['state'] in ('applied','deferred') for item in actions) and not errors}


def _run_copy_worker(copy, operation, data):
    code=('import sys,json; from pathlib import Path; sys.path.insert(0,sys.argv[1]); '
          'from memory_freshness_plan import _worker; '
          'print(json.dumps(_worker(Path(sys.argv[2]),sys.argv[3],json.load(sys.stdin))))')
    done=subprocess.run([sys.executable,'-I','-B','-c',code,str(ROOT/'scripts'),str(copy),operation],
        input=json.dumps(data,ensure_ascii=False,allow_nan=False).encode(),
        cwd=copy,env=child_env(),capture_output=True,timeout=120)
    if done.returncode:raise PilotError('maintenance evidence or checked replay unavailable')
    return strict_json(done.stdout)


def evidence_plan(root: Path, *, selected_chunks=None, limit=50, owner='local'):
    """Opt-in full source capture; hashes describe bytes, not semantic truth.

    Explicit selectors also capture fresh knowledge before a future source edit.
    A previously exported packet remains a historical observation; replay always
    requires its original current input image and never reconstructs old code.
    """
    _selection(selected_chunks,limit)
    with tempfile.TemporaryDirectory(prefix='memory-evidence-') as temporary:
        copy=Path(temporary)/'snapshot';before=copy_snapshot(Path(root),copy)
        packet=_run_copy_worker(copy,'evidence',{'selected_chunks':selected_chunks,'limit':limit,'owner':owner})
        if snapshot_digest(Path(root))!=before:raise PilotError('input changed during evidence capture')
        return packet


def replay_reviews(root: Path, packet, decisions, *, export: Path):
    """Apply supplied checked decisions solely on a new physical export copy."""
    _check_seal(packet)
    if (packet.get('schema')!=EVIDENCE_PROFILE or type(decisions) is not list
            or len(decisions)>MAX_SELECTED or type(packet.get('items')) is not list
            or not 1<=len(packet['items'])<=MAX_SELECTED
            or len(json.dumps(decisions,ensure_ascii=False,allow_nan=False).encode('utf-8'))>131072
            or len(json.dumps(packet,ensure_ascii=False,allow_nan=False).encode('utf-8'))>PACKET_BYTES):
        raise PilotError('invalid bounded maintenance input')
    for row in packet['items']:_check_seal(row)
    root,export=Path(root),Path(export)
    if (export.exists() or export.is_symlink() or export.resolve()==root.resolve()
            or root.resolve() in export.resolve().parents or export.resolve() in root.resolve().parents
            or export.resolve()==ROOT or ROOT in export.resolve().parents):
        raise PilotError('export must be a new directory outside input and tooling')
    with tempfile.TemporaryDirectory(prefix='memory-maintenance-') as temporary:
        copy=Path(temporary)/'snapshot';before=copy_snapshot(root,copy)
        if before!=packet['snapshot_digest']:raise PilotError('input image changed since evidence capture')
        report=_run_copy_worker(copy,'replay',{'packet':packet,'decisions':decisions})
        if snapshot_digest(root)!=before:raise PilotError('input changed during checked replay')
        copy_snapshot(copy,export)
        report.update(export_digest=snapshot_digest(export),input_unchanged=True,copy_only=True)
        return report


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--limit', type=int, default=50)
    parser.add_argument('--json', action='store_true')
    parser.add_argument('--evidence', action='store_true', help='opt-in complete source evidence capture')
    parser.add_argument('--chunk', action='append', help='explicit fresh or stale chunk for evidence capture')
    parser.add_argument('--packet', type=Path, help='previously captured evidence packet for copy-only replay')
    parser.add_argument('--decisions', type=Path, help='supplied agent-attested checked decisions')
    parser.add_argument('--export', type=Path, help='new copy-only replay export outside input/tooling')
    parser.add_argument('--out', type=Path, help='private evidence/report outside input/tooling')
    parser.add_argument('--owner', default='local')
    args = parser.parse_args(argv)
    try:
        if args.out:
            from memory_eval import inside_clone, write_json
            if (inside_clone(args.out) or args.out.resolve()==args.root.resolve()
                    or args.root.resolve() in args.out.resolve().parents):
                raise PilotError('private report must be outside input/tooling')
        if args.packet:
            if not args.decisions or not args.export or args.evidence or args.chunk:
                raise PilotError('replay requires packet, decisions and export')
            result=replay_reviews(args.root,strict_json(safe_read(args.packet.parent,args.packet.name,
                                  memory=True,limit=PACKET_BYTES)),strict_json(safe_read(args.decisions.parent,
                                  args.decisions.name,memory=True,limit=131072)),export=args.export)
        elif args.evidence:
            if args.decisions or args.export:raise PilotError('unexpected replay argument')
            result=evidence_plan(args.root,selected_chunks=args.chunk,limit=args.limit,owner=args.owner)
        else:
            if args.chunk or args.decisions or args.export:raise PilotError('explicit evidence or replay required')
            result=plan_reviews(args.root,limit=args.limit)
        if args.out:
            write_json(args.out,result)
        else:
            print(json.dumps(result, ensure_ascii=False, allow_nan=False, indent=2))
        return 0
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError):
        parser.exit(2, 'freshness plan: unsafe input or audit unavailable\n')


if __name__ == '__main__': raise SystemExit(main())
