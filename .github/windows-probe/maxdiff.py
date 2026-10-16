"""Run unittest names with full assertion diffs (temporary diagnostics)."""
import sys, os, unittest
sys.path.insert(0, os.getcwd())
unittest.TestCase.maxDiff = None
unittest.main(module=None, argv=['maxdiff', *sys.argv[1:]])
