#!/usr/bin/env python3
"""Maintainer-only pre-session excerpt experiment; never installed into projects.

Document eligibility stays lexical. Optional quotes are prepared before session
identity, validated through the existing packer, and fall back as one unit.
"""
from __future__ import annotations

import argparse
from collections import Counter
import importlib.util
import json
import math
from pathlib import Path
import signal
import sys
import time
from typing import Any, Optional, Sequence

from memory_onnx import Scorer


class ExcerptPilot:
    """Legacy late-render callable, retained as a lexical-only compatibility shim.

    A late quote cannot update the already-recorded session identity. New
    callers use StatefulExcerptPilot through main; this interface returns the
    original exact lexical object and never invokes a scorer.
    """
    def __init__(self, context: Any, runtime: Any, scorer: Any) -> None:
        self.context, self.runtime, self.scorer = context, runtime, scorer
        self.original = context.capsule_excerpts
        self.cache: dict[tuple[str, str], float] = {}
        self.stats: Counter[str] = Counter()

    def __call__(self, connection: Any, capsule: dict[str, Any]) -> dict[str, str]:
        self.stats["fallback:legacy-interface"] += 1
        return self.original(connection, capsule)


def main(argv: Optional[Sequence[str]] = None) -> int:
    started = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--context", type=Path, required=True)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument("--worker-timeout", type=float, default=5.0)
    parser.add_argument("--invocation-timeout", type=float, default=30.0)
    parser.add_argument("--invocation-deadline", type=float)
    parser.add_argument("--stats", type=Path, required=True)
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    if not math.isfinite(args.invocation_timeout) or args.invocation_timeout <= 0:
        parser.error("--invocation-timeout must be finite and positive")
    if args.invocation_deadline is not None and not math.isfinite(args.invocation_deadline):
        parser.error("--invocation-deadline must be finite")
    sys.path.insert(0, str(args.context.parent))
    spec = importlib.util.spec_from_file_location("memory_pilot_context", args.context)
    if spec is None or spec.loader is None:
        return 2
    context = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = context
    spec.loader.exec_module(context)
    import context_retrieval as runtime
    from stateful_pilot import StatefulExcerptPilot
    deadline = started + args.invocation_timeout
    if args.invocation_deadline is not None:
        deadline = min(deadline, args.invocation_deadline)
    pilot = StatefulExcerptPilot(context, runtime, Scorer(args.assets, args.python,
                                                        timeout_seconds=args.worker_timeout),
                                 deadline=deadline)
    pilot.install()
    arguments = args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments
    sys.argv = [str(args.context), *arguments]
    previous_term = signal.getsignal(signal.SIGTERM)
    def cancelled(_signal: int, _frame: Any) -> None:
        raise SystemExit(143)
    signal.signal(signal.SIGTERM, cancelled)
    try:
        return int(context.main())
    finally:
        pilot.close()
        signal.signal(signal.SIGTERM, previous_term)
        write_stats(args.stats, pilot.stats)


def write_stats(path: Path, stats: Counter[str]) -> bool:
    try:
        path.write_text(json.dumps(dict(stats), sort_keys=True) + "\n", encoding="utf-8")
        return True
    except OSError:
        return False


if __name__ == "__main__":
    raise SystemExit(main())
