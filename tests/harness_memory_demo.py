"""A demo project for Knowledge › Memory use: a year of Project Brain, promotion and Memory bank history.

Records, promotions, re-attestations, retirements and promoted chunks go through the
copied runtime's own API, with its clock moved to each event's moment, so every
shape is one the runtime writes and every rule it enforces still applies. Two
chunks pass their review date three days before today, and because an overdue
chunk fails bank validation, the two automatic promotions after it stall.
Retrieval manifests and refresh-health lines are written directly against the
manifest schema: 200 real retrievals over a small fixture would select the same
few documents.

    python3 tests/harness_memory_demo.py <empty directory>
"""
from __future__ import annotations

from datetime import datetime, timedelta, timezone
import hashlib
import json
from pathlib import Path
import random
import subprocess
import sys
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tests.test_harness_knowledge import install_knowledge_fixture

KNOWLEDGE = [
    "Retry webhook deliveries with backoff", "Totals are computed in minor units", "Idempotency keys expire after a day",
    "Refunds reverse the original capture only", "Inventory holds release after fifteen minutes",
    "Tax is rounded per line, not per order", "Currency conversion uses the daily reference rate",
    "Order numbers are sequential per store", "Payment provider timeouts are retried once",
    "Discount codes never stack with gift cards", "Shipping labels are generated after capture",
    "Customer emails are queued, never sent inline", "Cart prices are revalidated at checkout",
    "Partial shipments split the invoice", "Chargebacks freeze the customer wallet", "Product slugs are immutable after publish",
    "Search indexing runs after the transaction commits", "Admin exports are capped at fifty thousand rows",
    "Password reset links expire after one hour", "Guest checkout stores no payment tokens",
    "Subscription renewals run at two in the morning UTC", "Failed renewals retry on days one, three and seven",
    "Webhook signatures use HMAC with SHA-256", "Stock is decremented on payment, not on cart",
    "Cancelled orders restock automatically", "Gift cards hold a single currency", "Address validation never blocks checkout",
    "Free shipping thresholds use the pre-tax total", "Coupon codes are case-insensitive", "Price changes apply to new carts only",
    "Order emails include the tax breakdown", "Fraud checks run before authorization", "Card authorizations expire after seven days",
    "Captures happen when the order ships", "Multi-warehouse picks prefer the closest stock",
    "Backorders require explicit customer consent", "Returns are accepted within thirty days",
    "Restocking fees apply only to opened items", "Store credit never expires", "Reviews are published after moderation",
    "Image uploads are resized asynchronously", "Catalog imports run in batches of five hundred",
    "API rate limits apply per token, not per address", "Pagination uses opaque cursors",
    "Deleted customers keep anonymised orders", "Audit logs are append-only", "Feature flags default to off in production",
    "Cache keys include the store and currency", "Price rounding uses banker's rounding", "VAT numbers are validated online",
    "Invoice numbers have no gaps per fiscal year", "Payment tokens are scoped to one customer",
    "Abandoned carts are purged after ninety days", "Locale fallbacks end at English", "Timezones are stored as IANA names",
    "Order status changes emit domain events", "Webhooks are delivered at least once", "Consumers deduplicate by event ID",
    "Bulk price updates lock the catalog briefly", "Checkout totals are recomputed on the server",
    "Promotions are evaluated in priority order", "Bundle items share one inventory reservation",
    "Digital goods skip shipping entirely", "Download links expire after seventy-two hours",
    "Loyalty points post after the return window", "Shipping rates are cached for ten minutes",
    "Out-of-stock items hide from search", "Customer groups override catalog prices", "Minimum order values exclude shipping",
    "Pre-orders capture at the release date", "Split payments allow at most two methods",
    "Currency is fixed when the cart is created", "Packing slips never show prices", "Warehouse cut-off is three in the afternoon",
    "Gift wrapping is priced per item", "Marketplace sellers settle weekly", "Saved carts follow the customer across devices",
    "Courier tracking numbers are validated per carrier", "Product variants share one review thread",
    "Sales reports use the store's timezone", "Customs forms list the country of origin", "Order notes are limited to 500 characters",
    "Price alerts fire once per drop", "Archived products keep their order history", "Stock thresholds trigger reorder emails",
    "Payment retries never change the amount",
]
TASKS = ["Checkout v2 migration", "Webhook retry dashboard", "Catalog import speed-up", "Tax engine upgrade",
         "Refund flow audit", "Returns portal", "Multi-currency carts", "Inventory sync rewrite"]
