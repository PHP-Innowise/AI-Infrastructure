---
name: infra-validate
description: Review every generated file for uniqueness, completeness, accuracy, and coherence, repairing blocking findings through the owning forges.
---

# /infra-validate

Run the content review-and-repair phase: parallel readers judge every
generated file on uniqueness, completeness, accuracy, and coherence, and
blocking findings are repaired through the owning forges or escalated.

Usage: `/infra-validate <path-to-target-php-project>`

Inside `infra-generate`/`infra-update` this phase runs automatically against
staging before publication. Standalone, it requires the target's
`.infra-manifest.json`, reviews the published accelerator, and publishes
repairs transactionally with a manifest hash refresh.
