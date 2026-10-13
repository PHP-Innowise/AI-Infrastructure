#!/usr/bin/env python3
"""Optional local ONNX scorer for the maintainer-only memory experiment.

The importing process remains standard-library-only.  It starts an explicitly
configured Python worker, which alone imports ONNX Runtime, NumPy and
``tokenizers``.  A failure is data, not a ranking decision: callers receive no
scores and retain their exact lexical order.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path, PurePath
import signal
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from typing import Any, Optional, Sequence


MAX_PASSAGES = 32
MAX_QUERY_CHARACTERS = 1024
MAX_PASSAGE_CHARACTERS = 1200
MAX_REQUEST_BYTES = 48 * 1024
MAX_RESPONSE_BYTES = 16 * 1024
DEFAULT_TIMEOUT_SECONDS = 5.0
MODEL_FILE = "model_O3.onnx"
REQUIRED_FILES = frozenset({
    MODEL_FILE, "tokenizer.json", "config.json", "tokenizer_config.json",
    "special_tokens_map.json",
})
JINA_MODEL_FILE = "model.onnx"
JINA_METADATA = {
    "model_id": "jinaai/jina-reranker-v1-tiny-en",
    "revision": "aca45de6945b5dc6399abcd2a9c55ded5dc9111f",
    "variant": "onnx/model.onnx",
    "pad_token": "<pad>",
    "pad_id": 1,
}
# Worker-owned pins: a self-consistent replacement manifest cannot change profiles.
JINA_FILES = {
    "tokenizer.json": ("0046da43cc8c424b317f56b092b0512aaaa65c4f925d2f16af9d9eeb4d0ef902", 2030772),
    "config.json": ("dc70646aa6c9e75e3c513cc9c037f35ad54308001c3961d45c0f69749bcfb022", 1206),
    "tokenizer_config.json": ("d291c6652d96d56ffdbcf1ea19d9bae5ed79003f7648c627e725a619227ce8fa", 1215),
    "special_tokens_map.json": ("06e405a36dfe4b9604f484f6a1e619af1a7f7d09e34a8555eb0b77b66318067f", 280),
    JINA_MODEL_FILE: ("e0e743251c7566e2b1e4f5ad091c681a700d7d7a3d85541ea56ca3acf43d1afa", 132350375),
}
WORKER_ERRORS = frozenset({
    "asset-missing", "dependency-unavailable", "hash-mismatch",
    "invalid-assets", "invalid-request", "invalid-shape", "inference-failed",
})
_WORKER_LOCK = threading.Lock()


@dataclass(frozen=True)
class ScoreResult:
    """Scores are present only when every local worker check succeeded."""

    scores: Optional[list[float]]
    reason: Optional[str] = None


def _valid_text(value: object, maximum: int, *, allow_empty: bool = False) -> bool:
    return isinstance(value, str) and (allow_empty or bool(value)) and len(value) <= maximum


def _valid_request(query: object, passages: object) -> bool:
    return (
        _valid_text(query, MAX_QUERY_CHARACTERS)
        and isinstance(passages, Sequence)
        and not isinstance(passages, (str, bytes))
        and 0 < len(passages) <= MAX_PASSAGES
        and all(_valid_text(passage, MAX_PASSAGE_CHARACTERS) for passage in passages)
    )


def _terminate(process: subprocess.Popen[bytes]) -> None:
    """An exited leader may still have owned descendants holding its pipes."""
    try:
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGKILL)
        elif process.poll() is None:
            process.kill()
    except (OSError, ProcessLookupError):
        pass


def _bounded_worker_output(
    process: subprocess.Popen[bytes], payload: bytes, deadline: float
) -> tuple[Optional[bytes], Optional[str]]:
    """Nonblocking bounded pipes share one absolute deadline; no helper threads."""
    assert process.stdin is not None and process.stdout is not None
    output = bytearray()
    sent = 0
    input_open, output_open = True, True
    try:
        os.set_blocking(process.stdin.fileno(), False)
        os.set_blocking(process.stdout.fileno(), False)
    except (OSError, AttributeError):
        return None, "worker-unavailable"
    while True:
        if time.monotonic() >= deadline:
            return None, "timeout"
        progressed = False
        if input_open:
            try:
                written = os.write(process.stdin.fileno(), payload[sent:sent + 4096])
                sent += written
                progressed = bool(written)
                if sent == len(payload):
                    process.stdin.close()
                    input_open = False
            except BlockingIOError:
                pass
            except (BrokenPipeError, OSError):
                return None, "worker-exit"
        if output_open:
            try:
                block = os.read(process.stdout.fileno(), 4096)
                if not block:
                    output_open = False
                else:
                    progressed = True
                    if len(output) + len(block) > MAX_RESPONSE_BYTES:
                        return None, "output-too-large"
                    output.extend(block)
            except BlockingIOError:
                pass
            except OSError:
                return None, "worker-exit"
        status = process.poll()
        if status is not None and not output_open:
            if status or input_open:
                return None, "worker-exit"
            return bytes(output), None
        if not progressed:
            time.sleep(min(0.005, max(0.0, deadline - time.monotonic())))


def _owner_guard() -> bool:
    """Linux real workers die if the enclosing optional pilot is killed."""
    expected = os.environ.get("MEMORY_ONNX_OWNER_PID")
    if expected is None or not sys.platform.startswith("linux"):
        return True
    try:
        owner = int(expected)
        import ctypes
        libc = ctypes.CDLL(None, use_errno=True)
        if libc.prctl(1, signal.SIGKILL, 0, 0, 0) != 0:
            return False
        return os.getppid() == owner
    except (ValueError, OSError, AttributeError):
        return False


class Scorer:
    """A one-shot, CPU-only worker client with no dependency or network side effects."""

    def __init__(
        self,
        assets: Path | str,
        python_executable: Path | str,
        *,
        timeout_seconds: float = DEFAULT_TIMEOUT_SECONDS,
        worker_script: Optional[Path | str] = None,
    ) -> None:
        self.assets = Path(assets)
        self.python_executable = Path(python_executable)
        self.timeout_seconds = timeout_seconds
        self.worker_script = Path(worker_script) if worker_script is not None else Path(__file__)

    def _worker_command(self) -> list[str]:
        return [
            str(self.python_executable), "-B", str(self.worker_script), "--worker",
            "--assets", str(self.assets),
        ]

    def score(self, query: str, passages: Sequence[str], *, deadline: Optional[float] = None) -> ScoreResult:
        """Return one finite score per passage, or ``None`` plus a stable reason."""
        started = time.monotonic()
        if not _valid_request(query, passages):
            return ScoreResult(None, "invalid-request")
        if not isinstance(self.timeout_seconds, (int, float)) or isinstance(self.timeout_seconds, bool):
            return ScoreResult(None, "invalid-timeout")
        try:
            timeout = float(self.timeout_seconds)
        except (OverflowError, ValueError):
            return ScoreResult(None, "invalid-timeout")
        if not math.isfinite(timeout) or timeout <= 0:
            return ScoreResult(None, "invalid-timeout")
        timeout = min(timeout, DEFAULT_TIMEOUT_SECONDS)
        if deadline is not None:
            try:
                valid_deadline = (not isinstance(deadline, bool) and isinstance(deadline, (int, float))
                                  and math.isfinite(deadline))
            except (OverflowError, TypeError, ValueError):
                valid_deadline = False
            if not valid_deadline:
                return ScoreResult(None, "invalid-timeout")
            timeout = min(timeout, deadline - started)
            if timeout <= 0:
                return ScoreResult(None, "timeout")
        deadline = started + timeout
        cleanup_reserve = min(0.15, timeout * 0.2)
        if not self.python_executable.is_file() or not self.worker_script.is_file():
            return ScoreResult(None, "worker-unavailable")
        try:
            request = json.dumps({"query": query, "passages": list(passages)}, ensure_ascii=False).encode("utf-8")
        except (UnicodeError, ValueError, TypeError):
            return ScoreResult(None, "invalid-request")
        if len(request) > MAX_REQUEST_BYTES:
            return ScoreResult(None, "request-too-large")
        if not _WORKER_LOCK.acquire(blocking=False):
            return ScoreResult(None, "busy")
        process = None
        try:
            options: dict[str, Any] = {
                "stdin": subprocess.PIPE,
                "stdout": subprocess.PIPE,
                "stderr": subprocess.DEVNULL,
                "start_new_session": os.name == "posix",
                "env": {
                    "PATH": os.environ.get("PATH", ""),
                    "PYTHONDONTWRITEBYTECODE": "1",
                    "PYTHONNOUSERSITE": "1",
                    "OMP_NUM_THREADS": "1",
                    "TOKENIZERS_PARALLELISM": "false",
                    "ORT_INTRA_OP_NUM_THREADS": "1",
                    "ORT_INTER_OP_NUM_THREADS": "1",
                    "MEMORY_ONNX_OWNER_PID": str(os.getpid()),
                },
            }
            if os.name == "nt":
                options["creationflags"] = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
            try:
                process = subprocess.Popen(self._worker_command(), **options)
            except OSError:
                return ScoreResult(None, "worker-unavailable")
            self._process = process
            raw, reason = _bounded_worker_output(process, request, deadline - cleanup_reserve)
        finally:
            if process is not None:
                # Even a successful leader can leave group members behind.
                _terminate(process)
                try:
                    process.wait(timeout=max(0.0, deadline - time.monotonic()))
                except subprocess.TimeoutExpired:
                    _terminate(process)
                    # An unreaped child is a failed optional invocation, never scores.
                    reason = "worker-unreaped"
                for stream in (process.stdin, process.stdout):
                    if stream is not None:
                        stream.close()
                self._process = None
            _WORKER_LOCK.release()
        if reason is not None:
            return ScoreResult(None, reason)
        try:
            response = json.loads(raw.decode("utf-8"), object_pairs_hook=_unique_object,
                                  parse_constant=lambda _value: (_ for _ in ()).throw(ValueError()))
        except (AttributeError, UnicodeDecodeError, ValueError):
            return ScoreResult(None, "invalid-output")
        if not isinstance(response, dict):
            return ScoreResult(None, "invalid-output")
        if (set(response) == {"error"} and isinstance(response["error"], str)
                and response["error"] in WORKER_ERRORS):
            return ScoreResult(None, str(response["error"]))
        values = response.get("scores")
        if set(response) != {"scores"} or not isinstance(values, list) or len(values) != len(passages):
            return ScoreResult(None, "invalid-output")
        try:
            if any(isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) for value in values):
                return ScoreResult(None, "invalid-output")
        except (OverflowError, TypeError):
            return ScoreResult(None, "invalid-output")
        return ScoreResult([float(value) for value in values])


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate-key")
        result[key] = value
    return result


def _read_request() -> tuple[Optional[dict[str, Any]], Optional[str]]:
    payload = sys.stdin.buffer.read(MAX_REQUEST_BYTES + 1)
    if len(payload) > MAX_REQUEST_BYTES:
        return None, "invalid-request"
    try:
        value = json.loads(payload.decode("utf-8"), object_pairs_hook=_unique_object,
                           parse_constant=lambda _value: (_ for _ in ()).throw(ValueError()))
    except (UnicodeDecodeError, ValueError, json.JSONDecodeError):
        return None, "invalid-request"
    if not isinstance(value, dict) or set(value) != {"query", "passages"}:
        return None, "invalid-request"
    if not _valid_request(value["query"], value["passages"]):
        return None, "invalid-request"
    return value, None


def _assets(assets: Path) -> tuple[Optional[dict[str, Path]], Optional[str]]:
    try:
        manifest = json.loads((assets / "manifest.json").read_text(encoding="utf-8"),
                              object_pairs_hook=_unique_object,
                              parse_constant=lambda _value: (_ for _ in ()).throw(ValueError()))
    except (OSError, UnicodeDecodeError, ValueError):
        return None, "asset-missing"
    files = manifest.get("files") if isinstance(manifest, dict) else None
    jina = isinstance(manifest, dict) and ("schema_version" in manifest or "profile" in manifest)
    if jina and (type(manifest.get("schema_version")) is not int
                 or manifest.get("schema_version") != 1
                 or manifest.get("profile") != "jina-tiny-fp32"
                 or type(manifest.get("pad_id")) is not int
                 or any(manifest.get(key) != value for key, value in JINA_METADATA.items())):
        return None, "invalid-assets"
    required = set(JINA_FILES) if jina else REQUIRED_FILES
    if not isinstance(files, dict) or set(files) != required:
        return None, "invalid-assets"
    resolved: dict[str, Path] = {}
    base = assets.resolve()
    for name, details in files.items():
        if not isinstance(name, str) or PurePath(name).name != name or not isinstance(details, dict):
            return None, "invalid-assets"
        digest, size = details.get("sha256"), details.get("size")
        if not isinstance(digest, str) or len(digest) != 64 or any(char not in "0123456789abcdef" for char in digest):
            return None, "invalid-assets"
        if not isinstance(size, int) or isinstance(size, bool) or size < 1:
            return None, "invalid-assets"
        if jina and (digest, size) != JINA_FILES[name]:
            return None, "invalid-assets"
        target = (assets / name).resolve()
        if not target.is_file() or not target.is_relative_to(base):
            return None, "asset-missing"
        try:
            if target.stat().st_size != size:
                return None, "hash-mismatch"
            accumulator = hashlib.sha256()
            with target.open("rb") as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    accumulator.update(block)
            actual = accumulator.hexdigest()
        except OSError:
            return None, "asset-missing"
        if actual != digest:
            return None, "hash-mismatch"
        resolved[name] = target
    if jina and not _jina_metadata(resolved):
        return None, "invalid-assets"
    return resolved, None


def _jina_metadata(files: dict[str, Path]) -> bool:
    """Publisher JSON is inert data; its config pad zero differs from tokenizer pad one."""
    try:
        values = [json.loads(files[name].read_text(encoding="utf-8"),
                             object_pairs_hook=_unique_object,
                             parse_constant=lambda _value: (_ for _ in ()).throw(ValueError()))
                  for name in ("config.json", "tokenizer_config.json", "special_tokens_map.json")]
        config, tokenizer_config, special = values
        if not all(isinstance(value, dict) for value in values):
            return False
        def token(value: object) -> object:
            return value.get("content") if isinstance(value, dict) else value
        return (type(config.get("pad_token_id")) is int and config["pad_token_id"] == 0
                and tokenizer_config.get("tokenizer_class") == "RobertaTokenizer"
                and token(tokenizer_config.get("pad_token")) == "<pad>"
                and token(special.get("pad_token")) == "<pad>")
    except (OSError, UnicodeError, ValueError, KeyError):
        return False


def _jina_signature(session: Any) -> bool:
    inputs, outputs = session.get_inputs(), session.get_outputs()
    def dynamic(value: object) -> bool:
        return value is None or isinstance(value, str) and bool(value)
    return (session.get_providers() == ["CPUExecutionProvider"]
            and len(inputs) == 2
            and {entry.name for entry in inputs} == {"input_ids", "attention_mask"}
            and all(entry.type == "tensor(int64)" and len(entry.shape) == 2
                    and all(dynamic(value) for value in entry.shape) for entry in inputs)
            and len(outputs) == 1 and outputs[0].name == "logits"
            and outputs[0].type == "tensor(float)" and len(outputs[0].shape) == 2
            and dynamic(outputs[0].shape[0])
            and type(outputs[0].shape[1]) is int and outputs[0].shape[1] == 1)


def _worker(assets: Path) -> int:
    request, error = _read_request()
    if error is None:
        files, error = _assets(assets)
    else:
        files = None
    if error is not None:
        print(json.dumps({"error": error}))
        return 0
    jina = JINA_MODEL_FILE in files
    try:
        import numpy as np
        import onnxruntime as ort
        import tokenizers
        from tokenizers import Tokenizer
    except ImportError:
        print(json.dumps({"error": "dependency-unavailable"}))
        return 0
    if jina and (sys.version_info[:2] != (3, 14)
                 or getattr(np, "__version__", None) != "2.5.3"
                 or getattr(ort, "__version__", None) != "1.31.0"
                 or getattr(tokenizers, "__version__", None) != "0.23.3"):
        print(json.dumps({"error": "dependency-unavailable"}))
        return 0
    try:
        options = ort.SessionOptions()
        options.intra_op_num_threads = 1
        options.inter_op_num_threads = 1
        options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
        model_file = JINA_MODEL_FILE if jina else MODEL_FILE
        session = ort.InferenceSession(str(files[model_file]), sess_options=options, providers=["CPUExecutionProvider"])
        inputs = {entry.name for entry in session.get_inputs()}
        expected = {"input_ids", "attention_mask"} if jina else {"input_ids", "attention_mask", "token_type_ids"}
        if inputs != expected or jina and not _jina_signature(session):
            raise ValueError("invalid-shape")
        tokenizer = Tokenizer.from_file(str(files["tokenizer.json"]))
        if jina:
            pad_id = tokenizer.token_to_id("<pad>")
            if type(pad_id) is not int or pad_id != 1:
                print(json.dumps({"error": "invalid-assets"}))
                return 0
            tokenizer.enable_truncation(max_length=384, strategy="longest_first", direction="right", stride=0)
            tokenizer.enable_padding(pad_id=1, pad_token="<pad>", pad_type_id=0, direction="right")
        else:
            tokenizer.enable_truncation(max_length=384)
            tokenizer.enable_padding(pad_id=0, pad_token="[PAD]")
        encodings = tokenizer.encode_batch([(request["query"], passage) for passage in request["passages"]])
        if jina and len(encodings) != len(request["passages"]):
            raise ValueError("invalid-shape")
        feed = {
            "input_ids": np.asarray([item.ids for item in encodings], dtype=np.int64),
            "attention_mask": np.asarray([item.attention_mask for item in encodings], dtype=np.int64),
        }
        if not jina:
            feed["token_type_ids"] = np.asarray([item.type_ids for item in encodings], dtype=np.int64)
        if jina and any(value.ndim != 2 or value.shape != feed["input_ids"].shape
                        or not 0 < value.shape[1] <= 384 for value in feed.values()):
            raise ValueError("invalid-shape")
        outputs = session.run(["logits"] if jina else None, feed)
        names = [entry.name for entry in session.get_outputs()]
        if names != ["logits"] or len(outputs) != 1:
            raise ValueError("invalid-shape")
        logits = np.asarray(outputs[0])
        if (logits.shape != (len(encodings), 1)
                or (logits.dtype != np.float32 if jina else not np.issubdtype(logits.dtype, np.number))
                or not np.isfinite(logits).all()):
            raise ValueError("invalid-shape")
        response = {"scores": [float(value) for value in logits[:, 0]]}
        print(json.dumps(response, allow_nan=False, separators=(",", ":")))
        return 0
    except ValueError as error:
        print(json.dumps({"error": "invalid-shape" if str(error) == "invalid-shape" else "inference-failed"}))
        return 0
    except Exception:
        print(json.dumps({"error": "inference-failed"}))
        return 0


def main(arguments: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--assets", type=Path)
    args = parser.parse_args(arguments)
    if not args.worker or args.assets is None:
        parser.error("this module is a library; the worker needs --worker --assets")
    if not _owner_guard():
        return 2
    return _worker(args.assets)


if __name__ == "__main__":
    raise SystemExit(main())
