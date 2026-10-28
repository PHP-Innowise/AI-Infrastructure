#!/usr/bin/env python3
"""Drive random session and Git lifecycles through an installation's memory
hooks, offline, and check what must hold after any of them.

Every other test of the hooks runs a scripted sequence: a session in the
root, one in a worktree, a turn or two each. What happens in between in a
person's day - a branch made or renamed under a running session, a worktree
removed while a session still names it, a session that enters a worktree,
two sessions of one branch answering in the same seconds - was never driven.
This tool installs PHP Core into a scratch project (its code committed, the
installation not) and takes random steps from a seed:

- a turn of a session: its prompt hook, its Stop hook, the save the Stop
  asks for - made, skipped or ignored - and the Stop that settles it. Claude
  Code sessions start in the root, below it (src/) or in a worktree, whose
  hooks run through the dispatcher the installation carries, and a session
  in the root may enter a worktree during a turn, as Claude Code's
  EnterWorktree does: the root's hooks, the input's cwd the worktree. Codex
  sessions start in the root or below it, Cursor sessions in the root;
- a burst: turns of up to three sessions at once, two of them in one folder
  too (never two Cursor sessions, which would share one rule file);
- a new session, a /clear or a compaction;
- Git: commits, branches made, renamed, detached and re-attached, a rebase
  stopped on a conflict - HEAD detached, the branch named only in the
  rebase's state - then continued or aborted, and a commit made or a branch
  moved under a session between its Stop and its save;
- worktrees added in .claude/worktrees and beside the root, some with spaces
  in their names, and removed; worktrees Codex makes itself, on a detached
  HEAD whose owning thread (codex-thread.json) names the task;
- Codex sessions in a worktree, which the Codex dispatcher in the person's
  configuration serves as Claude Code's serves its own;
- a checkpoint as the checkpoint skill makes it - an update of the task -
  half of them under a sandbox that keeps the memory read-only: the update
  waits, and the session's Stop makes it.

What must hold afterwards: every hook exited 0, the runtime journaled no
error and no prompt hook's refresh failed or timed out - a refresh that
another session's write lock kept past its share of the hook's budget
delivers the working state without retrieval and says so, and is counted
(`refresh degraded`); no prompt was told that the memory was not
consulted; every save landed in the task of the branch it was asked on -
in a stopped rebase, the branch being rebased - a save another session
overtook included, and a sample of the learnings saved is found by
`search` from the root and from every worktree left; after a turn
checkpoint in the root, no index of a removed worktree is left; Project
Brain and the Memory Bank validate; no worktree was written into; a
checkpoint a sandbox held was made by the Stop after it, and none was
refused. Only a save asked for before its branch moved may be refused.

With --attached nothing is installed: every session is launched as the
Harness launches it, with the edition lent from this clone (the overlay of
scripts/accelerator_attach.py, project = the folder the session starts in)
and one state directory for the project and all its worktrees. Claude Code
and Codex sessions start in the root or in a worktree, and a Claude Code
session in the root may enter one. There, besides the above, neither the
project nor this clone's edition may be written into.

Nothing leaves the work directory: HOME, TMPDIR, the dispatcher's folder and
an attached project's state are inside it, a new temporary directory unless
--work names a missing or empty one. No model and no network.
Maintainer-only and standard library only; never installed into a project.

    memory_lifecycle_fuzz.py --seed 7 --steps 60
    memory_lifecycle_fuzz.py --seed 21 --steps 70 --edition Laravel --verbose --work /var/tmp/fuzz-21
    memory_lifecycle_fuzz.py --seed 3 --attached

Prints a JSON summary. Exit status: 0 when everything held, 1 otherwise.
"""
from __future__ import annotations

import argparse
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import threading
from collections import Counter
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import memory_rollout_rehearsal as rehearsal  # noqa: E402

GIT = ["git", "-c", "user.name=Fuzz", "-c", "user.email=fuzz@example.invalid", "-c", "commit.gpgsign=false",
       "-c", "core.hooksPath=/dev/null", "-c", "maintenance.auto=false", "-c", "core.editor=true"]
EDITIONS = ("PHP Core", "Laravel", "Symfony", "WordPress")
HOOK_TIMEOUT = 60
# Who may run where: Codex runs the project's hooks from below the root;
# Cursor there was not checked, and only Claude Code runs a worktree's hooks
# through the dispatcher or enters a worktree.
HOSTS = {"root": ("claude", "claude", "codex", "cursor"), "sub": ("claude", "codex"), "worktree": ("claude", "codex")}
# The Harness launches a session in the project or one of its worktrees.
ATTACHED_HOSTS = {"root": ("claude", "claude", "codex"), "worktree": ("claude", "codex")}
STAGES = ("start", "prompt", "stop")


