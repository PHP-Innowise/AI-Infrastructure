"""Fixed refresh observation metadata for checkout-owned consumers."""
from __future__ import annotations

from typing import Any, Optional

LOSS_KEYS = ('term_coverage', 'relative_score_floor', 'budget', 'hard_budget',
             'layer', 'repeat', 'gate', 'pack', 'authority', 'lifecycle', 'stale',
             'source_missing', 'map_unverifiable', 'map_drift')
BOUNDARY_KEYS = ('queue_after_runtime', 'budget_admitted', 'layer_pre_novelty',
                 'repeat_withheld', 'gate_input', 'gate_withheld', 'pack_input',
                 'pack_omitted', 'selected_post_pack')
RENDER_KEYS = ('excerpt', 'render-limit', 'sanitized-empty', 'no-excerpt')
COUNT_LIMIT = 65535


def unavailable_funnel_diagnosis() -> dict[str, Any]:
    def counters(keys):
        return {key: {'count': None, 'state': 'unknown'} for key in keys}
    return {
        'schema_version': 1, 'observation_state': 'unavailable',
        'source_coverage': 'unknown', 'quality': 'unmeasured',
        'count_scope': 'authorized-observation-occurrences',
        'coverage': {'history_pre_admission': 'not-instrumented',
                     'episodes': 'not-instrumented', 'working_sources': 'not-instrumented',
                     'path_only_events': 'authorized-retained-records-only'},
        'loss_observations': counters(LOSS_KEYS), 'boundaries': counters(BOUNDARY_KEYS),
        'render_projection': {'scope': 'authorized-retained-records',
                              'headers': {'count': None, 'state': 'unknown'},
                              'reasons': counters(RENDER_KEYS)},
        'emission': 'unknown', 'consumer': 'unknown', 'next_action': 'unknown',
    }


def _counter(value: Any) -> Optional[dict[str, Any]]:
    if type(value) is not dict or set(value) != {'count', 'state'}:
        return None
    count, state = value['count'], value['state']
    if count is None and state == 'unknown':
        return {'count': None, 'state': 'unknown'}
    if (type(count) is int and 0 <= count <= COUNT_LIMIT
            and state in ('exact', 'lower-bound')
            and (state != 'lower-bound' or count == COUNT_LIMIT)):
        return {'count': count, 'state': state}
    return None


def normalize_funnel_diagnosis(value: Any) -> Optional[dict[str, Any]]:
    """Rebuild only the fixed child; missing or rejected metadata has no child."""
    clean = unavailable_funnel_diagnosis()
    if (type(value) is not dict or set(value) != set(clean)
            or type(value['schema_version']) is not int or value['schema_version'] != 1
            or value['observation_state'] not in ('observed', 'unavailable')
            or value['next_action'] not in ('unknown', 'inspect-observed-loss')
            or any(value[key] != clean[key] for key in
                   ('source_coverage', 'quality', 'count_scope', 'coverage', 'emission', 'consumer'))):
        return None
    for key in ('loss_observations', 'boundaries'):
        group = value[key]
        if type(group) is not dict or set(group) != set(clean[key]):
            return None
        for name in clean[key]:
            item = _counter(group[name])
            if item is None:
                return None
            clean[key][name] = item
    render = value['render_projection']
    if (type(render) is not dict or set(render) != set(clean['render_projection'])
            or render['scope'] != clean['render_projection']['scope']
            or type(render['reasons']) is not dict or set(render['reasons']) != set(RENDER_KEYS)):
        return None
    headers = _counter(render['headers'])
    if headers is None:
        return None
    clean['render_projection']['headers'] = headers
    for name in RENDER_KEYS:
        item = _counter(render['reasons'][name])
        if item is None:
            return None
        clean['render_projection']['reasons'][name] = item
    if value['observation_state'] == 'unavailable':
        return clean if value == unavailable_funnel_diagnosis() else None
    action = ('inspect-observed-loss' if any(type(item['count']) is int and item['count'] > 0
              for item in clean['loss_observations'].values()) else 'unknown')
    if value['next_action'] != action:
        return None
    clean['observation_state'] = 'observed'
    clean['next_action'] = action
    return clean


def unavailable_ordinary_receipt() -> dict[str, Any]:
    """Unavailable metadata says nothing about an attempt or its effects."""
    return {"schema_version": 1, "observation_state": "unavailable",
            "plans_present": None, "returned_status": "unknown"}


def normalize_ordinary_receipt(value: Any) -> Optional[dict[str, Any]]:
    """Rebuild the fixed scalar sibling without touching arbitrary value hooks."""
    keys = ("schema_version", "observation_state", "plans_present", "returned_status")
    if (type(value) is not dict or any(type(key) is not str for key in value)
            or set(value) != set(keys) or type(value["schema_version"]) is not int
            or value["schema_version"] != 1 or type(value["observation_state"]) is not str
            or type(value["returned_status"]) is not str):
        return None
    state, plans, status = value["observation_state"], value["plans_present"], value["returned_status"]
    if state == "unavailable":
        if plans is not None or status != "unknown":
            return None
    elif state == "observed":
        if (type(plans) is not bool or status not in ("skipped", "deferred", "committed")
                or (plans is False and status != "skipped")):
            return None
    else:
        return None
    return {"schema_version": 1, "observation_state": state,
            "plans_present": plans, "returned_status": status}
