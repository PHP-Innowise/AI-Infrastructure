# Confirmation responsibility

Notifications consumes refund.completed. The implementation plan must ensure
that delivery follows a confirmed refund and repeated events do not send
duplicate messages. Specify delayed confirmations and delivery retries.
This example contains no real customer identifiers or delivery credentials.
