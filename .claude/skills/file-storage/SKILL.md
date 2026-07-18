---
name: file-storage
description: Implement Laravel file storage and uploads - the Filesystem/Storage abstraction, disk configuration (local/S3/etc.), secure upload handling, and signed/temporary URLs. Use for any feature that stores, serves, or accepts user-uploaded files.
phase: execution
flow-next: security-reviewer
flow-alternatives: [test-generator, code-reviewer]
related: [coder, coder-frontend, security-reviewer]
---

# File Storage

## Overview

Implement file storage, uploads, and file serving using Laravel's `Storage` facade and Filesystem abstraction. Covers disk configuration, secure upload handling, private vs. public visibility, signed/temporary URLs, and streaming large files without exhausting memory.

Targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release); both share the same Filesystem API described here.

## Scope Boundary

`file-storage` **implements** the feature: disk configuration, upload validation, filename handling, visibility, and serving/streaming files back to users. `security-reviewer` **audits** upload handling afterwards as one item in its broader OWASP Top 10 pass (type/size/storage-location/visibility checklist). The two are sequential, not overlapping: run `file-storage` to build the feature correctly the first time, then `security-reviewer` to independently verify it. Don't skip straight to `security-reviewer` expecting it to design the implementation — it reviews what already exists.

`file-storage` also owns general non-upload `Storage`/Filesystem usage: reading, listing, moving, and deleting files already on a disk, not just the initial upload path.

## Disks And The Storage Abstraction

Disks are configured in `config/filesystems.php`. Code should be written against the `Storage` facade's disk-agnostic API, never against a disk's underlying path or SDK calls directly, so switching disks (e.g. local in dev, S3 in production) requires only a config change:

```php
use Illuminate\Support\Facades\Storage;

Storage::disk('local')->put('avatars/1.jpg', $contents);
Storage::disk('s3')->get('avatars/1.jpg');
Storage::disk('s3')->delete('avatars/1.jpg');
```

Common disks:

- **`local`** — stores under `storage/app/private` (Laravel 11+) or `storage/app` by default. Private: not web-accessible unless served through a route/controller, or unless `serve => true` is set on the disk (Laravel 11+), which lets `Storage::temporaryUrl()`/`Storage::url()` work for it. Do not confuse "has a `url()` method" with "safe to expose publicly" — it is still your app serving the request.
- **`public`** — a distinct disk rooted at `storage/app/public`, exposed via a symlink created by `php artisan storage:link` (`public/storage` -> `storage/app/public`). Files on this disk are directly reachable by anyone with the URL once linked; treat placement on this disk itself as the access-control decision, not an afterthought.
- **`s3`** (or another cloud disk via `league/flysystem` adapters) — requires `league/flysystem-aws-s3-v3` and AWS credentials in `config/filesystems.php`/env. Same `Storage::disk('s3')->...` API; visibility is controlled per-object (`'visibility' => 'private'`) or at the bucket/IAM level, not by symlinks.

Choose a **private disk by default** for anything not explicitly meant for public, direct access (user documents, invoices, KYC uploads, anything tied to authorization). Only place a file on `public` (or a public-visibility S3 path) when there is no access-control requirement at all — e.g. a public avatar image, not an uploaded contract.

## Secure Upload Handling

Validate uploads through a Form Request, never inline in the controller:

```php
// app/Http/Requests/StoreAttachmentRequest.php
public function rules(): array
{
    return [
        'file' => ['required', 'file', 'mimes:pdf,jpg,png', 'max:10240'], // KB
    ];
}
```

Key points:

- Prefer `mimes:`/`mimetypes:` over trusting the client-supplied `Content-Type` header or original extension — Laravel's `mimes`/`mimetypes` rules sniff the actual file content (via `finfo`), which is what stops a renamed `.php` file masquerading as `image.jpg`.
- Always set `max:` (in kilobytes) to bound upload size; pair with `upload_max_filesize`/`post_max_size` in `php.ini` so a request can't exhaust memory/disk before validation even runs.
- Never store with the user-supplied original filename. It enables path traversal (`../../etc/passwd`-style names) and overwrite of unrelated files, and it can leak internal naming/PII. Generate a new name instead:

```php
use Illuminate\Support\Str;

// Laravel-generated random name + original extension, no client input in the path
$path = $request->file('file')->store('attachments', 'private');

// Or generate explicitly when you need control over the name/extension:
$filename = Str::uuid().'.'.$request->file('file')->extension();
$path = $request->file('file')->storeAs('attachments', $filename, 'private');
```