SPECS = ["specs/payments.md", "specs/catalog.md", "specs/shipping.md", "specs/orders.md"]
DOC_TYPES = ["domain", "constraint", "operations", "decision", "convention", "integration", "architecture"]
FILES = {'AGENTS.md': '# Shop API agents\n\nFollow the specs.\n', 'CHANGELOG.md': '# Changelog\n',
         'specs/authority.md': '# Cobalt authority\n\nThe cobalt allocation rule requires one owner.\n',
         'docs/tax-rounding.md': '# Tax rounding\n\nRound tax per line.\n',
         'specs/refunds.md': '# Refunds\n\nRefunds reverse the original capture only.\n',
         **{spec: f'# {Path(spec).stem.title()}\n\nThe {Path(spec).stem} rules of the shop API.\n' for spec in SPECS}}

RUNTIME = r'''
import json, sys
from datetime import date as real_date, datetime as real_datetime
from pathlib import Path
root, plan = Path(sys.argv[1]), json.loads(Path(sys.argv[2]).read_text())
sys.path.insert(0, str(root / 'memory-bank/scripts'))
import brain_runtime as b
import validate
clock = {}
class Clock(real_datetime):
    @classmethod
    def now(cls, tz=None):
        return clock['now'].astimezone(tz) if tz else clock['now'].replace(tzinfo=None)
class Today(real_date):
    @classmethod
    def today(cls):
        return clock['now'].date()
b.datetime, validate.date = Clock, Today
owner = (b.load_config(root).get('owners') or ['*'])[0]
records, chunks, promotions, log = {}, {}, {}, []
for step in plan:
    clock['now'] = real_datetime.fromisoformat(step['at'])
    action = step['do']
    if action == 'chunk':
        path = root / 'memory-bank/chunks' / step['name']
        path.write_text('---\n' + json.dumps(step['metadata'], indent=2) + '\n---\n\n# ' + step['metadata']['title'] + '\n\n' + step['body'] + '\n', encoding='utf-8')
        b.reindex_bank(root)
        chunks[step['key']] = step['metadata']['id']
    elif action == 'record':
        record = b.create_record(root, step['type'], step['external_id'], step['title'], [], step['sources'], owner=owner,
                                 privacy=step['privacy'], authority=step['authority'], goal=step['title'])
        records[step['key']] = record
    elif action == 'transition':
        record = records[step['key']]
        records[step['key']] = b.update_record(root, record['id'], expected_revision=record['revision'], progress=step['progress'],
                                               next_steps=[], files=[], sources=record['sources'], actor=owner,
                                               transition_to=step['to'], reason='Fixture lifecycle')
    elif action == 'auto_promote':
        result = b.auto_promote(root, owner=owner, limit=20)
        for item in result['promoted']:
            key = next(key for key, record in records.items() if record['id'] == item['record_id'])
            chunks[key] = item['memory_id']
        log.append({'at': step['at'], 'promoted': len(result['promoted']), 'failed': len(result['failed']), 'blocked': len(result['blocked'])})
    elif action == 'propose':
        promotions[step['key']] = b.create_promotion(root, [records[step['key']]['id']], step['title'], step['content'], proposer=owner)
    elif action == 'review':
        b.review_promotion(root, promotions[step['key']]['id'], 'fixture-reviewer', True)
    elif action == 'apply':
        chunks[step['key']] = b.apply_promotion(root, promotions[step['key']]['id'])['destination_memory_id']
    elif action == 'compact':
        b.compact(root)
    elif action == 'reverify':
        b.reverify_chunk(root, chunks[step['key']])
    elif action == 'retire':
        b.retire_chunk(root, chunks[step['key']], valid_to=clock['now'].date().isoformat(),
                       superseded_by=chunks.get(step.get('by')), reason='Fixture retirement')
    elif action == 'edit':
        (root / step['path']).write_text(step['text'], encoding='utf-8')
print(json.dumps({'records': {key: record['id'] for key, record in records.items()}, 'chunks': chunks, 'log': log}))
'''


def _sha(text):
    return hashlib.sha256(text.encode('utf-8')).hexdigest()


