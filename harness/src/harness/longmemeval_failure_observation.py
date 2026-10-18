"""Fixed projection guard labels, never exception prose or root-cause proof.

This is an optional local benchmark observation. Missing observation is unknown;
it does not certify no failures, native readiness, provider usage or billing.
"""
from .longmemeval_projection import ProjectionError

PROFILE = 'longmemeval/projection-body-failure-v1'
PHASES = ('initialization', 'insert', 'query', 'cleanup')
GUARDS = frozenset(('ambiguous_alias', 'conflicting_identity', 'context_bound',
    'corpus_bound', 'duplicate_key', 'input_bound', 'input_encoding',
    'input_schema', 'instance_unavailable', 'native_configuration',
    'native_profile', 'query_bound', 'screenshot_reference', 'source_changed'))


def validate_phase(phase):
    if type(phase) is not str or len(phase) > 32 or phase not in PHASES:
        raise ValueError('failure_phase_profile')


def _record(phase, exception, guard, state):
    return {'profile': PROFILE, 'scope': 'last_operation_body_exception_only',
            'phase': phase, 'reported_exception': exception,
            'reported_guard': guard, 'observation_state': state,
            'root_cause': None, 'provider_tokens': None, 'usd_micros': None}


def empty_failure():
    return _record(None, None, None, 'unobserved')


def unmeasured_failure(phase):
    return _record(phase, 'unclassified', None, 'unmeasured')


def projection_failure_metadata(error, *, phase):
    """Project one owned exception label without formatting or iterating input.

    Exact type excludes subclasses overriding attributes/formatting. Even an
    owned class with caller-forged arguments establishes only a supplied guard
    label. One bounded ASCII argument must match the source-defined allowlist.
    """
    validate_phase(phase)
    if type(error) is not ProjectionError:
        return unmeasured_failure(phase)
    args = error.args
    guard = (args[0] if type(args) is tuple and len(args) == 1
             and type(args[0]) is str and len(args[0]) <= 64
             and args[0].isascii() and args[0] in GUARDS else None)
    return _record(phase, 'ProjectionError', guard,
                   'fixed_guard_reported' if guard is not None else 'unmeasured')
