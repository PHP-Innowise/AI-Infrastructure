"""Pure maintainer opt-in response constraints and unsigned review bindings (Python 3.9+)."""
import copy
import hashlib
import json


MARKER = 'MEMORY_TASK_ANSWER'
LIMITS = {
    'max_messages': (1, 64),
    'max_message_bytes': (1, 262144),
    'max_total_bytes': (1, 1048576),
    'max_answer_string_chars': (0, 65536),
    'integer_abs_max': (1, 9007199254740991),
    'min_citations': (0, 16),
    'max_citations': (0, 16),
    'max_citation_span_lines': (1, 10000),
    'max_citation_quote_chars': (1, 65536),
    'max_citation_path_chars': (1, 1024),
    'min_explanation_chars': (0, 65536),
    'max_explanation_chars': (0, 65536),
}
CONTEXT_KEYS = {'protocol_sha256', 'slot_id', 'verifier_sha256', 'source_image_sha256'}
BINDING_KEYS = CONTEXT_KEYS | {
    'case_id', 'public_contract_sha256', 'messages_sha256',
    'selected_message_index', 'selected_message_sha256',
}
RECEIPT_MAX_BYTES = 16384
CORPUS_MAX_FILES = 256
CORPUS_MAX_FILE_BYTES = 1048576
CORPUS_MAX_TOTAL_BYTES = 16777216
CORPUS_MAX_DESCRIPTOR_BYTES = 65536
V2_MAX_RENDERED_BYTES = 131072


def _canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True,
                      separators=(',', ':'), allow_nan=False).encode('utf-8')


def _digest(value):
    return hashlib.sha256(_canonical(value)).hexdigest()


def _keys(value, expected):
    return type(value) is dict and set(value) == set(expected)


def _identity(value, maximum=128):
    if type(value) is not str or not value.strip() or len(value) > maximum or '\0' in value:
        return False
    try:
        value.encode('utf-8')
    except UnicodeError:
        return False
    return True


def _hash(value):
    return type(value) is str and len(value) == 64 and all(c in '0123456789abcdef' for c in value)


def _strict_json(text):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError('duplicate JSON key')
            result[key] = value
        return result

    def nonfinite(_):
        raise ValueError('nonfinite JSON value')

    try:
        value = json.loads(text, object_pairs_hook=pairs, parse_constant=nonfinite)
        _canonical(value)  # Also rejects overflow such as JSON 1e999 and invalid UTF-8 strings.
        return value
    except (ValueError, TypeError, UnicodeError, RecursionError) as error:
        raise ValueError('invalid strict JSON') from error


def _citation_path(value, maximum):
    if (type(value) is not str or not 1 <= len(value) <= maximum
            or '\\' in value or ':' in value
            or any(ord(char) < 32 or ord(char) == 127 for char in value)
            or any(part in ('', '.', '..') for part in value.split('/'))):
        return False
    try:
        value.encode('utf-8')
    except UnicodeError:
        return False
    return True


def _validate_corpus(public):
    corpus = public['citation_corpus']
    if (not _keys(corpus, {'schema', 'files'})
            or type(corpus['schema']) is not str or corpus['schema'] != 'memory-citation-corpus-v1'
            or type(corpus['files']) is not list or len(corpus['files']) > CORPUS_MAX_FILES):
        raise ValueError('invalid citation corpus')
    previous, total = None, 0
    for entry in corpus['files']:
        if (not _keys(entry, {'path', 'sha256', 'bytes'})
                or not _citation_path(entry['path'], public['limits']['max_citation_path_chars'])
                or not _hash(entry['sha256'])
                or type(entry['bytes']) is not int or not 0 <= entry['bytes'] <= CORPUS_MAX_FILE_BYTES):
            raise ValueError('invalid citation corpus entry')
        if previous is not None and entry['path'] <= previous:
            raise ValueError('citation corpus paths must be unique and sorted')
        previous = entry['path']
        total += entry['bytes']
    if total > CORPUS_MAX_TOTAL_BYTES or len(_canonical(corpus)) > CORPUS_MAX_DESCRIPTOR_BYTES:
        raise ValueError('citation corpus exceeds its byte bounds')
    if not corpus['files'] and (public['limits']['min_citations'] != 0 or public['limits']['max_citations'] != 0):
        raise ValueError('empty citation corpus requires zero citation limits')


