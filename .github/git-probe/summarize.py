#!/usr/bin/env python3
"""Summarize loop.py's records: Git process times, the runtime's Git checks
against their deadlines, whole-machine stalls, and every failure's raise site.

    python summarize.py probe-out [probe-out-2 ...]
"""
from __future__ import annotations

import bisect
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path


def quantiles(values):
    if not values:
        return "n=0"
    values = sorted(values)

    def at(fraction):
        return values[min(len(values) - 1, int(fraction * len(values)))]

    return (f"n={len(values)} p50={at(0.5) * 1000:.0f} p90={at(0.9) * 1000:.0f} p99={at(0.99) * 1000:.0f} "
            f"p99.9={at(0.999) * 1000:.0f} max={values[-1] * 1000:.0f} ms")


def subcommand(argv):
    return next((item for item in argv if not item.startswith("-")), "?")


def over(values, limits=(0.25, 0.5, 1.0, 2.0)):
    return " ".join(f">{limit}s:{sum(value > limit for value in values)}" for limit in limits)


def main(directories):
    calls, machines, trace2 = [], [], []
    for directory in map(Path, directories):
        for path in sorted(directory.rglob("calls-*.jsonl")):
            with path.open(encoding="utf-8") as handle:
                calls.extend(json.loads(line) for line in handle if line.strip())
        for path in sorted(directory.rglob("machine-*.json")):
            machines.append(json.loads(path.read_text(encoding="utf-8")))
        for path in sorted(directory.rglob("trace2-*.jsonl")):
            with path.open(encoding="utf-8") as handle:
                trace2.extend(json.loads(line) for line in handle if line.strip())
    print(f"== {len(calls)} CLI calls from {len(machines)} machine(s)")
    by_command = defaultdict(list)
    git_times = defaultdict(list)
    spawn_times = defaultdict(list)
    preflight, probes, probe_gits = [], [], []
    slow_git = []
    failures = []
    by_process = defaultdict(list)
    for call in calls:
        by_command[call["cmd"]].append(call["dur"])
        for trace in call.get("traces", ()):
            argv = trace.get("argv") or []
            command = argv[2] if argv[:1] == ["--root"] and len(argv) > 2 else (argv[0] if argv else "?")
            if "end" in trace:
                by_process[command].append(trace["end"])
            first_frame = True
            for event in trace.get("events", ()):
                kind = event.get("k")
                if kind == "git" and "dur" in event:
                    name = subcommand(event["argv"])
                    git_times[name].append(event["dur"])
                    if "spawn" in event:
                        spawn_times[name].append(event["spawn"])
                    if event["dur"] > 0.25:
                        slow_git.append((event["wall"], event["dur"], event.get("spawn"), name, command,
                                         call["call"], event.get("err")))
                elif kind == "_source_git_frame" and first_frame and command in (
                        "refresh", "context", "retrieve", "hook-context", "index"):
                    # These commands' preflight (context.py, deadline 1.0 s)
                    # is the first frame they ask for.
                    first_frame = False
                    preflight.append(event["dur"])
                elif kind == "_source_git_ignored":
                    probes.append(event["dur"])
        raised = [event for trace in call.get("traces", ()) for event in trace.get("events", ())
                  if event.get("k") == "raise"]
        if call["rc"] != 0 or raised:
            failures.append((call, raised))
    print("\n== CLI call duration by command (parent's view)")
    for name, values in sorted(by_command.items()):
        print(f"  {name:16s} {quantiles(values)}  {over(values, (1, 2, 4))}")
    print("\n== context.py process time by command (site import to exit)")
    for name, values in sorted(by_process.items()):
        print(f"  {name:16s} {quantiles(values)}  {over(values, (1, 2, 4))}")
    print("\n== Git process duration (parent's view: spawn to exit and pipe EOF)")
    for name, values in sorted(git_times.items(), key=lambda item: -len(item[1])):
        print(f"  {name:14s} {quantiles(values)}  {over(values)}")
    print("\n== CreateProcess (Popen._execute_child) duration")
    for name, values in sorted(spawn_times.items(), key=lambda item: -len(item[1])):
        print(f"  {name:14s} {quantiles(values)}  {over(values, (0.05, 0.1, 0.25, 0.5))}")
    print("\n== The runtime's checks against their deadlines")
    print(f"  refresh preflight _source_git_frame (deadline 1.0 s): {quantiles(preflight)}  {over(preflight)}")
    print(f"  _source_git_ignored (deadline 2.0 s): {quantiles(probes)}  {over(probes)}")
    if trace2:
        print("\n== Git's own trace2 split of each process (pre = parent spawn -> git main, git = main -> exit, "
              "post = exit -> parent sees EOF and exit)")
        for part in ("pre", "git", "post"):
            print(f"  {part:5s} {quantiles([row[part] for row in trace2])}")
        print("  processes over 250 ms:")
        for row in sorted(trace2, key=lambda row: -row["dur"])[:25]:
            if row["dur"] <= 0.25:
                break
            print(f"    {row['dur'] * 1000:7.0f} ms {row['git_cmd']:13s} pre={row['pre'] * 1000:.0f} "
                  f"git={row['git'] * 1000:.0f} post={row['post'] * 1000:.0f} spawn={(row['spawn'] or 0) * 1000:.0f}"
                  f" regions={row['regions'][:3]} ({row['cmd']} {row['call']})")
    gaps, cpu = [], []
    for machine in machines:
        gaps.extend(machine.get("heartbeat_gaps", ()))
        cpu.extend(machine.get("cpu", ()))
        print(f"\n== machine: workers={machine.get('workers')} mode={machine.get('mode')} trace2={machine.get('trace2')}"
              f" minutes={(machine['ended'] - machine['started']) / 60:.1f} git={machine.get('git')}")
        values = [gap for _, gap in machine.get("heartbeat_gaps", ())]
        print(f"  heartbeat (5 ms sleeps) gaps: {over(values, (0.05, 0.1, 0.25, 0.5, 1.0))} "
              f"max={max(values, default=0) * 1000:.0f} ms of {machine.get('heartbeat_ticks')} ticks")
        load = [busy for _, busy, _ in machine.get("cpu", ())]
        if load:
            load.sort()
            print(f"  system CPU busy: p50={load[len(load) // 2]:.2f} p90={load[int(len(load) * 0.9)]:.2f} "
                  f"max={load[-1]:.2f}; steal max={max(steal for _, _, steal in machine['cpu']):.2f}")
    gaps.sort()
    cpu.sort()
    gap_times = [start for start, _ in gaps]
    cpu_times = [stamp for stamp, _, _ in cpu]
    print(f"\n== Git processes over 250 ms: {len(slow_git)} (with any heartbeat gap and the CPU load around them)")
    for wall, duration, spawn, name, command, call_id, error in sorted(slow_git, key=lambda row: -row[1])[:40]:
        low = bisect.bisect_left(gap_times, wall - 1.0)
        high = bisect.bisect_right(gap_times, wall + duration)
        overlapping = [gap for start, gap in gaps[low:high] if start + gap >= wall and start <= wall + duration]
        low = bisect.bisect_left(cpu_times, wall)
        high = bisect.bisect_right(cpu_times, wall + duration + 0.25)
        load = [busy for _, busy, _ in cpu[low:high]]
        print(f"  {duration * 1000:7.0f} ms {name:13s} spawn={(spawn or 0) * 1000:5.0f} ms "
              f"stall={max(overlapping, default=0) * 1000:5.0f} ms "
              f"cpu={(sum(load) / len(load)) if load else float('nan'):.2f} {command} {call_id}"
              f"{' ' + error if error else ''}")
    print(f"\n== Failures: {len(failures)}")
    sites = Counter()
    for call, raised in failures:
        site = raised[0]["site"][-1] if raised and raised[0].get("site") else None
        sites[(tuple(site) if site else None, raised[0].get("cause_type") if raised else None)] += 1
        print(f"\n-- {call['call']} {call['cmd']} rc={call['rc']} dur={call['dur']:.3f}s stderr={call['stderr'][-200:]!r}")
        for trace in call.get("traces", ()):
            for event in trace.get("events", ()):
                if event.get("k") == "git":
                    print(f"     t={event['t']:.3f} git {subcommand(event['argv'])} dur={event.get('dur', 0) * 1000:.0f}"
                          f" spawn={(event.get('spawn') or 0) * 1000:.0f} timeout={event.get('timeout')}"
                          f" rc={event.get('rc')} err={event.get('err')} stderr={event.get('stderr')!r}")
                elif event.get("k") == "raise":
                    print(f"     t={event['t']:.3f} RAISE {event.get('message')!r} cause={event.get('cause')}")
                    for frame in event.get("site", ()):
                        print(f"         {frame[0]}:{frame[1]} {frame[2]}")
                elif event.get("k") in ("_source_git_frame", "_source_git_ignored", "_source_git_checkout"):
                    print(f"     t={event['t']:.3f} {event['k']} dur={event.get('dur', 0) * 1000:.0f} "
                          f"value={event.get('value')} err={event.get('err')}")
    if sites:
        print("\n== Failure sites (innermost frame, cause type): count")
        for (site, cause), count in sites.most_common():
            print(f"  {site} {cause}: {count}")


if __name__ == "__main__":
    main(sys.argv[1:] or ["probe-out"])
