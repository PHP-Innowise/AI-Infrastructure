"""One explicitly selected SDD phase per native-provider turn."""
import re

PHASES = [
    {'id': 'initialize', 'name': 'Initialize', 'description': 'Create missing project principles and SDD guidance.'},
    {'id': 'specify', 'name': 'Specify', 'description': 'Write the feature specification: what and why.'},
    {'id': 'plan', 'name': 'Plan', 'description': 'Translate the reviewed specification into a technical plan.'},
    {'id': 'tasks', 'name': 'Tasks', 'description': 'Break the reviewed plan into test-first tasks and checkpoints.'},
    {'id': 'implement', 'name': 'Implement / resume', 'description': 'Execute the next task group and stop at its checkpoint.'},
    {'id': 'review', 'name': 'Review', 'description': 'Check implementation against acceptance criteria without editing files.'},
]
REQUIRED = {'initialize': (), 'specify': (), 'plan': ('spec.md',),
            'tasks': ('spec.md', 'plan.md'),
            'implement': ('spec.md', 'plan.md', 'tasks.md'),
            'review': ('spec.md', 'plan.md', 'tasks.md')}
READ_LIMIT = 256 * 1024


def validate(value, workflow, previous=None):
    from .sessions import SessionError
    if workflow != 'sdd':
        if value is not None:
            raise SessionError('SDD settings require the SDD workflow.')
        return None
    if (not isinstance(value, dict) or set(value) != {'phase', 'feature'}
            or not isinstance(value['phase'], str) or value['phase'] not in REQUIRED
            or not isinstance(value['feature'], str)
            or not re.fullmatch(r'[a-z0-9]+(?:-[a-z0-9]+)*', value['feature'])
            or len(value['feature']) > 80 or value['feature'] == 'memory'):
        raise SessionError('Choose an SDD phase and a feature slug of 1–80 lowercase letters, digits and single hyphens (not memory).')
    if previous and value['feature'] != previous['feature']:
        raise SessionError('The SDD feature stays fixed for this session.')
    return dict(value)


def mode(settings):
    return 'plan' if settings['phase'] == 'review' else 'edit'


def document_paths(root, settings):
    """New documents follow the accelerator's skill-prefix naming contract."""
    legacy = ['specs/memory/constitution.md'] + [
        f"specs/{settings['feature']}/{name}" for name in ('spec.md', 'plan.md', 'tasks.md', 'progress.md')]
    paths = []
    for path in legacy:
        folder, name = path.rsplit('/', 1)
        prefixed = f'{folder}/sdd-{name}'
        # Retain existing documents in place, including pre-prefix Harness sessions.
        paths.append(path if root is not None and (root / path).exists()
                     and not (root / prefixed).exists() else prefixed)
    return paths


def artifacts(root, settings):
    from .sessions import read_context
    result = []
    for path in document_paths(root, settings):
        content = read_context(root, path, READ_LIMIT)
        result.append({'path': path, 'available': content is not None,
                       'text': content[1] if content else '',
                       'truncated': bool(content and content[0] > READ_LIMIT)})
    return result


def check(root, settings):
    from .sessions import SessionError
    files = dict(zip(('constitution.md', 'spec.md', 'plan.md', 'tasks.md', 'progress.md'),
                     artifacts(root, settings)))
    for name in REQUIRED[settings['phase']]:
        item = files[name]
        path = item['path']
        if not item['available'] or not item['text'].strip() or item['truncated']:
            raise SessionError(f'SDD requires a readable, nonempty {path} (at most 256 KiB). Complete the preceding phase first.')
        if name == 'spec.md' and '[NEEDS CLARIFICATION' in item['text']:
            raise SessionError('Resolve the specification’s NEEDS CLARIFICATION markers in Specify before continuing.')


def instructions(settings, root=None):
    phase = settings['phase']; folder = f"specs/{settings['feature']}"
    steps = {
        'initialize': 'Create only missing specs/memory/constitution.md with project principles. Append concise SDD guidance to AGENTS.md only if absent. Preserve all existing project instructions and specifications. Do not reinitialize or replace existing documents.',
        'specify': 'Write spec.md: Overview, User Stories, testable Acceptance Criteria, Non-Goals, Open Questions, Dependencies. Describe WHAT and WHY, not implementation details. Ask only unanswered questions and mark unresolved facts [NEEDS CLARIFICATION: question]. Do not write implementation code.',
        'plan': 'Read spec.md and write plan.md: Technical Approach, Key Decisions with rationale, Data Model, Interface Contracts, Implementation Phases, Risks and Mitigations. Respect non-goals and project principles. Do not write implementation code.',
        'tasks': 'Read spec.md and plan.md, then write tasks.md with small dependent tasks, test-before-implementation ordering and explicit verification. Use [ ] pending, [~] in progress, [x] verified complete, [P] independent groups and [C] checkpoints. Include final build and test verification from project commands. Do not implement tasks.',
        'implement': 'Read spec.md, plan.md, tasks.md and any progress.md. Resume the first unfinished task or independent group. Write and run failing tests before implementation, make them pass, then run the project verification commands. Mark completion only with evidence. At the next checkpoint record commands, outcomes, completed tasks, acceptance criteria and issues in progress.md, then STOP for review. Never mark unavailable checks as passed.',
        'review': 'Read spec.md, plan.md, tasks.md and progress.md. Review the actual implementation against each acceptance criterion and task completion claim. Report met, unmet and unverified criteria with file references and test evidence. Do not modify any files.',
    }
    text = (f'Harness SDD phase: {phase}. Feature documents: {folder}/.\n'
            'Perform only the selected phase; do not automatically proceed to another phase. '
            'Read AGENTS.md and existing specs/memory/constitution.md. For planning, tasks and implementation, '
            'read graphify-out/GRAPH_REPORT.md first if present. Existing documents are authoritative inputs; '
            'preserve them and make requested revisions in place, never discard or silently replace them. '
            'Stop for unresolved requirements or conflicting instructions. Follow the session delegation limits.\n'
            + steps[phase] + '\nReport changed document paths and decisions needed, then stop.\n\n')
    for name, path in zip(('constitution.md', 'spec.md', 'plan.md', 'tasks.md', 'progress.md'),
                          document_paths(root, settings)):
        text = text.replace(name, path.rsplit('/', 1)[1])
    if phase != 'review':
        text += 'Register feature documents in specs/MANIFEST.md. '
    return text + 'Follow existing naming hooks; do not disable them.\n\n'
