#!/usr/bin/env python3
"""Mirror rewrite map for the Infrastructure-Creator edition.

This edition has no memory-bank runtime, so the map the PHP editions keep in
`memory-bank/scripts/context_retrieval.py` lives in this standalone module
instead. `scripts/build_mirrors.py` at the monorepo root imports MIRROR_RULES
from here and can verify (--check) or regenerate (--write) every mirror. The
map travels with the edition, so a copied edition keeps its own contract.

Schema and transform semantics are identical to the PHP editions' map (see
the MIRROR_RULES block in any edition's context_retrieval.py). Differences
specific to Infrastructure-Creator:

- Skills are byte-identical across all three mirrors, including
  `SKILL FLOW.md` (there is no per-tool adaptation and no skill-creator).
- Hooks are byte-identical across all three mirrors, including README.md -
  a documented invariant of `.claude/hooks/README.md`. The one exception is
  `subagent-gate.sh`, which is tool-owned: each host exposes a different
  subagent-gate contract (Claude PreToolUse exit codes, Cursor subagentStart
  permission JSON, Codex spawn_agent deny).
- Cursor command descriptions are hand-written one-liners (not derived from
  the body), so they are pinned here as description_overrides.
"""

MIRROR_RULES = {
    "version": 1,
    "classes": [
        {
            # Skill bodies are byte-identical in every mirror, SKILL FLOW.md
            # included. The canonical tree is .agents/skills.
            "name": "skills",
            "canonical": ".agents/skills",
            "mirrors": {
                ".claude/skills": {"transform": "copy"},
                ".cursor/skills": {"transform": "copy"},
            },
        },
        {
            # Hook scripts and their README are byte-identical in all three
            # editions - the documented invariant of .claude/hooks/README.md.
            "name": "hooks",
            "canonical": ".claude/hooks",
            "mirrors": {
                ".cursor/hooks": {"transform": "copy"},
                ".codex/hooks": {"transform": "copy"},
            },
            "skip": [
                # Tool-owned: each host exposes a different subagent-gate
                # contract (Claude PreToolUse exit codes, Cursor subagentStart
                # permission JSON, Codex spawn_agent deny) and reads a
                # different roster source, so the three copies are separate
                # generations by design.
                "subagent-gate.sh",
            ],
        },
        {
            # Slash commands: Claude's orchestration frontmatter is reduced to
            # Cursor's `name` + `description`; bodies are shared verbatim.
            # Descriptions are hand-written (unquoted), hence the overrides.
            "name": "commands",
            "canonical": ".claude/commands",
            "mirrors": {
                ".cursor/commands": {
                    "transform": "cursor-command",
                    "quote_description": False,
                    "description_overrides": {
                        "infra-adapt.md": (
                            "Build and verify an independent 23-skill, "
                            "three-edition generator for a confirmed non-PHP "
                            "target stack."
                        ),
                        "infra-build.md": (
                            "One-shot path that scans then generates in a "
                            "single command, pausing only when a blocking "
                            "ambiguity or a collision is detected."
                        ),
                        "infra-generate.md": (
                            "Turn an approved Project Profile into a working "
                            "accelerator inside the target PHP project, for "
                            "the selected AI-tool edition(s) only."
                        ),
                        "infra-scan.md": (
                            "Scan a target PHP project read-only through "
                            "seven scanners, grounded research, clarifying "
                            "questions, and one reviewable Project Profile."
                        ),
                        "infra-update.md": (
                            "Upgrade a previously generated accelerator to "
                            "the current generator version via its "
                            ".infra-manifest.json, never overwriting "
                            "user-modified files without an explicit "
                            "decision."
                        ),
                    },
                },
            },
        },
        {
            # Agent wrappers: Cursor mirrors every Claude agent with reduced
            # frontmatter (exactly `name` + `description`); bodies are shared.
            "name": "agents",
            "canonical": ".claude/agents",
            "mirrors": {
                ".cursor/agents": {"transform": "cursor-agent"},
            },
            "skip": [
                # Mirror-owned: each edition's README documents its own agent
                # layer (Claude's model/phase conventions vs Cursor's reduced
                # frontmatter).
                "README.md",
            ],
        },
        {
            # Governance documents are byte-identical in all three editions;
            # they name every edition's directories explicitly, so no
            # self-reference rewriting is needed.
            "name": "governance-docs",
            "canonical": ".claude",
            "only": ["DOD.md", "GOLDEN-PRINCIPLES.md", "STABILIZATION.md"],
            "mirrors": {
                ".cursor": {"transform": "copy"},
                ".codex": {"transform": "copy"},
            },
        },
    ],
}
