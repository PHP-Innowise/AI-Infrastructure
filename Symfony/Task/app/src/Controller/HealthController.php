<?php

declare(strict_types=1);

namespace App\Controller;

use Doctrine\DBAL\Connection;
use Symfony\Component\HttpFoundation\JsonResponse;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;

/**
 * Liveness and readiness for the Compose stack.
 *
 * Beyond "is the database reachable", this asserts the property the whole
 * tenancy model rests on: that the application is connected as a role which
 * Row-Level Security actually applies to. Connecting as the schema owner or as
 * a BYPASSRLS role silently disables every tenancy policy, and it is the one
 * RLS failure that produces no error of its own.
 *
 * @see specs/architect-architecture.md "The RLS disagreement, resolved"
 */
final class HealthController
{
    public function __construct(private readonly Connection $connection)
    {
    }

    #[Route('/health', name: 'app_health', methods: ['GET'])]
    public function __invoke(): JsonResponse
    {
        $checks = [];

        $checks['database'] = $this->check(function (): string {
            $this->connection->executeQuery('SELECT 1')->fetchOne();

            return 'ok';
        });

        $checks['tenancy_isolation'] = $this->check(function (): string {
            /** @var array{rolbypassrls: bool, rolsuper: bool}|false $role */
            $role = $this->connection->executeQuery(
                'SELECT rolbypassrls, rolsuper FROM pg_roles WHERE rolname = current_user'
            )->fetchAssociative();

            if (false === $role) {
                throw new \RuntimeException('current role not found in pg_roles');
            }

            if ($role['rolbypassrls'] || $role['rolsuper']) {
                throw new \RuntimeException(
                    'the application is connected as a role that bypasses Row-Level Security'
                );
            }

            return 'ok';
        });

        $healthy = !\in_array(false, array_map(
            static fn (array $check): bool => 'ok' === $check['status'],
            $checks,
        ), true);

        return new JsonResponse(
            [
                'status' => $healthy ? 'healthy' : 'unhealthy',
                'checks' => $checks,
            ],
            $healthy ? Response::HTTP_OK : Response::HTTP_SERVICE_UNAVAILABLE,
        );
    }

    /**
     * @param callable(): string $probe
     *
     * @return array{status: string, error?: string}
     */
    private function check(callable $probe): array
    {
        try {
            return ['status' => $probe()];
        } catch (\Throwable $e) {
            return ['status' => 'failed', 'error' => $e->getMessage()];
        }
    }
}
