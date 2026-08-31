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
  "created": "2026-07-16",
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
      "sha256": "fd58e447434c51f07b283c28d5c54b24919cd335cc9dcc7266fb4c929910d34e"
    },
    {
      "path": ".codex/README.md",
      "sha256": "27cfaa234442a2e25223489a55d86bc1f61ca0ee0dd336b8ad388d46d81f104d"
    },
    {
      "path": ".cursor/README.md",
      "sha256": "81da073e070f891fc1b72c4bf995ed42c9a5a124e161e1c76cb07325a77bea27"
    },
    {
      "path": "specs/MANIFEST.md",
      "sha256": "67af89f43d52152bcefb8dddaa72959aba17fed638798840211889193ccdeab4"
    }
  ]
}
---

# Keep Shared Workflows Aligned Across AI Editions

## Durable Context

Shared Symfony skill behavior is authored canonically in `.agents/skills` and mirrored into `.claude/skills` and `.cursor/skills`. Each mirror must preserve the workflow contract while adapting paths, frontmatter, commands, agents, hooks, and tool-integrated behavior to the target platform.

Codex discovers skills from `.agents/skills`; it must not receive duplicated `.codex/skills`, command, or agent trees. Cursor remains self-contained under `.cursor/` and should not simultaneously load `.claude/` integration files.

## Consequences

When changing a shared skill:

1. Review the canonical `.agents/skills` workflow and both native mirrors.
2. Preserve semantic parity without blindly copying unsupported metadata or CLI behavior.
3. Compare skill inventories and validate frontmatter, internal paths, hooks, and edition documentation.
4. Keep tool-specific implementations separate when their native discovery or execution models differ.

## Verification

The synchronization model is documented in `specs/MANIFEST.md` and the edition READMEs. Review this memory whenever those files or the supported tool layouts change.
