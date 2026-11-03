#!/usr/bin/env python3
"""Offline facts from explicit JSON or opt-in canonical frontmatter selectors."""
from __future__ import annotations

import argparse
from datetime import date, datetime, timedelta, timezone
import hashlib
import json
from pathlib import Path
import re
import stat
from typing import Any

from memory_pilot_support import (MAX_FILE, MAX_TREE, PilotError, canonical_module,
                                  digest, public_text, safe_read, strict_json)


ADAPTER_LIMITS = {'max_file_bytes': MAX_FILE, 'max_total_read_bytes': MAX_TREE,
                  'max_unique_reads': 1000, 'max_sources_per_record': 100,
                  'max_claim_characters': 2000}
_BRAIN_PATH = re.compile(r'project-brain/dynamic/(findings|bugs|incidents|decisions)/'
                         r'([0-9a-f-]{36})\.md\Z')


def _frontmatter(raw: bytes) -> tuple[dict, str, str]:
    # Parse one image while keeping the explicitly selected body exact.
    text = raw.decode('utf-8')
    opening = re.match(r'---\r?\n', text)
    if opening is None: raise PilotError('missing frontmatter')
    header, body = re.split(r'\r?\n---\r?\n', text[opening.end():], maxsplit=1)
    metadata = strict_json(header.encode('utf-8'))
    if not isinstance(metadata, dict): raise PilotError('frontmatter must be an object')
    return metadata, body, text


class _AdapterReader:
    """Bounded, call-local images; no index, filesystem cache or attachment lookup."""
    def __init__(self, root: Path):
        self.root, self.images, self.total = root, {}, 0
        self.documents, self.origins, self.hashes = {}, {}, {}
        self.admission_config = None

    def admission(self) -> dict:
        if self.admission_config is None:
            config = {'allowed_privacy': ['public', 'team'],
                      'allowed_authority': ['observed', 'verified']}
            name = 'project-brain/config/runtime.json'
            path = self.root
            for part in name.split('/'):
                path = path / part
                try:
                    image = path.lstat()
                except FileNotFoundError:
                    break  # Only observed ordinary absence permits canonical defaults.
                if stat.S_ISLNK(image.st_mode) or getattr(image, 'st_file_attributes', 0) & 0x400:
                    raise PilotError('linked admission configuration')
            else:
                value = strict_json(self.read(name, memory=True))
                if not isinstance(value, dict): raise PilotError('invalid admission configuration')
                config.update(value)
            for key in ('allowed_privacy', 'allowed_authority'):
                if (type(config[key]) is not list
                        or any(type(item) is not str for item in config[key])):
                    raise PilotError('invalid admission configuration')
            self.admission_config = config
        return self.admission_config

    def read(self, name: str, *, memory: bool = False) -> bytes:
        key = (name, memory)
        if key not in self.images:
            if len(self.images) >= ADAPTER_LIMITS['max_unique_reads']:
                raise PilotError('adapter read limit')
            raw = safe_read(self.root, name, memory=memory,
                            limit=min(MAX_FILE, MAX_TREE - self.total))
            self.total += len(raw)
            self.images[key] = raw
        return self.images[key]

    def document(self, name: str) -> tuple[dict, str, str]:
        if name not in self.documents:
            metadata, body, text = _frontmatter(self.read(name, memory=True))
            public_text(text)
            if canonical_module('brain_runtime').instruction_like(metadata.get('title'), body):
                raise PilotError('instruction-like source')
            self.documents[name] = metadata, body, text
        return self.documents[name]

    def pins(self, sources: Any, pins: Any, *, origins: bool = False) -> tuple[list, bool]:
        brain = canonical_module('brain_runtime')
        if (not isinstance(sources, list) or not 1 <= len(sources) <= ADAPTER_LIMITS['max_sources_per_record']
                or any(not isinstance(source, str) or not source for source in sources)):
            raise PilotError('complete source evidence required')
        expected = brain.canonical_source_image(pins)
        paths = {source.split('#', 1)[0] for source in sources}
        if not expected or {row['path'] for row in expected} != paths:
            raise PilotError('complete original source pins required')
        stale = False
        for row in expected:
            name = row['path']
            origin = origins and _BRAIN_PATH.fullmatch(name) is not None
            key = (name, origin)
            if key not in self.hashes:
                raw = self.read(name, memory=origin)
                self.hashes[key] = hashlib.sha256(brain._line_ending_neutral(raw)).hexdigest()
            if self.hashes[key] != row['sha256']:
                stale = True
        return expected, stale


