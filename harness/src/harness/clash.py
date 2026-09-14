"""Clash option: two native providers confront each other over one task.

Standard library only. This module holds the pure parts (settings validation,
turn prompts, the objection/finding ledger and its report); process execution
lives in ``clash_runner.py`` and the session plumbing in ``sessions.py``.

Clash is a checkbox on Workspace and Review sessions, not a workflow of its
own. The stage follows the session mode: Edit mode runs the implementation
clash, Plan mode runs the review clash.

One clash cycle::

    opening turn (protagonist: implementer or reviewer A)
    for round in 1..rounds:
        challenger turn -> verdict            # converged when it accepts
        protagonist response turn             # skipped after the last round

The challenger's final verdict ends the cycle; a human reads the ledger and
can continue with a follow-up message, which starts the next cycle while both
native sessions resume. Nothing here decides that the work is correct: the
ledger records what each participant claimed and which claims survived.
"""
from __future__ import annotations

import json
import re

from . import providers

STAGES = {
    'implement': 'The first participant implements the task, the challenger attacks the resulting change, and the implementer fixes or rebuts each objection.',
    'review': 'The first participant reviews the scope, the challenger reviews it independently and disputes findings, and the reviewer defends or withdraws each one.',
}
WORKFLOWS = ('native', 'review')
MAX_ROUNDS = 3
DEFAULT_ROUNDS = 2
FIELDS = ('challenger', 'rounds', 'challenger_model', 'challenger_thinking_effort')
SEVERITIES = ('high', 'medium', 'low')
PROTAGONIST_ROLE = {'implement': 'implementer', 'review': 'reviewer'}
TERMINAL = ('resolved', 'withdrawn', 'deferred', 'confirmed')
IMPLEMENT_UNRESOLVED = ('open',)
REVIEW_UNRESOLVED = ('proposed', 'disputed', 'unverified', 'defended')
MAX_ITEMS = 60
MAX_TURNS = 60
TEXT_LIMIT = 600
SUMMARY_LIMIT = 4000
TASK_LIMIT = 8000
LEDGER_PROMPT_LIMIT = 24000
DIFF_LIMIT = 40 * 1024
REPORT_LIMIT = 32000


def validate(value, workflow, provider):
    """Normalize clash settings (None keeps clash off); the session layer checks CLI availability."""
    from .sessions import SessionError
    if value is None:
        return None
    if workflow not in WORKFLOWS:
        raise SessionError('Clash is available for Workspace and Review sessions.')
    if not isinstance(value, dict) or set(value) - set(FIELDS):
        raise SessionError('Clash settings accept challenger, rounds and the challenger model and thinking effort.')
    challenger = value.get('challenger')
    rounds = value.get('rounds', DEFAULT_ROUNDS)
    if not isinstance(challenger, str) or challenger not in providers.PROVIDERS:
        raise SessionError('Choose a challenger provider.')
    if challenger == provider:
        raise SessionError('The challenger must be a different provider from the first participant.')
    if type(rounds) is not int or not 1 <= rounds <= MAX_ROUNDS:
        raise SessionError(f'Rounds must be a whole number from 1 to {MAX_ROUNDS}.')
    model, effort = value.get('challenger_model'), value.get('challenger_thinking_effort')
    if model == '':
        model = None
    if effort == '':
        effort = None
    if model is not None and (not isinstance(model, str) or not model.strip() or model.startswith('-') or len(model) > 120
                              or any(ord(char) < 32 or ord(char) == 127 for char in model)):
        raise SessionError('Invalid challenger model name.')
    if effort == 'ultracode':
        raise SessionError('Clash participants run without additional agents; Ultracode is unavailable for the challenger.')
    try:
        providers.validate_model_effort(challenger, model, effort)
    except ValueError as error:
        raise SessionError('Challenger: ' + str(error)) from error
    return {'challenger': challenger, 'rounds': rounds, 'challenger_model': model, 'challenger_thinking_effort': effort}


def stage_for(mode):
    """Edit mode contests the change in the workspace; Plan mode contests findings."""
    return 'implement' if mode == 'edit' else 'review'


def _string(value, limit):
    return ' '.join(value.split())[:limit] if isinstance(value, str) else ''


