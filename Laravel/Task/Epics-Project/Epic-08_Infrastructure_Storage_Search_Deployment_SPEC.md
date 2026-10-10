# Epic 08: Infrastructure, Storage, Search & Deployment

## Outcome

Project builds, deploys, and runs reliably with its Symfony/Nuxt application, MySQL data, S3 media, Meilisearch index, mail, scheduled jobs, and Stripe billing integration.

**Scope status:** Existing baseline from Project history, extended only where Epic 07 requires Stripe deployment support.

## Capability tasks

### E08-T01 — Application runtime

- [ ] Run the PHP/Symfony backend and generated Nuxt admin assets with production-safe configuration.
- [ ] Keep database schema setup, cache warmup, and application boot fail-fast.
- [ ] Maintain health checks that reflect application readiness without exposing internals.
- [ ] Keep runtime secrets outside source control.

### E08-T02 — Database and background work

- [ ] Operate MySQL with repeatable schema deployment and backups.
- [ ] Run messenger workers under a process supervisor where asynchronous work is configured.
- [ ] Restart workers safely after deployment.
- [ ] Run deadline, maintenance, search, OCR, and related scheduled commands through cron without overlap or duplicate effects.

### E08-T03 — S3-compatible media storage

- [ ] Configure separate non-production and production buckets/endpoints.
- [ ] Validate storage configuration during deployment or health checks.
- [ ] Keep confidential file authorization in the application rather than public bucket policy.
- [ ] Treat failed uploads and downloads as recoverable errors without partial document state.

### E08-T04 — Meilisearch

- [ ] Run a supported Meilisearch image/service with persistent storage.
- [ ] Protect the instance with a non-public master key and network restrictions.
- [ ] Initialize and rebuild indexes repeatably.
- [ ] Detect and recover from stale index data without treating search as the authorization source.

### E08-T05 — CI/CD and deployment

- [ ] Build and verify the backend and frontend in Jenkins.
- [ ] Deploy only the configured branch/environment.
- [ ] Keep the platform app specification reusable and free of plaintext secrets.
- [ ] Detect superseded deployments and report deployment status accurately.
- [ ] Apply Nginx/basic-auth controls only where the environment requires them.
- [ ] Maintain supervisor and cron configuration as deployed artifacts.

### E08-T06 — Stripe production operations

- [ ] Provision separate Stripe test/live Price, Customer Portal, secret-key, and webhook configurations.
- [ ] Expose the webhook endpoint over TLS without interactive session authentication.
- [ ] Verify webhook signatures in the application and monitor delivery failures.
- [ ] Provide a safe per-Mandant resynchronization command/action without bulk destructive repair.
- [ ] Document key rotation and environment promotion without copying live secrets into development.

## Acceptance criteria

- A deployment either completes with healthy application, worker, storage, search, and scheduled-job configuration or fails visibly.
- S3 and Meilisearch outages do not bypass document authorization or corrupt primary database state.
- Superseded builds do not overwrite a newer deployment.
- No environment file, secret, credential, private key, or customer payment data is committed.
- Stripe test and live resources cannot be mixed silently.

## Commit evidence

- `55f0046`, `d710f5d` — setup and operating documentation.
- `58c0103`, `d2987c2`, `82faa88` — initial IT-42 deployment activation.
- `05e7596`, `f170115`, `dbc5c95`, `6eaaa19` — deployment status fixes.
- `401f3c0`, `8a6f52c` — versioned deployment specification with protected secret handling.
- `e5501c4` — deployment branch restriction.
- `05ae63c`, `e0f2630`, `0ceb62a`, `84e855b` — cron and environment access controls.
- `53d9ba9` — Nginx/basic-auth improvements.
- `344c3c3` — superseded deployment handling.
- `1c761dc` — reusable deployment-library integration.
- `af9a948` — supervisor configuration improvements.
- `41d5642`, `bceb137`, `072b068` — environment access and Meilisearch image improvements.
- `bc97cef`, `1d4326a` — S3 storage fixes.
- `bd42cf8` — application specification update.

## Dependencies

- Epics 01–07 define the application behavior operated here.

## Excluded

- Multi-region active-active deployment.
- Kubernetes migration.
- A new observability platform unless production evidence requires it.

