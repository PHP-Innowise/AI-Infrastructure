#!/usr/bin/env python3
"""Temporary: loop the refresh that once failed on windows-latest with
"Git source context unavailable", with every Git process timed.

Mode `fresh` repeats tests/test_cursor_continuation.py's
test_other_host_and_unsanitized_profiles_keep_prompt_query step by step:
a new temporary project, `git init`, the two sources, `start`, then
`refresh` for codex, claude and unsanitized cursor. Mode `unittest` runs that
test module itself, again and again. Each context.py process loads
sitecustomize.py from this directory, which records its Git processes, the
frames the runtime asked for and the line any SourceGitError was raised at.

While it loops, a heartbeat thread sleeps 5 ms at a time and records every
longer wake-up gap (the whole machine stalled, not one Git process), and the
system CPU load is sampled four times a second. Removed once the cause is
known; not a test.
"""
from __future__ import annotations

import argparse
import datetime
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EDITION = ROOT / "PHP Core"
FIXTURES = ROOT / "tests/fixtures"
HERE = Path(__file__).resolve().parent
LOCK = threading.Lock()


class Heartbeat(threading.Thread):
    def __init__(self, threshold=0.03):
        super().__init__(daemon=True)
        self.threshold = threshold
        self.gaps = []
        self.ticks = 0
        self.running = True

    def run(self):
        last = time.perf_counter()
        while self.running:
            time.sleep(0.005)
            now = time.perf_counter()
            self.ticks += 1
            if now - last > self.threshold:
                self.gaps.append((round(time.time() - (now - last), 4), round(now - last, 4)))
            last = now


class CpuSampler(threading.Thread):
    """System-wide busy fraction (and steal on Linux), every 250 ms."""

    def __init__(self):
        super().__init__(daemon=True)
        self.samples = []
        self.running = True

    @staticmethod
    def read():
        if os.name == "nt":
            import ctypes
            from ctypes import wintypes

            class FILETIME(ctypes.Structure):
                _fields_ = [("low", wintypes.DWORD), ("high", wintypes.DWORD)]

            idle, kernel, user = FILETIME(), FILETIME(), FILETIME()
            ctypes.windll.kernel32.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kernel), ctypes.byref(user))
            value = lambda item: (item.high << 32) | item.low
            # Kernel time includes idle time.
            return {"idle": value(idle), "total": value(kernel) + value(user), "steal": 0}
        with open("/proc/stat", encoding="ascii") as handle:
            fields = [int(item) for item in handle.readline().split()[1:]]
        return {"idle": fields[3] + fields[4], "total": sum(fields[:8]), "steal": fields[7]}

    def run(self):
        previous = self.read()
        while self.running:
            time.sleep(0.25)
            current = self.read()
            total = current["total"] - previous["total"]
            if total > 0:
                self.samples.append((round(time.time(), 3),
                                     round(1 - (current["idle"] - previous["idle"]) / total, 3),
                                     round((current["steal"] - previous["steal"]) / total, 3)))
            previous = current


def subcommand(argv) -> str:
    return next((item for item in argv if not item.startswith("-")), "?")


def environment(trace_dir: Path, call: str) -> dict:
    # tests/test_cursor_continuation.py's environment, plus the probe's.
    result = {key: value for key, value in os.environ.items()
              if not key.startswith(("GIT_", "ACCELERATOR_", "CONTEXT_"))}
    result.update(GIT_OPTIONAL_LOCKS="0", PYTHONDONTWRITEBYTECODE="1")
    result["PYTHONPATH"] = str(HERE) + (os.pathsep + result["PYTHONPATH"] if result.get("PYTHONPATH") else "")
    result.update(PROBE_TRACE_DIR=str(trace_dir), PROBE_CALL_ID=call)
    return result


def collect(trace_dir: Path, call: str) -> list:
    found = []
    for path in sorted(trace_dir.glob(call + "-*.json")):
        try:
            found.append(json.loads(path.read_text(encoding="utf-8")))
        except (OSError, ValueError) as error:
            found.append({"unreadable": str(path), "error": repr(error)})
        try:
            path.unlink()
        except OSError:
            pass
    return found


def remove(path: Path):
    for attempt in range(20):
        try:
            shutil.rmtree(path)
            return
        except FileNotFoundError:
            return
        except OSError:
            time.sleep(0.1 * (attempt + 1))
    shutil.rmtree(path, ignore_errors=True)