def _line(value):
    return value if type(value) is int and 0 <= value <= 10 ** 9 else 0


def _name(provider):
    return providers.PROVIDERS.get(provider, provider)


def new_state(session, stage, previous=None):
    """Fresh cycle state; a follow-up cycle carries the ledger and turns forward.

    A previous cycle of another stage keeps only its turns: objections and
    findings have different state machines. A previous challenger of another
    provider keeps the ledger but not its native session.
    """
    settings = session['clash']
    previous = previous if isinstance(previous, dict) else None
    same_stage = bool(previous) and previous.get('stage') == stage
    previous_challenger = (previous.get('challenger') or {}).get('provider') if previous else None
    cycle = previous.get('cycle') if previous else 0
    cycle = cycle + 1 if type(cycle) is int and cycle > 0 else 1
    items = [item for item in (previous.get('items') if same_stage else []) or [] if isinstance(item, dict)][:MAX_ITEMS]
    turns = [turn for turn in (previous.get('turns') if previous else []) or [] if isinstance(turn, dict)][-MAX_TURNS:]
    challenger_id = previous.get('challenger_session_id') if previous and previous_challenger == settings['challenger'] else None
    task = previous.get('task') if same_stage else ''
    return {
        'kind': 'clash_state', 'stage': stage, 'status': 'running', 'outcome': None,
        'cycle': cycle, 'round': 0, 'rounds': settings['rounds'],
        'protagonist': {'provider': session['provider'], 'role': PROTAGONIST_ROLE[stage],
                        'model': session.get('model'), 'thinking_effort': session.get('thinking_effort')},
        'challenger': {'provider': settings['challenger'], 'role': 'challenger',
                       'model': settings.get('challenger_model'), 'thinking_effort': settings.get('challenger_thinking_effort')},
        'challenger_session_id': challenger_id if isinstance(challenger_id, str) else None,
        'task': task if isinstance(task, str) else '', 'followup': '',
        'items': items, 'turns': turns, 'summary': '',
        'verdict': None, 'verdict_note': '', 'message': '', 'report': '',
    }


def extract_json(text, required):
    """Return the last JSON object carrying one of ``required`` keys, leniently."""
    if not isinstance(text, str) or not text:
        return None
    fenced = re.findall(r"```(?:json)?\s*(.*?)```", text, re.S)
    decoder = json.JSONDecoder()
    for candidate in [*reversed(fenced), text]:
        starts = [match.start() for match in re.finditer(r'\{', candidate)]
        for start in reversed(starts):
            try:
                decoded, _ = decoder.raw_decode(candidate[start:])
            except (ValueError, RecursionError):
                continue
            if isinstance(decoded, dict) and any(key in decoded for key in required):
                return decoded
    return None


def prose(text, limit=TEXT_LIMIT):
    """Human text around the structured block, for summaries and the next prompt."""
    if not isinstance(text, str):
        return ''
    stripped = re.sub(r"```(?:json)?\s*\{.*?\}\s*```", ' ', text, flags=re.S).rstrip()
    if stripped.endswith('}'):
        start = stripped.rfind('\n{')
        if start != -1:
            try:
                json.loads(stripped[start + 1:])
                stripped = stripped[:start]
            except ValueError:
                pass
    return ' '.join(stripped.split())[:limit]


def _new_item(raw, item_id, origin, kind, cycle, round_):
    return {'id': item_id, 'kind': kind, 'origin': origin,
            'severity': raw.get('severity') if raw.get('severity') in SEVERITIES else 'unrated',
            'file': _string(raw.get('file'), 300), 'line': _line(raw.get('line')),
            'claim': _string(raw.get('claim'), TEXT_LIMIT) or 'No description was returned.',
            'evidence': _string(raw.get('evidence'), TEXT_LIMIT),
            'status': 'open' if kind == 'objection' else 'proposed',
            'history': [{'cycle': cycle, 'round': round_, 'actor': origin, 'action': 'raised', 'note': ''}]}


def _entry(state, round_, actor, action, note=''):
    return {'cycle': state['cycle'], 'round': round_, 'actor': actor, 'action': action, 'note': _string(note, TEXT_LIMIT)}