class Fuzz:
    def __init__(self, work: Path, seed: int, verbose: bool, edition: str = "PHP Core",
                 attached: bool = False) -> None:
        self.work, self.seed, self.verbose, self.edition = work, seed, verbose, edition
        self.attached = attached
        self.overlays: Dict[Tuple[str, str], Any] = {}
        self.rng = random.Random(seed)
        self.lock = threading.RLock()
        self.log: List[str] = []
        self.violations: List[str] = []
        self.checkouts: Dict[str, Path] = {}
        self.removed: set = set()
        self.sessions: Dict[str, Dict[str, Any]] = {}
        # marker -> (branch asked on, task ID the save returned, session)
        self.saved: Dict[str, Tuple[str, Optional[str], str]] = {}
        self.counts: Counter = Counter()   # sessions by host and kind
        self.events: Counter = Counter()   # bursts, worktrees entered, commits during a save, rebases
        self.counter = 0

    # -- setup -------------------------------------------------------------

    def install(self) -> None:
        shop = self.shop = self.work / "shop"
        (shop / "src").mkdir(parents=True)
        (shop / "composer.json").write_text('{"name": "acme/shop"}\n', encoding="utf-8")
        (shop / "README.md").write_text("# Shop\n\nThe saffron checkout settles every order.\n", encoding="utf-8")
        (shop / "src/Invoice.php").write_text("<?php\nfinal class Invoice {}\n", encoding="utf-8")
        for name in ("home", "tmp"):
            (self.work / name).mkdir()
        self.env = rehearsal.child_env(self.work / "home", self.work / "tmp", {})
        for argv in (["init", "-q", "-b", "feature/SHOP-1"], ["add", "."], ["commit", "-qm", "Shop"]):
            subprocess.run([*GIT, *argv], cwd=shop, env=self.env, check=True, capture_output=True)
        self.checkouts["root"] = shop
        if self.attached:
            # Nothing installed: the edition is lent from this clone, the
            # state kept apart, as the Harness keeps it.
            import accelerator_attach as attach
            self.attach = attach
            self.home = attach.edition_directory(self.edition)
            self.data = attach.state_directory(shop, self.work / "state-base")
            attach.prepare_state(self.edition, shop, self.data)
            self.dispatcher = None
            self.clone_before = self.clone_status()
            return
        self.home = self.data = shop
        done = subprocess.run([sys.executable, str(rehearsal.INSTALLER), "--edition", self.edition, "--target",
                               str(shop), "--merge-existing"], cwd=ROOT, env=self.env, capture_output=True,
                              text=True, stdin=subprocess.DEVNULL)
        if done.returncode != 0:
            raise RuntimeError("install failed: " + (done.stdout + done.stderr)[-600:])
        # The dispatcher a first session in the root installs into the
        # person's configuration, here in the work directory.
        folder = self.work / "dispatcher"
        folder.mkdir()
        for script, name in (("claude_dispatch.sh", "ai-accelerator-dispatch.sh"),
                             ("claude_dispatch.py", "ai-accelerator-dispatch.py")):
            shutil.copy2(shop / "memory-bank/scripts" / script, folder / name)
        (folder / "projects").write_text(str(shop) + "\n", encoding="utf-8")
        self.dispatcher = folder / "ai-accelerator-dispatch.sh"
        # And Codex's, which a Codex session in the root sets up.
        folder = self.work / "codex-dispatcher"
        folder.mkdir()
        for script, name in (("codex_dispatch.sh", "ai-accelerator-dispatch.sh"),
                             ("codex_dispatch.py", "ai-accelerator-dispatch.py")):
            shutil.copy2(shop / "memory-bank/scripts" / script, folder / name)
        (folder / "projects").write_text(str(shop) + "\n", encoding="utf-8")
        self.codex_dispatcher = folder / "ai-accelerator-dispatch.sh"

    def clone_status(self) -> str:
        """What Git sees changed in this clone's edition, bytecode caches aside."""
        done = subprocess.run(["git", "status", "--porcelain", "--ignored", "--", str(self.home)], cwd=ROOT,
                              capture_output=True, text=True)
        return "\n".join(line for line in done.stdout.splitlines() if "__pycache__" not in line)

    def overlay(self, host: str, launch: Path) -> Any:
        """The Harness's overlay for a session of `host` started in `launch`."""
        with self.lock:
            key = (host, str(launch))
            if key not in self.overlays:
                self.overlays[key] = self.attach.overlay(host, self.edition, launch, self.data)
            return self.overlays[key]

    # -- helpers -----------------------------------------------------------

    def note(self, text: str) -> None:
        with self.lock:
            self.log.append(text)
            if self.verbose:
                print(text, file=sys.stderr)

    def fail(self, text: str) -> None:
        with self.lock:
            self.violations.append(text)
        self.note("VIOLATION " + text)

    def git(self, folder: Path, *argv: str) -> subprocess.CompletedProcess:
        return subprocess.run([*GIT, *argv], cwd=folder, env=self.env, capture_output=True, text=True)

    def branch(self, folder: Path) -> str:
        return self.git(folder, "branch", "--show-current").stdout.strip()

    def rebasing(self, folder: Path) -> str:
        """The branch a rebase stopped in this checkout is replaying, or ''."""
        directory = Path(self.git(folder, "rev-parse", "--absolute-git-dir").stdout.strip() or folder)
        for state in ("rebase-merge", "rebase-apply"):
            try:
                name = (directory / state / "head-name").read_text(encoding="utf-8").strip()
            except OSError:
                continue
            if name.startswith("refs/heads/"):
                return name[len("refs/heads/"):]
        return ""

    def codex_thread(self, folder: Path) -> str:
        """The task a worktree Codex made names while its HEAD is detached:
        codex-thread/<the thread that owns it>, or ''."""
        directory = Path(self.git(folder, "rev-parse", "--absolute-git-dir").stdout.strip() or folder)
        try:
            owner = json.loads((directory / "codex-thread.json").read_text(encoding="utf-8"))["ownerThreadId"]
        except (OSError, ValueError, KeyError, TypeError):
            return ""
        return f"codex-thread/{owner}"

    def task_branch(self, folder: Path) -> str:
        """The branch whose task a turn here works for, as the hooks find it:
        the branch, a stopped rebase's, or the thread that owns a worktree
        Codex made."""
        return self.branch(folder) or self.rebasing(folder) or self.codex_thread(folder)

    def place(self, session: Dict[str, Any]) -> Path:
        folder = self.checkouts[session["checkout"]]
        return folder / session["sub"] if session["sub"] else folder

    def gone(self, session: Dict[str, Any]) -> bool:
        return session["checkout"] in self.removed or not self.place(session).exists()

    def hooks_of(self, session: Dict[str, Any]) -> Tuple[Dict[str, str], Optional[Path], Path]:
        """How the host runs this session's hooks: the environment, the
        dispatcher they go through, and the folder they run in."""
        place = self.place(session)
        if self.attached:
            # The overlay of the folder the Harness launched the session in.
            launch = self.checkouts[session["launch"]]
            overlay = self.overlay(session["host"], launch)
            env = dict(self.env, **(overlay.settings.get("env") or {}), **overlay.environment)
            if session["host"] == "claude":
                env["CLAUDE_PROJECT_DIR"] = str(launch)
            return env, None, place
        if session["host"] == "codex" and session["kind"] == "worktree":
            # Codex reads no .codex/ there: the dispatcher in the person's
            # configuration runs the project's hooks.
            return dict(self.env), self.codex_dispatcher, place
        if session["host"] != "claude":
            return dict(self.env), None, place
        if session["kind"] in ("worktree", "sub"):
            # Claude Code finds no settings there: the dispatcher in the
            # person's configuration runs the project's hooks.
            return dict(self.env, CLAUDE_PROJECT_DIR=str(place)), self.dispatcher, place
        # In the root, and after EnterWorktree: the root's own hooks.
        return dict(self.env, CLAUDE_PROJECT_DIR=str(self.shop)), None, place

    def event(self, session_id: str, stage: str, **fields: Any) -> List[Dict[str, Any]]:
        session = self.sessions[session_id]
        host = session["host"]
        name = rehearsal.EVENTS[host][STAGES.index(stage)]
        env, through, place = self.hooks_of(session)
        if host == "cursor":
            payload: Dict[str, Any] = {"conversation_id": session_id, "generation_id": f"g{self.counter}",
                                       "workspace_roots": [str(self.checkouts[session["checkout"]])],
                                       "cursor_version": rehearsal.CURSOR_VERSION, "hook_event_name": name}
            if stage == "stop":
                payload.update(status="completed", loop_count=int(bool(fields.get("stop_hook_active"))))
            elif stage == "prompt":
                payload.update(prompt=fields.get("prompt", ""), attachments=[])
        else:
            payload = {"session_id": session_id, "cwd": str(place), "hook_event_name": name,
                       "transcript_path": str(self.work / "transcripts" / f"{session_id}.jsonl"), **fields}
            if host == "codex":
                payload["turn_id"] = f"t{self.counter}"
        if self.attached:
            results = self.run_attached(session, name, payload, env, place)
        else:
            results = rehearsal.run_event(self.shop, host, name, payload, env, HOOK_TIMEOUT, through, place)
        for item in results:
            if item["exit"] != 0:
                self.fail(f"{host} {name} {item['hook']} exit {item['exit']} in {session['kind']} {place}: "
                          f"{item['stderr'][-200:]}")
        return results

    def run_attached(self, session: Dict[str, Any], name: str, payload: Dict[str, Any], env: Dict[str, str],
                     place: Path) -> List[Dict[str, Any]]:
        """Run the lent hooks for `name` as the host would: Claude Code's from
        the overlay's settings, Codex's as `-c hooks.*` names them."""
        if session["host"] == "claude":
            groups = (self.overlay("claude", self.checkouts[session["launch"]]).settings.get("hooks") or {}).get(name, [])
        else:
            groups = self.attach.codex_hooks(self.edition).get(name, [])
        results = []
        for command in [hook["command"] for group in groups for hook in group.get("hooks", [])]:
            try:
                done = subprocess.run(["bash", "-c", command], input=json.dumps(payload).encode("utf-8"), cwd=place,
                                      env=env, capture_output=True, timeout=HOOK_TIMEOUT)
                code, stdout, stderr = done.returncode, done.stdout.decode("utf-8", "replace"), \
                    done.stderr.decode("utf-8", "replace")
            except subprocess.TimeoutExpired:
                code, stdout, stderr = "timeout", "", ""
            results.append({"hook": command.rsplit("/", 1)[-1][:60], "exit": code, "stdout": stdout,
                            "stderr": stderr.strip()[-400:]})
        return results

    @staticmethod
    def request_of(results: List[Dict[str, Any]]) -> Optional[str]:
        """The save step a Stop hook's output asks for, as each host carries it."""
        for item in results:
            try:
                decision = json.loads(item["stdout"])
            except ValueError:
                continue
            if not isinstance(decision, dict):
                continue
            feedback = decision.get("hookSpecificOutput")
            if isinstance(decision.get("followup_message"), str):
                return decision["followup_message"]
            if isinstance(feedback, dict) and "native-result" in str(feedback.get("additionalContext")):
                return str(feedback["additionalContext"])
            if decision.get("decision") == "block":
                return str(decision.get("reason") or "")
        return None

    # -- actions -----------------------------------------------------------

    def turn(self, session_id: str, rng: random.Random, burst: bool = False) -> None:
        session = self.sessions[session_id]
        if self.gone(session):
            # A host cannot start a hook in a folder that is gone.
            self.note(f"turn {session_id}: its folder is gone")
            return
        with self.lock:
            self.counter += 1
            marker = f"m{self.counter:03d}x{rng.randrange(16 ** 4):04x}"
        prompt = self.event(session_id, "prompt", prompt=f"How is the invoice numbered? {marker}")
        said = "".join(item["stdout"] for item in prompt)
        if session["host"] == "cursor":
            # Cursor's capsule is the rule its prompt hook renders.
            rule = self.checkouts[session["checkout"]] / rehearsal.CURSOR_RULE
            said = rule.read_text(encoding="utf-8") if rule.is_file() else ""
        if "Memory refresh incomplete" in said:
            # Another session held the index's write lock past this refresh's
            # share of its budget: the capsule carries the working state and
            # says retrieval was not consulted - the runtime's own fallback.
            self.events["refresh degraded"] += 1
        for words in ("Memory not consulted", "Memory refresh unavailable"):
            if words in said:
                line = said[said.index(words):].split("\n", 1)[0]
                self.fail(f"prompt {session_id} ({session['host']} {session['kind']}) in {self.place(session)}: "
                          f"{line[:300]}")
        live = [name for name in self.checkouts if name != "root" and name not in self.removed]
        if (not burst and session["host"] == "claude" and session["kind"] == "root" and live
                and rng.random() < 0.15):
            # The session enters a worktree during this turn.
            target = rng.choice(live)
            session.update(kind="entered", checkout=target, sub="")  # launched where it was
            self.note(f"turn {session_id}: enters {target}")
            self.events["worktree entered"] += 1
        request = self.request_of(self.event(session_id, "stop", stop_hook_active=False,
                                             last_assistant_message="Checked."))
        current = self.task_branch(self.place(session))
        if request is None:
            self.note(f"turn {session_id} ({session['host']} {session['kind']}, {current or 'detached'}): "
                      "no save asked")
            return
        found = rehearsal.SAVE_COMMAND.search(request)
        if found is None:
            self.fail(f"turn {session_id}: a request without a command")
            return
        moved, roll = None, rng.random()
        if self.rebasing(self.checkouts[session["checkout"]]):
            pass
        elif not burst and roll < 0.12:
            moved = f"feature/SHOP-moved-{self.counter}"
            self.git(self.place(session), "checkout", "-q", "-b", moved)
            self.note(f"turn {session_id}: branch moved to {moved} before the save")
        elif not burst and roll < 0.2:
            # The person commits while the agent writes the save.
            folder = self.checkouts[session["checkout"]]
            (folder / "src" / f"Commit{self.counter}.php").write_text("<?php\n", encoding="utf-8")
            self.git(folder, "add", "src")
            self.git(folder, "commit", "-qm", f"Commit {self.counter}")
            self.note(f"turn {session_id}: a commit before the save")
            self.events["commit during a save"] += 1
        env, _, place = self.hooks_of(session)
        if session["kind"] == "worktree" and session["host"] == "claude" and not self.attached:
            # Codex has no file for its commands' variables: its save step
            # names the roots itself.
            env.update(rehearsal.dispatched_environment(self.dispatcher, place, dict(self.env), HOOK_TIMEOUT))
        choice = rng.random()
        if choice < 0.8:
            self.save(session_id, session, found.group(1), marker, current, moved, env, place)
        elif choice < 0.9:
            command = found.group(1).replace("echo -E '<JSON>' | ", "")
            command = (command.replace("--input - --json", "--skip --json") if "--input - --json" in command
                       else command[:command.rindex(" --json")] + " --skip --json" + command[command.rindex(" --json") + 7:])
            subprocess.run(["bash", "-c", command], cwd=place, env=env, capture_output=True, timeout=HOOK_TIMEOUT)
            self.note(f"turn {session_id}: skipped")
        else:
            self.note(f"turn {session_id}: the request is ignored")
        self.event(session_id, "stop", stop_hook_active=True, last_assistant_message="Done.")

    def save(self, session_id: str, session: Dict[str, Any], command: str, marker: str, current: str,
             moved: Optional[str], env: Dict[str, str], place: Path) -> None:
        data = json.dumps({"progress": f"Turn {marker}.", "verified": True, "learnings": [
            {"type": "finding", "title": f"Invoice numbering {marker}",
             "consequence": f"Invoices are numbered per tenant ({marker}).", "sources": ["src/Invoice.php"]}]})
        stdin: Optional[bytes] = data.encode("utf-8")
        if "'<JSON>'" in command:
            command, stdin = command.replace("'<JSON>'", "'" + data.replace("'", "'\\''") + "'"), None
        done = subprocess.run(["bash", "-c", command], cwd=place, env=env, input=stdin, capture_output=True,
                              timeout=HOOK_TIMEOUT)
        output = (done.stdout + done.stderr).decode("utf-8", "replace")
        if done.returncode != 0 and moved is not None:
            self.note(f"turn {session_id}: save refused after the move: {output.strip()[-120:]}")
        elif done.returncode != 0:
            self.fail(f"save {session_id} ({session['host']} {session['kind']}, {current}) in {place}: "
                      f"exit {done.returncode}: {output[-240:]}")
        else:
            try:
                task = json.loads(done.stdout)["task"]["id"]
            except (ValueError, KeyError, TypeError):
                task = None
            with self.lock:
                self.saved[marker] = (current, task, session_id)
            self.note(f"turn {session_id} ({session['host']} {session['kind']}, {current}): saved {marker}")

    def new_session(self) -> str:
        session_id = f"s{len(self.sessions) + 1}"
        live = [name for name in self.checkouts if name != "root" and name not in self.removed]
        kind = self.rng.choice(["root", *([] if self.attached else ["sub"]), *(["worktree"] if live else [])])
        checkout = self.rng.choice(live) if kind == "worktree" else "root"
        sub = "src" if kind == "sub" or (kind == "worktree" and not self.attached and self.rng.random() < 0.3) else ""
        host = self.rng.choice((ATTACHED_HOSTS if self.attached else HOSTS)[kind])
        self.sessions[session_id] = {"kind": kind, "checkout": checkout, "sub": sub, "host": host,
                                     "launch": checkout}
        self.counts[f"{host} {kind}"] += 1
        self.note(f"session {session_id}: {host} {kind} in {checkout}/{sub}")
        self.event(session_id, "start", source="startup")
        return session_id

    def git_action(self) -> None:
        name = self.rng.choice([name for name in self.checkouts if name not in self.removed])
        folder = self.checkouts[name]
        stopped = self.rebasing(folder)
        if stopped:
            self.settle(name, folder, stopped, abort=self.rng.random() < 0.5)
            return
        current = self.branch(folder)
        action = self.rng.choices(["commit", "branch", "rename", "detach", "attach", "conflict"],
                                  weights=[30, 25, 15, 10, 20, 8] if current else [10, 10, 0, 0, 80, 0])[0]
        if action == "conflict":
            # A rebase that stops on a conflict: HEAD detached, the branch's
            # name only in the rebase's state, until it is continued or aborted.
            side = f"{current}-side-{self.counter}"
            path = folder / "src" / f"Conflict{self.counter}.php"
            if self.git(folder, "checkout", "-q", "-b", side).returncode == 0:
                for text, title in (("side", "Side"), ("main", "Main")):
                    path.write_text(f"<?php // {text}\n", encoding="utf-8")
                    self.git(folder, "add", "src")
                    self.git(folder, "commit", "-qm", title)
                    if text == "side":
                        self.git(folder, "checkout", "-q", current)
                self.git(folder, "rebase", side)
                if self.rebasing(folder):
                    self.events["rebase stopped"] += 1
        elif action == "commit":
            (folder / "src" / f"Note{self.counter}.php").write_text("<?php\n", encoding="utf-8")
            self.git(folder, "add", "src")
            self.git(folder, "commit", "-qm", f"Note {self.counter}")
        elif action == "branch":
            stem = "feature/ШОП-" if self.rng.random() < 0.3 else "feature/SHOP-"
            self.git(folder, "checkout", "-q", "-b", f"{stem}{100 + len(self.log)}")
        elif action == "rename" and current:
            self.git(folder, "branch", "-m", current, current + "-renamed")
        elif action == "detach":
            self.git(folder, "checkout", "-q", "--detach")
        elif action == "attach" and not current:
            busy = set(self.git(self.shop, "worktree", "list", "--porcelain").stdout.split())
            free = [line.strip("*+ ").strip() for line in self.git(folder, "branch").stdout.splitlines()
                    if "HEAD" not in line]
            free = [candidate for candidate in free if f"refs/heads/{candidate}" not in busy]
            if free:
                self.git(folder, "checkout", "-q", self.rng.choice(free))
        self.note(f"git {action} in {name}: {current or 'detached'} -> {self.branch(folder) or 'detached'}")

    def settle(self, name: str, folder: Path, stopped: str, abort: bool) -> None:
        """Continue a stopped rebase with the replayed side of every conflict, or abort it."""
        if abort:
            self.git(folder, "rebase", "--abort")
        else:
            self.git(folder, "checkout", "--theirs", "--", "src")
            self.git(folder, "add", "src")
            self.git(folder, "rebase", "--continue")
        self.events["rebase aborted" if abort else "rebase continued"] += 1
        self.note(f"git rebase {'--abort' if abort else '--continue'} in {name} ({stopped}): "
                  f"{self.branch(folder) or 'detached, ' + (self.rebasing(folder) or 'no rebase')}")

    def worktree_action(self) -> None:
        live = [name for name in self.checkouts if name != "root" and name not in self.removed]
        if live and self.rng.random() < 0.3:
            name = self.rng.choice(live)
            self.git(self.shop, "worktree", "remove", "--force", str(self.checkouts[name]))
            self.removed.add(name)
            self.note(f"worktree removed: {name}")
            return
        name = f"wt{len(self.checkouts)}"
        if not self.attached and self.rng.random() < 0.25:
            # Codex's own worktree: detached, in a folder of its own, the
            # thread that owns it written into its Git directory.
            folder = self.work / "codex-worktrees" / name / "shop"
            folder.parent.mkdir(parents=True)
            if self.git(self.shop, "worktree", "add", "-q", "--detach", str(folder)).returncode == 0:
                directory = Path(self.git(folder, "rev-parse", "--absolute-git-dir").stdout.strip())
                owner = "%08x-%04x-%04x-%04x-%012x" % tuple(self.rng.getrandbits(bits)
                                                            for bits in (32, 16, 16, 16, 48))
                (directory / "codex-thread.json").write_text(
                    json.dumps({"version": 1, "ownerThreadId": owner}), encoding="utf-8")
                self.checkouts[name] = folder
                self.events["codex worktree made"] += 1
                self.note(f"worktree made by Codex: {name} at {folder}, thread {owner}")
            return
        spelled = f"{name} with spaces" if self.rng.random() < 0.3 else name
        folder = (self.shop / ".claude" / "worktrees" / spelled if self.rng.random() < 0.5
                  else self.work / f"shop-{spelled}")
        if self.git(self.shop, "worktree", "add", "-q", str(folder), "-b",
                    f"feature/SHOP-{200 + len(self.checkouts)}").returncode == 0:
            self.checkouts[name] = folder
            self.note(f"worktree added: {name} at {folder}")

    def checkpoint(self) -> None:
        """A checkpoint as the checkpoint skill makes it, in a live session's
        folder: an update of the task its hooks name, through the CLI. Half
        of them under a sandbox that keeps the memory read-only to it: the
        update waits, and the session's Stop - the turn ends - makes it."""
        live = [session_id for session_id, session in self.sessions.items() if not self.gone(session)]
        if not live:
            return
        session_id = self.rng.choice(sorted(live))
        session = self.sessions[session_id]
        place, checkout = self.place(session), self.checkouts[session["checkout"]]
        task = self.task_branch(checkout)
        cli = [sys.executable, str(self.home / "memory-bank/scripts/context.py")]
        if not task or self.rebasing(checkout) or subprocess.run(
                [*cli, "get", "--task-id", task, "--json"], cwd=place, env=self.cli_env(place),
                capture_output=True, timeout=HOOK_TIMEOUT).returncode != 0:
            self.note(f"checkpoint {session_id}: no task {task or '(none)'} to update")
            return
        locked = [folder for folder in (self.data / "project-brain/local", self.data / "memory-bank/local")
                  if folder.is_dir()] if not self.attached and self.rng.random() < 0.5 else []
        modes = [folder.stat().st_mode & 0o777 for folder in locked]
        try:
            for folder in locked:
                folder.chmod(0o555)
            done = subprocess.run([*cli, "update", "--task-id", task, "--revision", "auto", "--progress",
                                   f"Checkpoint {self.counter}.", "--json"],
                                  cwd=place, env=self.cli_env(place), capture_output=True, text=True,
                                  timeout=HOOK_TIMEOUT)
        finally:
            for folder, mode in zip(locked, modes):
                folder.chmod(mode)
        if done.returncode != 0:
            self.fail(f"checkpoint {session_id} ({session['host']} {session['kind']}, {task}): exit "
                      f"{done.returncode}: {(done.stdout + done.stderr)[-240:]}")
            return
        if not json.loads(done.stdout).get("queued"):
            self.events["checkpoint made"] += 1
            self.note(f"checkpoint {session_id} ({task}): made")
            return
        self.events["checkpoint held"] += 1
        journal = self.data / "memory-bank/local/native-results.ndjson"
        before = len(journal.read_text(encoding="utf-8").splitlines()) if journal.exists() else 0
        self.event(session_id, "stop", stop_hook_active=False, last_assistant_message="Checkpointed.")
        self.event(session_id, "stop", stop_hook_active=True, last_assistant_message="Done.")
        made = [line.get("event") for line in map(json.loads, journal.read_text(encoding="utf-8").splitlines()[before:])
                if line.get("stage") == "queue"] if journal.exists() else []
        if made != ["made"]:
            self.fail(f"checkpoint {session_id} ({session['host']} {session['kind']}, {task}) held by the "
                      f"sandbox: the Stop after it made {made or 'nothing'}")
        else:
            self.note(f"checkpoint {session_id} ({task}): held, then made by its Stop")

    def burst(self) -> None:
        """Turns of up to three sessions at once - two in one folder too, as two
        terminals or two Harness chats on a project - but never two Cursor
        sessions in one folder, which share its rule file."""
        chosen, cursors = [], set()
        for session_id in self.rng.sample(sorted(self.sessions), min(3, len(self.sessions))):
            session = self.sessions[session_id]
            place = (session["checkout"], session["sub"])
            if self.gone(session) or (session["host"] == "cursor" and place in cursors):
                continue
            chosen.append(session_id)
            if session["host"] == "cursor":
                cursors.add(place)
        self.note(f"burst: {chosen}")
        self.events["burst"] += 1
        threads = [threading.Thread(target=self.turn, args=(session_id, random.Random(self.rng.random()), True))
                   for session_id in chosen]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()

    def run(self, steps: int) -> None:
        self.new_session()
        for _ in range(steps):
            roll = self.rng.random()
            if roll < 0.12 and len(self.sessions) > 1:
                self.burst()
            elif roll < 0.55:
                self.turn(self.rng.choice(sorted(self.sessions)), self.rng)
            elif roll < 0.65:
                self.new_session()
            elif roll < 0.8:
                self.git_action()
            elif roll < 0.89:
                self.worktree_action()
            elif roll < 0.93:
                self.checkpoint()
            else:
                session_id = self.rng.choice(sorted(self.sessions))
                source = self.rng.choice(["clear", "compact"])
                if self.gone(self.sessions[session_id]):
                    self.note(f"session {session_id}: {source} skipped, its folder is gone")
                    continue
                self.note(f"session {session_id}: {source}")
                self.event(session_id, "start", source=source)

    # -- what must hold ----------------------------------------------------

    def cli_env(self, folder: Path) -> Dict[str, str]:
        """The environment the memory CLI runs with for a session in `folder`."""
        if self.attached:
            return dict(self.env, **self.attach.environment(self.edition, folder, self.data))
        return dict(self.env, CLAUDE_PROJECT_DIR=str(folder))

    def check(self) -> Dict[str, Any]:
        for name, folder in self.checkouts.items():
            if name not in self.removed and self.rebasing(folder):
                self.git(folder, "rebase", "--abort")
        tasks = {}
        for path in (self.data / "project-brain/dynamic/tasks").glob("*.md"):
            record = json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
            tasks[record["id"]] = record["external_id"]
        for marker, (branch, task, session_id) in self.saved.items():
            if task is None or tasks.get(task) is None:
                self.fail(f"{marker} ({session_id}): its task is not in Project Brain")
            elif tasks[task] != branch and not tasks[task].startswith(branch):
                # A rename after the save keeps the task.
                self.fail(f"{marker} ({session_id}): saved under {tasks[task]}, asked on {branch}")
        scripts = self.home / "memory-bank/scripts"
        # An attached bank keeps its README in the clone (ACCELERATOR_HOME):
        # `context.py validate` checks it there.
        for argv in ([str(scripts / "context.py"), "validate", "--json"],
                     *([] if self.attached else [[str(scripts / "validate.py"), str(self.data / "memory-bank")]])):
            done = subprocess.run([sys.executable, *argv], cwd=self.shop, env=self.cli_env(self.shop),
                                  capture_output=True, text=True)
            if done.returncode != 0:
                self.fail(f"{Path(argv[0]).name}: exit {done.returncode}: {(done.stdout + done.stderr)[-300:]}")
        for name, folder in self.checkouts.items():
            if (name == "root" and not self.attached) or name in self.removed:
                continue
            # An attached project, root included, is never written into; the
            # worktrees made in its .claude/worktrees are the host's.
            status = subprocess.run(["git", "status", "--porcelain", "--ignored", "--untracked-files=all"],
                                    cwd=folder, env=self.env, capture_output=True, text=True).stdout
            written = [line for line in status.splitlines()
                       if not line[3:].strip('"').startswith(".claude/worktrees/")]
            if written:
                self.fail(f"{'project' if name == 'root' else 'worktree ' + name} written into: {written[:3]}")
        if self.attached and self.clone_status() != self.clone_before:
            self.fail(f"this clone's edition written into: {self.clone_status().splitlines()[:3]}")
        # A saved learning is found from the root and from every worktree left.
        places = [self.shop] + [folder for name, folder in self.checkouts.items()
                                if name != "root" and name not in self.removed]
        for marker in self.rng.sample(sorted(self.saved), min(4, len(self.saved))):
            for folder in places:
                done = subprocess.run([sys.executable, str(scripts / "context.py"), "search", marker, "--json"],
                                      cwd=folder, env=self.cli_env(folder),
                                      capture_output=True, text=True, timeout=HOOK_TIMEOUT)
                if done.returncode != 0 or marker not in done.stdout:
                    self.fail(f"search {marker} from {folder.name}: exit {done.returncode}: "
                              f"{(done.stdout + done.stderr)[-300:]}")
        # After a turn checkpoint in the root - a turn that committed, then
        # its flush - only worktrees that are left keep an index of their own.
        # A task's first turn only records its HEAD.
        branch = self.branch(self.shop)
        if branch:
            def turn(*flush: str) -> None:
                subprocess.run([sys.executable, str(scripts / "context.py"), "turn", "--task-id", branch, *flush],
                               cwd=self.shop, env=self.cli_env(self.shop),
                               capture_output=True, text=True, timeout=HOOK_TIMEOUT)
            turn()
            (self.shop / "src" / "Final.php").write_text("<?php\n", encoding="utf-8")
            self.git(self.shop, "add", "src")
            self.git(self.shop, "commit", "-qm", "Final")
            turn()
            turn("--flush")
            live = {os.path.realpath(str(folder)) for name, folder in self.checkouts.items()
                    if name != "root" and name not in self.removed}
            for marker in (self.data / "memory-bank/local/worktrees").glob("*/worktree"):
                if marker.read_text(encoding="utf-8") not in live:
                    self.fail(f"the index of a removed worktree is left: {marker.read_text(encoding='utf-8')}")
        # A hook exits 0 whatever its runtime did: the journal says what failed.
        journal = self.data / "memory-bank/local/native-results.ndjson"
        lines = [json.loads(line) for line in journal.read_text(encoding="utf-8").splitlines()] \
            if journal.exists() else []
        for line in lines:
            if line.get("event") == "error" or (line.get("stage") == "stop-hook" and line.get("status") not in (None, 0)):
                self.fail(f"journal: {line}")
            if line.get("stage") == "queue" and line.get("event") != "made":
                self.fail(f"journal: a held checkpoint was not made: {line}")
        # The prompt hook records its refresh's exit status: 4 is Cursor's
        # "keep the rule you have", 1 a degraded capsule (counted above) or a
        # failure (reported above), anything else - 124, a timeout - a failure.
        health = self.data / "memory-bank/local/refresh-health.ndjson"
        records = [json.loads(line) for line in health.read_text(encoding="utf-8").splitlines()] \
            if health.exists() else []
        hooks = [record.get("hook_status") for record in records if record.get("source") == "hook"]
        for status in hooks:
            if status not in (0, 1, 4):
                self.fail(f"prompt hook refresh exited {status}")
        unexplained = hooks.count(1) - self.events["refresh degraded"] - sum(
            "Memory refresh unavailable" in violation for violation in self.violations)
        if unexplained > 0:
            self.fail(f"{unexplained} prompt hook refresh(es) exited 1 without a degraded capsule")
        saves = Counter(line.get("code") or line.get("event") for line in lines if line.get("stage") == "save")
        return {"seed": self.seed, "edition": self.edition, "attached": self.attached,
                "sessions": dict(sorted(self.counts.items())),
                "events": dict(sorted(self.events.items())), "saves": len(self.saved),
                "save_lines": dict(sorted(saves.items())),
                "worktrees": len(self.checkouts) - 1, "removed": len(self.removed),
                "violations": self.violations, "log_tail": self.log[-15:] if self.violations else []}


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--edition", choices=EDITIONS, default="PHP Core")
    parser.add_argument("--attached", action="store_true",
                        help="launch every session as the Harness does, the edition lent from this clone")
    parser.add_argument("--steps", type=int, default=60)
    parser.add_argument("--work", type=Path, help="a missing or empty directory to work in, kept afterwards")
    parser.add_argument("--verbose", action="store_true", help="print each step to stderr")
    args = parser.parse_args(argv)
    if shutil.which("bash") is None or shutil.which("git") is None:
        parser.error("needs bash and git")
    if args.work is not None:
        if args.work.exists() and (not args.work.is_dir() or any(args.work.iterdir())):
            parser.error(f"--work {args.work} is not empty")
        args.work.mkdir(parents=True, exist_ok=True)
        work = args.work.resolve()
    else:
        work = Path(tempfile.mkdtemp(prefix="memory-fuzz-"))
    try:
        fuzz = Fuzz(work, args.seed, args.verbose, args.edition, args.attached)
        fuzz.install()
        fuzz.run(args.steps)
        summary = fuzz.check()
    finally:
        if args.work is None:
            shutil.rmtree(work, ignore_errors=True)
    print(json.dumps(summary, indent=1, ensure_ascii=False))
    return 1 if summary["violations"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