def validate_contract(value):
    """Validate/copy public shape data; no expected factual values are accepted."""
    version2 = (type(value) is dict and type(value.get('schema')) is str
                and value['schema'] == 'memory-response-contract-v2')
    keys = {'schema', 'case_id', 'marker', 'answer_fields', 'limits'}
    if version2:
        keys.add('citation_corpus')
    if not _keys(value, keys):
        raise ValueError('unexpected public contract keys')
    if (type(value['schema']) is not str or type(value['marker']) is not str
            or value['schema'] not in ('memory-response-contract-v1', 'memory-response-contract-v2')
            or value['marker'] != MARKER):
        raise ValueError('unsupported public contract or marker')
    if not _identity(value['case_id']):
        raise ValueError('invalid public case_id')
    fields = value['answer_fields']
    if type(fields) is not dict or not 1 <= len(fields) <= 32:
        raise ValueError('one to 32 flat answer fields required')
    for name, spec in fields.items():
        if not _identity(name, 64) or not _keys(spec, {'type', 'nullable'}):
            raise ValueError('invalid public answer field')
        if (type(spec['type']) is not str or spec['type'] not in ('boolean', 'integer', 'string')
                or type(spec['nullable']) is not bool):
            raise ValueError('invalid public answer field type/nullability')
    limits = value['limits']
    if not _keys(limits, LIMITS):
        raise ValueError('all public limits must be declared')
    for name, (low, high) in LIMITS.items():
        if type(limits[name]) is not int or not low <= limits[name] <= high:
            raise ValueError('invalid public limit: ' + name)
    if (limits['min_citations'] > limits['max_citations']
            or limits['min_explanation_chars'] > limits['max_explanation_chars']
            or limits['max_total_bytes'] < limits['max_message_bytes']):
        raise ValueError('incoherent public limits')
    if version2:
        _validate_corpus(value)
        if len(_render(value).encode('utf-8')) > V2_MAX_RENDERED_BYTES:
            raise ValueError('rendered v2 contract exceeds its byte bound')
    return copy.deepcopy(value)


def contract_digest(contract):
    return _digest(validate_contract(contract))


def render_contract(contract):
    """Render the same public object consumed by the shape checker, without gold."""
    public = validate_contract(contract)
    return _render(public)


def _render(public):
    rules = (
        'Every answer attempt must use the exact ASCII marker on its first line: ' + public['marker'] + '\n'
        'LF and CRLF marker line endings are accepted. Follow it with one plain JSON object; '
        'no fences or surrounding prose. The exact keys are case_id, answer, citations, explanation.\n'
        'case_id must equal the public case_id. answer has exactly the declared flat fields. '
        'Boolean is not integer; integers (including citation lines) are bounded by integer_abs_max. '
        'Only fields declared nullable accept null. String lengths count Unicode code points.\n'
        'Each citation has exactly path, start_line, end_line, quote. Path and quote are nonempty '
        'strings within their declared character limits; line numbers are positive integers, '
        'end_line >= start_line, and inclusive span is bounded. This checks shape, not source truth.\n'
        'Explanation length is bounded as declared; length does not prove substance. '
        'Message limits count exact UTF-8 bytes; transcript limits cover all supplied assistant messages. '
        'Duplicate JSON keys and nonfinite values are refused.\n'
        'The last first-line-marked attempt is selected, even when its JSON or case_id is invalid. '
        'There is no fallback. Later unmarked acknowledgments are ignored; unmarked intent is not inferred.\n'
    )
    if public['schema'] == 'memory-response-contract-v2':
        rules += (
            'Only exact paths in citation_corpus.files are eligible citation sources, at their declared '
            'raw-byte SHA-256 and byte sizes. Readable, installed, indexed, generated or retrieved files '
            'absent from that complete list are not eligible. Paths are literal relative POSIX names: '
            'no empty, dot or dot-dot components, backslashes, colons, C0 controls or DEL. No percent '
            'decoding, case folding or Unicode normalization is performed.\n'
            'Source bytes decode as strict UTF-8 without BOM removal or newline conversion. Lines are '
            'LF-delimited, one-based and inclusive; a final LF does not create a phantom extra line, '
            'and an empty file has zero lines. CR, including CR in CRLF, is preserved; lone CR and '
            'Unicode line separators do not start new lines. The quote must be an exact contiguous '
            'substring within the requested original line window (excluding only its final LF). '
            'No trimming, whitespace folding, ellipsis expansion or newline normalization is applied.\n'
            'An empty corpus permits only an empty citation list. Source eligibility establishes '
            'literal presence, not factual truth, current authority, an adopted decision or approval. '
            'Source bytes and complete prompt transport must be frozen and verified by the caller; '
            'shape checking alone does not verify source contents.\n'
        )
    return rules + '\nPUBLIC_CONTRACT\n' + json.dumps(public, ensure_ascii=False, sort_keys=True, indent=2)