def _next_id(items):
    return max((item['id'] for item in items if type(item.get('id')) is int), default=0) + 1


def apply_challenge(state, payload, round_):
    """Implementation stage: the challenger lists every objection that still stands."""
    items = state['items']
    by_id = {item['id']: item for item in items if item.get('kind') == 'objection'}
    resolved = {value for value in payload.get('resolved', []) if type(value) is int} if isinstance(payload.get('resolved'), list) else set()
    listed, fresh = {}, []
    next_id = _next_id(items)
    raw_list = payload.get('objections') if isinstance(payload.get('objections'), list) else []
    for raw in raw_list[:MAX_ITEMS]:
        if not isinstance(raw, dict):
            continue
        identifier = raw.get('id')
        if type(identifier) is int and identifier in by_id and identifier not in listed:
            listed[identifier] = raw
        elif len(items) + len(fresh) < MAX_ITEMS:
            fresh.append(_new_item(raw, next_id, 'challenger', 'objection', state['cycle'], round_))
            next_id += 1
    for item in by_id.values():
        previous = item['status']
        if item['id'] in resolved:
            if previous != 'resolved':
                item['status'] = 'resolved'
                item['history'].append(_entry(state, round_, 'challenger', 'resolved'))
        elif item['id'] in listed:
            raw = listed[item['id']]
            for key in ('claim', 'evidence'):
                text = _string(raw.get(key), TEXT_LIMIT)
                if text:
                    item[key] = text
            if raw.get('severity') in SEVERITIES:
                item['severity'] = raw['severity']
            if _string(raw.get('file'), 300):
                item['file'] = _string(raw.get('file'), 300)
            if type(raw.get('line')) is int:
                item['line'] = _line(raw['line'])
            item['status'] = 'open'
            item['history'].append(_entry(state, round_, 'challenger',
                                          'maintained' if previous in ('open', 'fixed', 'rebutted', 'deferred') else 'reopened'))
        elif previous in ('open', 'fixed', 'rebutted', 'deferred'):
            item['status'] = {'fixed': 'resolved', 'rebutted': 'withdrawn', 'deferred': 'deferred', 'open': 'withdrawn'}[previous]
            item['history'].append(_entry(state, round_, 'challenger', 'not maintained'))
    items.extend(fresh)
    verdict = payload.get('verdict') if payload.get('verdict') in ('accept', 'reject') else None
    state['verdict'] = verdict
    blocking = [item for item in items if item['status'] == 'open' and item['severity'] != 'low']
    if verdict is None:
        state['verdict_note'] = 'The challenger returned no valid verdict; the cycle continues as rejected.'
    elif verdict == 'accept' and blocking:
        state['verdict_note'] = 'The challenger said accept while high, medium or unrated objections stay open; the ledger does not converge.'
    else:
        state['verdict_note'] = ''


def apply_response(state, payload, round_):
    """Implementation stage: the implementer fixes, rebuts or defers each open objection."""
    by_id = {item['id']: item for item in state['items'] if item.get('kind') == 'objection'}
    responses = payload.get('responses') if isinstance(payload.get('responses'), list) else []
    seen = set()
    for raw in responses[:MAX_ITEMS * 2]:
        if (not isinstance(raw, dict) or type(raw.get('id')) is not int or raw.get('action') not in ('fixed', 'rebutted', 'deferred')
                or raw['id'] in seen):
            continue
        item = by_id.get(raw['id'])
        if item is None or item['status'] != 'open':
            continue
        seen.add(raw['id'])
        item['status'] = raw['action']
        item['history'].append(_entry(state, round_, 'implementer', raw['action'], raw.get('note')))
    for item in by_id.values():
        if item['status'] == 'open' and item['id'] not in seen:
            item['history'].append(_entry(state, round_, 'implementer', 'no response'))