def _brain_origin(reader: _AdapterReader, name: str) -> tuple[dict, str, dict]:
    brain = canonical_module('brain_runtime')
    match = _BRAIN_PATH.fullmatch(name)
    if match is None: raise PilotError('unsupported Brain source path')
    if name in reader.origins: return reader.origins[name]
    config = reader.admission()
    metadata, body, _ = reader.document(name)
    brain.validate_record(metadata)
    folders = {'finding': 'findings', 'bug': 'bugs', 'incident': 'incidents', 'decision': 'decisions'}
    if (metadata['id'] != match[2] or folders.get(metadata['type']) != match[1]
            or metadata['privacy'] not in ('public', 'team')
            or metadata['privacy'] not in config['allowed_privacy']
            or metadata['authority'] != 'verified'
            or metadata['authority'] not in config['allowed_authority']
            or metadata['status'] not in brain.PROMOTABLE_STATES.get(metadata['type'], set())
            or metadata['superseded_by'] is not None):
        raise PilotError('Brain provenance not admitted')
    attestation = brain.record_attestation(metadata)
    if attestation not in brain.ATTESTATIONS: raise PilotError('unknown attestation')
    created, updated = instant(metadata['created_at']), instant(metadata['updated_at'])
    transitions = [instant(row['at']) for row in metadata['transitions']]
    if created > updated or any(at < created or at > updated for at in transitions):
        raise PilotError('inconsistent recorded time')
    pins, stale = reader.pins(metadata['sources'], metadata['source_fingerprints'])
    provenance = {'record_id': metadata['id'], 'authority': metadata['authority'],
        'attestation': attestation, 'privacy': metadata['privacy'],
        'recorded_at': updated.isoformat(), 'recorded_at_precision': 'instant',
        'valid_from': updated.isoformat(), 'valid_to': None,
        'lineage': 'brain:' + metadata['id'], 'revision': metadata['revision'],
        'conflicts': metadata['conflicts'], 'supersedes': metadata['supersedes'],
        'superseded_by': metadata['superseded_by'], 'source_pins': pins,
        'reason': 'stale' if stale else 'eligible'}
    reader.origins[name] = metadata, body, provenance
    return reader.origins[name]


def _day(value: str, *, ceiling: bool = False) -> datetime:
    if not isinstance(value, str) or not re.fullmatch(r'\d{4}-\d{2}-\d{2}', value):
        raise PilotError('invalid day precision timestamp')
    day = date.fromisoformat(value)
    if ceiling: day += timedelta(days=1)
    return datetime(day.year, day.month, day.day, tzinfo=timezone.utc)


def _frontmatter_candidate(reader: _AdapterReader, selector: dict) -> dict:
    brain = canonical_module('brain_runtime')
    name, kind = selector['source'], selector['source_kind']
    if kind == 'brain-frontmatter':
        metadata, body, provenance = _brain_origin(reader, name)
    else:
        if (not name.startswith('memory-bank/chunks/') or name.count('/') != 2
                or brain.BANK_FILENAME_PATTERN.fullmatch(Path(name).name) is None):
            raise PilotError('unsupported bank source path')
        metadata, body, _ = reader.document(name)
        brain.validate_bank_metadata(Path(name), metadata, reader.root)
        if metadata['status'] != 'active' or metadata['superseded_by'] is not None:
            raise PilotError('inactive bank source')
        pins, stale = reader.pins(metadata['sources'], metadata.get('source_digests'), origins=True)
        origins = []
        for pin in pins:
            if _BRAIN_PATH.fullmatch(pin['path']):
                _, _, origin = _brain_origin(reader, pin['path'])
                origins.append(origin)
        if not origins: raise PilotError('verified bank origins required')
        # A bank cannot omit an origin's checked file or substitute today's digest.
        pin_map = {pin['path']: pin['sha256'] for pin in pins}
        for origin in origins:
            if any(pin_map.get(pin['path']) != pin['sha256'] for pin in origin['source_pins']):
                raise PilotError('bank original source image incomplete')
            stale = stale or origin['reason'] == 'stale'
        recorded = max(_day(metadata['created'], ceiling=True),
                       _day(metadata['last_verified'], ceiling=True),
                       *(instant(origin['recorded_at']) for origin in origins))
        end = _day(metadata['review_after'], ceiling=True)
        if metadata.get('valid_to') is not None: end = min(end, _day(metadata['valid_to'], ceiling=True))
        provenance = {'record_id': metadata['id'], 'authority': 'verified',
            'attestation': 'agent' if any(origin['attestation'] == 'agent' for origin in origins) else 'person',
            'privacy': 'team' if any(origin['privacy'] == 'team' for origin in origins) else 'public',
            'recorded_at': recorded.isoformat(), 'recorded_at_precision': 'day',
            'valid_from': _day(metadata.get('valid_from', metadata['created'])).isoformat(),
            'valid_to': end.isoformat(), 'lineage': None, 'revision': None,
            'canonical_scope': sorted(metadata['scope']),
            'conflicts': sorted({conflict for origin in origins for conflict in origin['conflicts']}),
            'supersedes': metadata['supersedes'], 'superseded_by': metadata['superseded_by'],
            'source_pins': pins, 'origin_records': [
                {key: origin[key] for key in ('record_id', 'revision', 'authority', 'attestation',
                    'privacy', 'recorded_at', 'conflicts', 'supersedes', 'superseded_by')}
                for origin in origins],
            'reason': 'stale' if stale else 'eligible'}
    document = dict(metadata, body=body)
    claim = json_pointer(document, selector['pointer'])
    if isinstance(claim, (dict, list)) or isinstance(claim, str) and len(claim) > ADAPTER_LIMITS['max_claim_characters']:
        raise PilotError('only bounded scalar claims supported')
    public_text(json.dumps(claim, ensure_ascii=False, allow_nan=False))
    return dict(provenance, claim=claim, source=name, source_kind=kind,
                source_digest=hashlib.sha256(reader.read(name, memory=True)).hexdigest())


