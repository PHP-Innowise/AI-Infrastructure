---
name: woocommerce
description: Build and review WooCommerce extensions using public CRUD APIs, extension hooks, HPOS-compatible order access, checkout/cart/store API boundaries, payments, webhooks, Action Scheduler, and compatibility declarations.
phase: execution
flow-next: code-reviewer
flow-alternatives: [security-reviewer, test-generator, verify]
related: [plugin-development, rest-api, performance-optimization]
---

# WooCommerce

## Procedure

1. Verify required WooCommerce and WordPress versions, declared compatibility,
   HPOS support, Blocks/Store API usage, classic checkout support, and external
   gateway or webhook contracts from current project sources.
2. Use WooCommerce CRUD objects and data stores for products, customers,
   orders, coupons, and order items. Do not query or update legacy order posts
   or postmeta directly; code must remain compatible with HPOS when declared.
3. Register integrations after WooCommerce is available and fail gracefully
   when inactive or below the supported version. Do not crash unrelated admin
   or frontend requests.
4. Treat prices, taxes, stock, currency, refunds, and order status as domain
   behavior. Use WooCommerce calculation and formatting APIs and preserve
   decimal precision; never use binary floats for persisted money decisions.
5. Keep checkout/cart hooks performant and idempotent. State classic versus
   block checkout coverage explicitly; server-side validation and permissions
   remain authoritative.
6. Payment/webhook callbacks authenticate the sender, verify signatures using
   the raw request as required, protect against replay, use idempotency keys or
   event IDs, and never log secrets or full payment/customer payloads.
7. Use Action Scheduler for durable WooCommerce background work when the
   project supports it; design unique actions, retries, failure visibility,
   cleanup, and duplicate-safe handlers.
8. Test HPOS enabled/disabled where supported, checkout surface(s), guest and
   account flows, taxes/currencies, refunds/status transitions, concurrency,
   webhook retries, and plugin activation/deactivation compatibility.

## Output

Return supported surfaces/versions, CRUD and HPOS decisions, money/order
invariants, async/webhook security, compatibility declarations, test evidence,
Context Summary, and next step.