def apply_review_turn(state, actor, payload, round_):
    """Review stage: assess the other side's findings, answer disputes, add findings."""
    other = 'challenger' if actor == 'reviewer' else 'reviewer'
    items = state['items']
    by_id = {item['id']: item for item in items if item.get('kind') == 'finding'}
    next_id = _next_id(items)
    fresh = []
    for key in ('findings', 'additional'):
        raw_list = payload.get(key) if isinstance(payload.get(key), list) else []
        for raw in raw_list[:MAX_ITEMS]:
            if isinstance(raw, dict) and len(items) + len(fresh) < MAX_ITEMS:
                fresh.append(_new_item(raw, next_id, actor, 'finding', state['cycle'], round_))
                next_id += 1
    assessed = set()
    assessments = payload.get('assessments') if isinstance(payload.get('assessments'), list) else []
    for raw in assessments[:MAX_ITEMS * 2]:
        if not isinstance(raw, dict) or type(raw.get('id')) is not int or raw['id'] in assessed:
            continue
        stance = raw.get('stance')
        item = by_id.get(raw['id'])
        if stance not in ('agree', 'dispute', 'unverified') or item is None or item['origin'] != other or item['status'] not in ('proposed', 'defended'):
            continue
        assessed.add(raw['id'])
        item['status'] = {'agree': 'confirmed', 'dispute': 'disputed', 'unverified': 'unverified'}[stance]
        item['history'].append(_entry(state, round_, actor, stance, raw.get('evidence')))
    answered = set()
    responses = payload.get('responses') if isinstance(payload.get('responses'), list) else []
    for raw in responses[:MAX_ITEMS * 2]:
        if not isinstance(raw, dict) or type(raw.get('id')) is not int or raw['id'] in answered or raw.get('action') not in ('defend', 'withdraw'):
            continue
        item = by_id.get(raw['id'])
        if item is None or item['origin'] != actor or item['status'] not in ('disputed', 'unverified'):
            continue
        answered.add(raw['id'])
        item['status'] = 'defended' if raw['action'] == 'defend' else 'withdrawn'
        item['history'].append(_entry(state, round_, actor, raw['action'], raw.get('note')))
    for item in by_id.values():
        if item['origin'] == other and item['status'] in ('proposed', 'defended') and item['id'] not in assessed:
            item['history'].append(_entry(state, round_, actor, 'not assessed'))
        elif item['origin'] == actor and item['status'] in ('disputed', 'unverified') and item['id'] not in answered:
            item['history'].append(_entry(state, round_, actor, 'no response'))
    items.extend(fresh)
    if actor == 'challenger':
        verdict = payload.get('verdict') if payload.get('verdict') in ('accept', 'reject') else None
        state['verdict'] = verdict
        if verdict is None:
            state['verdict_note'] = 'The challenger returned no valid verdict; the cycle continues as rejected.'
        elif verdict == 'accept' and unresolved(state):
            state['verdict_note'] = 'The challenger said accept while findings still await assessment or defense; the ledger does not converge.'
        else:
            state['verdict_note'] = ''


def unresolved(state):
    statuses = IMPLEMENT_UNRESOLVED if state['stage'] == 'implement' else REVIEW_UNRESOLVED
    return [item for item in state['items'] if item.get('status') in statuses
            and (state['stage'] != 'implement' or item.get('severity') != 'low')]


def converged(state):
    return state.get('verdict') == 'accept' and not unresolved(state)


def ledger_for_prompt(items, limit=LEDGER_PROMPT_LIMIT):
    """Bounded JSON ledger: active items in full, closed items as identifiers."""
    def row(item, full, evidence_limit=TEXT_LIMIT):
        data = {'id': item['id'], 'origin': item['origin'], 'status': item['status'], 'severity': item['severity'],
                'file': item['file'], 'line': item['line'], 'claim': item['claim']}
        if full:
            data['evidence'] = item['evidence'][:evidence_limit]
            noted = [entry for entry in item['history'] if entry.get('note')]
            if noted:
                data['last_response'] = {'actor': noted[-1]['actor'], 'action': noted[-1]['action'], 'note': noted[-1]['note'][:evidence_limit]}
        return data
    active = [item for item in items if item['status'] not in TERMINAL]
    closed = [item for item in items if item['status'] in TERMINAL]
    for evidence_limit in (TEXT_LIMIT, 200, 0):
        payload = {'active': [row(item, True, evidence_limit) for item in active],
                   'closed': [row(item, False) for item in closed] if evidence_limit == TEXT_LIMIT else
                   {status: [item['id'] for item in closed if item['status'] == status] for status in TERMINAL if any(i['status'] == status for i in closed)}}
        text = json.dumps(payload, ensure_ascii=False)
        if len(text) <= limit:
            return text
    return text[:limit - 40] + ' …[ledger truncated; inspect the workspace]'