Persist the original filename separately (e.g. an `original_name` column) if you need to display it to users — display it escaped through Blade's default `{{ }}`, never used to build a path.

## Signed And Temporary URLs

Two mechanisms exist; use the one that matches the disk:

- **`Storage::disk('s3')->temporaryUrl($path, $expiration)`** — for cloud disks with native expiring-URL support (S3 pre-signed URLs). As of Laravel 11+, the `local` disk also supports `temporaryUrl()` if the disk config has `'serve' => true`.
- **`URL::temporarySignedRoute()`** — a signed route your own controller verifies (`$request->hasValidSignature()`, or the `signed` middleware). Needed when you want full control over the response (authorization re-check, custom headers, streaming) rather than delegating to native disk pre-signing, or when integrating with a disk that has no native temporary-URL support.

```php
// routes/web.php
Route::get('/files/{attachment}', DownloadAttachmentController::class)
    ->name('attachments.download')
    ->middleware('signed');

// app/Http/Controllers/DownloadAttachmentController.php
public function __invoke(Request $request, Attachment $attachment): StreamedResponse
{
    abort_unless($request->hasValidSignature(), 403);
    $this->authorize('view', $attachment);

    return Storage::disk('private')->download($attachment->path, $attachment->original_name);
}
```

Both mechanisms enforce expiration through the signature itself (a tampered or expired `expires`/`signature` query parameter fails verification), but that check must actually execute server-side (`hasValidSignature()` or the `signed` middleware) — do not treat "the link looks like a signed URL" as sufficient without the app verifying it on every request. A signed route without the `signed` middleware (or an explicit `hasValidSignature()` check) provides no protection at all.

For a disk that doesn't natively support `temporaryUrl()`, override the generator once in a Service Provider to route through your signed controller instead of hand-rolling it per call site:

```php
// app/Providers/AppServiceProvider.php boot()
Storage::disk('local')->buildTemporaryUrlsUsing(
    fn (string $path, \DateTime $expiration, array $options) =>
        URL::temporarySignedRoute('attachments.download', $expiration, array_merge($options, ['path' => $path]))
);
```

## Streaming And Serving Large Files

Avoid `Storage::get($path)` for large files — it reads the entire file into memory before returning it. Prefer Laravel's response helpers, which stream:

```php
// Streams directly to the client, sets Content-Type from the file
return Storage::disk('private')->response($path);

// Same, but forces a download with Content-Disposition: attachment
return Storage::disk('private')->download($path, $downloadName);
```

Use `Storage::response()`/`download()` (or `StreamedResponse` for programmatically generated content) whenever a file could plausibly exceed a few MB, and always when the disk is a remote/cloud disk where `get()` would otherwise buffer a full network round-trip in memory.

## Testing Uploads

Use `Storage::fake()` to swap a disk for an in-memory fake for the duration of a test — no real files touch disk, and you get assertions for what was written:

```php
use Illuminate\Http\UploadedFile;
use Illuminate\Support\Facades\Storage;

it('stores a validated attachment on the private disk', function () {
    Storage::fake('private');

    $file = UploadedFile::fake()->create('report.pdf', 500, 'application/pdf');

    $response = $this->actingAs($user)
        ->post('/attachments', ['file' => $file]);

    $response->assertRedirect();
    Storage::disk('private')->assertExists(Attachment::first()->path);
});

it('rejects an oversized or wrong-type upload', function () {
    Storage::fake('private');

    $file = UploadedFile::fake()->create('malware.exe', 20000);

    $this->actingAs($user)
        ->post('/attachments', ['file' => $file])
        ->assertInvalid(['file']);

    Storage::disk('private')->assertDirectoryEmpty('attachments');
});
```

`UploadedFile::fake()->image(...)`/`->create(...)` generates realistic fake files (with a genuine MIME type) so MIME-sniffing validation rules are actually exercised, not bypassed. Cover: the happy-path store, a validation-failure case (bad MIME, oversized file), and — for anything served back through a signed route or controller — an authorization-denied case for a user who shouldn't be able to download it.

## Verification

Possible checks:

```bash
php artisan test --filter=Attachment
vendor/bin/pint --test
vendor/bin/phpstan analyse
php artisan storage:link   # confirm the public disk symlink exists where used
```

Use only commands present in the project. Test the validation-failure path (wrong MIME, oversized file) in addition to the happy path, and confirm a signed/temporary URL rejects an expired or tampered signature.

## Final Output

Return what changed (disks configured, upload/validation code, signed-route or temporary-URL wiring), which disk each file class is stored on and why (public vs. private), tests run, Context Summary, and next step.
