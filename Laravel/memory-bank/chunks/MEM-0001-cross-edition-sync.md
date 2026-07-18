---
{
  "id": "MEM-0001",
  "title": "Keep shared workflows aligned across AI editions",
  "type": "convention",
  "status": "active",
  "scope": ["accelerator", "tooling"],
  "tags": ["claude", "cursor", "codex", "skills", "synchronization"],
  "created": "2026-07-18",
  "last_verified": "2026-07-18",
  "review_after": "2027-01-18",
  "sources": [
    "specs/MANIFEST.md",
    ".agents/README.md",
    ".cursor/README.md",
    ".codex/README.md"
  ],
  "supersedes": [],
  "superseded_by": null
}
---

# Keep Shared Workflows Aligned Across AI Editions

## Durable Context

Shared Laravel workflow behavior is authored canonically in `.claude/skills` and mirrored into `.cursor/skills` and `.agents/skills`. Each mirror must preserve the workflow contract while adapting paths, frontmatter, commands, agents, hooks, and tool-integrated behavior to the target platform.

Codex discovers skills from `.agents/skills`; it must not receive duplicated `.codex/skills`, command, or agent trees. Cursor remains self-contained under `.cursor/` and should not simultaneously load `.claude/` integration files.

## Consequences

When changing a shared skill:

1. Review the canonical Claude workflow and both native mirrors.
2. Preserve semantic parity without blindly copying unsupported metadata or CLI behavior.
3. Compare skill inventories and validate frontmatter, internal paths, hooks, and edition documentation.
4. Keep tool-specific implementations separate when their native discovery or execution models differ.

## Verification

The synchronization model is documented in `specs/MANIFEST.md` and the edition READMEs. Review this memory whenever those files or the supported tool layouts change.