def _header(state):
    stage = 'implementation' if state['stage'] == 'implement' else 'review'
    protagonist, challenger = state['protagonist'], state['challenger']
    return (f"Harness clash workflow — {stage} stage, cycle {state['cycle']}. Two AI providers work on the same task and confront each other: "
            f"{_name(protagonist['provider'])} as {protagonist['role']} and {_name(challenger['provider'])} as challenger. "
            f"Up to {state['rounds']} challenge round(s) per cycle; the challenger's final verdict ends the cycle, and a human reviews the ledger "
            "before any follow-up. Do not ask questions; state assumptions explicitly. Neither participant may spawn additional agents.\n\n")


FINDING_SHAPE = '{"id": 1, "severity": "high", "file": "src/example.php", "line": 42, "claim": "One sentence describing the defect.", "evidence": "What you verified and how."}'


def opening_prompt(state, task):
    protagonist, challenger = _name(state['protagonist']['provider']), _name(state['challenger']['provider'])
    if state['stage'] == 'implement':
        return (_header(state) + f"You are the IMPLEMENTER ({protagonist}). Implement the task below completely in this workspace and verify it with the "
                f"project's tests or checks where available. The challenger ({challenger}) will then inspect the resulting change and try to find real "
                "defects; you will be able to fix or rebut each objection afterwards. Finish with a concise summary: what changed (files), how it was "
                "verified, and known limitations.\n\nTask:\n" + task + "\n")
    return (_header(state) + f"You are REVIEWER A ({protagonist}). Review the scope below without modifying any files. Report only findings you verified, "
            f"with file references and evidence, and rate severity honestly. The challenger ({challenger}) will review the same scope independently, "
            "dispute your findings and add what you missed; you will then defend or withdraw each disputed finding.\n\nScope:\n" + task +
            '\n\nEnd your answer with exactly one fenced JSON block in this shape:\n```json\n{"findings": [' + FINDING_SHAPE + ']}\n```\n'
            'Number findings from 1. Severity is high, medium or low.\n')


def continue_prompt(state, followup):
    protagonist = _name(state['protagonist']['provider'])
    ledger = ledger_for_prompt(state['items'])
    if state['stage'] == 'implement':
        return (_header(state) + f"You are the IMPLEMENTER ({protagonist}), continuing the clash after human review. Address the follow-up below in this "
                "workspace and verify your work. The objection ledger from previous cycles is attached as reference data: open objections still count "
                "unless the follow-up says otherwise. Finish with a concise summary of what changed and how it was verified.\n\nFollow-up:\n"
                + followup + "\n\nObjection ledger:\n" + ledger + "\n")
    return (_header(state) + f"You are REVIEWER A ({protagonist}), continuing the clash after human review. Address the follow-up below without modifying "
            "any files. The finding ledger from previous cycles is attached; keep existing finding ids and add new findings only with evidence.\n\nFollow-up:\n"
            + followup + "\n\nFinding ledger:\n" + ledger +
            '\n\nEnd your answer with exactly one fenced JSON block in this shape:\n```json\n{"findings": [' + FINDING_SHAPE + ']}\n```\n'
            f'New finding ids continue from {_next_id(state["items"])}.\n')


