# Epic 05: Documents, Versioning, OCR, Search & Export

## Outcome

Authorized users can store, classify, protect, search, version, reconstruct, and export revision-proof building-register documents.

**Scope status:** Existing baseline from Project history.

## Capability tasks

### E05-T01 — Document ownership and metadata

- [ ] Parent each Document under its BuildingRegister.
- [ ] Store category, classifications, external references, and interoperability metadata.
- [ ] Preserve author, Mandant, timestamps, and file association.
- [ ] Validate that related categories and references belong to the permitted scope.

### E05-T02 — Confidential read/write policy

- [ ] Default confidential records to the restrictive policy.
- [ ] Enforce read policy in lists, direct lookups, nested fields, search results, downloads, REST/GraphQL, and admin screens.
- [ ] Enforce create/update/delete policy in API and SPA paths.
- [ ] Avoid revealing confidential metadata or file existence through errors and counts.
- [ ] Keep role mapping and tenant/grant rules consistent across every path.

### E05-T03 — File storage and download access

- [ ] Store production media in configured S3-compatible storage.
- [ ] Keep test/local storage replaceable without changing document authorization.
- [ ] Issue transient download access only after authorization.
- [ ] Preserve file metadata and fail without creating a partial Document when storage fails.

### E05-T04 — Revision chain

- [ ] Replace a Document by creating a new version and linking the old version through `replacedBy`.
- [ ] Keep prior versions immutable and readable only to authorized users.
- [ ] Prevent cycles and cross-register/cross-Mandant version links.
- [ ] Clearly identify the current version in lists and detail views.

### E05-T05 — OCR capture

- [ ] Queue or execute OCR when an eligible document is uploaded.
- [ ] Persist extracted text for authorized search without changing the original file.
- [ ] Represent OCR pending, completed, and failed states safely.
- [ ] Allow retry without duplicating documents or corrupting indexed text.

### E05-T06 — Full-text and metadata search

- [ ] Index authorized document text and metadata in Meilisearch.
- [ ] Filter by building/register, category, classifications, metadata, and current-version state.
- [ ] Apply authorization after search so stale index data cannot leak results.
- [ ] Reindex deterministically after OCR, metadata, confidentiality, or version changes.

### E05-T07 — Historical reconstruction

- [ ] Reconstruct the revision-proof document/register state as of a requested date.
- [ ] Resolve version chains and typed changes consistently.
- [ ] Keep current-state and as-of queries tenant-scoped and authorization-aware.
- [ ] Make boundary timestamps deterministic and covered by focused tests.

### E05-T08 — Import, interoperability, and authority export

- [ ] Preserve document interoperability fields required by the domain contract.
- [ ] Validate import rows before applying them and report row-level errors without partial silent writes.
- [ ] Produce an authority-facing export from the authorized BuildingRegister state.
- [ ] Ensure exports respect confidentiality and selected historical date.

## Acceptance criteria

- Confidential documents never appear through unauthorized list, search, nested, count, metadata, or download paths.
- Replacing a document retains an immutable, acyclic history and one clear current version.
- OCR failures are visible and retryable without data loss.
- Search combines text and metadata while remaining tenant- and policy-safe.
- An as-of reconstruction yields the state valid at the requested instant.
- Exports contain only authorized register data and documents.

## Commit evidence

- `9becdd9` — search index foundation.
- `d6da9e0`, `c2fcbab` — Document re-parenting under BuildingRegister.
- `3044a38`, `e2fdc01` — interoperability fields, classifications, and inline references.
- `30c4008`, `3372484`, `4a3a501`, `8135278`, `211f323` — confidential document defaults and read/write hardening.
- `f633532` — `replacedBy` version chain.
- `9112642` — OCR text capture on upload.
- `6332bcb` — full-text and metadata search/filter.
- `40c777c`, `ef258f4` — historical state reconstruction and cleanup.
- `bc97cef`, `1d4326a` — S3 storage corrections.

## Dependencies

- Epic 01 supplies tenant/grant authorization.
- Epic 03 supplies BuildingRegister, classifications, references, and Change history.
- Epic 08 supplies S3, Meilisearch, workers, and cron.

## Excluded

- Collaborative document editing.
- Public document sharing links.
- AI-generated legal conclusions from OCR content.

