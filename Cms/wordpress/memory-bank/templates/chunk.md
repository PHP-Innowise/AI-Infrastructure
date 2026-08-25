---
{
  "id": "MEM-YYYYMMDD-xxxxxxxx",
  "title": "Replace with one durable concept",
  "type": "convention",
  "status": "needs-review",
  "scope": ["application"],
  "tags": ["replace-me"],
  "created": "YYYY-MM-DD",
  "last_verified": "YYYY-MM-DD",
  "review_after": "YYYY-MM-DD",
  "sources": ["path/to/authoritative-source"],
  "supersedes": [],
  "superseded_by": null,
  "valid_from": "YYYY-MM-DD",
  "valid_to": null,
  "source_digests": []
}
---

# Replace With The Memory Title

## Durable Context

State the smallest reusable fact, constraint, decision, or lesson. Do not copy an entire specification or task transcript.

## Consequences

Explain what future work should do differently because of this memory and where it applies.

## Verification

Record how the cited sources establish the claim and what change should trigger review.

Leave `source_digests` empty when writing by hand and run `python3 memory-bank/scripts/context.py bank-reverify --id MEM-...` once the chunk is saved: it digests every local path in `sources` and stamps `last_verified`, so the chunk can later notice that what it summarizes has moved on.