def challenge_prompt(state, round_, diff=None):
    protagonist, challenger = _name(state['protagonist']['provider']), _name(state['challenger']['provider'])
    rounds = state['rounds']
    task = state['task'] or '(the task text is unavailable; infer it from the workspace and ledger)'
    followup = f"\n\nLatest human follow-up:\n{state['followup']}" if state.get('followup') else ''
    next_id = _next_id(state['items'])
    if state['stage'] == 'implement':
        text = (_header(state) + f"You are the CHALLENGER ({challenger}), round {round_} of {rounds}. The implementer ({protagonist}) claims to have completed "
                "the task below. Find real defects in the current workspace state: bugs, missed or misread requirements, security issues, broken or missing "
                "tests, and unverified claims in the summary. Be adversarial but factual: verify each objection by reading the workspace (and running "
                "read-only checks where your tools allow) and cite file paths and lines. Do not modify any files. Do not raise style preferences as objections.\n")
        if state['items']:
            text += ("The implementer responded to earlier objections (ledger below). Re-check each one in the workspace: list the ids of objections that are "
                     "now fixed under \"resolved\"; keep an objection (same id) only if it still stands; add new objections only with evidence.\n")
        text += f"\nTask given to the implementer:\n{task}{followup}\n\nImplementer's latest summary:\n{state.get('summary') or '(no summary was returned)'}\n\n"
        if diff and diff.get('text'):
            text += ("Workspace diff versus the session baseline" + (" (truncated; inspect the workspace for the rest)" if diff.get('truncated') else '')
                     + ":\n```diff\n" + diff['text'] + "\n```\n\n")
        else:
            text += "No Git diff is available for this workspace; inspect the files directly.\n\n"
        if state['items']:
            text += "Objection ledger:\n" + ledger_for_prompt(state['items']) + "\n\n"
        return (text + 'End your answer with exactly one fenced JSON block in this shape:\n```json\n{"verdict": "reject", "objections": ['
                + FINDING_SHAPE + '], "resolved": []}\n```\n'
                f'Rules: keep previous objection ids; new objections continue from {next_id}; severity is high, medium or low; '
                'use "accept" only when no high or medium objection remains, with an empty objections list.\n')
    text = (_header(state) + f"You are the CHALLENGER (reviewer B, {challenger}), round {round_} of {rounds}. Reviewer A ({protagonist}) reported the findings in "
            "the ledger below. First review the scope independently. Then assess every ledger item that awaits your assessment (status proposed or defended, "
            "origin reviewer): \"agree\" only after verifying it yourself, \"dispute\" with evidence when it is wrong, overstated or out of scope, "
            "\"unverified\" when you could not check it. Respond to disputes on your own findings (status disputed or unverified, origin challenger) with "
            "\"defend\" or \"withdraw\". Add findings reviewer A missed under \"additional\", with evidence. Do not modify any files.\n\n"
            f"Scope:\n{task}{followup}\n\nFinding ledger:\n{ledger_for_prompt(state['items'])}\n\n")
    return (text + 'End your answer with exactly one fenced JSON block in this shape:\n```json\n{"verdict": "reject", "assessments": [{"id": 1, "stance": "dispute", '
            '"evidence": "What you verified."}], "responses": [{"id": 101, "action": "defend", "note": "Why it stands."}], "additional": [' + FINDING_SHAPE + ']}\n```\n'
            f'Rules: new finding ids continue from {next_id}; use "accept" only when you agree with every remaining finding and have nothing to add.\n')


def response_prompt(state, round_):
    protagonist, challenger = _name(state['protagonist']['provider']), _name(state['challenger']['provider'])
    rounds = state['rounds']
    ledger = ledger_for_prompt(state['items'])
    if state['stage'] == 'implement':
        return (_header(state) + f"You are the IMPLEMENTER ({protagonist}), round {round_} of {rounds}: respond to the challenger. {challenger} raised the open "
                "objections below against your implementation. For each open objection either FIX it in the workspace now (and rerun the relevant tests) or "
                "REBUT it with concrete evidence that it is wrong, already handled or out of scope. Do not ignore any objection; do not concede a wrong objection "
                "just to end the debate; do not dismiss a valid defect. Use \"deferred\" only for work explicitly outside the task's scope, and say why.\n\n"
                "Objection ledger:\n" + ledger + '\n\nEnd your answer with exactly one fenced JSON block in this shape:\n```json\n{"responses": [{"id": 1, '
                '"action": "fixed", "note": "What changed and how it was verified."}, {"id": 2, "action": "rebutted", "note": "Why the objection does not hold."}]}\n```\n'
                'Actions: fixed, rebutted, deferred. Include every open objection id.\n')
    return (_header(state) + f"You are REVIEWER A ({protagonist}), round {round_} of {rounds}: respond to the challenger. For each of your findings the challenger "
            "disputed or could not verify (status disputed or unverified, origin reviewer): \"defend\" it with stronger evidence or \"withdraw\" it. For each "
            "finding the challenger added (status proposed or defended, origin challenger): \"agree\" after verifying it or \"dispute\" with evidence. Do not "
            "modify any files. Do not withdraw a correct finding to end the debate; do not defend a finding you cannot support.\n\nFinding ledger:\n" + ledger +
            '\n\nEnd your answer with exactly one fenced JSON block in this shape:\n```json\n{"assessments": [{"id": 101, "stance": "agree", "evidence": "What you '
            'verified."}], "responses": [{"id": 1, "action": "defend", "note": "Stronger evidence."}]}\n```\n')


