---
name: multisite
description: Design and review WordPress multisite behavior including network versus site scope, activation, provisioning, blog switching, users/capabilities, data ownership, caching, cron, REST, and large-network operations.
phase: planning
flow-next: coder
flow-alternatives: [plugin-development, database-designer, writing-plans]
related: [wp-cli, content-modeling, security-reviewer]
---

# Multisite

## Procedure

1. Establish whether the feature is per-site, network-wide, per-user, or
   globally external. Record the source of truth and administrator who owns it.
2. Inspect network activation, new-site initialization, deletion/archive,
   existing-site backfill, user membership, domain mapping, cache groups,
   cron, and data tables touched by the feature.
3. Do not assume network activation runs per-site activation code. Design a
   bounded, resumable provisioning/backfill path for existing and future sites.
4. Pair every `switch_to_blog()` with `restore_current_blog()` using control
   flow that restores even on failure. Do not retain site-scoped objects,
   option values, table names, or cache keys across switches.
5. Choose `get_option()` versus `get_site_option()` deliberately. Make cache
   keys and invalidation match site/network ownership.
6. Use capabilities appropriate to site or network administration. Do not
   treat super-admin status as a substitute for the feature's object-level
   authorization model.
7. Avoid synchronous all-site loops on web requests. Large-network work uses
   bounded WP-CLI/background batches with retry and observability.
8. Test at least two sites, site switching/restoration, network and site
   administrators, provisioning, deletion/deactivation, cache isolation, and
   partial-failure recovery.

## Output

Return scope matrix, lifecycle/provisioning plan, data/cache/capability
decisions, large-network risks, tests, Context Summary, and next step.
