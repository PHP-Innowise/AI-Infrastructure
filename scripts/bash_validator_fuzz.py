#!/usr/bin/env python3
"""Generate shell command lines and check bash-validator against bash itself.

Each line puts a command, CMD, inside the forms whose parsing the validator
has got wrong before: ${...} (default values, patterns, nesting), $((...))
and the subshell bash reads in place of one, backquotes (escaped, nested,
ended inside quotes or a heredoc body), quotes, heredocs under quoted and
unquoted delimiters, line continuations, bash 5.3's ${ command; } - with
commands after them, so that a form read too far shows too.

bash runs each line with CMD replaced by a function that only writes a
marker; the validator is handed the same line with CMD replaced by
`gh repo delete x`, which it always refuses. A line where bash ran the marker
and the validator let the command through is a miss. The run fails on any
miss, and on any hook run that neither passed nor refused. A line bash ran
cleanly without the marker but the validator refused is counted, not failed:
those depend on values a static check cannot know (`${HOME:-$(...)}` with
HOME set, `${X#$(...)}` with X empty).

Nothing but the marker function ever runs. The oracle is the bash on PATH:
bash 5 or later, as the validator's rules follow it.

    python3 scripts/bash_validator_fuzz.py
    python3 scripts/bash_validator_fuzz.py --seeds 7,8 --cases 2000 --show 10
    python3 scripts/bash_validator_fuzz.py --hook Laravel/.claude/hooks/bash-validator.sh
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
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Optional, Sequence

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_HOOK = REPO_ROOT / "PHP Core" / ".claude" / "hooks" / "bash-validator.sh"
BASH = shutil.which("bash") or "/bin/bash"
# A function: its name echoed, or glued into a word, runs nothing - a marker
# that is a command (echo ...) would be "run" by any echo that prints it.
PRELUDE = "ran_marker() { echo RAN-$((6*7)) >&2; }\n"
MARK = "ran_marker"
DESTRUCTIVE = "gh repo delete x"
NL = "\n"
HOOK_LIMIT = 30


class Grammar:
    """Random command lines around CMD, depth-limited."""

    def __init__(self, rng: random.Random) -> None:
        self.rng = rng

    def pick(self, *options):
        return self.rng.choice(options)()

    def sub(self, d: int) -> str:
        """Something that runs CMD wherever bash expands it."""
        return self.pick(
            lambda: "$(CMD)", lambda: "`CMD`", lambda: "$( CMD )", lambda: "$(\nCMD\n)",
            lambda: "$\\\n(CMD)", lambda: "$\\\n\\\n(CMD)", lambda: "${ CMD; }", lambda: "${| CMD; }",
            lambda: "$(echo " + self.word(d - 1) + "; CMD)" if d > 0 else "$(CMD)",
        )

    def literal(self) -> str:
        return self.rng.choice(["text", "a b", "}", "{", ")", "(", "'", '"', "\\", "\\}", "\\)", "$", "#", ";",
                                "&", "|", "<", ">", "<<", "x y", "-", "%", "/", ":", "\\\n", "\t", "`"])

    def brace(self, d: int) -> str:
        op = self.rng.choice([":-", ":+", "-", "#", "##", "%", "/", "//", ":=", ""])
        body = "".join(self.piece(d - 1, "brace") for _ in range(self.rng.randint(0, 3)))
        if op in ("/", "//"):
            body += "/" + "".join(self.piece(d - 1, "brace") for _ in range(self.rng.randint(0, 2)))
        start = self.rng.choice(["${", "${", "$\\\n{"])
        return start + self.rng.choice(["X", "HOME", "#X", "!X"]) + op + body + self.rng.choice(["}", "}", "}", ""])

    def arith(self, d: int) -> str:
        parts = []
        for _ in range(self.rng.randint(1, 3)):
            parts.append(self.pick(
                lambda: self.rng.choice(["1", " + ", "*", " << 2", "<", " ? 1 : 0", "X", "X[1]", " ", ",", "-"]),
                lambda: self.sub(d - 1),
                lambda: "(" + "".join(self.rng.choice(["1", "+", "X"]) for _ in range(2)) + ")",
                lambda: "X[" + self.sub(d - 1) + "]",
                lambda: self.brace(d - 1),
                lambda: '"' + self.rng.choice(["1", self.sub(d - 1)]) + '"',
                lambda: "'1'",
                lambda: "CMD",
            ))
        return (self.rng.choice(["$((", "$((", "$( (", "$(\\\n("]) + "".join(parts)
                + self.rng.choice(["))", "))", ") )", ")", ")+1)", "));CMD;:"]))

    def dquoted(self, d: int) -> str:
        return '"' + "".join(self.piece(d - 1, "dq") for _ in range(self.rng.randint(0, 3))) + self.rng.choice(['"', '"', '"', ""])

    def squoted(self) -> str:
        inside = ["$(CMD)", "`CMD`", "x", "}", '"', "\\", "${X:-$(CMD)}"]
        return "'" + "".join(self.rng.choice(inside) for _ in range(self.rng.randint(0, 2))) + "'"

    def backquoted(self, d: int) -> str:
        parts = []
        for _ in range(self.rng.randint(1, 3)):
            parts.append(self.pick(
                lambda: "echo " + self.word(d - 1),
                lambda: "CMD",
                lambda: "\\`CMD\\`",
                lambda: "echo \\$(CMD)",
                lambda: ("cat <<" + self.rng.choice(["'EOF'", "EOF", '"EOF"']) + NL
                         + self.rng.choice(["x", "`", "x ` ; CMD ; : `", "$(CMD)", "\\`CMD\\`", "'"]) + NL + "EOF" + NL),
                lambda: "echo '" + self.rng.choice(["`", "x", "$(CMD)"]) + "'",
                lambda: 'echo "' + self.rng.choice(["`", "x", "$(CMD)", '\\"', "\\`CMD\\`"]) + '"',
                lambda: "echo $(echo " + self.rng.choice(["`", "x", "\\`"]) + ")",
                lambda: "; CMD ;",
            ))
        return "`" + self.rng.choice([" ", ""]).join(parts) + self.rng.choice(["`", "`", "`", ""])

    def piece(self, d: int, context: str) -> str:
        options = [self.literal, self.literal, lambda: "x"]
        if d > 0:
            options += [lambda: self.sub(d), lambda: self.brace(d), lambda: self.arith(d), lambda: self.backquoted(d)]
            if context != "dq":
                options += [lambda: self.dquoted(d), self.squoted]
            else:
                options += [lambda: "'", lambda: "'$(CMD)'", lambda: '\\"', lambda: "\\$(CMD)"]
        return self.rng.choice(options)()

    def word(self, d: int) -> str:
        return "".join(self.piece(d, "word") for _ in range(self.rng.randint(1, 3)))

    def heredoc(self, d: int) -> str:
        delimiter = self.rng.choice(["EOF", "'EOF'", "EOF", '"EOF"', "\\EOF"])
        operator = self.rng.choice(["<<", "<<", "<<-"])
        body = "".join(self.piece(d, "dq") + self.rng.choice(["", NL]) for _ in range(self.rng.randint(1, 3)))
        end = self.rng.choice(["EOF", "EOF", "\tEOF", "EO\\\nF"])
        return self.rng.choice(["cat ", "cat > f ", ": "]) + operator + delimiter + NL + body + NL + end

    def focused(self) -> str:
        """One form with CMD as the only command in it, plain text around:
        each form the parser got wrong, on its own."""
        filler = lambda: self.rng.choice(["", "a", "x y", "'q'", '"d"', "1", "/tmp/"])
        sub = lambda: self.rng.choice(["$(CMD)", "`CMD`", "$\\\n(CMD)", "${ CMD; }"])
        op = lambda: self.rng.choice([":-", "-", ":=", "#", "%", "/x/", ":+"])
        return self.pick(
            lambda: "${X" + op() + filler() + sub() + filler() + "}",
            lambda: '"${X' + op() + filler() + sub() + filler() + '}"',
            lambda: "\"${X:-'" + sub() + "'}\"",
            lambda: "${X:-${Y:-" + sub() + "}}",
            lambda: "${X:-'}'" + sub() + "}",
            lambda: "$(( " + self.rng.choice(["1 + ", "X[", "(1+2) * "]) + sub() + self.rng.choice([" ))", "] ))", " ))"]),
            lambda: "$((CMD) )",
            lambda: '"$(( ' + sub() + ' ))"',
            lambda: "$\\\n(CMD)",
            lambda: "${ CMD; }",
            lambda: "${| CMD; }",
            lambda: "`echo \\`CMD\\``",
            lambda: "`cat <<'EOF'\nx ` ; CMD ; : `\nEOF\n`",
            lambda: "`echo '`; CMD",
            lambda: "$(cat <<EOF\n" + filler() + sub() + filler() + "\nEOF\n)",
        )

    def line(self) -> str:
        d = self.rng.randint(1, 3)
        head = self.rng.choice(["echo ", ": ", "x=", "echo a ", "printf %s "])
        if self.rng.random() < 0.3:
            return head + self.focused()
        text = self.pick(
            lambda: head + self.word(d),
            lambda: head + self.word(d) + " " + self.word(d),
            lambda: self.heredoc(d),
            lambda: ": " + self.backquoted(d),
            lambda: head + self.dquoted(d),
        )
        roll = self.rng.random()
        if roll < 0.35:
            text += self.rng.choice(["; CMD", NL + "CMD", " && CMD", " | CMD"])
        elif roll < 0.45:
            text = "CMD; " + text
        return text


@dataclass
class Result:
    lines: int = 0
    ran: int = 0
    syntax: int = 0
    missed: List[str] = field(default_factory=list)
    value_dependent: List[str] = field(default_factory=list)
    errors: List[str] = field(default_factory=list)


def bash_outcome(line: str, work: str) -> str:
    """'ran' (the marker), 'clean' (ran nothing, said nothing), 'syntax' (bash
    refused the line) or 'error' (an expansion failed at run time)."""
    environment = dict(os.environ, X="", HOME=work, LC_ALL="C")
    try:
        done = subprocess.run([BASH, "-c", PRELUDE + line.replace("CMD", MARK)], stdin=subprocess.DEVNULL,
                              capture_output=True, text=True, cwd=work, timeout=10, env=environment)
    except subprocess.TimeoutExpired:
        return "error"
    if "RAN-42" in done.stderr:
        return "ran"
    if any(sign in done.stderr for sign in ("syntax error", "unexpected EOF", "bad substitution")):
        return "syntax"
    return "error" if done.stderr.strip() else "clean"


def hook_outcome(hook: Path, line: str, work: str) -> str:
    payload = {"session_id": "fuzz", "hook_event_name": "PreToolUse", "tool_name": "Bash", "cwd": work,
               "tool_input": {"command": line.replace("CMD", DESTRUCTIVE)}}
    try:
        done = subprocess.run([BASH, str(hook)], input=json.dumps(payload), capture_output=True, text=True,
                              cwd=work, timeout=HOOK_LIMIT)
    except subprocess.TimeoutExpired:
        return "error: no answer in %d s" % HOOK_LIMIT
    if done.returncode == 2:
        return "block"
    if done.returncode == 0 and not done.stderr.strip():
        return "pass"
    return "error: exit %d: %s" % (done.returncode, done.stderr.strip()[:200])


def lines_for(seed: int, cases: int) -> List[str]:
    grammar, seen, lines = Grammar(random.Random(seed)), set(), []
    while len(lines) < cases:
        line = grammar.line()
        if "CMD" in line and line not in seen:
            seen.add(line)
            lines.append(line)
    return lines


def run(hook: Path, seeds: Sequence[int], cases: int, jobs: int = 8) -> Result:
    """Every seed's lines through bash and the hook; the tally."""
    result = Result()
    work = tempfile.mkdtemp(prefix="bash-validator-fuzz-")
    try:
        lines = [line for seed in seeds for line in lines_for(seed, cases)]

        def one(line: str):
            return line, bash_outcome(line, work), hook_outcome(hook, line, work)

        with ThreadPoolExecutor(max(1, jobs)) as pool:
            for line, bash, verdict in pool.map(one, lines):
                result.lines += 1
                result.ran += bash == "ran"
                result.syntax += bash == "syntax"
                if verdict.startswith("error"):
                    result.errors.append(line + "\n    " + verdict)
                elif bash == "ran" and verdict == "pass":
                    result.missed.append(line)
                elif bash == "clean" and verdict == "block":
                    result.value_dependent.append(line)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return result