def _body(text, marker):
    first, separator, body = text.partition('\n')
    if first.endswith('\r'):
        first = first[:-1]
    return body if first == marker and separator else None


def _marked(text, marker):
    first = text.partition('\n')[0]
    if first.endswith('\r'):
        first = first[:-1]
    return first == marker


def _encoded(text, maximum):
    if type(text) is not str or len(text) > maximum:
        raise ValueError('message exceeds declared UTF-8 byte limit or has wrong type')
    try:
        raw = text.encode('utf-8')
    except UnicodeError as error:
        raise ValueError('message is not valid UTF-8') from error
    if len(raw) > maximum:
        raise ValueError('message exceeds declared UTF-8 byte limit')
    return raw


def check_response(contract, text):
    """Check disclosed shape and v2 membership, never factual correctness."""
    public = validate_contract(contract)
    limits = public['limits']
    errors = []
    try:
        _encoded(text, limits['max_message_bytes'])
        body = _body(text, public['marker'])
        if body is None:
            raise ValueError('missing first-line marker and JSON body')
        value = _strict_json(body)
    except ValueError as error:
        return {'valid': False, 'errors': [str(error)]}
    if not _keys(value, {'case_id', 'answer', 'citations', 'explanation'}):
        return {'valid': False, 'errors': ['response must have exactly the disclosed envelope keys']}
    if type(value['case_id']) is not str or value['case_id'] != public['case_id']:
        errors.append('case_id differs from the public case_id')
    answers = value['answer']
    if not _keys(answers, public['answer_fields']):
        errors.append('answer must have exactly the declared flat fields')
    else:
        for name, spec in public['answer_fields'].items():
            item = answers[name]
            if item is None and spec['nullable']:
                continue
            valid = ((spec['type'] == 'boolean' and type(item) is bool)
                     or (spec['type'] == 'integer' and type(item) is int
                         and abs(item) <= limits['integer_abs_max'])
                     or (spec['type'] == 'string' and type(item) is str
                         and len(item) <= limits['max_answer_string_chars']))
            if not valid:
                errors.append('invalid answer field: ' + name)
    citations = value['citations']
    if (type(citations) is not list
            or not limits['min_citations'] <= len(citations) <= limits['max_citations']):
        errors.append('citation count is outside the declared bounds')
    else:
        for citation in citations:
            if not _keys(citation, {'path', 'start_line', 'end_line', 'quote'}):
                errors.append('citation must have exactly the disclosed keys')
                continue
            for field, limit in (('path', 'max_citation_path_chars'), ('quote', 'max_citation_quote_chars')):
                if type(citation[field]) is not str or not 1 <= len(citation[field]) <= limits[limit]:
                    errors.append('citation ' + field + ' is outside the declared character bounds')
            if public['schema'] == 'memory-response-contract-v2':
                if not _citation_path(citation['path'], limits['max_citation_path_chars']):
                    errors.append('citation path is not a canonical relative POSIX path')
                elif citation['path'] not in {entry['path'] for entry in public['citation_corpus']['files']}:
                    errors.append('citation path is not in the public citation corpus')
            start, end = citation['start_line'], citation['end_line']
            if (type(start) is not int or type(end) is not int
                    or not 1 <= start <= end <= limits['integer_abs_max']
                    or end - start + 1 > limits['max_citation_span_lines']):
                errors.append('citation line range is outside the declared bounds')
    explanation = value['explanation']
    if (type(explanation) is not str
            or not limits['min_explanation_chars'] <= len(explanation) <= limits['max_explanation_chars']):
        errors.append('explanation is outside the declared character bounds')
    return {'valid': not errors, 'errors': errors}


