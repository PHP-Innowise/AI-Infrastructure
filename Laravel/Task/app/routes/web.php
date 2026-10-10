<?php

declare(strict_types=1);

use App\Http\Controllers\HealthController;
use Illuminate\Support\Facades\Route;

Route::get('/', function () {
    return view('welcome');
});

// Liveness and readiness. Deliberately outside every auth and tenant
// middleware: the container healthcheck calls it before anyone is signed in,
// and it must be able to report a tenancy misconfiguration rather than fail
// because of one.
Route::get('/health', HealthController::class)->name('health');
