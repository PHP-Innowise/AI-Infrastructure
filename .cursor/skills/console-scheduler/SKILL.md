---
name: console-scheduler
description: Build custom Artisan console commands and schedule recurring tasks - command signatures/arguments/options, output styling, task scheduling in routes/console.php, overlap prevention, and scheduled-task failure handling.
phase: execution
flow-next: test-generator
flow-alternatives: [code-reviewer, verify]
related: [coder, architect]
---

# Console Scheduler

## Overview

Build a dedicated Artisan console command's interface and, when the command needs to run on a recurring basis, configure it on the schedule correctly: no overlapping runs, no duplicate runs across servers, and no silent failures.

This branch targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release). For framework-agnostic native PHP, use the `main` branch instead.

## Scope Boundary

`console-scheduler` owns building a command's **interface** (signature, arguments, options, output) and the **recurring-schedule configuration** around it. Use a sibling skill when the task is different:

- **The business logic a command performs** -> `coder` owns Actions/Services and general feature behavior. If a command's `handle()` method would contain more than simple orchestration (a few lines calling an existing Action/Service), extract that logic into an Action/Service per `coder`'s guidance rather than writing it inline in the command — this keeps the logic testable and reusable outside the console context (e.g. from a queued job or an HTTP endpoint).
- **Architecture decisions** about whether a task should be a command, a queued job, or a scheduled Event -> `architect`.

## Where The Schedule Lives (Version-Dependent)

Confirm the project's Laravel version (`composer.json`'s `laravel/framework` constraint, or `php artisan --version`) before assuming a location:

- **Laravel 11 and later:** the schedule is defined directly in `routes/console.php` using the `Illuminate\Support\Facades\Schedule` facade (`Schedule::command(...)`, `Schedule::call(...)`). There is no `app/Console/Kernel.php` in a fresh Laravel 11+ skeleton.
- **Laravel 10 and earlier:** the schedule is defined in `app/Console/Kernel.php`'s `schedule(Schedule $schedule): void` method, using `$schedule->command(...)` on the injected `$schedule` instance.

Both forms are functionally equivalent (same underlying `Illuminate\Console\Scheduling\Schedule` API); only where the schedule is declared changed. A project upgraded from Laravel 10 may still have a `Kernel.php` with a `schedule()` method that delegates or coexists with `routes/console.php` — check what actually exists before adding a second definition site.

## Command Anatomy

Scaffold with `php artisan make:command SendWeeklyDigest`, which generates a class extending `Illuminate\Console\Command`.

```php
<?php

declare(strict_types=1);

namespace App\Console\Commands;

use App\Actions\SendWeeklyDigestEmails;
use Illuminate\Console\Command;

final class SendWeeklyDigest extends Command
{
    protected $signature = 'digest:send {team? : Optional team ID to limit the run} {--dry-run : Preview recipients without sending}';

    protected $description = 'Send the weekly digest email to active team members';

    public function __construct(private readonly SendWeeklyDigestEmails $sendWeeklyDigestEmails)
    {
        parent::__construct();
    }

    public function handle(): int
    {
        $teamId = $this->argument('team');
        $isDryRun = (bool) $this->option('dry-run');

        $recipients = $this->sendWeeklyDigestEmails->recipients($teamId);

        if ($recipients->isEmpty()) {
            $this->warn('No eligible recipients found.');

            return self::SUCCESS;
        }

        if ($isDryRun) {
            $this->table(['Email', 'Team'], $recipients->map(fn ($r) => [$r->email, $r->team->name])->toArray());

            return self::SUCCESS;
        }

        $this->withProgressBar($recipients, function ($recipient): void {
            $this->sendWeeklyDigestEmails->handle($recipient);
        });

        $this->newLine();
        $this->info("Sent digest to {$recipients->count()} recipients.");

        return self::SUCCESS;
    }
}
```

Key building blocks:

- **`$signature`**: `{argument}` is required, `{argument?}` is optional (with an optional `= 'default'` value), `{--flag}` is a boolean option, `{--option=}` takes a value (`{--option=default}` for a default). Document each with `{arg : description}` / `{--option= : description}` so `php artisan digest:send --help` is self-explanatory.
- **Output helpers** (`$this->info()`, `$this->error()`, `$this->warn()`, `$this->line()`, `$this->table()`, `$this->withProgressBar()`, `$this->ask()`/`$this->confirm()` for interactive prompts) — prefer these over raw `echo`/`print` so output is styled consistently and testable via `expectsOutput()`.
- **Return codes**: `self::SUCCESS` (0), `self::FAILURE` (1), `self::INVALID` (2, for misuse — e.g. invalid combination of arguments/options) from `handle()`. A queue worker, CI job, or another script relying on the exit code needs these to be accurate, not just an implicit `null`/`void` return.