def check_citations(contract, text, sources):
    """Check v2 literal evidence over supplied bytes; invalid context raises ValueError."""
    if type(sources) is not dict or len(sources) > CORPUS_MAX_FILES:
        raise ValueError('citation sources must be a bounded plain dict')
    # Snapshot the exact dict before any validation. Values must be immutable
    # bytes, so later caller mutations cannot change this check's source image.
    sources = sources.copy()
    public = validate_contract(contract)
    if public['schema'] != 'memory-response-contract-v2':
        raise ValueError('citation source checking requires a v2 contract')
    entries = public['citation_corpus']['files']
    if (any(type(path) is not str for path in sources)
            or set(sources) != {entry['path'] for entry in entries}):
        raise ValueError('citation source context paths differ from the public corpus')
    decoded = {}
    for entry in entries:
        raw = sources[entry['path']]
        if (type(raw) is not bytes or len(raw) != entry['bytes']
                or hashlib.sha256(raw).hexdigest() != entry['sha256']):
            raise ValueError('citation source context bytes differ from the public corpus')
        try:
            decoded[entry['path']] = raw.decode('utf-8')
        except UnicodeError as error:
            raise ValueError('citation source context is not strict UTF-8') from error
    checked = check_response(public, text)
    if not checked['valid']:
        return checked
    citations = _strict_json(_body(text, public['marker']))['citations']
    errors = []
    for path in sorted({citation['path'] for citation in citations}):
        requested = [citation for citation in citations if citation['path'] == path]
        starts, ends = {}, {}
        wanted_starts = {citation['start_line'] for citation in requested}
        wanted_ends = {citation['end_line'] for citation in requested}
        last_line = max(wanted_ends)
        source, position, line = decoded[path], 0, 1
        # Keep only requested boundary offsets, not a list of every source line.
        while position < len(source) and line <= last_line:
            newline = source.find('\n', position)
            finish = len(source) if newline == -1 else newline
            if line in wanted_starts:
                starts[line] = position
            if line in wanted_ends:
                ends[line] = finish
            position, line = finish + 1, line + 1
        for citation in requested:
            start, end = citation['start_line'], citation['end_line']
            if start not in starts or end not in ends:
                errors.append('citation line range does not exist in the declared source')
            elif source.find(citation['quote'], starts[start], ends[end]) == -1:
                errors.append('citation quote is not literal within its declared line range')
    return {'valid': not errors, 'errors': errors}


def preflight_reference(contract, reference_text):
    """Refuse a reference that violates public shape; this is not a truth check."""
    checked = check_response(contract, reference_text)
    if not checked['valid']:
        raise ValueError('reference violates public response contract: ' + '; '.join(checked['errors']))


def select_answer(contract, messages):
    """Select the last explicit marker, preserving exact text/hash/index."""
    public = validate_contract(contract)
    empty = {'status': 'unavailable', 'index': None, 'text': None,
             'message_sha256': None, 'messages_sha256': None,
             'check': {'valid': None, 'errors': []}}
    try:
        if type(messages) is not list or len(messages) > public['limits']['max_messages']:
            raise ValueError('assistant message count exceeds the declared bound or has wrong type')
        total = 0
        for text in messages:
            total += len(_encoded(text, public['limits']['max_message_bytes']))
            if total > public['limits']['max_total_bytes']:
                raise ValueError('assistant transcript exceeds the declared byte bound')
        stream_digest = _digest(messages)
    except ValueError as error:
        empty['check']['errors'] = [str(error)]
        return empty
    indices = [index for index, text in enumerate(messages) if _marked(text, public['marker'])]
    if not indices:
        empty.update(status='unmarked', messages_sha256=stream_digest,
                     check={'valid': False, 'errors': ['no first-line-marked attempt']})
        return empty
    index = indices[-1]
    text = messages[index]
    return {'status': 'selected', 'index': index, 'text': text,
            'message_sha256': hashlib.sha256(text.encode('utf-8')).hexdigest(),
            'messages_sha256': stream_digest, 'check': check_response(public, text)}


