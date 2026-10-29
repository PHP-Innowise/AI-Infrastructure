"""bash-validator against bash itself, on generated command lines.

The corpus (test_bash_validator_corpus.py) holds the cases someone thought
of; scripts/bash_validator_fuzz.py writes lines around the forms the parser
got wrong before - ${...}, $((...)), backquotes, heredocs, quotes, line
continuations, bash 5.3's ${ command; }, a case pattern's ) - and asks bash
which of them run the command. A line bash runs it in must be refused. The
generic section is identical in every copy, so PHP Core's stands for all.
"""
from __future__ import annotations

import importlib.util
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
SCRIPT = REPO_ROOT / "scripts" / "bash_validator_fuzz.py"
SPEC = importlib.util.spec_from_file_location("bash_validator_fuzz", SCRIPT)
fuzz = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = fuzz  # its dataclass looks itself up there
SPEC.loader.exec_module(fuzz)
HOOK = REPO_ROOT / "PHP Core" / ".claude" / "hooks" / "bash-validator.sh"


@unittest.skipUnless(fuzz.bash_major() >= 5, "the oracle needs bash 5 or later on PATH")
class BashValidatorFuzzTest(unittest.TestCase):
    def test_no_generated_line_runs_past_the_validator(self) -> None:
        result = fuzz.run(HOOK, seeds=(1, 2), cases=300)
        self.assertEqual([], result.errors)
        self.assertEqual([], result.missed, "bash ran the command in these, the validator let them through")
        # The grammar still reaches bash: most lines are not malformed.
        self.assertGreater(result.ran, 200)

    def test_the_check_finds_what_the_validator_used_to_miss(self) -> None:
        # A copy that skips the inside of ${...} again, as the validator did
        # until 2026-10-29: ${X:-$(gh repo delete x)} goes through it.
        text = HOOK.read_text(encoding="utf-8")
        walk = 'bv_brace "$depth" "$seg" "$indq" ;;'
        self.assertEqual(1, text.count(walk))
        folder = Path(tempfile.mkdtemp(prefix="bash-validator-fuzz-old-"))
        self.addCleanup(shutil.rmtree, folder, True)
        old = folder / "edition" / ".claude" / "hooks" / "bash-validator.sh"
        old.parent.mkdir(parents=True)
        old.write_text(text.replace(walk, "bv_upto '}'; BV_P=$(( BV_P + ${#BV_UPTO} + 1 )) ;;"), encoding="utf-8")
        result = fuzz.run(old, seeds=(1,), cases=300)
        self.assertTrue(result.missed, "the fuzzer no longer reaches what a parser that skips ${...} lets through")


if __name__ == "__main__":
    unittest.main()
