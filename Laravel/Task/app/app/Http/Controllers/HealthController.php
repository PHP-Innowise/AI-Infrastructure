<?php

declare(strict_types=1);

namespace App\Http\Controllers;

use Illuminate\Http\JsonResponse;
use Illuminate\Support\Facades\DB;
use Symfony\Component\HttpFoundation\Response;
use Throwable;

/**
 * Liveness and readiness for the Compose stack.
 *
 * Beyond "is the database reachable", this asserts the property the whole
 * tenancy model rests on: that the application is connected as a role which
 * Row-Level Security actually applies to. Connecting as the schema owner or as
 * a BYPASSRLS role silently disables every tenancy policy, and it is the one
 * RLS failure that produces no error of its own.
 *
 * @see specs/architect-architecture.md "Tenancy enforcement"
 */
final class HealthController extends Controller
{
    public function __invoke(): JsonResponse
    {
        $checks = [
            'database' => $this->check(static function (): string {
                DB::select('select 1');

                return 'ok';
            }),
            'tenancy_isolation' => $this->check(static function (): string {
                $role = DB::selectOne(
                    'select rolbypassrls, rolsuper from pg_roles where rolname = current_user'
                );

                if ($role === null) {
                    throw new \RuntimeException('current role not found in pg_roles');
                }

                if ($role->rolbypassrls || $role->rolsuper) {
                    throw new \RuntimeException(
                        'the application is connected as a role that bypasses Row-Level Security'
                    );
                }

                return 'ok';
            }),
        ];

        $healthy = ! in_array(false, array_map(
            static fn (array $check): bool => $check['status'] === 'ok',
            $checks,
        ), true);

        return response()->json(
            ['status' => $healthy ? 'healthy' : 'unhealthy', 'checks' => $checks],
            $healthy ? Response::HTTP_OK : Response::HTTP_SERVICE_UNAVAILABLE,
        );
    }

    /**
     * @param  callable(): string  $probe
     * @return array{status: string, error?: string}
     */
    private function check(callable $probe): array
    {
        try {
            return ['status' => $probe()];
        } catch (Throwable $e) {
            return ['status' => 'failed', 'error' => $e->getMessage()];
        }
    }
}
