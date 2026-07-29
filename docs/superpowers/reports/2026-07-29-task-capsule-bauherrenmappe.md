# Task Capsule pressure test: Bauherrenmappe

Date: 2026-07-29

## Scope

The source checkout was `/home/aliaksei/Desktop/bauherrenmappe`, on branch
`BAUMAS-133` at commit `627d11f7dd3d05428b7f17677bd34f3a9bf3c8cf`.
Capsules were generated only in a disposable `git clone --local --no-hardlinks`
clone. The clone used the mirrored Symfony Context Engine and an external
temporary SQLite database.

The deterministic baseline is the compact Working JSON plus the complete text
of every selected source, joined with newlines. The capsule measurement is the
serialized JSON output without its trailing newline. Both values are Unicode
character counts, not runtime token counts.

## Measurements

| Scenario | Baseline characters | Capsule characters | Reduction | Selected source paths |
| --- | ---: | ---: | ---: | --- |
| Password session invalidation | 52,075 | 1,751 | 96.6% | `CLAUDE.md`<br>`README.md`<br>`docs/superpowers/plans/2026-07-23-password-change-session-invalidation.md`<br>`docs/superpowers/specs/2026-07-23-password-change-session-invalidation-design.md` |
| Mandant sender-email branding | 48,450 | 1,373 | 97.2% | `CLAUDE.md`<br>`README.md`<br>`docs/superpowers/plans/2026-07-23-password-change-session-invalidation.md` |
| ReplaceDocument confidentiality | 48,526 | 1,386 | 97.1% | `CLAUDE.md`<br>`README.md`<br>`docs/superpowers/plans/2026-07-23-password-change-session-invalidation.md` |

All three capsules retained their required source, stayed below 8,000
characters, and reduced transferred text by more than 60%. The specified
read-only measurement program produced the recorded values without a benchmark
framework or added dependency.

Runtime token counts were unavailable because neither the Context Engine nor
the test runtime reported them.

## DDEV PHPUnit verification

The commands were run unchanged from the source checkout. They did not all
pass, so this report records the failures rather than treating completed test
cases as successful commands.

| Command | Result |
| --- | --- |
| `ddev exec php bin/phpunit tests/Integration/GraphQL/ChangePasswordTest.php` | **FAIL, exit 1.** PHPUnit completed 7 tests with 52 assertions and printed `OK, but there were issues!`; its single runner warning was that `phpunit.xml.dist` does not validate under PHPUnit 10.5.63. |
| `ddev exec php bin/phpunit tests/Smoke/BrandingTest.php` | **FAIL, exit 1.** 15 tests and 29 assertions; one error and one runner warning. `testRegistrationEmailUsesTenantSenderIdentity` could not convert an `encrypted_string` because legacy ciphertext did not authenticate with the configured `MFA_ENCRYPTION_KEY` or previous keys. |
| `ddev exec php bin/phpunit tests/Integration/GraphQL/DocumentConfidentialityTest.php` | **FAIL, exit 1.** PHPUnit completed 8 tests with 29 assertions and printed `OK, but there were issues!`; its single runner warning was the same invalid PHPUnit XML configuration. |

The first DDEV start also exceeded its two-minute web-container health timeout:
Node setup delayed PHP-FPM and Mailpit until the timeout boundary. The web
container became healthy immediately afterward, allowing the exact PHPUnit
commands above to execute. The PHPUnit XML warning identified unsupported
`convertDeprecationsToExceptions`, `include`, and `listeners` configuration.

## Source checkout safety

Before cloning or running DDEV, the source status (including every untracked
file), HEAD, index tree, and `memory-bank/local/context.db` state were captured.
After all three test commands:

- the NUL-delimited `git status --porcelain=v1 -z --untracked-files=all`
  snapshot matched byte-for-byte;
- HEAD remained `627d11f7dd3d05428b7f17677bd34f3a9bf3c8cf`;
- the index tree remained `2222b3e196b2bb83a2d705c2ff461c1c5a9b06b4`;
- `memory-bank/local/context.db` remained absent.

The disposable clone copied the source checkout's two untracked
password-session design documents for retrieval, without changing the source.
The validated temporary benchmark directory and its external database were
removed after the evidence was recorded.

## Conclusion

The Task Capsule size and retention criteria passed on all three real-project
scenarios. The original checkout safety criteria also passed. The requested
DDEV verification criterion did not pass in the current environment: two
commands were non-zero solely because of the PHPUnit 10 configuration warning,
while Branding additionally exposed an incompatible persisted MFA ciphertext.
