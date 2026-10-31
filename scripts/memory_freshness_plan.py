#!/usr/bin/env python3
"""Read-only maintenance packets over the canonical bank audit, off prompt path."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

from memory_pilot_support import ROOT, PilotError, child_env, copy_snapshot, public_text, relative_path, snapshot_digest


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


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--limit', type=int, default=50)
    parser.add_argument('--json', action='store_true')
    args = parser.parse_args(argv)
    try:
        print(json.dumps(plan_reviews(args.root, limit=args.limit), ensure_ascii=False, allow_nan=False, indent=2))
        return 0
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError):
        parser.exit(2, 'freshness plan: unsafe input or audit unavailable\n')


if __name__ == '__main__': raise SystemExit(main())
