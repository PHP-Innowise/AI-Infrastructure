---
name: queues-jobs
description: Design and implement Laravel queued Jobs - job classes, queue configuration, job middleware, unique jobs, batching, chaining, failed-job handling and retries, and Horizon supervisor configuration. Use for any asynchronous/background work.
phase: execution
flow-next: test-generator
flow-alternatives: [code-reviewer, verify]
related: [architect, coder, performance-optimization]
---

# Queues & Jobs

## Overview

Design and implement asynchronous work as Laravel queued Jobs: job class anatomy, job middleware (overlap prevention, rate limiting, throttled retries), unique jobs, batching vs chaining, failed-job handling, and Horizon supervisor configuration for Redis-backed queues.

This branch targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release). For framework-agnostic native PHP, use the `main` branch instead.

## Scope Boundary

`queues-jobs` owns **how to implement a queued job well**, not whether something should be queued. Use a sibling skill when the task is different:

- **Deciding whether an operation should be synchronous or asynchronous in the first place** -> `architect` (its pattern table already has "expensive side effect -> queued Job or queued Listener"). Come here once that decision is made and the job's own behavior needs to be built out.
- **A simple job written inline as part of a larger feature** (dispatch a straightforward `ShouldQueue` job with no special retry/uniqueness/batching needs) -> `coder` can write that directly. Use `queues-jobs` when the job's own behavior — retries/backoff, uniqueness, batching, middleware, Horizon tuning — is the non-trivial part of the task.
- **The job is slow, not incorrect** (it works, but is a resource hog or throughput bottleneck) -> `performance-optimization` for profiling and fixing hotspots inside `handle()`.

## Job Class Anatomy

```php
// php artisan make:job ProcessPayment
final class ProcessPayment implements ShouldQueue
{
    use Dispatchable, InteractsWithQueue, Queueable, SerializesModels;

    public int $tries = 3;
    public int $timeout = 30;

    /** @var array<int, int>|int */
    public $backoff = [10, 30, 60]; // increasing backoff between retries; or a plain int for a fixed delay

    public function __construct(
        public readonly int $orderId,
    ) {
    }

    public function handle(): void
    {
        $order = Order::findOrFail($this->orderId);
        // ...
    }

    public function failed(?Throwable $exception): void
    {
        Log::error('Payment processing failed permanently', ['order_id' => $this->orderId, 'exception' => $exception]);
    }
}
```

