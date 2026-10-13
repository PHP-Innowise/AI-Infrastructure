"""Contract tests for the optional, isolated memory ONNX scorer."""

from __future__ import annotations

import contextlib
import copy
import hashlib
import io
from types import SimpleNamespace

import importlib.util
import json
import os
import signal
import subprocess
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "memory_onnx.py"
SPEC = importlib.util.spec_from_file_location("memory_onnx_test", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
memory_onnx = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = memory_onnx
SPEC.loader.exec_module(memory_onnx)


FAKE_WORKER = '''\
import json
from pathlib import Path
import sys
import time

mode = Path(sys.argv[-1]).name
if mode == "noread":
    time.sleep(2)
    sys.exit(0)
request = json.loads(sys.stdin.read())
if mode == "sleep":
    time.sleep(2)
elif mode == "huge":
    sys.stdout.write("x" * 20000)
elif mode == "nan":
    print('{"scores":[NaN]}')
elif mode == "nested":
    print('{"scores":[[1.0]]}')
elif mode == "short":
    print('{"scores":[]}')
elif mode == "missing":
    print('{"error":"dependency-unavailable"}')
elif mode == "error-list":
    print('{"error":[]}')
elif mode == "error-dict":
    print('{"error":{}}')
elif mode == "huge-number":
    print('{"scores":[' + '1' * 1000 + ']}')
else:
    print(json.dumps({"scores": [float(len(value)) for value in request["passages"]]}))
'''


class MemoryOnnxTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="memory-onnx-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.worker = self.root / "fake_worker.py"
        self.worker.write_text(FAKE_WORKER, encoding="utf-8")

    def scorer(self, mode: str = "ok", timeout: float = 1.0):
        assets = self.root / mode
        assets.mkdir(exist_ok=True)
        return memory_onnx.Scorer(assets, sys.executable, timeout_seconds=timeout, worker_script=self.worker)

    def test_scores_preserve_passage_order(self) -> None:
        result = self.scorer().score("rank lexical candidates", ["first", "second passage"])
        self.assertEqual([5.0, 14.0], result.scores)
        self.assertIsNone(result.reason)

    def test_invalid_request_never_starts_worker(self) -> None:
        invalid = [
            ("", ["passage"]),
            ("x" * 1025, ["passage"]),
            ("query", []),
            ("query", ["x" * 1201]),
            ("query", "not a list"),
            ("query", ["p"] * 33),
            ("query", ["\ud800"]),
        ]
        for query, passages in invalid:
            with self.subTest(query_length=len(query), passages=str(passages)[:20]):
                result = self.scorer().score(query, passages)
                self.assertEqual((None, "invalid-request"), (result.scores, result.reason))

    def test_worker_failures_are_baseline_fallbacks(self) -> None:
        for mode, reason in (
            ("missing", "dependency-unavailable"),
            ("nan", "invalid-output"),
            ("nested", "invalid-output"),
            ("short", "invalid-output"),
            ("huge", "output-too-large"),
            ("error-list", "invalid-output"),
            ("error-dict", "invalid-output"),
            ("huge-number", "invalid-output"),
        ):
            with self.subTest(mode=mode):
                result = self.scorer(mode).score("query", ["passage"])
                self.assertEqual((None, reason), (result.scores, result.reason))

    def test_timeout_reaps_the_worker(self) -> None:
        started = time.monotonic()
        result = self.scorer("sleep", timeout=0.05).score("query", ["passage"])
        self.assertEqual((None, "timeout"), (result.scores, result.reason))
        self.assertLess(time.monotonic() - started, 2)

    def test_timeout_covers_a_worker_that_never_reads_a_pipe_filling_request(self) -> None:
        passages = ["x" * 1200] * 32
        started = time.monotonic()
        result = self.scorer("noread", timeout=0.05).score("query", passages)
        self.assertEqual((None, "timeout"), (result.scores, result.reason))
        self.assertLess(time.monotonic() - started, 2)

    def test_busy_call_does_not_wait_or_change_order(self) -> None:
        first_result = []

        def first() -> None:
            first_result.append(self.scorer("sleep", timeout=1).score("query", ["passage"]))

        thread = threading.Thread(target=first)
        thread.start()
        time.sleep(0.1)
        second = self.scorer().score("query", ["first", "second"])
        thread.join(timeout=3)
        self.assertFalse(thread.is_alive())
        self.assertEqual((None, "busy"), (second.scores, second.reason))
        self.assertEqual((None, "timeout"), (first_result[0].scores, first_result[0].reason))

    def test_hash_mismatch_is_checked_before_optional_imports(self) -> None:
        assets = self.root / "mismatch"
        assets.mkdir()
        files = {}
        for name in memory_onnx.REQUIRED_FILES:
            (assets / name).write_bytes(b"x")
            files[name] = {"sha256": "0" * 64, "size": 1}
        (assets / "manifest.json").write_text(json.dumps({"files": files}), encoding="utf-8")
        result = memory_onnx.Scorer(assets, sys.executable, timeout_seconds=2).score("query", ["passage"])
        self.assertEqual((None, "hash-mismatch"), (result.scores, result.reason))

    def test_missing_executable_and_invalid_timeout_are_safe(self) -> None:
        missing = memory_onnx.Scorer(self.root, self.root / "no-python").score("query", ["passage"])
        self.assertEqual((None, "worker-unavailable"), (missing.scores, missing.reason))
        for value in (0, float("inf"), 10**400):
            invalid = self.scorer(timeout=value).score("query", ["passage"])
            self.assertEqual((None, "invalid-timeout"), (invalid.scores, invalid.reason))



WORKER = '''import json, os, subprocess, sys, time
from pathlib import Path
folder=Path(sys.argv[-1])
request=json.loads(sys.stdin.read())
if folder.name.startswith("child"):
    sink=subprocess.DEVNULL if folder.name == "child-quiet" else sys.stdout
    child=subprocess.Popen([sys.executable,"-c","import time; time.sleep(30)"], stdin=subprocess.DEVNULL,stdout=sink,stderr=subprocess.DEVNULL)
    (folder/"child.pid").write_text(str(child.pid))
if folder.name == "sleep": time.sleep(30)
if folder.name == "duplicate": print('{"scores":[0],"scores":[1]}')
else: print(json.dumps({"scores":[1.0] * len(request["passages"])}))
'''


def running(pid):
    try:
        text = Path("/proc") / str(pid) / "stat"
        return text.read_text().rsplit(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux process inspection")
class WorkerLifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="onnx-owned-lifecycle-")
        self.addCleanup(self.temporary.cleanup)
        self.folder = Path(self.temporary.name)
        self.script = self.folder / "worker.py"
        self.script.write_text(WORKER)

    def scorer(self, mode, timeout=.3):
        assets = self.folder / mode
        assets.mkdir()
        return memory_onnx.Scorer(assets, sys.executable, worker_script=self.script, timeout_seconds=timeout)

    def stopped(self, pid):
        deadline = time.monotonic() + .5
        while running(pid) and time.monotonic() < deadline:
            time.sleep(.005)
        self.assertFalse(running(pid))

    def test_success_reaps_exited_leaders_quiet_descendant(self):
        scorer = self.scorer("child-quiet")
        result = scorer.score("query", ["passage"])
        self.assertEqual([1.0], result.scores)
        self.stopped(int((scorer.assets / "child.pid").read_text()))

    def test_dead_leader_with_inherited_pipe_obeys_single_deadline(self):
        scorer = self.scorer("child-pipe")
        started = time.monotonic()
        result = scorer.score("query", ["passage"])
        self.assertEqual("timeout", result.reason)
        self.assertLess(time.monotonic() - started, .4)
        self.stopped(int((scorer.assets / "child.pid").read_text()))

    def test_shared_deadline_is_tighter_than_worker_configuration(self):
        scorer = self.scorer("sleep", timeout=3)
        started = time.monotonic()
        result = scorer.score("query", ["passage"], deadline=started + .07)
        self.assertEqual("timeout", result.reason)
        self.assertLess(time.monotonic() - started, .15)

    def test_cancellation_reaps_process_and_releases_lock(self):
        scorer = self.scorer("sleep")
        processes = []
        original = subprocess.Popen
        def launch(*args, **kwargs):
            process = original(*args, **kwargs)
            processes.append(process)
            return process
        with mock.patch.object(memory_onnx.subprocess, "Popen", launch), mock.patch.object(memory_onnx, "_bounded_worker_output", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                scorer.score("query", ["passage"])
        self.assertIsNotNone(processes[0].returncode)
        self.assertFalse(memory_onnx._WORKER_LOCK.locked())

    def test_duplicate_score_keys_fail_closed(self):
        result = self.scorer("duplicate").score("query", ["passage"])
        self.assertEqual((None, "invalid-output"), (result.scores, result.reason))

    def test_huge_external_deadline_is_invalid_data(self):
        scorer = self.scorer("sleep")
        result = scorer.score("query", ["passage"], deadline=10**400)
        self.assertEqual((None, "invalid-timeout"), (result.scores, result.reason))

    def test_duplicate_worker_request_keys_fail_closed(self):
        import io
        from types import SimpleNamespace
        payload = b'{"query":"first","query":"second","passages":["p"]}'
        with mock.patch.object(memory_onnx.sys, "stdin", SimpleNamespace(buffer=io.BytesIO(payload))):
            request, reason = memory_onnx._read_request()
        self.assertEqual((None, "invalid-request"), (request, reason))

    def test_parent_death_guard_kills_guarded_worker_after_hard_parent_kill(self):
        guard = self.folder / "guarded_worker.py"
        guard.write_text("import sys,time,os\nfrom pathlib import Path\nsys.path.insert(0," + repr(str(Path(memory_onnx.__file__).parent)) + ")\nfrom memory_onnx import _owner_guard\nassert _owner_guard()\nPath(sys.argv[-1]).joinpath('worker.pid').write_text(str(os.getpid()))\ntime.sleep(30)\n")
        assets = self.folder / "guard-assets"
        assets.mkdir()
        code = "import sys\nsys.path.insert(0," + repr(str(Path(memory_onnx.__file__).parent)) + ")\nfrom memory_onnx import Scorer\nScorer(" + repr(str(assets)) + ",sys.executable,worker_script=" + repr(str(guard)) + ",timeout_seconds=30).score('query',['passage'])\n"
        parent = subprocess.Popen([sys.executable, "-B", "-c", code], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 2
            pid_file = assets / "worker.pid"
            while not pid_file.exists() and time.monotonic() < deadline:
                time.sleep(.005)
            self.assertTrue(pid_file.exists())
            pid = int(pid_file.read_text())
            parent.kill()
            parent.wait(timeout=1)
            self.stopped(pid)
        finally:
            if parent.poll() is None:
                parent.kill()
                parent.wait(timeout=1)


class JinaAssetTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="jina-profile-contract-")
        self.addCleanup(temporary.cleanup)
        self.folder = Path(temporary.name)
        payloads = {
            "model.onnx": b"inert graph fixture",
            "tokenizer.json": b"inert tokenizer fixture",
            "config.json": json.dumps({"pad_token_id": 0}).encode(),
            "tokenizer_config.json": json.dumps({"tokenizer_class": "RobertaTokenizer", "pad_token": "<pad>"}).encode(),
            "special_tokens_map.json": json.dumps({"pad_token": {"content": "<pad>"}}).encode(),
        }
        self.pins = {}
        for name, payload in payloads.items():
            (self.folder / name).write_bytes(payload)
            self.pins[name] = hashlib.sha256(payload).hexdigest(), len(payload)
        self.manifest = {
            "schema_version": 1, "profile": "jina-tiny-fp32",
            "model_id": "jinaai/jina-reranker-v1-tiny-en",
            "revision": "aca45de6945b5dc6399abcd2a9c55ded5dc9111f",
            "variant": "onnx/model.onnx", "pad_token": "<pad>", "pad_id": 1,
            "files": {name: {"sha256": digest, "size": size} for name, (digest, size) in self.pins.items()},
        }
        patcher = mock.patch.dict(memory_onnx.JINA_FILES, self.pins, clear=True)
        patcher.start()
        self.addCleanup(patcher.stop)

    def assets(self, manifest=None):
        (self.folder / "manifest.json").write_text(json.dumps(self.manifest if manifest is None else manifest))
        return memory_onnx._assets(self.folder)

    def test_explicit_profile_accepts_config_pad_zero_and_inert_provenance(self):
        self.manifest["metadata_url"] = "https://invalid.example/no-fetch"
        self.manifest["remote_python_executed"] = False
        files, reason = self.assets()
        self.assertEqual(set(self.pins), set(files))
        self.assertIsNone(reason)
        self.assertEqual(0, json.loads(files["config.json"].read_text())["pad_token_id"])

    def test_partial_unknown_or_inexact_discriminators_fail_closed(self):
        variations = [
            ("schema_version", None), ("schema_version", True), ("schema_version", 1.0),
            ("schema_version", "1"), ("schema_version", 2),
            ("profile", None), ("profile", "jina-other"), ("profile", ["jina-tiny-fp32"]),
            ("model_id", "other"), ("revision", "main"), ("variant", "model.onnx"),
            ("pad_id", True), ("pad_id", 0), ("pad_token", "[PAD]"),
        ]
        for key, value in variations:
            with self.subTest(key=key, value=value):
                manifest = copy.deepcopy(self.manifest)
                if value is None:
                    manifest.pop(key)
                else:
                    manifest[key] = value
                self.assertEqual((None, "invalid-assets"), self.assets(manifest))

    def test_invalid_profile_returns_before_optional_imports(self):
        self.manifest["profile"] = "unknown"
        self.assets()
        output = io.StringIO()
        request = {"query": "synthetic", "passages": ["a passage"]}
        original = __import__
        def guarded(name, *args, **kwargs):
            if name in ("numpy", "onnxruntime", "tokenizers"):
                raise AssertionError("optional import before asset rejection")
            return original(name, *args, **kwargs)
        with mock.patch.object(memory_onnx, "_read_request", return_value=(request, None)), \
             mock.patch("builtins.__import__", guarded), contextlib.redirect_stdout(output):
            self.assertEqual(0, memory_onnx._worker(self.folder))
        self.assertEqual({"error": "invalid-assets"}, json.loads(output.getvalue()))

    def test_self_consistent_replacements_cannot_change_any_pinned_file(self):
        for name in self.pins:
            with self.subTest(name=name):
                ref = self.folder / name
                before = ref.read_bytes()
                replacement = before + b"changed"
                ref.write_bytes(replacement)
                manifest = copy.deepcopy(self.manifest)
                manifest["files"][name] = {"sha256": hashlib.sha256(replacement).hexdigest(), "size": len(replacement)}
                self.assertEqual((None, "invalid-assets"), self.assets(manifest))
                ref.write_bytes(before)

    def test_actual_bytes_must_match_pins_and_outside_root_is_rejected(self):
        graph = self.folder / "model.onnx"
        before = graph.read_bytes()
        graph.write_bytes(b"x" * len(before))
        self.assertEqual((None, "hash-mismatch"), self.assets())
        graph.write_bytes(before)
        outside = self.folder.parent / (self.folder.name + "-outside")
        outside.write_bytes(before)
        self.addCleanup(lambda: outside.unlink(missing_ok=True))
        graph.unlink()
        try:
            graph.symlink_to(outside)
        except (OSError, NotImplementedError):
            self.skipTest("symlinks unavailable")
        self.assertEqual((None, "asset-missing"), self.assets())

    def test_legacy_schema_less_manifest_and_metadata_remain_accepted(self):
        manifest = {"model_id": "local legacy", "license": "retained", "files": {}}
        for name in memory_onnx.REQUIRED_FILES:
            payload = b"legacy fixture"
            (self.folder / name).write_bytes(payload)
            manifest["files"][name] = {"sha256": hashlib.sha256(payload).hexdigest(), "size": len(payload)}
        files, reason = self.assets(manifest)
        self.assertEqual(memory_onnx.REQUIRED_FILES, set(files))
        self.assertIsNone(reason)
        manifest["profile"] = "unsupported"
        self.assertEqual((None, "invalid-assets"), self.assets(manifest))

    def test_metadata_discrepancies_are_rejected_without_normalizing_config(self):
        for config in ({"pad_token_id": True}, {"pad_token_id": 1}, []):
            with self.subTest(config=config):
                (self.folder / "config.json").write_text(json.dumps(config))
                files = {name: self.folder / name for name in self.pins}
                self.assertFalse(memory_onnx._jina_metadata(files))


class ProfileArray:
    def __init__(self, values, dtype):
        self.values, self.dtype = values, dtype
        self.ndim = 2
        self.shape = (len(values), len(values[0]))

    def __getitem__(self, key):
        if key == (slice(None), 0):
            return [row[0] for row in self.values]
        raise AssertionError(key)


class JinaRuntimeTests(unittest.TestCase):
    def setUp(self):
        worker_version = mock.patch.object(memory_onnx.sys, "version_info", (3, 14))
        worker_version.start()
        self.addCleanup(worker_version.stop)
        self.inputs = [SimpleNamespace(name=name, type="tensor(int64)", shape=["batch", "sequence"])
                       for name in ("input_ids", "attention_mask")]
        self.outputs = [SimpleNamespace(name="logits", type="tensor(float)", shape=["batch_size", 1])]
        self.providers = ["CPUExecutionProvider"]
        self.runs, self.token_settings, self.constructed = [], {}, []
        self.pad_id = 1
        self.encoded = [SimpleNamespace(ids=[0, 5, 2], attention_mask=[1, 1, 1], type_ids=[0, 0, 0]) for _ in range(3)]
        self.logits = ProfileArray([[-2.5], [-2.5], [1.75]], "float32")
        self.finite = True
        session = SimpleNamespace(get_inputs=lambda: self.inputs, get_outputs=lambda: self.outputs,
                                  get_providers=lambda: self.providers,
                                  run=lambda names, feed: self.runs.append((names, feed)) or [self.logits])
        def construct(*args, **kwargs):
            self.constructed.append((args, kwargs))
            return session
        tokenizer = SimpleNamespace(
            token_to_id=lambda token: self.pad_id,
            enable_truncation=lambda **kw: self.token_settings.update(truncation=kw),
            enable_padding=lambda **kw: self.token_settings.update(padding=kw),
            encode_batch=lambda pairs: self.encoded,
        )
        self.modules = {
            "numpy": SimpleNamespace(__version__="2.5.3", int64="int64", float32="float32", number="number",
                                     asarray=lambda values, dtype=None: values if isinstance(values, ProfileArray) else ProfileArray(values, dtype),
                                     isfinite=lambda values: SimpleNamespace(all=lambda: self.finite),
                                     issubdtype=lambda dtype, kind: dtype in ("float32", "int64")),
            "onnxruntime": SimpleNamespace(__version__="1.31.0", SessionOptions=SimpleNamespace,
                                            ExecutionMode=SimpleNamespace(ORT_SEQUENTIAL="sequential"), InferenceSession=construct),
            "tokenizers": SimpleNamespace(__version__="0.23.3", Tokenizer=SimpleNamespace(from_file=lambda name: tokenizer)),
        }

    def worker(self, *, legacy=False):
        names = memory_onnx.REQUIRED_FILES if legacy else set(memory_onnx.JINA_FILES)
        files = {name: Path(name) for name in names}
        request = {"query": "one synthetic query", "passages": ["first", "first", "last"]}
        output = io.StringIO()
        with mock.patch.object(memory_onnx, "_read_request", return_value=(request, None)), \
             mock.patch.object(memory_onnx, "_assets", return_value=(files, None)), \
             mock.patch.dict(sys.modules, self.modules), contextlib.redirect_stdout(output):
            code = memory_onnx._worker(Path("inert fixture"))
        self.assertEqual(0, code)
        return json.loads(output.getvalue())

    def test_jina_uses_two_inputs_pad_one_and_raw_ordered_logits(self):
        self.assertEqual({"scores": [-2.5, -2.5, 1.75]}, self.worker())
        names, feed = self.runs[0]
        self.assertEqual(["logits"], names)
        self.assertEqual({"input_ids", "attention_mask"}, set(feed))
        self.assertTrue(all(value.dtype == "int64" for value in feed.values()))
        self.assertEqual({"pad_id": 1, "pad_token": "<pad>", "pad_type_id": 0, "direction": "right"}, self.token_settings["padding"])
        self.assertEqual({"max_length": 384, "strategy": "longest_first", "direction": "right", "stride": 0}, self.token_settings["truncation"])
        options = self.constructed[0][1]["sess_options"]
        self.assertEqual((1, 1, "sequential"), (options.intra_op_num_threads, options.inter_op_num_threads, options.execution_mode))

    def test_legacy_keeps_three_inputs_pad_zero_and_original_call(self):
        self.inputs.append(SimpleNamespace(name="token_type_ids"))
        self.modules["numpy"].__version__ = "legacy-compatible"
        self.assertEqual({"scores": [-2.5, -2.5, 1.75]}, self.worker(legacy=True))
        self.assertEqual({"input_ids", "attention_mask", "token_type_ids"}, set(self.runs[0][1]))
        self.assertIsNone(self.runs[0][0])
        self.assertEqual({"pad_id": 0, "pad_token": "[PAD]"}, self.token_settings["padding"])
        self.assertEqual({"max_length": 384}, self.token_settings["truncation"])

    def test_graph_contract_is_checked_before_inference(self):
        changes = [
            lambda: self.inputs.append(copy.copy(self.inputs[0])),
            lambda: setattr(self.inputs[0], "type", "tensor(int32)"),
            lambda: setattr(self.inputs[0], "shape", [1, "sequence"]),
            lambda: setattr(self.inputs[0], "shape", [True, "sequence"]),
            lambda: setattr(self.inputs[0], "name", "attention_mask"),
            lambda: setattr(self.outputs[0], "type", "tensor(double)"),
            lambda: setattr(self.outputs[0], "shape", ["batch", True]),
            lambda: setattr(self.outputs[0], "shape", [1, 1]),
            lambda: self.outputs.append(copy.copy(self.outputs[0])),
            lambda: self.providers.append("CUDAExecutionProvider"),
        ]
        for index, change in enumerate(changes):
            with self.subTest(index=index):
                self.setUp()
                change()
                self.assertEqual({"error": "invalid-shape"}, self.worker())
                self.assertEqual([], self.runs)

    def test_tokenizer_pad_mismatch_is_rejected_before_inference(self):
        for value in (0, True, None):
            with self.subTest(value=value):
                self.pad_id = value
                self.assertEqual({"error": "invalid-assets"}, self.worker())
                self.assertEqual([], self.runs)

    def test_unvalidated_runtime_versions_do_not_construct_native_session(self):
        for package in self.modules:
            with self.subTest(package=package):
                before = self.modules[package].__version__
                self.modules[package].__version__ = "unsupported"
                self.assertEqual({"error": "dependency-unavailable"}, self.worker())
                self.assertEqual([], self.constructed)
                self.modules[package].__version__ = before
        with mock.patch.object(memory_onnx.sys, "version_info", (3, 13)):
            self.assertEqual({"error": "dependency-unavailable"}, self.worker())
        self.assertEqual([], self.constructed)

    def test_encoded_row_or_shape_mismatch_is_rejected_before_inference(self):
        self.encoded.pop()
        self.assertEqual({"error": "invalid-shape"}, self.worker())
        self.assertEqual([], self.runs)
        self.setUp()
        for encoding in self.encoded:
            encoding.ids = [0] * 385
            encoding.attention_mask = [1] * 385
        self.assertEqual({"error": "invalid-shape"}, self.worker())
        self.assertEqual([], self.runs)
        self.setUp()
        self.encoded[0].attention_mask = [1, 1]
        self.assertEqual({"error": "invalid-shape"}, self.worker())
        self.assertEqual([], self.runs)

    def test_non_float32_wrong_cardinality_and_nonfinite_logits_fail_whole_result(self):
        for logits, finite in [(ProfileArray([[1], [2]], "float32"), True),
                               (ProfileArray([[1], [2], [3]], "int64"), True),
                               (ProfileArray([[1], [2], [3]], "float32"), False)]:
            with self.subTest(dtype=logits.dtype, shape=logits.shape, finite=finite):
                self.logits, self.finite = logits, finite
                self.assertEqual({"error": "invalid-shape"}, self.worker())


class ScorerDeadlineCeilingTests(unittest.TestCase):
    def test_large_override_is_capped_and_smaller_deadlines_keep_cleanup_reserve(self):
        cases = [(30, None, 5.0), (30, 103.0, 3.0), (0.1, None, 0.1)]
        for configured, external, effective in cases:
            with self.subTest(configured=configured, external=external):
                process = SimpleNamespace(stdin=io.BytesIO(), stdout=io.BytesIO(), wait=mock.Mock())
                scorer = memory_onnx.Scorer(Path("inert assets"), sys.executable,
                                             timeout_seconds=configured, worker_script=memory_onnx.__file__)
                with mock.patch.object(memory_onnx.time, "monotonic", return_value=100.0), \
                     mock.patch.object(memory_onnx.subprocess, "Popen", return_value=process), \
                     mock.patch.object(memory_onnx, "_terminate") as terminate, \
                     mock.patch.object(memory_onnx, "_bounded_worker_output", return_value=(b'{"scores":[1.0]}', None)) as bounded:
                    result = scorer.score("query", ["passage"], deadline=external)
                self.assertEqual([1.0], result.scores)
                reserve = min(0.15, effective * 0.2)
                self.assertAlmostEqual(100.0 + effective - reserve, bounded.call_args.args[2])
                self.assertAlmostEqual(effective, process.wait.call_args.kwargs["timeout"])
                terminate.assert_called_once_with(process)
                self.assertTrue(process.stdin.closed)
                self.assertTrue(process.stdout.closed)
                self.assertFalse(memory_onnx._WORKER_LOCK.locked())



if __name__ == "__main__":
    unittest.main()