def record_turn(state, turn):
    state['turns'] = (state['turns'] + [turn])[-MAX_TURNS:]


def outcome_message(state):
    counts = {}
    for item in state['items']:
        counts[item['status']] = counts.get(item['status'], 0) + 1
    summary = ', '.join(f'{count} {status}' for status, count in sorted(counts.items())) or 'no ledger items'
    noun = 'objections' if state['stage'] == 'implement' else 'findings'
    if state['outcome'] == 'converged':
        return f"Converged in round {state['round']} of cycle {state['cycle']}: the challenger accepted. {noun.capitalize()}: {summary}."
    if state['outcome'] == 'unresolved':
        return f"Unresolved after {state['round']} round(s) of cycle {state['cycle']}: the challenger's last verdict was {state['verdict'] or 'missing'}. {noun.capitalize()}: {summary}."
    return f"Incomplete: {state.get('message') or 'a turn did not finish'}. {noun.capitalize()}: {summary}."


def render_report(state):
    protagonist, challenger = state['protagonist'], state['challenger']
    lines = [f"# Clash — {'implementation' if state['stage'] == 'implement' else 'review'} stage", '',
             f"{protagonist['role'].capitalize()}: {_name(protagonist['provider'])} ({protagonist.get('model') or 'default model'}, {protagonist.get('thinking_effort') or 'default effort'})",
             f"Challenger: {_name(challenger['provider'])} ({challenger.get('model') or 'default model'}, {challenger.get('thinking_effort') or 'default effort'})",
             f"Cycle {state['cycle']}, round {state['round']} of {state['rounds']}; outcome: {state.get('outcome') or 'running'}; challenger verdict: {state.get('verdict') or 'none'}.",
             '']
    if state.get('verdict_note'):
        lines += [state['verdict_note'], '']
    if state.get('message'):
        lines += [state['message'], '']
    lines.append('## ' + ('Objections' if state['stage'] == 'implement' else 'Findings'))
    if not state['items']:
        lines.append('None recorded.')
    for item in state['items']:
        location = item['file'] or '-'
        if item['line']:
            location += f":{item['line']}"
        lines.append(f"- #{item['id']} [{item['severity']}] {item['status']} — {location} ({item['origin']}): {item['claim']}")
        if item['evidence']:
            lines.append(f"  evidence: {item['evidence']}")
        for entry in item['history'][1:]:
            note = f" — {entry['note']}" if entry.get('note') else ''
            lines.append(f"  cycle {entry['cycle']} round {entry['round']} · {entry['actor']}: {entry['action']}{note}")
    lines += ['', '## Turns']
    for turn in state['turns']:
        cost = f"${turn['cost_usd']:.4f}" if isinstance(turn.get('cost_usd'), (int, float)) else 'cost unknown'
        seconds = f"{turn['seconds']:.1f}s" if isinstance(turn.get('seconds'), (int, float)) else ''
        status = 'ok' if turn.get('ok') else f"failed{': ' + turn['error'] if turn.get('error') else ''}"
        verdict = f"; verdict {turn['verdict']}" if turn.get('verdict') else ''
        lines.append(f"- cycle {turn.get('cycle')} round {turn.get('round')} · {turn.get('role')} · {_name(turn.get('provider'))}: {status}, {cost}, {seconds}{verdict}"
                     + (f" — {turn['summary']}" if turn.get('summary') else ''))
    text = '\n'.join(lines) + '\n'
    return text if len(text) <= REPORT_LIMIT else text[:REPORT_LIMIT - 30] + '\n…[report truncated]\n'
