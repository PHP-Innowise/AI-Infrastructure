# Refund responsibility

Payments consumes order.cancelled and proposes refund.completed after the
refund is confirmed. Define correlation, duplicate handling, temporary errors
and final failures before implementing this consumer/producer. A retry must
not create a second refund. No actual payment provider is configured here.