## Scheduling

Register a command or an inline closure on the schedule:

```php
// routes/console.php (Laravel 11+)
use Illuminate\Support\Facades\Schedule;

Schedule::command('digest:send')->weeklyOn(1, '08:00');

Schedule::call(function (): void {
    DB::table('sessions')->where('last_activity', '<', now()->subDays(7)->getTimestamp())->delete();
})->daily();
```

Common frequency helpers: `->everyMinute()`, `->hourly()`, `->daily()`, `->dailyAt('13:00')`, `->weekly()`, `->weeklyOn(1, '08:00')` (Monday), `->monthly()`, `->cron('* * * * *')` for a raw expression. Use `->timezone('Europe/Madrid')` whenever the schedule must run at a specific local time regardless of the server's configured timezone (e.g. a report that must land at 9am for a specific region even if the server runs in UTC) — otherwise the schedule silently follows the server's `date.timezone`/`app.timezone` and can drift when either changes.

## Overlap And Multi-Server Safety

Both matter the moment a project runs more than one worker or more than one app server — neither is optional at that point:

- **`->withoutOverlapping($expiresAtMinutes = 1440)`**: prevents a new run from starting while a previous run of the *same task* is still in progress (an execution mutex). Pass an explicit expiration in minutes sized to comfortably exceed the task's real worst-case duration — the default is 24 hours, which means a crashed run without a clean lock release blocks the task for a full day if left unset.
- **`->onOneServer()`**: when the schedule runs on more than one server (each with its own cron calling `schedule:run`), ensures only one server executes that tick — the first server to claim the per-minute lock wins, others skip it (a scheduling mutex, distinct from `withoutOverlapping`'s execution mutex).

```php
Schedule::command('reports:rebuild')
    ->dailyAt('02:30')
    ->timezone('UTC')
    ->onOneServer()
    ->withoutOverlapping(90);
```

Use `withoutOverlapping()` alone on a single-server topology; add `onOneServer()` once the schedule runs from more than one server's cron. For long-running or variable-duration tasks, prefer both together.

## Failure Handling

A scheduled task's failure (non-zero exit code for `command()`/`exec()` tasks) is invisible unless something surfaces it — cron does not alert on its own, and a silently-broken scheduled task can go unnoticed for a long time.

```php
Schedule::command('reports:rebuild')
    ->dailyAt('02:30')
    ->emailOutputOnFailure('ops@example.com')
    ->onFailure(function (): void {
        report(new \RuntimeException('reports:rebuild failed'));
    });
```

- `->emailOutputOnFailure($addresses)` mails the captured output only on a non-zero exit — only valid for `command()`/`exec()` tasks (they capture process output; `call()`/`job()` tasks do not).
- `->onFailure(callback)` / `->onSuccess(callback)` run arbitrary code on outcome; type-hint `Illuminate\Support\Stringable $output` on the closure parameter to access captured output inside the callback.
- For `Schedule::call()` closures, wrap the body in `try`/`catch` and `report($e)` explicitly — the scheduler cannot detect a closure's internal failure the way it detects a command's exit code.
- Do not rely on the scheduler alone to notice its own outages (e.g. the server's cron itself silently stops firing); pair `onFailure()`/`emailOutputOnFailure()` with external monitoring (a heartbeat ping service, Laravel Pulse, or an uptime check) for anything business-critical.

## Testing

Assert on exit code and expected output via Laravel's console testing helpers:

```php
it('sends the weekly digest successfully', function (): void {
    $this->artisan('digest:send')
        ->expectsOutput('Sent digest to 3 recipients.')
        ->assertSuccessful(); // shorthand for assertExitCode(0)
});

it('warns when there are no recipients', function (): void {
    $this->artisan('digest:send', ['team' => (string) $emptyTeam->id])
        ->expectsOutput('No eligible recipients found.')
        ->assertExitCode(Command::SUCCESS);
});
```

Use `->expectsQuestion($question, $answer)` to mock interactive prompts (`$this->ask()`/`$this->confirm()`), and `->expectsOutputToContain()` for partial-string assertions when exact output text is brittle to assert on.

## Verification

Possible checks:

```bash
php artisan test --filter=Digest
vendor/bin/pint --test
vendor/bin/phpstan analyse
php artisan schedule:list     # confirm the task is registered and shows the expected next run
```

Use only commands present in the project.

## Final Output

Return: the command file(s) created/modified, the signature and return codes used, the schedule entry (frequency, overlap/server-safety flags, failure handling), tests run, Context Summary, and next step.