class Trace2:
    """Git's own trace2 events, configured globally (the runtime strips GIT_*
    from its Git environment, so only configuration reaches its Git)."""

    def __init__(self, directory: Path):
        self.directory = directory
        directory.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "config", "--global", "trace2.eventTarget", str(directory)], check=True)
        subprocess.run(["git", "config", "--global", "trace2.eventNesting", "2"], check=True)

    def close(self):
        subprocess.run(["git", "config", "--global", "--unset", "trace2.eventTarget"], check=False)

    def harvest(self) -> list:
        processes = []
        for path in self.directory.iterdir():
            try:
                lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
                path.unlink()
            except OSError:
                continue
            record = {"regions": []}
            for line in lines:
                try:
                    event = json.loads(line)
                except ValueError:
                    continue
                kind = event.get("event")
                if kind == "version":
                    record["start"] = datetime.datetime.strptime(event["time"], "%Y-%m-%dT%H:%M:%S.%fZ").replace(
                        tzinfo=datetime.timezone.utc).timestamp()
                elif kind == "start":
                    argv = event.get("argv", [])
                    record["argv"] = argv[argv.index("-C") + 2:] if "-C" in argv else argv[1:]
                    record["t_start"] = event.get("t_abs")
                elif kind == "exit":
                    record["t_exit"] = event.get("t_abs")
                    record["code"] = event.get("code")
                elif kind == "region_leave" and event.get("t_rel", 0) > 0.05:
                    record["regions"].append((event.get("category"), event.get("label"), event.get("t_rel")))
            if "start" in record:
                processes.append(record)
        return processes


