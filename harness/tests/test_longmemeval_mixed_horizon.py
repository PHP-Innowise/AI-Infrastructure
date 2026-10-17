"""Fresh two-slot supplied accounting; not native/provider/quality evidence."""
import copy
import hashlib
import json
from pathlib import Path
import unittest

from harness.longmemeval_lifecycle import Controller, ControllerError


CONTROLS = Path(__file__).parent / 'fixtures/longmemeval_mixed_horizon/controls.json'


class SuppliedMemory:
    """Own only the new supplied local images; never initialize a native bank."""
    def __init__(self, case):
        self.view = copy.deepcopy(case['supplied_memory_initial'])
        self.questions = copy.deepcopy(case['questions'])
        self.contexts = copy.deepcopy(case['contexts'])
        self.receipts = copy.deepcopy(case['supplied_receipts'])
        self.actions = dict.fromkeys(('snapshot', 'query', 'set_context',
                                     'clear_context', 'post_hook', 'close'), 0)

    def snapshot(self):
        self.actions['snapshot'] += 1
        return copy.deepcopy(self.view)

    def set_query_context(self, *, query_invocation_id):
        # Do not retain/export the random controller identity.
        assert type(query_invocation_id) is str and query_invocation_id.startswith('controller-')
        self.actions['set_context'] += 1

    def clear_query_context(self):
        self.actions['clear_context'] += 1

    def query(self, question, query_image):
        self.actions['query'] += 1
        observed = self.view['local_observation']['observed']
        ordinal = observed['query_calls']
        assert question == self.questions[ordinal]['question'] and query_image is None
        observed['query_receipts'].append(copy.deepcopy(self.receipts[ordinal]))
        observed['query_calls'] += 1
        return copy.deepcopy(self.contexts[ordinal])

    def post_query_hook(self, *, query, query_image, memory_context):
        self.actions['post_hook'] += 1
        ordinal = self.view['local_observation']['observed']['query_calls'] - 1
        assert query == self.questions[ordinal]['question'] and query_image is None
        assert memory_context == self.contexts[ordinal]
        return {'query_receipt': copy.deepcopy(self.receipts[ordinal])}

    def close(self):
        self.actions['close'] += 1
        self.view['closed'] = True
        self.view['local_observation']['ledger']['frozen'] = True
        self.view['local_observation']['observed']['cleanup_outcome'] = 'succeeded'
        return self.snapshot()


class FixedClock:
    def __init__(self, values):
        self.values = tuple(values)
        self.calls = 0

    def __call__(self):
        index = self.calls
        self.calls += 1
        assert index < len(self.values)
        return self.values[index]


class MixedHorizonTests(unittest.TestCase):
    def _case(self, name):
        case = next(row for row in json.loads(CONTROLS.read_text())['cases']
                    if row['name'] == name)
        expected = case['expected']
        self.observation = {key: None for key in expected}
        seen = self.observation
        seen.update(reader_request_utf8=[], judge_request_utf8=[], judge_factories=0)
        memory = SuppliedMemory(case)
        clock = FixedClock(case['clock_values'])
        controller = Controller(memory, copy.deepcopy(case['questions']),
                                reader_budget=4096, clock=clock)
        for index, item in enumerate(case['reader_reports']):
            ordinal = item['ordinal']
            self.assertEqual(controller.query(ordinal), case['contexts'][ordinal])
            ticket = controller.prepare_reader(ordinal)

            def reader(body, index=index, item=item):
                self.assertIs(type(body), bytes)
                seen['reader_request_utf8'].append(body.decode('utf-8'))
                self.assertEqual(seen['reader_request_utf8'][-1],
                                 expected['reader_request_utf8'][index])
                return copy.deepcopy(item['report'])

            self.assertEqual(controller.dispatch(ticket, reader), item['report']['value'])
        construction = controller.seal_construction()
        seen['construction_utf8'] = construction.body.decode('utf-8')
        seen['construction_sha256'] = hashlib.sha256(construction.body).hexdigest()
        self.assertEqual(seen['construction_utf8'], expected['construction_utf8'])
        self.assertEqual(seen['construction_sha256'], expected['construction_sha256'])
        for index, item in enumerate(case['judge_reports']):
            ticket = controller.prepare_judge(item['ordinal'], construction)

            def factory(index=index, item=item):
                # This callback is reached only after the owned construction seal.
                seen['judge_factories'] += 1
                self.assertIs(controller._seal, construction)
                self.assertEqual(construction.body.decode('utf-8'), expected['construction_utf8'])
                self.assertEqual(controller.snapshot()['construction_sha256'],
                                 expected['construction_sha256'])

                def judge(body):
                    self.assertIs(type(body), bytes)
                    seen['judge_request_utf8'].append(body.decode('utf-8'))
                    self.assertEqual(seen['judge_request_utf8'][-1],
                                     expected['judge_request_utf8'][index])
                    return copy.deepcopy(item['report'])

                return judge

            self.assertEqual(controller.dispatch(ticket, factory), item['report']['value'])
        if len(case['judge_reports']) == 1:
            with self.assertRaises(ControllerError) as caught:
                controller.prepare_judge(1, construction)
            self.assertEqual(caught.exception.args, ('controller_judge_state',))
            seen['second_judge_guard'] = 'controller_judge_state'
        seen['final_snapshot'] = controller.finalize()
        self.assertEqual(controller.snapshot(), seen['final_snapshot'])
        before = (copy.deepcopy(memory.actions), clock.calls,
                  len(seen['reader_request_utf8']) + len(seen['judge_request_utf8'])
                  + seen['judge_factories'])
        seen['repeat_final_snapshot'] = controller.finalize()
        self.assertEqual(controller.snapshot(), seen['repeat_final_snapshot'])
        seen['repeat_new_work'] = {
            'memory_actions': sum(memory.actions[key] - value for key, value in before[0].items()),
            'clock': clock.calls - before[1],
            'callbacks': len(seen['reader_request_utf8']) + len(seen['judge_request_utf8'])
                         + seen['judge_factories'] - before[2]}
        seen['supplied_memory_final'] = copy.deepcopy(memory.view)
        seen['memory_actions'] = copy.deepcopy(memory.actions)
        seen['clock_calls'] = clock.calls
        self.assertEqual(seen, expected)

    def test_graded_plus_unlaunched(self):
        self._case('graded_plus_unlaunched')

    def test_graded_plus_cancelled_paid_reader(self):
        self._case('graded_plus_cancelled_paid_reader')

    def test_mixed_correct_grading_incomplete_provider(self):
        self._case('mixed_correct_grading_incomplete_provider')


if __name__ == '__main__':
    unittest.main()
