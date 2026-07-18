---
name: events-notifications
description: Design and implement Laravel Events, Listeners, model Observers, Notifications (mail/database/broadcast/Slack), and Mailables. Use for decoupled side effects and multi-channel user communication.
phase: execution
flow-next: test-generator
flow-alternatives: [code-reviewer]
related: [architect, coder, queues-jobs]
---

# Events & Notifications

## Overview

Implement decoupled side effects (Events + Listeners, model Observers) and user-facing communication (Notifications, Mailables) using Laravel's own building blocks. Read the relevant Actions/controllers, existing `app/Events`, `app/Listeners`, `app/Notifications`, and `app/Mail` directories, and the model(s) involved before adding new classes.

Targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release).

## Scope Boundary

`events-notifications` owns **how** to implement Events/Listeners/Notifications/Mailables well. It does not own two adjacent decisions:

- **Whether** something should be event-driven at all — that call belongs to `architect`. Its pattern table already recommends "Notification class (queued) rather than manual `Mail::` calls scattered around" for async notifications, and its "When Not To Add A Layer" section warns against dispatching an Event for one synchronous listener with no other subscriber. If that decision hasn't been made yet, go to `/architect` first.
- **Generic job/queue mechanics** — queue middleware (`WithoutOverlapping`, rate limiting), batching, chaining, and retry/backoff tuning apply identically to queued Listeners and queued Notifications as they do to plain Jobs. That mechanics layer belongs to the sibling `queues-jobs` skill; here, only `ShouldQueue` usage is covered as the on/off switch for a Listener or Notification. Cross-reference `/queues-jobs` rather than duplicating middleware/backoff details.

Also distinct from **model lifecycle events** (`creating`, `created`, `updating`, `deleting`, etc.) and Eloquent Observers, which are covered by the `eloquent` skill. This skill focuses on custom **domain Events** (e.g. `OrderShipped`, `InvitationAccepted`) dispatched explicitly from Actions/controllers — see "Domain Events vs Model Events" below for where the line sits and why it matters.

## When To Reach For An Event (And When Not To)

Use an Event + Listener when multiple, independent things should happen in reaction to one action, and especially when:

- Other parts of the app — or features that don't exist yet — need to react without the triggering code knowing about them (a plugin/extension point).
- The reactions are genuinely decoupled: sending a welcome email, notifying an admin Slack channel, and updating an analytics counter are three unrelated concerns that shouldn't be sequenced by hand in one method.

Do **not** reach for an Event as a default indirection layer:

