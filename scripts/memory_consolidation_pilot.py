#!/usr/bin/env python3
"""Replay canonical memory maintenance on a private physical snapshot copy."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from memory_pilot_support import (ROOT, CANON, PilotError, canonical_module, child_env,
                                  copy_snapshot, digest, snapshot_digest)

VARIANTS = ('baseline', 'consolidate', 'consolidate-verify')


def _worker(root: Path, variant: str, owner: str) -> dict:
    brain = canonical_module('brain_runtime')
    roots = brain.workspace_roots.roots(root)
    if (any(value.resolve() != root.resolve() for value in (roots.project, roots.state, roots.tooling))
            or roots.shared or roots.attached): raise PilotError('copy layout is not isolated')
    config = root / 'project-brain/config/runtime.json'
    flags = json.loads(config.read_text(encoding='utf-8')) if config.exists() else {}
    flags.update(automatic_consolidation=variant != 'baseline',
                 automatic_consolidation_verification=variant == 'consolidate-verify')
    config.parent.mkdir(parents=True, exist_ok=True)
    config.write_text(json.dumps(flags), encoding='utf-8')
    planned = brain.consolidate(root, apply=False, owner=owner) if variant != 'baseline' else {}
    boundaries = []
    def knowledge():
        return snapshot_digest(root)
    before_last = None
    for _ in range(3):
        before_last = knowledge()
        consolidation = brain.auto_consolidate(root, owner=owner)
        promotion = brain.auto_promote(root, owner=owner)
        compaction = brain.auto_compact(root, owner=owner)
        boundaries.append({'created': consolidation['created'], 'updated': consolidation['updated'],
            'superseded': consolidation['superseded'], 'promoted': len(promotion['promoted']),
            'promotion_failed': len(promotion['failed']), 'promotion_blocked': len(promotion['blocked']),
            'compacted': compaction['moved'], 'failed': bool(consolidation['error'] or compaction['error'])})
    mappings = []
    import re
    for _, record, _ in brain.iter_records(root, include_archive=True):
        if record['type'] == 'finding' and record['external_id'].startswith('CONSOLIDATED-'):
            destinations = []
            for _, promotion in brain.iter_promotions(root):
                if (promotion.get('status') == 'applied' and any(
                        source['id'] == record['id'] and source['revision'] == record['revision']
                        for source in promotion.get('source_records', []))):
                    destinations.append(promotion['destination_memory_id'])
            mappings.append({'record': record['id'], 'revision': record['revision'], 'chunks': sorted(destinations),
                'events': sorted(set(re.findall(r'\[event ([a-f0-9-]+)\]', record['progress']))),
                'authority': record['authority'], 'attestation': brain.record_attestation(record)})
    validation = brain.validate_repository(root, check_freshness=False)
    errors = validation if isinstance(validation, list) else validation.get('errors', [])
    errors = ['brain-validation'] * len(errors) + ['bank-validation'] * len(brain.validate_bank(root / 'memory-bank'))
    return {'boundaries': boundaries, 'mappings': sorted(mappings, key=lambda row: row['record']),
            'planned_tickets': len(planned.get('tickets', [])), 'idempotent': before_last == knowledge(),
            'validation_errors': errors}


def run_pilot(root: Path, *, variant: str, owner: str = 'local', export: Path | None = None) -> dict:
    if variant not in VARIANTS: raise PilotError('unknown variant')
    root = Path(root)
    if export is not None:
        export = Path(export)
        if (export.exists() or export.is_symlink() or root.resolve() in export.resolve().parents
                or export.resolve() == ROOT or ROOT in export.resolve().parents):
            raise PilotError('export must be a new directory outside input')
    with tempfile.TemporaryDirectory(prefix='memory-consolidation-') as temporary:
        copy = Path(temporary) / 'snapshot'
        before = copy_snapshot(root, copy)
        # Schemas are trusted tooling, never executable input from a client.
        schemas = copy / 'project-brain/schemas'
        if schemas.exists(): shutil.rmtree(schemas)
        shutil.copytree(ROOT / 'PHP Core/project-brain/schemas', schemas)
        code = ('import sys; from pathlib import Path; sys.path.insert(0,sys.argv[1]); '
                'from memory_consolidation_pilot import _worker; import json; '
                'print(json.dumps(_worker(Path(sys.argv[2]),sys.argv[3],sys.argv[4])))')
        done = subprocess.run([sys.executable, '-I', '-B', '-c', code, str(ROOT / 'scripts'),
            str(copy), variant, owner], cwd=copy, env=child_env(), capture_output=True, timeout=120)
        if done.returncode: raise PilotError('canonical maintenance replay failed')
        result = json.loads(done.stdout)
        if snapshot_digest(root) != before: raise PilotError('input changed during replay')
        runtime_inputs = {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                          for path in sorted(CANON.glob('*.py'))}
        runtime_inputs.update({'schemas/' + path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                              for path in sorted((ROOT / 'PHP Core/project-brain/schemas').glob('*.json'))})
        result.update(schema='memory-consolidation-pilot-v1', variant=variant, input_digest=before,
            runtime_digest=digest(runtime_inputs), runtime_inputs=runtime_inputs,
            evidence='lifecycle-replay', task_outcome='not-measured', human_reviewed=False,
            recipe={'boundaries': 3, 'owner': owner, 'variant': variant})
        if export is not None:
            copy_snapshot(copy, export)  # fresh-clone export excludes local locks/cache/receipts
            result['export_digest'] = snapshot_digest(export)
        return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--variant', choices=VARIANTS, required=True)
    parser.add_argument('--owner', default='local')
    parser.add_argument('--export', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        from memory_eval import inside_clone, write_json
        if inside_clone(args.out) or args.root.resolve() in args.out.resolve().parents:
            raise PilotError('report must be outside tooling clone and input')
        write_json(args.out, run_pilot(args.root, variant=args.variant, owner=args.owner, export=args.export))
        return 0
    except (OSError, ValueError, subprocess.SubprocessError):
        parser.exit(2, 'consolidation pilot: unsafe input or replay unavailable\n')


if __name__ == '__main__': raise SystemExit(main())
