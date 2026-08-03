# scripts/build_mirrors.py — single source of truth for the tool mirrors

Every edition (`Laravel/`, `Symfony/`, `PHP Core/`, `Infrastructure-Creator/`)
ships the same content to several AI tools at once: skills in
`.agents/skills` + `.claude/skills` + `.cursor/skills`, hooks in
`.claude/hooks` + `.cursor/hooks` + `.codex/hooks`, commands/agents in
`.claude` + `.cursor`, and the governance documents (`DOD.md`,
`GOLDEN-PRINCIPLES.md`, `STABILIZATION.md`) in `.claude` + `.cursor` +
`.codex`. `build_mirrors.py` verifies — or regenerates — every mirror from
its canonical copy.

## Where the rules live

The rewrite map (`MIRROR_RULES`) is **data inside each edition**, so it
travels with the edition when the folder is copied into another project:

| Edition | Rules module |
|---|---|
| Laravel, Symfony, PHP Core | `memory-bank/scripts/context_retrieval.py` |
| Infrastructure-Creator | `mirror_rules.py` (no memory-bank runtime there) |

The three PHP editions share one byte-identical `context_retrieval.py`;
edition-specific exemptions are keyed by the `framework` value from
`project-brain/config/runtime.json` (`skip_by_framework`), which keeps the
file identical while still expressing, e.g., Symfony-only exceptions.

## Canon per file class

| Class | Canonical | Mirrors | Transformation |
|---|---|---|---|
| skills | `.agents/skills` | `.claude/skills`, `.cursor/skills` | identity copy |
| SKILL FLOW.md | `.claude/skills` | `.cursor/skills` | identity copy (the `.agents` copy is a per-tool adaptation: bare skill names, no slash commands) |
| hooks | `.claude/hooks` | `.cursor/hooks`, `.codex/hooks` | ordered literal substitutions: event-name header comment, `/tmp/<tool>-loop-detection-` prefix, `SKILLS_DIR`, scanned dot-directories, JSON `"path"` fallback, `/debugger` → `systematic-debugger` (Codex only). Byte-identical in Infrastructure-Creator (documented invariant). |
| commands | `.claude/commands` | `.cursor/commands` | `cursor-command`: Claude orchestration frontmatter (`spawns`/`phase`/`flow-*`) becomes `name` + `description` (first body paragraph, or a pinned override); `.claude/skills/` → `.cursor/skills/` in bodies |
| agents | `.claude/agents` | `.cursor/agents` | `cursor-agent`: drop the Claude-only `model`/`invokes`/`phase` frontmatter keys; body verbatim |
| governance docs | `.claude` | `.cursor`, `.codex` | self-reference rewrite (`.claude/…` → mirror's own tree; skills go to `.agents/skills` for Codex) |

Documented exemptions (each carries a justification comment in the rules
module): the `skill-creator` skill and, in Symfony, its command/agent
wrappers (tool-owned, three deliberate generations); `SKILL FLOW.md` in
`.agents`; every `hooks/README.md` in the PHP editions (each tool documents
its own registration model); `working-memory-read.sh` absent from
`.cursor/hooks` (Cursor has no prompt-submit hook event);
`codebase-mapper.md` command and the `memory-bank`/`project-brain` agents in
`.cursor` (intentionally restructured/condensed for Cursor);
Infrastructure-Creator's `agents/README.md` (edition-specific docs).

## Usage

```bash
python3 scripts/build_mirrors.py --check              # verify all editions, exit != 0 on drift
python3 scripts/build_mirrors.py --check --edition Symfony
python3 scripts/build_mirrors.py --write              # regenerate mirrors from canon
```

`--check` never writes; it lists every mirror file that differs from what
the rules derive, plus stray mirror files no rule accounts for. `--write`
prints each file it changes. Python 3 stdlib only.

## Adding or changing files

1. Edit the **canonical** copy only (see the table above), then run
   `--write` to regenerate the mirrors — never hand-edit a mirror.
2. A new file inside an existing class needs no rule change; it is picked
   up automatically.
3. A file that must legitimately differ per tool gets an entry in `skip`
   (or `skip_by_framework` for one PHP edition) **with a justification
   comment** — an exemption is documentation, not a way to silence drift.
4. When editing the PHP editions' `context_retrieval.py`, copy the file
   byte-identically to all three editions (the Python core is
   parity-checked), and keep `MIRROR_RULES` edition-agnostic.
5. Finish with `--check` (green) and each edition's test suite.

The runtime parity gate (`memory-bank/scripts/context.py parity`) executes
the same `MIRROR_RULES` contract from inside the edition, so a copied-out
edition can verify all of its mirrors (skills including non-markdown files,
hooks, commands, agents, governance documents) without this repository;
`--skills-only` keeps the historical light check used on the indexing hot
path, and `--cross-edition` additionally compares the byte-identical Python
core against sibling editions when the edition sits inside the monorepo
(standalone editions skip it politely). `build_mirrors.py` remains the
build-time regenerator (`--write`) with an equivalent `--check`.
