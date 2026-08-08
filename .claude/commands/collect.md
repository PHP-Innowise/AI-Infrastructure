---
name: collect
description: "Collect a scoped context bundle from this monorepo for an external model. Repository-maintainer tool; not part of any shipped edition."
argument-hint: "[scope] [--edition NAME] [--dry-run] — no arguments lists the scopes"
allowed-tools: Bash(${CLAUDE_PROJECT_DIR}/scripts/collect_context.py:*)
disable-model-invocation: true
---

# Collect context

Ran `scripts/collect_context.py $ARGUMENTS` (output capped at 40 lines):

!`"${CLAUDE_PROJECT_DIR}/scripts/collect_context.py" $ARGUMENTS 2>&1 | tail -40`

## What to do with this

Report the summary above — scope, file count, size, token count and the output
path — and stop.

**Do not open the bundle.** It exists to be handed to a model that cannot see
this checkout; reading it here would spend in this session exactly the context
the bundle was built to move elsewhere. The same goes for `--stdout`: it is for
piping to a clipboard or a file, not for this conversation.

If the run failed, the message above says why and what to pass instead. The
usual causes are a missing `--edition`, patterns that matched nothing, or an
absent `code2prompt` binary (`cargo install code2prompt`).

Token counts are cl100k, the OpenAI BPE tokenizer the CLI carries. Do not
describe them as Claude tokens.
