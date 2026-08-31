---
{
  "id": "MEM-0001",
  "title": "Keep shared workflows aligned across AI editions",
  "type": "convention",
  "status": "active",
  "scope": [
    "accelerator",
    "tooling"
  ],
  "tags": [
    "claude",
    "cursor",
    "codex",
    "skills",
    "synchronization"
  ],
  "created": "2026-07-18",
  "last_verified": "2026-08-31",
  "review_after": "2027-08-31",
  "sources": [
    "specs/MANIFEST.md",
    ".agents/README.md",
    ".cursor/README.md",
    ".codex/README.md"
  ],
  "supersedes": [],
  "superseded_by": null,
  "source_digests": [
    {
      "path": ".agents/README.md",
      "sha256": "9d20f6eb410c3b27b95251e9fa379a94d5ca5e13341b0f54591ccc5b3bffbf3f"
    },
    {
      "path": ".codex/README.md",
      "sha256": "9b8203e717eba78ed4bae1ffef21ca8f9dfc6745b8b7ff3a3a0f7d93fad1c753"
    },
    {
      "path": ".cursor/README.md",
      "sha256": "19ff5bc6cef9d8f61ca6483a9df2b273ab8409b16877b4d86db3cf256095a9e5"
    },
    {
      "path": "specs/MANIFEST.md",
      "sha256": "cf4d8cb9c29da2634a4792d77ac51de2bba24701dc78e84801b6508b07397110"
    }
  ]
}
---

# Keep Shared Workflows Aligned Across AI Editions

## Durable Context

Shared native-PHP skill behavior is authored canonically in `.agents/skills` and mirrored into `.claude/skills` and `.cursor/skills`. Each mirror must preserve the workflow contract while adapting paths, frontmatter, commands, agents, hooks, and tool-integrated behavior to the target platform.

Codex discovers skills from `.agents/skills`; it must not receive duplicated `.codex/skills`, command, or agent trees. Cursor remains self-contained under `.cursor/` and should not simultaneously load `.claude/` integration files.

## Consequences

When changing a shared skill:

1. Review the canonical `.agents/skills` workflow and both native mirrors.
2. Preserve semantic parity without blindly copying unsupported metadata or CLI behavior.
3. Compare skill inventories and validate frontmatter, internal paths, hooks, and edition documentation.
4. Keep tool-specific implementations separate when their native discovery or execution models differ.

## Verification

The synchronization model is documented in `specs/MANIFEST.md` and the edition READMEs. Review this memory whenever those files or the supported tool layouts change.