def _plan(now):
    """The project's history as runtime steps, oldest first, with the story's expected numbers."""
    day0 = now.replace(hour=9, minute=0, second=0, microsecond=0)
    if day0 >= now:
        day0 -= timedelta(days=1)

    def at(days, hour=9, minute=0):
        return (day0 - timedelta(days=days)).replace(hour=hour, minute=minute)
    titles = iter(KNOWLEDGE)
    steps, expected = [], {'hand_written': 0, 'drafts': 0}

    def add(moment, action, **fields):
        steps.append({'at': moment.isoformat(), 'do': action, **fields})

    def chunk(key, days, number, status='active', review_days=None, sources=('specs/authority.md',)):
        """A hand-written chunk, with digests of what it cites as they read when it was written."""
        title = next(titles)
        created = at(days).date()
        # Hand-written knowledge carries a longer review period than promoted chunks' year.
        review = at(review_days).date() if review_days is not None else created + timedelta(days=490)
        metadata = {'id': f'MEM-{number:04d}', 'title': title, 'type': DOC_TYPES[number % len(DOC_TYPES)], 'status': status,
                    'scope': ['application'], 'tags': ['shop-api'], 'created': created.isoformat(),
                    'last_verified': created.isoformat(), 'review_after': review.isoformat(), 'sources': list(sources),
                    'supersedes': [], 'superseded_by': None,
                    'source_digests': [{'path': source, 'sha256': _sha(FILES[source])} for source in sources]}
        slug = '-'.join(title.lower().replace(',', '').replace("'", '').split()[:4])
        add(at(days), 'chunk', key=key, name=f'MEM-{number:04d}-{slug}.md', metadata=metadata,
            body=f'{title}. Written by the team while building the shop API.')
        expected['hand_written'] += 1
        expected['drafts'] += status == 'needs-review'

    def resolve(key, days, kind='finding', authority='verified', privacy='team', source=None):
        title = next(titles)
        external = f'{kind[0].upper()}-{len(steps) + 100}'
        add(at(days + 1, 15), 'record', key=key, type=kind, external_id=external, title=title,
            sources=[source or SPECS[len(steps) % len(SPECS)]], privacy=privacy, authority=authority)
        progress = f'{title}: confirmed against the specification while working on {external}.'
        target = {'finding': 'resolved', 'bug': 'resolved', 'incident': 'closed', 'decision': 'accepted'}[kind]
        path = {'finding': ['investigating'], 'bug': ['triaged', 'fixing', 'verifying'], 'incident': ['contained', 'resolved'],
                'decision': []}[kind]
        for index, state in enumerate(path):
            add(at(days + 1, 16, index), 'transition', key=key, to=state, progress=progress)
        add(at(days), 'transition', key=key, to=target, progress=progress)

    # Hand-written knowledge from the project's first year: 19 active, 2 drafts.
    for number in range(1, 20):
        chunk(f'hand-{number}', 430 - number * 7, number,
              sources=('docs/tax-rounding.md',) if number == 6 else ('specs/authority.md',),
              review_days=3 if number == 7 else None)
    chunk('draft-1', 200, 20, status='needs-review')
    chunk('draft-2', 120, 21, status='needs-review')
    # 34 older automatic promotions and 6 reviewed by a person, before the 30-day window.
    older = [(f'auto-{index}', 368 if index == 0 else 340 - index * 9) for index in range(34)]
    human = [(f'human-{index}', 300 - index * 40) for index in range(6)]
    for index, (key, days) in enumerate(sorted(older + human, key=lambda item: -item[1])):
        kind = ('finding', 'bug', 'finding', 'incident', 'bug')[index % 5]
        resolve(key, days, kind, source='specs/refunds.md' if key == 'auto-5' else None)
        if key.startswith('human'):
            add(at(days, 10), 'propose', key=key, title=f'Reviewed: {key}', content=f'Durable rule from {key}, reviewed by a person.')
            add(at(days - 1, 11), 'review', key=key)
            add(at(days - 1, 12), 'apply', key=key)
        else:
            add(at(days, 10), 'auto_promote')
    # Nine automatic chunks were re-attested since promotion; five chunks were retired, two inside the window.
    for index in range(10, 19):
        add(at(340 - index * 9 - 40 - index * 5, 13), 'reverify', key=f'auto-{index}')
    for key, by, days in (('auto-20', 'auto-21', 150), ('auto-22', 'auto-23', 90), ('hand-3', 'auto-24', 24),
                          ('hand-4', None, 60), ('hand-5', None, 11)):
        add(at(days, 14), 'retire', key=key, **({'by': by} if by else {}))
    # Completed tasks and private findings that never become durable memory.
    for index in range(5):
        add(at(200 - index * 30, 9), 'record', key=f'done-{index}', type='task', external_id=f'TASK-{index + 1}',
            title=TASKS[index], sources=['specs/orders.md'], privacy='team', authority='verified')
        add(at(199 - index * 30, 9), 'transition', key=f'done-{index}', to='completed', progress=f'{TASKS[index]} shipped.')
    for index in range(2):
        resolve(f'private-{index}', 100 - index * 30, privacy='private')
    add(at(45, 18), 'compact')
    # The 30-day window: 12 records resolved.
    window = [('w-auto-0', 27, 'finding'), ('w-auto-1', 24, 'bug'), ('w-auto-2', 22, 'decision'), ('w-auto-3', 18, 'finding'),
              ('w-human', 16, 'finding'), ('w-observed', 12, 'finding'), ('w-auto-4', 11, 'incident'),
              ('w-restricted', 9, 'bug'), ('w-waiting', 5, 'finding'), ('w-auto-5', 4, 'finding'),
              ('w-stalled-0', 2, 'bug'), ('w-stalled-1', 1, 'finding')]
    for key, days, kind in window:
        if key == 'w-observed':
            resolve(key, days, kind, authority='observed')
        elif key == 'w-restricted':
            resolve(key, days, kind, privacy='restricted')
        else:
            resolve(key, days, kind)
        if key == 'w-human':
            add(at(days, 9, 30), 'propose', key=key, title='Refund retries use the payment ledger', content='Retry refunds from the payment ledger, never from the order.')
            add(at(days - 2, 10), 'review', key=key)
            add(at(days - 2, 10, 30), 'apply', key=key)
        elif key == 'w-waiting':
            add(at(days, 9, 30), 'propose', key=key, title='Webhook consumers acknowledge within five seconds', content='Acknowledge webhook deliveries within five seconds and process later.')
        add(at(days, 10), 'auto_promote')
    add(at(6, 18), 'compact')
    # Open work.
    for index, (kind, state) in enumerate((('task', 'active'), ('task', 'active'), ('task', 'active'),
                                            ('finding', 'investigating'), ('finding', 'investigating'), ('bug', 'fixing'))):
        key = f'open-{index}'
        title = TASKS[5 + index % 3] if kind == 'task' else f'Open question {index}: {next(titles)}'
        add(at(10 - index, 11), 'record', key=key, type=kind, external_id=f'OPEN-{index + 1}', title=title,
            sources=['specs/orders.md'], privacy='team', authority='verified')
        for step_index, step_state in enumerate(['investigating'] if state == 'investigating' else ['triaged', 'fixing'] if state == 'fixing' else []):
            add(at(9 - index, 11, step_index), 'transition', key=key, to=step_state, progress=f'Working on {title}.')
    # Two cited files changed after their chunks were last attested.
    add(now - timedelta(hours=2), 'edit', path='specs/refunds.md', text='# Refunds\n\nPartial refunds now reverse the capture in order.\n')
    add(now - timedelta(hours=2), 'edit', path='docs/tax-rounding.md', text='# Tax rounding\n\nRound tax per order for B2B invoices.\n')
    steps.sort(key=lambda step: step['at'])
    return steps, expected