Keep the constructor payload small and serializable — pass IDs (`orderId`), not loaded Eloquent models with large relations. `SerializesModels` already re-fetches an Eloquent model from the database by its key when the job runs (rather than serializing the full row/relations into the queue payload), so passing a model instance directly is fine for a single lightweight model, but passing IDs is more explicit about what the job actually needs and avoids surprises if a caller passes a model with relations eager-loaded. `$backoff` accepts either a fixed integer delay or an array for increasing (exponential-ish) backoff across successive retries; `$timeout` bounds how long a single attempt may run before the worker kills it (must be less than the queue worker's own `--timeout`).

## Job Middleware

Attach reusable per-job rules by returning them from a `middleware()` method:

```php
public function middleware(): array
{
    return [
        (new WithoutOverlapping($this->orderId))->expireAfter(180),
        new RateLimited('payment-gateway'),
        (new ThrottlesExceptions(3, 5 * 60))->backoff(30),
    ];
}
```

- **`WithoutOverlapping($key)`** - prevents duplicate concurrent runs of the same logical job (e.g. two "recalculate this order's total" jobs racing for the same order). Powered by an atomic cache lock; always set `expireAfter()` so a job that crashes without releasing the lock doesn't permanently block future runs of the same key.
- **`RateLimited($limiterName)`** (or `RateLimitedWithRedis` for a Redis-only, lower-overhead variant) - throttles job execution against a rate limiter defined with `RateLimiter::for()` in a Service Provider, releasing the job back to the queue with a delay when the limit is hit. Use this to respect a third-party API's rate limit across all workers collectively, not per-worker.
- **`ThrottlesExceptions($maxExceptions, $decayMinutes)`** - a circuit-breaker for a flaky external dependency: after `$maxExceptions` consecutive failures within the decay window, further attempts are short-circuited (released without running `handle()`) until the window passes, protecting the dependency (and the queue) from a thundering herd of retries against something that's already down.

## Unique Jobs

Implement `ShouldBeUnique` to prevent the same logical job from being dispatched twice while one is already pending/running — distinct from `WithoutOverlapping`, which only limits *concurrent execution* rather than dispatch itself:

```php
final class SyncInventory implements ShouldQueue, ShouldBeUnique
{
    public int $uniqueFor = 3600; // release the uniqueness lock after 1 hour even if still processing

    public function __construct(public readonly int $warehouseId)
    {
    }

    public function uniqueId(): string
    {
        return (string) $this->warehouseId;
    }
}
```

Use `ShouldBeUniqueUntilProcessing` instead of `ShouldBeUnique` when a re-queue should be allowed as soon as the current instance *starts* processing (rather than blocking until it fully finishes) — useful when a fresher duplicate dispatched later should still be allowed to queue up right behind the one currently running.

## Batching vs Chaining

**Batching** (`Bus::batch()`) coordinates many *independent* jobs that can run in parallel, with a single completion callback once they're all done:

```php
Bus::batch([
    new ProcessOrder($order1->id),
    new ProcessOrder($order2->id),
    new ProcessOrder($order3->id),
])->then(function (Batch $batch): void {
    // all jobs completed successfully
})->catch(function (Batch $batch, Throwable $e): void {
    // first failure detected within the batch
})->finally(function (Batch $batch): void {
    // batch finished, success or not
})->dispatch();
```

**Chaining** (`Bus::chain()`) runs jobs strictly *sequentially*, where each job only runs if the previous one succeeded — use it when job N genuinely depends on job N-1's output/side effect, not merely for "these happen to be related":

```php
Bus::chain([
    new ExtractReportData($reportId),
    new TransformReportData($reportId),
    new PublishReport($reportId),
])->dispatch();
```

Choose batching when jobs are independent and you need an aggregate "all done" signal; choose chaining when order and dependency matter. A chain can itself contain a batch step, and a batch can add jobs to itself dynamically while running.

## Failed-Job Handling

Failed jobs (all retries exhausted) land in the `failed_jobs` table by default. Inspect and act on them with:

```bash
php artisan queue:failed              # list
php artisan queue:retry <id>          # retry one
php artisan queue:retry all           # retry all
php artisan queue:forget <id>         # discard one
```

Decide deliberately whether a job's failure should **fail loudly** (alert on-call, e.g. a payment capture that must not silently disappear — handle this in `failed()` with a notification/alert) or **degrade gracefully** (log and move on, e.g. a best-effort analytics event where losing one record doesn't matter). Do not let every job default to the same failure behavior; the business impact of "this never ran" differs wildly by job.

## Horizon

Reach for **Laravel Horizon** (`laravel/horizon`, Redis-backed queues only) when the project needs a supervisor dashboard (throughput, wait times, failed jobs across queues at a glance), per-queue worker balancing (`balance: 'auto'` scales worker counts per queue based on load), or is running enough queue volume that plain `queue:work` processes become hard to observe/tune by hand. For simple or low-volume queue needs, a plain `php artisan queue:work` (optionally under Supervisor) is sufficient and avoids the Redis dependency Horizon requires.

For zero-downtime deploys, run `php artisan horizon:terminate` as a post-deploy step: it gracefully finishes any in-flight jobs, then exits so the process monitor (Supervisor) restarts Horizon on the new code. This ties into `architect`'s deployment considerations — ensure the job's own `timeout`, the Horizon supervisor's `timeout`, and Supervisor's `stopwaitsecs` are each larger than the previous so `horizon:terminate` never force-kills a job mid-run.

## Testing

`Queue::fake()` / `Bus::fake()` intercept dispatch so you can assert *that* a job was pushed, without actually running it:

```php
Queue::fake();

// ... code under test dispatches ProcessPayment ...

Queue::assertPushed(ProcessPayment::class, fn (ProcessPayment $job): bool => $job->orderId === $order->id);
```

Faking only proves the job was dispatched with the right arguments — it does not prove `handle()` works. Always pair a fake-based dispatch assertion with a direct unit/feature test of the job's `handle()` method (or a queue-connection-`sync` feature test that lets it actually run) that exercises its real logic, including the failure/backoff path where it matters.

## Verification

Possible checks:

```bash
php artisan test --filter=ProcessPayment
vendor/bin/pint --test
vendor/bin/phpstan analyse
php artisan queue:failed        # confirm no unexpected failures accumulated during manual testing
```

Use only commands present in the project.

## Final Output

Return what changed (job classes, middleware, batching/chaining wiring, Horizon config), the retry/uniqueness/failure-handling decisions made and why, tests run, Context Summary, and next step.
