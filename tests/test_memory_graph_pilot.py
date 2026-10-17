"""The pilot owns a new external fixture and compares the same inputs."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import memory_graph_pilot as pilot


class GraphPilotTest(unittest.TestCase):
    def test_refuses_an_existing_directory_without_touching_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "keep.txt"
            marker.write_text("keep")
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                pilot.main(["--out-dir", temporary])
            self.assertEqual("keep", marker.read_text())
            self.assertEqual([marker], list(Path(temporary).iterdir()))

    def test_refuses_the_repository_before_creating_a_fixture(self):
        with mock.patch.object(pilot, "fixture") as fixture:
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                pilot.main(["--out-dir", str(ROOT / "graph-pilot-forbidden")])
            fixture.assert_not_called()
            self.assertFalse((ROOT / "graph-pilot-forbidden").exists())

    def test_versions_share_inputs_without_runtime_modes_and_future_control_predates_chunk(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "pilot"
            def fake_evaluation(root, arguments):
                result = Path(arguments[arguments.index("--out") + 1])
                result.parent.mkdir(parents=True, exist_ok=True)
                result.write_text(json.dumps({"meta": {}}))
                return 0

            with mock.patch.object(pilot, "evaluate_runtime", side_effect=fake_evaluation) as evaluator, \
                    mock.patch.object(pilot.memory_eval, "main", return_value=0):
                self.assertEqual(0, pilot.main(["--out-dir", str(output)]))
            calls = evaluator.call_args_list
            runs = [call.args[1] for call in calls]
            self.assertEqual(2, len(runs))
            self.assertEqual(output / "baseline-runtime", calls[0].args[0])
            self.assertEqual(ROOT, calls[1].args[0])
            self.assertTrue(all("--graph-expansion" not in args for args in runs))
            for option in ("--set", "--judgments", "--passages", "--host", "--edition"):
                self.assertEqual(1, len({args[args.index(option) + 1] for args in runs}))
            baseline = output / "baseline-runtime/PHP Core/memory-bank/scripts/context.py"
            self.assertTrue(baseline.is_file())
            for edition in pilot.memory_eval.installer.EDITION_PATHS.values():
                self.assertTrue((output / "baseline-runtime" / edition / "memory-bank/scripts/context.py").is_file())
            self.assertEqual((ROOT / "scripts/memory_eval.py").read_bytes(),
                             (output / "baseline-runtime/scripts/memory_eval.py").read_bytes())
            for label in ("baseline", "automatic"):
                result = json.loads((output / "results" / (label + ".json")).read_text())
                provenance = result["meta"]["runtime_under_test"]
                self.assertEqual(label, provenance["label"])
                self.assertRegex(provenance["commit"], r"^[0-9a-f]{40}$")
                self.assertEqual("git-blobs" if label == "baseline" else "working-tree",
                                 provenance["materialization"])
                self.assertRegex(provenance["scorer_sha256"], r"^[0-9a-f]{64}$")
            prompts = json.loads((output / "inputs/set.json").read_text())
            self.assertTrue(all(item["ts"] < "2026-08-10" for item in prompts))
            future = output / "projects/billing" / pilot.FUTURE
            metadata = json.loads(future.read_text().split("---", 2)[1])
            self.assertEqual("2026-08-10", metadata["created"])
            first = output / "projects/billing" / pilot.FIRST
            first_metadata = json.loads(first.read_text().split("---", 2)[1])
            self.assertEqual("MEM-20260801-bbbbbbbb", first_metadata["id"])

    def test_real_report_accepts_shared_conditions_with_different_runtime_treatments(self):
        from tests.test_memory_eval import ComparisonCompatibilityTest
        with tempfile.TemporaryDirectory(prefix='graph-comparison-') as temporary:
            output=Path(temporary)/'pilot'
            def supplied_results(runtime, arguments):
                document=ComparisonCompatibilityTest.document('useful' if runtime==ROOT else 'silent')
                destination=Path(arguments[arguments.index('--out')+1])
                destination.parent.mkdir(parents=True,exist_ok=True)
                destination.write_text(json.dumps(document))
                return 0
            rendered=io.StringIO()
            with mock.patch.object(pilot,'evaluate_runtime',side_effect=supplied_results), \
                    contextlib.redirect_stdout(rendered):
                self.assertEqual(0,pilot.main(['--out-dir',str(output)]))
            report=rendered.getvalue()
            self.assertIn('comparison: compatible recorded metadata',report)
            self.assertIn('paired over 1 prompt(s)',report)
            useful=next(line for line in report.splitlines() if line.startswith('useful turns'))
            self.assertEqual('+1',useful.split()[-1])
            a=json.loads((output/'results/baseline.json').read_text())['meta']['runtime_under_test']
            b=json.loads((output/'results/automatic.json').read_text())['meta']['runtime_under_test']
            self.assertNotEqual(a['materialization'],b['materialization'])
            self.assertEqual(a['scorer_sha256'],b['scorer_sha256'])


if __name__ == "__main__":
    unittest.main()