def instant(value: str) -> datetime:
    if not isinstance(value, str): raise PilotError('timestamp must be an aware ISO instant')
    result = datetime.fromisoformat(value.replace('Z', '+00:00'))
    if result.tzinfo is None: raise PilotError('naive timestamp refused')
    try:
        return result.astimezone(timezone.utc)
    except (OverflowError, ValueError) as error:
        raise PilotError('timestamp is outside the supported UTC range') from error


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
    reader = _AdapterReader(root)
    brain = canonical_module('brain_runtime')
    for selector in selectors:
        if not isinstance(selector, dict): raise PilotError('invalid selector')
        kind = selector.get('source_kind', 'json')
        if kind not in ('json', 'brain-frontmatter', 'bank-frontmatter'):
            raise PilotError('unsupported source kind')
        required = {'id', 'namespace', 'subject', 'predicate', 'source', 'pointer'}
        optional = {'qualifiers', 'expected_sha256', 'source_kind'}
        if kind == 'json':
            required.add('recorded_at_pointer')
            optional.update({'lineage_pointer', 'revision_pointer', 'conflicts_pointer',
                             'valid_from_pointer', 'valid_to_pointer'})
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
            if kind != 'json':
                row.update(_frontmatter_candidate(reader, selector))
                if 'expected_sha256' in selector and selector['expected_sha256'] != row['source_digest']:
                    row['reason'] = 'stale'
                elif row['reason'] == 'eligible':
                    if instant(row['recorded_at']) > moment or instant(row['valid_from']) > moment:
                        row['reason'] = 'future'
                    elif row['valid_to'] is not None and moment >= instant(row['valid_to']):
                        row['reason'] = 'expired'
                result.append(row)
                continue
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
            if 'expected_sha256' in selector and selector['expected_sha256'] != row['source_digest']:
                row['reason'] = 'stale'
            elif recorded > moment or beginning > moment: row['reason'] = 'future'
            elif end is not None and moment >= end: row['reason'] = 'expired'
        except (OSError, ValueError, KeyError, IndexError, TypeError, UnicodeError, OverflowError,
                RecursionError, brain.BrainError, brain.BankValidationError):
            row = {key: row[key] for key in ('id', 'namespace', 'subject', 'predicate', 'qualifiers')}
            row.update(reason='unclassified', claim=None)
            if kind != 'json': row['source_kind'] = kind
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
        key = (row['subject'], row['predicate'], tuple(sorted(row['qualifiers'].items())),
               tuple(row.get('canonical_scope', [])))
        groups.setdefault(key, []).append(row)
    facts = []
    for key, rows in sorted(groups.items()):
        rows = sorted(rows, key=lambda row: row['id'])
        eligible = [row for row in rows if row['reason'] == 'eligible']
        conflict = any(row.get('conflicts') for row in eligible)
        # Direct canonical values outrank retellings; declared conflicts and
        # unavailable evidence still apply to the complete history below.
        canonical = [row for row in eligible if row.get('authority') == 'canonical-json']
        if canonical:
            eligible = canonical
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
        if key[3]: facts[-1]['canonical_scope'] = list(key[3])
    result = {'schema': 'memory-current-facts-v1', 'as_of': instant(as_of).isoformat(), 'facts': facts,
              'derived': True, 'runtime_integrated': False}
    if any(row['namespace'] == namespace and row.get('source_kind') in ('brain-frontmatter', 'bank-frontmatter')
           for row in candidates):
        result['adapter_limits'] = dict(ADAPTER_LIMITS)
    return result


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