def bash_major() -> int:
    done = subprocess.run([BASH, "-c", "echo ${BASH_VERSINFO[0]}"], capture_output=True, text=True)
    try:
        return int(done.stdout.strip())
    except ValueError:
        return 0


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--hook", type=Path, default=DEFAULT_HOOK, help="a .claude bash-validator.sh copy")
    parser.add_argument("--seeds", default="1,2,3", help="comma-separated seeds (default 1,2,3)")
    parser.add_argument("--cases", type=int, default=400, help="lines per seed (default 400)")
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--show", type=int, default=5, help="lines of each kind to print")
    arguments = parser.parse_args(argv)
    if bash_major() < 5:
        print("bash_validator_fuzz: the oracle needs bash 5 or later on PATH", file=sys.stderr)
        return 2
    seeds = [int(seed) for seed in arguments.seeds.split(",") if seed.strip()]
    result = run(arguments.hook, seeds, arguments.cases, arguments.jobs)
    print(f"{result.lines} lines, bash ran the marker in {result.ran}, refused {result.syntax} as malformed; "
          f"missed {len(result.missed)}, hook errors {len(result.errors)}, "
          f"refused though bash ran nothing (value-dependent) {len(result.value_dependent)}")
    for title, items in (("MISSED", result.missed), ("HOOK ERROR", result.errors),
                         ("VALUE-DEPENDENT", result.value_dependent)):
        for line in items[:arguments.show]:
            print(f"--- {title}: {line!r}")
    return 1 if result.missed or result.errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