def _retrievals(project, now, chunk_ids, task_ids, random_source):
    """200 retrieval manifests and their refresh-health lines, newest last, with the story's counts."""
    day0 = now.replace(hour=0, minute=0, second=0, microsecond=0)
    per_day = {21: 9, 20: 6, 18: 12, 17: 13, 16: 9, 15: 11, 14: 8, 13: 2, 11: 13, 10: 10, 9: 12, 8: 9, 7: 12, 6: 6, 5: 4,
               4: 13, 3: 12, 2: 15, 1: 14, 0: 10}
    moments = []
    for days, count in per_day.items():
        start = day0 - timedelta(days=days)
        for index in range(count):
            if days == 0:
                moments.append(now - timedelta(minutes=12 * (count - index)))
            else:
                moments.append(start + timedelta(hours=8, minutes=int(index * 600 / count) + random_source.randrange(5)))
    moments.sort()
    with_chunks = random_source.sample(range(200), 31)
    sizes = [1] * 23 + [2] * 6 + [3] * 2
    random_source.shuffle(sizes)
    bank = sorted(chunk_ids)
    favourites = random_source.sample(bank, 17)
    routes = ['claude'] * 152 + ['harness'] * 21 + ['cli'] * 9 + ['v1'] * 9 + ['v2'] * 9
    random_source.shuffle(routes)
    cuts = ['layer-limit'] * 9 + ['budget'] * 2 + ['source-changed'] * 1
    cut_at = random_source.sample(range(200), len(cuts))
    dropped = set(random_source.sample(range(200), 118))
    governed = project / 'project-brain/control/retrieval-manifests'
    local = project / 'memory-bank/local/retrieval-manifests'
    governed.mkdir(parents=True, exist_ok=True)
    local.mkdir(parents=True, exist_ok=True)
    health, selections, pairs = [], 0, 0
    used = {}
    for index, moment in enumerate(moments):
        task = task_ids[index % len(task_ids)]
        selected = [{'path': 'AGENTS.md', 'category': 'policy', 'estimated_tokens': 120, 'source_hash': _sha('agents')},
                    {'path': '.claude/skills/checkout/SKILL.md', 'category': 'policy', 'estimated_tokens': 90, 'source_hash': _sha('skill')}]
        count = sizes[with_chunks.index(index)] if index in with_chunks else 0
        for slot in range(count):
            identity = favourites[(index + slot * 5) % 17]
            used.setdefault(identity, set()).add(task)
            selected.append({'path': f'memory-bank/chunks/{chunk_ids[identity]}', 'category': 'durable',
                             'estimated_tokens': 80, 'source_hash': _sha(identity)})
            selections += 1
        for slot in range(3 - count):
            selected.append({'path': f'project-brain/dynamic/tasks/{task}.md', 'category': 'dynamic', 'estimated_tokens': 60,
                             'source_hash': _sha(task)} if slot == 0 else
                            {'path': SPECS[(index + slot) % len(SPECS)], 'category': 'evidence', 'estimated_tokens': 70,
                             'source_hash': _sha(SPECS[(index + slot) % len(SPECS)])})
        episode = index % 10
        if episode < 6:
            selected.append({'path': 'CHANGELOG.md', 'category': 'evidence', 'estimated_tokens': 50, 'source_hash': _sha('changelog')})
        excluded = []
        if index in cut_at:
            identity = bank[(index * 7) % len(bank)]
            excluded.append({'path': f'memory-bank/chunks/{chunk_ids[identity]}', 'reason': cuts[cut_at.index(index)]})
        route = routes[index]
        manifest = {'schema_version': 1 if route == 'v1' else 2 if route == 'v2' else 3, 'id': str(uuid.UUID(int=random_source.getrandbits(128), version=4)),
                    'created_at': moment.isoformat(), 'query': f'fixture query {index}', 'task_id': task, 'task_revision': 1 + index % 3,
                    'filters': {'privacy': ['public', 'team'], 'owners': ['*'], 'authority': ['verified', 'observed'], 'freshness': True,
                                'active_only': True},
                    'selected': selected, 'excluded': excluded,
                    'token_estimates': {'policy': 210, 'handoff': 0, 'durable': 80 * count, 'dynamic': 60, 'evidence': 140,
                                        'total': 410 + 80 * count, 'target': 8000, 'hard': 12000},
                    'provider': 'fixture', 'escalation_reason': None}
        if episode in (6, 7, 8):
            manifest['local_episode_count'] = 1
        if route == 'v2':
            manifest['query_source'] = 'prompt'
        elif route != 'v1':
            manifest.update(query_source='task' if route == 'harness' else 'prompt',
                            host='claude' if route == 'claude' else 'cli',
                            entry_point='hook-context' if route == 'claude' else 'retrieve',
                            gate={'decision': 'retrieve', 'mode': 'off' if route == 'harness' else 'shadow',
                                  'reason': 'gate-off' if route == 'harness' else 'new-selection',
                                  'signals': {'informative_terms': 3, 'distinctive_matches': 1, 'top_score': 0.4, 'no_match': [],
                                              'query_unchanged_from_previous_turn': False,
                                              'selection_identical_to_previous_turn': False}})
        # Hooks retrieve ephemerally into the local store; a plain CLI or Harness retrieval is governed.
        store = governed if route in ('harness', 'cli') else local
        (store / f"{manifest['id']}.json").write_text(json.dumps(manifest, indent=2), encoding='utf-8')
        # Every Harness launch re-checks freshness into the local store before running the provider.
        if route == 'harness' and pairs < 15:
            pairs += 1
            check = {**manifest, 'id': str(uuid.UUID(int=random_source.getrandbits(128), version=4)),
                     'created_at': (moment + timedelta(minutes=2)).isoformat()}
            (local / f"{check['id']}.json").write_text(json.dumps(check, indent=2), encoding='utf-8')
        if route != 'harness':
            health.append({'at': moment.isoformat(), 'mode': 'governed', 'source': 'hook-context' if route == 'claude' else 'refresh',
                           'omitted': {'procedural': 0, 'semantic': 2 if index in dropped else 0, 'episodic': 0}})
    (project / 'memory-bank/local/refresh-health.ndjson').write_text(
        ''.join(json.dumps(line) + '\n' for line in health), encoding='utf-8')
    return {'retrievals': 200, 'merged': pairs, 'with_chunks': 31, 'selections': selections, 'distinct': len(used),
            'reused': sum(len(tasks) > 1 for tasks in used.values()), 'cuts': len(cuts),
            'dropped': sum(1 for index in range(200) if index in dropped and routes[index] != 'harness'),
            'health': len(health), 'routes': {name: routes.count(name) for name in set(routes)},
            'harness_tasks': sorted({task_ids[index % len(task_ids)] for index in range(200) if routes[index] == 'harness'})}