class Probe:
    def __init__(self, arguments):
        self.arguments = arguments
        self.out = Path(arguments.out).resolve()
        self.out.mkdir(parents=True, exist_ok=True)
        self.trace_dir = self.out / "child-traces"
        self.trace_dir.mkdir(exist_ok=True)
        self.calls = open(self.out / f"calls-{arguments.shard}.jsonl", "a", encoding="utf-8")
        self.failures = open(self.out / f"failures-{arguments.shard}.jsonl", "a", encoding="utf-8")
        self.public = json.loads((FIXTURES / "cursor_continuation_public.json").read_text(encoding="utf-8"))
        self.trace2 = Trace2(self.out / "trace2") if arguments.trace2 else None
        self.counter = 0
        self.failed = 0
        self.iterations = 0

    def call_id(self):
        with LOCK:
            self.counter += 1
            return f"s{self.arguments.shard}-{self.counter:07d}"

    def record(self, entry: dict, failed: bool):
        line = json.dumps(entry)
        with LOCK:
            self.calls.write(line + "\n")
            if failed:
                self.failed += 1
                self.failures.write(line + "\n")
                self.failures.flush()
                print("FAILURE " + line[:6000], flush=True)

    def cli(self, worker, iteration, project, label, arguments, trace2_batch):
        call = self.call_id()
        started_wall = time.time()
        started = time.perf_counter()
        try:
            result = subprocess.run([sys.executable, "-B", str(project / "memory-bank/scripts/context.py"),
                                     "--root", str(project), *arguments],
                                    env=environment(self.trace_dir, call), text=True, encoding="utf-8",
                                    capture_output=True, timeout=15)
            code, stderr = result.returncode, result.stderr
        except subprocess.TimeoutExpired as error:
            code, stderr = "timeout", str(error)
        duration = time.perf_counter() - started
        traces = collect(self.trace_dir, call)
        entry = {"call": call, "worker": worker, "iter": iteration, "cmd": label, "rc": code,
                 "wall": round(started_wall, 4), "dur": round(duration, 4), "stderr": (stderr or "")[-400:],
                 "traces": traces}
        if trace2_batch is not None:
            trace2_batch.append(entry)
        failed = code != 0 or any(event.get("k") == "raise" for trace in traces for event in trace.get("events", ()))
        self.record(entry, failed)
        return code

    def fresh(self, worker, iteration):
        project = Path(tempfile.mkdtemp(prefix="juniper-continuation-"))
        batch = [] if self.trace2 is not None else None
        try:
            scripts = project / "memory-bank/scripts"
            shutil.copytree(EDITION / "memory-bank/scripts", scripts,
                            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
            for name in ("config", "schemas"):
                shutil.copytree(EDITION / "project-brain" / name, project / "project-brain" / name)
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith(("GIT_", "ACCELERATOR_", "CONTEXT_"))}
            env.update(GIT_OPTIONAL_LOCKS="0", PYTHONDONTWRITEBYTECODE="1")
            subprocess.run(["git", "init", "-q", str(project)], env=env, check=True, capture_output=True, timeout=10)
            for path_key, text_key in (("source_path", "source_text"), ("other_source_path", "other_source_text")):
                path = project / self.public[path_key]
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(self.public[text_key], encoding="utf-8")
            task, goal = self.public["task"], self.public["goal"]
            self.cli(worker, iteration, project, "start", ["start", "--task-id", task, "--goal", goal, "--json"], batch)
            for host, sanitize in (("codex", True), ("claude", True), ("cursor", False)):
                arguments = ["refresh", "--query", "go on", "--task-id", task, "--host", host, "--gate", "shadow",
                             "--ephemeral", "--json"] + (["--sanitize"] if sanitize else [])
                self.cli(worker, iteration, project, "refresh:" + host, arguments, batch)
        finally:
            remove(project)
        if batch is not None:
            self.attach_trace2(batch)

    def attach_trace2(self, batch):
        # Match Git's own events to the parent's records: same subcommand,
        # Git's start inside the parent's window, earliest first.
        processes = sorted(self.trace2.harvest(), key=lambda process: process["start"])
        with (self.out / f"trace2-{self.arguments.shard}.jsonl").open("a", encoding="utf-8") as handle:
            for entry in batch:
                for trace in entry["traces"]:
                    for event in trace.get("events", ()):
                        if event.get("k") != "git" or "dur" not in event:
                            continue
                        window = (event["wall"] - 0.05, event["wall"] + event["dur"] + 0.05)
                        best = next((process for process in processes
                                     if not process.get("used") and window[0] <= process["start"] <= window[1]
                                     and subcommand(process.get("argv", [])) == subcommand(event["argv"])), None)
                        if best is None:
                            continue
                        best["used"] = True
                        pre = best["start"] - event["wall"]
                        inside = best.get("t_exit") or 0.0
                        handle.write(json.dumps({"call": entry["call"], "cmd": entry["cmd"],
                                                 "git_cmd": subcommand(event["argv"]), "dur": event["dur"],
                                                 "spawn": event.get("spawn"), "pre": round(pre, 4), "git": inside,
                                                 "post": round(event["dur"] - pre - inside, 4),
                                                 "regions": best["regions"]}) + "\n")

    def unittest(self, worker, iteration):
        call = self.call_id()
        started_wall = time.time()
        started = time.perf_counter()
        result = subprocess.run([sys.executable, "-m", "unittest", "tests.test_cursor_continuation"], cwd=ROOT,
                                env=environment(self.trace_dir, call), text=True, encoding="utf-8",
                                capture_output=True, timeout=600)
        traces = collect(self.trace_dir, call)
        entry = {"call": call, "worker": worker, "iter": iteration, "cmd": "unittest", "rc": result.returncode,
                 "wall": round(started_wall, 4), "dur": round(time.perf_counter() - started, 4),
                 "stderr": result.stderr[-3000:] if result.returncode else "", "traces": traces}
        failed = result.returncode != 0 or any(event.get("k") == "raise" for trace in traces
                                               for event in trace.get("events", ()))
        self.record(entry, failed)

    def worker(self, worker, deadline):
        iteration = 0
        step = self.fresh if self.arguments.mode == "fresh" else self.unittest
        while time.monotonic() < deadline:
            iteration += 1
            try:
                step(worker, iteration)
            except Exception as error:  # the probe keeps going; the error is a datum
                print(f"worker {worker} iteration {iteration}: {error!r}", flush=True)
            with LOCK:
                self.iterations += 1
                if self.iterations % 50 == 0:
                    print(f"{time.strftime('%H:%M:%S')} iterations={self.iterations} failures={self.failed}",
                          flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--minutes", type=float, default=2)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--mode", choices=("fresh", "unittest"), default="fresh")
    parser.add_argument("--trace2", action="store_true")
    parser.add_argument("--out", default="probe-out")
    parser.add_argument("--shard", default="1")
    arguments = parser.parse_args()
    probe = Probe(arguments)
    git = shutil.which("git")
    print("git:", git, subprocess.run(["git", "version"], capture_output=True, text=True).stdout.strip(), flush=True)
    print("python:", sys.version, "monotonic:", time.get_clock_info("monotonic"), flush=True)
    print("temp:", tempfile.gettempdir(), "cpus:", os.cpu_count(), flush=True)
    heartbeat, cpu = Heartbeat(), CpuSampler()
    heartbeat.start()
    cpu.start()
    started = time.time()
    deadline = time.monotonic() + arguments.minutes * 60
    threads = [threading.Thread(target=probe.worker, args=(index, deadline)) for index in range(arguments.workers)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    heartbeat.running = cpu.running = False
    if probe.trace2 is not None:
        probe.trace2.close()
    probe.calls.close()
    probe.failures.close()
    (probe.out / f"machine-{arguments.shard}.json").write_text(json.dumps({
        "started": started, "ended": time.time(), "heartbeat_ticks": heartbeat.ticks,
        "heartbeat_gaps": heartbeat.gaps, "cpu": cpu.samples, "git": git, "python": sys.version,
        "workers": arguments.workers, "mode": arguments.mode, "trace2": arguments.trace2}), encoding="utf-8")
    print(f"done: iterations={probe.iterations} calls={probe.counter} failures={probe.failed}", flush=True)
    subprocess.run([sys.executable, str(HERE / "summarize.py"), str(probe.out)], check=False)
    return 0


if __name__ == "__main__":
    sys.exit(main())
