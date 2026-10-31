#!/usr/bin/env python3
"""Offline, rebuildable current facts from explicit public JSON selectors."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
from typing import Any

from memory_pilot_support import PilotError, digest, public_text, safe_read, strict_json


def instant(value: str) -> datetime:
    if not isinstance(value, str): raise PilotError('timestamp must be an aware ISO instant')
    result = datetime.fromisoformat(value.replace('Z', '+00:00'))
    if result.tzinfo is None: raise PilotError('naive timestamp refused')
    return result.astimezone(timezone.utc)


def json_pointer(value: Any, pointer: str) -> Any:
    if pointer == '': return value
    if not isinstance(pointer, str) or not pointer.startswith('/'): raise PilotError('invalid JSON Pointer')
    for token in pointer[1:].split('/'):
        if re.search(r'~(?![01])', token): raise PilotError('invalid JSON Pointer escape')
        token = token.replace('~1', '/').replace('~0', '~')
        if isinstance(value, list):
            if not re.fullmatch(r'0|[1-9][0-9]*', token): raise PilotError('invalid array index')
            value = value[int(token)]
        elif isinstance(value, dict): value = value[token]
        else: raise PilotError('JSON Pointer crosses scalar')
    return value


def collect_candidates(root: Path, selectors: list, *, as_of: str) -> list:
    moment = instant(as_of)
    if not isinstance(selectors, list) or len(selectors) > 1000: raise PilotError('invalid selector set')
    seen, result = set(), []
    required = {'id', 'namespace', 'subject', 'predicate', 'source', 'pointer', 'recorded_at_pointer'}
    optional = {'qualifiers', 'expected_sha256', 'lineage_pointer', 'revision_pointer',
                'conflicts_pointer', 'valid_from_pointer', 'valid_to_pointer'}
    for selector in selectors:
        if (not isinstance(selector, dict) or not required <= selector.keys()
                or selector.keys() - required - optional
                or any(not isinstance(selector[key], str) or not selector[key] for key in required - {'pointer'})
                or selector['id'] in seen): raise PilotError('invalid or duplicate selector')
        public_text(json.dumps(selector, ensure_ascii=False))
        seen.add(selector['id'])
        qualifiers = selector.get('qualifiers', {})
        if not isinstance(qualifiers, dict) or any(not isinstance(k, str) or not isinstance(v, str)
                                                 for k, v in qualifiers.items()):
            raise PilotError('qualifiers must be string pairs')
        row = {key: selector[key] for key in ('id', 'namespace', 'subject', 'predicate')}
        row.update(qualifiers=qualifiers, reason='unclassified', claim=None)
        try:
            if not selector['source'].endswith('.json'): raise PilotError('unsupported source kind')
            raw = safe_read(root, selector['source'])
            document = strict_json(raw)
            claim = json_pointer(document, selector['pointer'])
            if isinstance(claim, (dict, list)): raise PilotError('only scalar claims supported')
            if isinstance(claim, str) and len(claim) > 2000: raise PilotError('claim exceeds pilot limit')
            public_text(json.dumps(claim, ensure_ascii=False, allow_nan=False))
            recorded = instant(json_pointer(document, selector['recorded_at_pointer']))
            row.update(source=selector['source'], source_digest=hashlib.sha256(raw).hexdigest(),
                       recorded_at=recorded.isoformat(), claim=claim, authority='canonical-json',
                       lineage=None, revision=None, conflicts=[], reason='eligible')
            for name in ('lineage', 'revision', 'conflicts'):
                if name + '_pointer' in selector:
                    row[name] = json_pointer(document, selector[name + '_pointer'])
            if row['lineage'] is not None and not isinstance(row['lineage'], str): raise PilotError('invalid lineage')
            if row['revision'] is not None and (type(row['revision']) is not int or row['revision'] < 0):
                raise PilotError('invalid revision')
            if not isinstance(row['conflicts'], list) or any(not isinstance(v, str) for v in row['conflicts']):
                raise PilotError('invalid conflicts')
            public_text(json.dumps([row['lineage'], row['conflicts']], ensure_ascii=False))
            beginning = instant(json_pointer(document, selector['valid_from_pointer'])) if 'valid_from_pointer' in selector else recorded
            end_value = json_pointer(document, selector['valid_to_pointer']) if 'valid_to_pointer' in selector else None
            end = instant(end_value) if end_value is not None else None
            if end is not None and end <= beginning: raise PilotError('invalid validity range')
            row.update(valid_from=beginning.isoformat(), valid_to=end.isoformat() if end else None)
            if recorded > moment or beginning > moment: row['reason'] = 'future'
            elif end is not None and moment >= end: row['reason'] = 'expired'
            elif 'expected_sha256' in selector and selector['expected_sha256'] != row['source_digest']:
                row['reason'] = 'stale'
        except (OSError, ValueError, KeyError, IndexError, TypeError, UnicodeError, OverflowError):
            row = {key: row[key] for key in ('id', 'namespace', 'subject', 'predicate', 'qualifiers')}
            row.update(reason='unclassified', claim=None)
        result.append(row)
    return sorted(result, key=lambda row: row['id'])


def resolve_current(candidates: list, *, namespace: str, as_of: str) -> dict:
    moment = instant(as_of)
    groups = {}
    for candidate in candidates:
        row = dict(candidate)
        if row['namespace'] != namespace: continue
        if row['reason'] in ('eligible', 'future', 'expired'):
            start = instant(row['valid_from'])
            end = instant(row['valid_to']) if row.get('valid_to') is not None else None
            row['reason'] = ('future' if instant(row['recorded_at']) > moment or start > moment else
                             'expired' if end is not None and moment >= end else 'eligible')
        key = (row['subject'], row['predicate'], tuple(sorted(row['qualifiers'].items())))
        groups.setdefault(key, []).append(row)
    facts = []
    for key, rows in sorted(groups.items()):
        rows = sorted(rows, key=lambda row: row['id'])
        eligible = [row for row in rows if row['reason'] == 'eligible']
        conflict = any(row.get('conflicts') for row in eligible)
        # Only an explicit, shared lineage admits revision ordering.
        if eligible and len({r.get('lineage') for r in eligible}) == 1 and eligible[0].get('lineage'):
            if all(type(r.get('revision')) is int for r in eligible) and not conflict:
                revision = max(r['revision'] for r in eligible)
                eligible = [r for r in eligible if r['revision'] == revision]
        claims = {digest(row['claim']) for row in eligible}
        uncertain = any(row['reason'] in ('stale', 'unclassified') for row in rows)
        state = ('ambiguous' if conflict or len(claims) > 1 else
                 'stale' if any(row['reason'] == 'stale' for row in rows) else
                 'unclassified' if uncertain else 'current' if eligible else 'unclassified')
        facts.append({'namespace': namespace, 'subject': key[0], 'predicate': key[1],
                      'qualifiers': dict(key[2]), 'state': state,
                      'claim': eligible[0]['claim'] if state == 'current' else None,
                      'current_ids': sorted(r['id'] for r in eligible) if state == 'current' else [], 'history': rows})
    return {'schema': 'memory-current-facts-v1', 'as_of': instant(as_of).isoformat(), 'facts': facts,
            'derived': True, 'runtime_integrated': False}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--selectors', type=Path, required=True)
    parser.add_argument('--as-of', required=True)
    parser.add_argument('--json', action='store_true')
    args = parser.parse_args(argv)
    try:
        selectors = strict_json(safe_read(args.selectors.parent, args.selectors.name, memory=True, limit=1048576))
        candidates = collect_candidates(args.root, selectors, as_of=args.as_of)
        result = {'schema': 'memory-current-facts-set-v1', 'projections': [
            resolve_current(candidates, namespace=name, as_of=args.as_of)
            for name in sorted({row['namespace'] for row in candidates})]}
        print(json.dumps(result, ensure_ascii=False, allow_nan=False, indent=2))
        return 0
    except (OSError, ValueError, TypeError):
        parser.exit(2, 'current facts: invalid or unsafe input\n')


if __name__ == '__main__': raise SystemExit(main())
