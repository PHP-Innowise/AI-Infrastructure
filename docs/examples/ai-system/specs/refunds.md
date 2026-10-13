# Illustrative refund scenario

This is a synthetic example, not an inventory of a customer's services.

Orders owns cancellation. Payments owns refund state. Notifications owns
delivery of the confirmation. A cancellation requests a refund; a confirmed
refund allows a notification. Duplicate and delayed events require explicit
idempotency and state-transition rules in the implementation plan.

The example files describe contracts only. There is no application runtime,
deployed feature, test environment, or verified production behavior here.
