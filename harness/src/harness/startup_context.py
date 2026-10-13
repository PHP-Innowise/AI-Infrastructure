"""Accelerators: the exact bytes each ready-made edition puts in front of the model before any work.

Measured by scripts/context_budget.py with the rules its CI ceiling uses, so the page
and the gate cannot disagree. Bytes are exact; tokens are the script's calibrated
estimate (docs/TOKEN-ECONOMY-RESEARCH.md as of 9435dfc1^).
"""
from __future__ import annotations

import json


def startup_context(editions):
    """Startup bytes per edition next to its CI ceilings; an edition that cannot be measured says why."""
    import context_budget  # scripts/ is on the server's import path
    try:
        ceilings = json.loads(context_budget.BUDGET_FILE.read_text(encoding='utf-8')).get('editions', {})
    except (OSError, ValueError):
        ceilings = {}
    rows = []
    for name in editions:
        try:
            measured = context_budget.measure_edition(name)
        except (context_budget.BudgetError, OSError, KeyError, ValueError) as error:
            rows.append({'edition': name, 'error': str(error)[:300]})
            continue
        parts = {key: measured[key] for key in context_budget.STARTUP_CATEGORIES}
        limits = {key: ceilings.get(name, {}).get(key) for key in parts}
        rows.append({'edition': name, 'skills': measured['skills'], 'bytes': parts, 'total': sum(parts.values()),
                     'ceilings': limits,
                     'ceiling_total': sum(limits.values()) if all(type(value) is int for value in limits.values()) else None,
                     'tokens': context_budget.startup_tokens(measured)})
    return {'editions': rows, 'labels': dict(context_budget.STARTUP_LABELS),
            'ratios': {key: context_budget.BYTES_PER_TOKEN[key] for key in context_budget.STARTUP_CATEGORIES},
            'calibration': 'docs/TOKEN-ECONOMY-RESEARCH.md as of 9435dfc1^'}