def build(project, now=None, seed=7):
    """Write the demo into an empty directory and return the numbers its story should read back as."""
    project = Path(project)
    now = (now or datetime.now(timezone.utc)).astimezone(timezone.utc)
    project.mkdir(parents=True, exist_ok=True)
    install_knowledge_fixture(project)
    config_path = project / 'project-brain/config/runtime.json'
    config = json.loads(config_path.read_text(encoding='utf-8'))
    config.update(automatic_promotion=True, automatic_compaction=False)
    config_path.write_text(json.dumps(config, indent=2) + '\n', encoding='utf-8')
    for name, text in FILES.items():
        (project / name).parent.mkdir(parents=True, exist_ok=True)
        (project / name).write_text(text, encoding='utf-8')
    steps, expected = _plan(now)
    plan_path = project.parent / f'.{project.name}-plan.json'
    plan_path.write_text(json.dumps(steps), encoding='utf-8')
    try:
        result = subprocess.run([sys.executable, '-c', RUNTIME, str(project), str(plan_path)], capture_output=True, text=True,
                                timeout=600, env={'PATH': '/usr/bin:/bin', 'PYTHONDONTWRITEBYTECODE': '1'})
    finally:
        plan_path.unlink(missing_ok=True)
    if result.returncode:
        raise RuntimeError(result.stderr[-4000:])
    story = json.loads(result.stdout)
    files = {path.name.split('-', 3)[0] + '-' + path.name.split('-', 3)[1] if path.name.startswith('MEM-0') else '-'.join(path.name.split('-')[:3]): path.name
             for path in (project / 'memory-bank/chunks').glob('*.md')}
    active = [identity for identity, name in files.items()
              if '"status": "active"' in (project / 'memory-bank/chunks' / name).read_text(encoding='utf-8')]
    # Retrievals work for tasks: the five completed ones and the three still active.
    tasks = [story['records'][key] for key in sorted(story['records']) if key.startswith('done-') or key in ('open-0', 'open-1', 'open-2')]
    expected.update(_retrievals(project, now, {identity: files[identity] for identity in active}, tasks, random.Random(seed)))
    expected.update(chunks=len(files), records=len(story['records']), log=story['log'])
    return expected


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('Usage: python3 tests/harness_memory_demo.py <empty directory>')
    print(json.dumps(build(Path(sys.argv[1]).resolve()), indent=2))