def make_artifact(contract, messages, context):
    """Create unsigned in-memory bindings; caller owns capture custody/persistence."""
    public = validate_contract(contract)
    if not _keys(context, CONTEXT_KEYS) or not _identity(context['slot_id']):
        raise ValueError('invalid review context')
    if not all(_hash(context[key]) for key in CONTEXT_KEYS - {'slot_id'}):
        raise ValueError('invalid context digest')
    selection = select_answer(public, messages)
    binding = dict(context, case_id=public['case_id'], public_contract_sha256=_digest(public),
                   messages_sha256=selection['messages_sha256'],
                   selected_message_index=selection['index'],
                   selected_message_sha256=selection['message_sha256'])
    return {'schema': 'memory-answer-artifact-v1', 'binding': binding, 'selection': selection}


def _artifact_valid(public, artifact):
    if not _keys(artifact, {'schema', 'binding', 'selection'}) or artifact['schema'] != 'memory-answer-artifact-v1':
        return False
    bound, selection = artifact['binding'], artifact['selection']
    if not _keys(bound, BINDING_KEYS) or not _keys(selection, {
            'status', 'index', 'text', 'message_sha256', 'messages_sha256', 'check'}):
        return False
    if (bound['case_id'] != public['case_id'] or bound['public_contract_sha256'] != _digest(public)
            or not _identity(bound['slot_id'])
            or not all(_hash(bound[key]) for key in BINDING_KEYS - {'slot_id', 'case_id', 'selected_message_index'})):
        return False
    index = selection['index']
    if (type(index) is not int or type(bound['selected_message_index']) is not int
            or not 0 <= index < public['limits']['max_messages']):
        return False
    if (selection['status'] != 'selected' or index != bound['selected_message_index']
            or selection['messages_sha256'] != bound['messages_sha256']):
        return False
    try:
        raw = _encoded(selection['text'], public['limits']['max_message_bytes'])
    except ValueError:
        return False
    if (not _keys(selection['check'], {'valid', 'errors'})
            or type(selection['check']['valid']) is not bool):
        return False
    selected_hash = hashlib.sha256(raw).hexdigest()
    return (selected_hash == bound['selected_message_sha256'] == selection['message_sha256']
            and _marked(selection['text'], public['marker'])
            and selection['check'] == check_response(public, selection['text']))


def review_decision(contract, artifact, machine_correct, receipt_json=None):
    """Gate supplied machine/reviewer assertions; never authenticate or prove truth."""
    public = validate_contract(contract)
    if machine_correct is not None and type(machine_correct) is not bool:
        raise ValueError('machine_correct must be true, false or unknown')
    status, verdict, reviewer = 'missing', None, None
    artifact_valid = _artifact_valid(public, artifact)
    if not artifact_valid:
        status = 'artifact-invalid'
    elif receipt_json is not None:
        try:
            _encoded(receipt_json, RECEIPT_MAX_BYTES)
            receipt_value = _strict_json(receipt_json)
            if (not _keys(receipt_value, {'schema', 'binding', 'reviewer_id', 'reviewer_kind', 'verdict'})
                    or receipt_value['schema'] != 'independent-answer-review-v1'
                    or not _identity(receipt_value['reviewer_id'])
                    or receipt_value['reviewer_kind'] not in ('agent', 'human')
                    or receipt_value['verdict'] not in ('consistent', 'contradiction', 'uncertain')):
                raise ValueError('invalid review receipt')
            if _canonical(receipt_value['binding']) != _canonical(artifact['binding']):
                status = 'mismatch'
            else:
                verdict = receipt_value['verdict']
                status = verdict
                reviewer = {'id': receipt_value['reviewer_id'], 'claimed_kind': receipt_value['reviewer_kind']}
        except ValueError:
            status = 'invalid'
    # Preserve known machine failures even when a review is absent or invalid.
    task_correct = False if machine_correct is False else None
    if machine_correct is True and artifact_valid:
        if not artifact['selection']['check']['valid'] or verdict == 'contradiction':
            task_correct = False
        elif verdict == 'consistent':
            task_correct = True
    return {'machine_correct': machine_correct, 'task_correct': task_correct,
            'review_status': status, 'reviewer': reviewer, 'authenticated': False,
            'human_confirmation': False, 'semantic_truth_proven': False}
