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
      "sha256": "9b966797ad7925435c162ade513f9dfbe2c5a2ea8b8dd9376ca0b4d802f041e9"
    },
    {
      "path": ".codex/README.md",
      "sha256": "14e396ea1cc039557e55a8bfa800aa0f7d68592f2a16865e7b7d1bd83a4e38a4"
    },
    {
      "path": ".cursor/README.md",
      "sha256": "3e1a91193a403d4941a9d25694dab4568ac3e18cf60e0fb1cd032d839ac0b570"
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

Shared WordPress skill behavior is authored canonically in `.agents/skills` and mirrored into `.claude/skills` and `.cursor/skills`. Each mirror must preserve the workflow contract while adapting paths, frontmatter, commands, agents, hooks, and tool-integrated behavior to the target platform.

Codex discovers skills from `.agents/skills`; it must not receive duplicated `.codex/skills`, command, or agent trees. Cursor remains self-contained under `.cursor/` and should not simultaneously load `.claude/` integration files.

## Consequences

When changing a shared skill:

1. Review the canonical `.agents/skills` workflow and both native mirrors.
2. Preserve semantic parity without blindly copying unsupported metadata or CLI behavior.
3. Compare skill inventories and validate frontmatter, internal paths, hooks, and edition documentation.
4. Keep tool-specific implementations separate when their native discovery or execution models differ.

## Verification

The synchronization model is documented in `specs/MANIFEST.md` and the edition READMEs. Review this memory whenever those files or the supported tool layouts change.