- If there is exactly one listener and no other subscriber is realistically coming, call the method directly (`architect`'s guidance — see its "When Not To Add A Layer" section). An Event with a single listener adds a layer of indirection (find the listener, trace the dispatch) for no decoupling benefit.
- If the "reaction" must happen synchronously and its failure should fail the original operation (e.g. debiting a ledger as part of a purchase), that's a transaction step, not an event — events are for side effects that are allowed to happen independently of, and after, the triggering action succeeds.

## Domain Events vs Model Events

Eloquent models already fire lifecycle events (`saving`, `created`, `updating`, `deleted`, ...) that Observers hook into — that mechanism, and how to register an Observer, is covered by the `eloquent` skill; don't duplicate it here.

This skill's Events are a different, more explicit case: a named class describing something that happened in the domain (`OrderShipped`, `InvitationAccepted`, `PaymentFailed`), constructed with the relevant models, and dispatched explicitly from the Action/Service/controller that caused it — not fired implicitly by Eloquent's save cycle. Prefer a domain Event over relying on a model Observer when:

- The trigger is a specific business operation, not "this row was saved" (an `Order` can be saved for many reasons; `OrderShipped` means one specific thing happened).
- You want the dispatch site to be visible in the Action/controller that causes it, rather than implicit in model persistence.

```php
<?php

declare(strict_types=1);

namespace App\Events;

use App\Models\Order;
use Illuminate\Foundation\Events\Dispatchable;
use Illuminate\Queue\SerializesModels;

final class OrderShipped
{
    use Dispatchable;
    use SerializesModels;

    public function __construct(public readonly Order $order)
    {
    }
}
```

```php
<?php

declare(strict_types=1);

namespace App\Listeners;

use App\Events\OrderShipped;
use App\Notifications\OrderShippedNotification;
use Illuminate\Contracts\Queue\ShouldQueue;

final class SendOrderShippedNotification implements ShouldQueue
{
    public function handle(OrderShipped $event): void
    {
        $event->order->user->notify(new OrderShippedNotification($event->order));
    }
}
```

Register the pairing in `app/Providers/EventServiceProvider.php` (or via `#[AsListener]`/`Event::listen()` closures if the project relies on auto-discovery) and dispatch with `OrderShipped::dispatch($order)` from the Action once the transaction that shipped the order has committed.

## Queued Vs Synchronous Listeners

Default to `ShouldQueue` on a Listener for anything doing I/O — sending mail, calling an external API, writing to a slow external store — so the triggering HTTP request isn't blocked waiting on it:

```php
final class NotifyWarehouseOfShipment implements ShouldQueue
{
    public function handle(OrderShipped $event): void
    {
        Http::post('https://warehouse.example.com/shipments', [
            'order_id' => $event->order->id,
        ]);
    }
}
```

Keep a Listener synchronous only when it's cheap, in-process, and must complete before the request returns (e.g. updating an in-memory/request-scoped cache, or a strict invariant that must hold before the response is sent). Queued Listener middleware, uniqueness (`ShouldBeUnique`), batching, and backoff tuning are generic queue mechanics — see `/queues-jobs`.

## Notifications

A `Notification` is a single class describing one message, deliverable over one or more channels chosen per-notifiable via `via()`:

```php
<?php

declare(strict_types=1);

namespace App\Notifications;

use App\Models\Order;
use Illuminate\Bus\Queueable;
use Illuminate\Contracts\Queue\ShouldQueue;
use Illuminate\Notifications\Messages\MailMessage;
use Illuminate\Notifications\Notification;

final class OrderShippedNotification extends Notification implements ShouldQueue
{
    use Queueable;

    public function __construct(private readonly Order $order)
    {
    }

    /** @return array<int, string> */
    public function via(object $notifiable): array
    {
        return $notifiable->wants_sms ? ['vonage', 'database'] : ['mail', 'database'];
    }

    public function toMail(object $notifiable): MailMessage
    {
        return (new MailMessage)
            ->subject("Order #{$this->order->id} has shipped")
            ->line('Your order is on its way.')
            ->action('Track order', url("/orders/{$this->order->id}"));
    }

    /** @return array<string, mixed> */
    public function toArray(object $notifiable): array
    {
        return ['order_id' => $this->order->id, 'status' => 'shipped'];
    }
}
```

- **Built-in channels:** `mail`, `database` (stored in a `notifications` table, `HasMany` accessible via `$notifiable->notifications`, `->unreadNotifications`), and `broadcast` (real-time — see "Broadcasting Tie-In" below). Define `toDatabase()`/`toBroadcast()` instead of a single `toArray()` when the database payload and the real-time frontend payload need to differ.
- **Common third-party channels:** `vonage` (`laravel/vonage-notification-channel`, SMS, formerly Nexmo) and `slack` (`laravel/slack-notification-channel`) ship official first-party packages; dozens more community channels exist for other providers.
- **`Notifiable` trait:** not just for `User`. Add `use Illuminate\Notifications\Notifiable;` to any model that should receive notifications (an `Order`, a `Team`, an `Admin` guard model) to get `->notify()`, `->notifications`, and routing methods (`routeNotificationForMail()`, `routeNotificationForSlack()`) for that model.
- Send with `$notifiable->notify(new OrderShippedNotification($order))`, or `Notification::send($notifiables, ...)` to fan out to a collection at once.
- Add `ShouldQueue` (and the `Queueable` trait) to the Notification class itself to queue every channel's delivery — this is usually the right default the same way `ShouldQueue` on a Listener is, since `toMail()`/`toVonage()`/`toSlack()` all make an external call.

## Mailables

Use a dedicated `Mailable` class rather than building raw messages inline, especially for anything beyond a one-line notification email:

```bash
php artisan make:mail OrderShippedMail --markdown=mail.orders.shipped
```

```php
<?php

declare(strict_types=1);

namespace App\Mail;

use App\Models\Order;
use Illuminate\Bus\Queueable;
use Illuminate\Contracts\Queue\ShouldQueue;
use Illuminate\Mail\Mailable;
use Illuminate\Mail\Mailables\Content;
use Illuminate\Mail\Mailables\Envelope;
use Illuminate\Queue\SerializesModels;

final class OrderShippedMail extends Mailable implements ShouldQueue
{
    use Queueable;
    use SerializesModels;

    public function __construct(public readonly Order $order)
    {
    }

    public function envelope(): Envelope
    {
        return new Envelope(subject: "Order #{$this->order->id} has shipped");
    }

    public function content(): Content
    {
        return new Content(markdown: 'mail.orders.shipped', with: ['order' => $this->order]);
    }
}
```

- `--markdown` scaffolds both the Mailable class and a Markdown Blade view under `resources/views/mail/`, built on Laravel's mail components (`<x-mail::message>`, `<x-mail::button>`, `<x-mail::panel>`) so plain-text and HTML versions render from one source.
- Queue mail either by implementing `ShouldQueue` on the Mailable itself (as above), or with `Mail::to($user)->queue(new OrderShippedMail($order))` / `Mail::to($user)->later(now()->addMinutes(5), ...)` for a one-off deferred send without changing the class.
- Use a Notification's `toMail()` (returning `MailMessage`) for simple, templated transactional messages that benefit from being paired with other channels; use a `Mailable` directly when the email needs full Blade/Markdown control, attachments, or is not conceptually a multi-channel "notification".
- Test rendered content without sending: `Mail::fake()` plus `$mailable->assertSeeInHtml(...)`/`assertSeeInText(...)`, or render it directly with `(new OrderShippedMail($order))->render()` to snapshot the HTML.

## Broadcasting Tie-In

An Event implementing `ShouldBroadcast` (or a Notification's `broadcast` channel) is the mechanism that pushes real-time updates to the browser via Laravel Echo, served by **Laravel Reverb**. Reverb setup, channel authorization, and the Echo frontend wiring are already covered in `architect`'s "Real-Time And Live Updates" section — don't repeat that setup here. Within this skill, treat `ShouldBroadcast` purely as one more delivery channel alongside `mail`/`database` on an Event or Notification, decided once the real-time infrastructure choice has already been made upstream.

## Testing

Fake the dispatch mechanism, then separately unit-test the logic the fake bypassed — faking only proves something was dispatched/sent, not that its side effects are correct (the same caveat used in `/queues-jobs` for Job/Queue fakes):

```php
it('dispatches OrderShipped when an order ships', function (): void {
    Event::fake();

    (new ShipOrder())->handle($order);

    Event::assertDispatched(OrderShipped::class, fn (OrderShipped $event): bool => $event->order->is($order));
});

it('notifies the customer when an order ships', function (): void {
    Notification::fake();

    (new SendOrderShippedNotification())->handle(new OrderShipped($order));

    Notification::assertSentTo($order->user, OrderShippedNotification::class);
});

it('does not send mail during registration if the user opts out', function (): void {
    Mail::fake();

    $this->postJson('/api/register', ['email' => 'a@example.com', 'marketing_opt_in' => false]);

    Mail::assertNothingSent();
});
```

Cover, per Notification/Mailable/Listener added:

- The dispatch/send assertion (`Event::assertDispatched()`, `Notification::assertSentTo()`, `Mail::assertQueued()`).
- The underlying Listener/Notification logic directly (instantiate it, call `handle()`, assert its actual effect — e.g. the database row, the `toMail()`/`toArray()` payload shape) — this is what the fake skips over.
- A channel-selection branch if `via()` is conditional (e.g. SMS vs mail based on user preference).

## Verification

Possible checks:

```bash
php artisan test --filter=OrderShipped
vendor/bin/pint --test
vendor/bin/phpstan analyse
php artisan event:list       # confirm Event -> Listener registrations resolved as expected
```

Use only commands present in the project; report others as `N/A - tooling not configured`.

## Final Output

Return what changed (Event/Listener/Notification/Mailable/Observer files), which channels are used and why, whether listeners/notifications are queued, tests run, Context Summary, and next step.
