# Task Capsule pressure test: Bauherrenmappe

Date: 2026-07-29

## Scope

The source checkout was `/home/aliaksei/Desktop/bauherrenmappe`, on branch
`BAUMAS-133` at commit `627d11f7dd3d05428b7f17677bd34f3a9bf3c8cf`.
Capsules were generated only in a disposable `git clone --local --no-hardlinks`
clone. The clone used the final mirrored Symfony Context Engine and an external
temporary SQLite database.

The deterministic baseline is the compact projected Working JSON plus the
complete text of every selected source, joined with newlines. The capsule
measurement is the serialized JSON output without its trailing newline. Both
values are Unicode character counts, not runtime token counts.

## Bounded untracked evidence

The source checkout had exactly two untracked files and no tracked changes.
Only these two named documents were copied, after `git check-ignore` returned
`1` for each and the existing secret-pattern validator accepted each without
printing its contents:

| Document | SHA-256 |
| --- | --- |
| `docs/superpowers/plans/2026-07-23-password-change-session-invalidation.md` | `1f0b1554e796db5ec91b6746a907344591a0ca8835472d210a3a2b2ae4b677b3` |
| `docs/superpowers/specs/2026-07-23-password-change-session-invalidation-design.md` | `994c1b28ea57ffba3d24a09ee23dcfcc7446df6d9d7280d7d02551c483b0b9bb` |

`sha256sum --check` passed in the disposable clone and again in the source
checkout after the benchmark. No directory-wide copy was used.

## Measurements

| Scenario | Baseline characters | Capsule characters | Reduction |
| --- | ---: | ---: | ---: |
| Password session invalidation | 52,137 | 1,878 | 96.4% |
| Mandant sender-email branding | 48,377 | 1,375 | 97.2% |
| ReplaceDocument confidentiality | 48,441 | 1,376 | 97.2% |

All three capsules stayed below 8,000 characters and reduced transferred text
by more than 60%.

### Retained Working and authoritative evidence

| Scenario | Exact Working files | Working sources and required retrieved evidence |
| --- | --- | --- |
| Password session invalidation | `src/GraphQL/Resolver/ChangePasswordResolver.php`<br>`tests/Integration/GraphQL/ChangePasswordTest.php` | Both hashed password-session plan and design documents |
| Mandant sender-email branding | `src/Mailer/AdminMailer.php`<br>`src/Entity/Mandant.php`<br>`tests/Smoke/BrandingTest.php` | `CLAUDE.md` and `README.md` |
| ReplaceDocument confidentiality | `src/GraphQL/Operation/Mutation/ReplaceDocument.php`<br>`src/Security/DocumentConfidentiality.php`<br>`tests/Integration/GraphQL/DocumentConfidentialityTest.php` | `CLAUDE.md` |

The measurement program asserted each ordered Working file and source array
before measuring. It also asserted that the scenario-specific authoritative
sources appeared in the retrieval layers. `CLAUDE.md` documents the Mandant
model and confidentiality/revision rules; `README.md` documents mail sender
configuration. The tracked implementation and test paths above remain the
behavior authority for Branding and ReplaceDocument.

The complete selected source sets were:

- password: `CLAUDE.md`, `README.md`, and both password-session documents;
- branding: `CLAUDE.md`, `README.md`, and the password-session plan used only
  as a remaining enrichment fill;
- ReplaceDocument: `CLAUDE.md`, `README.md`, and the password-session plan used
  only as a remaining enrichment fill.

Request-first retrieval retained the authoritative Branding and
ReplaceDocument evidence before Working enrichment filled unused slots; the
unrelated enrichment result did not displace required context.

Runtime token counts were unavailable because neither the Context Engine nor
the test runtime reported them.

## DDEV PHPUnit verification

The commands were run unchanged from the source checkout. None returned zero,
so every result remains **FAIL**.

The first password command invocation triggered a DDEV restart and failed
before PHPUnit because `ddev-bauherrenmappe-minio-init` exited during the
120-second health wait. Repeating the same command immediately reached PHPUnit.

| Command | Result |
| --- | --- |
| `ddev exec php bin/phpunit tests/Integration/GraphQL/ChangePasswordTest.php` | **FAIL, exit 1.** PHPUnit completed 7 tests with 52 assertions and printed `OK, but there were issues!`; its one runner warning was that `phpunit.xml.dist` does not validate under PHPUnit 10.5.63. |
| `ddev exec php bin/phpunit tests/Smoke/BrandingTest.php` | **FAIL, exit 1.** 15 tests and 29 assertions; one error and one runner warning. `testRegistrationEmailUsesTenantSenderIdentity` could not convert an `encrypted_string` because legacy ciphertext did not authenticate with `MFA_ENCRYPTION_KEY` or its previous keys. |
| `ddev exec php bin/phpunit tests/Integration/GraphQL/DocumentConfidentialityTest.php` | **FAIL, exit 1.** PHPUnit completed 8 tests with 29 assertions and printed `OK, but there were issues!`; its one runner warning was the same invalid PHPUnit XML configuration. |

The XML warning identified unsupported `convertDeprecationsToExceptions`,
`include`, and `listeners` configuration. Bauherrenmappe was not edited to
repair these accepted environment failures.

## Source checkout safety

Before cloning or running DDEV, the source status (including every untracked
file), branch, HEAD, index tree, and `memory-bank/local/context.db` state were
captured. After all measurements and DDEV commands:

- the NUL-delimited `git status --porcelain=v1 -z --untracked-files=all`
  snapshot matched byte-for-byte; its SHA-256 remained
  `f23cb6a9de85d901e1dd9f271391cc73457a2878b61d838e73998d6a92335604`;
- branch remained `BAUMAS-133`;
- HEAD remained `627d11f7dd3d05428b7f17677bd34f3a9bf3c8cf`;
- the index tree remained `2222b3e196b2bb83a2d705c2ff461c1c5a9b06b4`;
- `memory-bank/local/context.db` remained absent;
- both named untracked document hashes still matched.

The exact validated temporary benchmark directory and its external database
were removed; no workspace or broader temporary path was cleaned.

## Conclusion

The Task Capsule size, explicit Working-file retention, and
scenario-authoritative-source criteria passed on all three real-project
scenarios. The original checkout safety criteria also passed. The requested
DDEV verification criterion did not pass in the current environment, and the
results above intentionally remain failures.
