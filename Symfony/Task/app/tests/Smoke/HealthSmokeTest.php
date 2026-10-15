<?php

declare(strict_types=1);

namespace App\Tests\Smoke;

use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\HttpFoundation\Response;

/**
 * The walking skeleton's proof of life: the application boots, routes a
 * request, reaches PostgreSQL, and is connected as a role that Row-Level
 * Security applies to.
 */
final class HealthSmokeTest extends WebTestCase
{
    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    public function testHealthEndpointReportsHealthy(): void
    {
        $this->client->request('GET', '/health');

        self::assertResponseIsSuccessful();
        self::assertResponseStatusCodeSame(Response::HTTP_OK);
        self::assertResponseHeaderSame('content-type', 'application/json');

        $payload = $this->decode();

        self::assertSame('healthy', $payload['status']);
    }

    public function testDatabaseIsReachable(): void
    {
        $this->client->request('GET', '/health');

        self::assertSame('ok', $this->decode()['checks']['database']['status']);
    }

    /**
     * Tenancy isolation is the highest-risk defect class in this product. If
     * the application ever connects as the schema owner or a BYPASSRLS role,
     * every row policy stops applying without raising an error of its own —
     * so the skeleton asserts it from the very first commit.
     */
    public function testApplicationRoleDoesNotBypassRowLevelSecurity(): void
    {
        $this->client->request('GET', '/health');

        self::assertSame(
            'ok',
            $this->decode()['checks']['tenancy_isolation']['status'],
            'The application connects as a role that bypasses Row-Level Security.',
        );
    }

    /**
     * @return array{status: string, checks: array<string, array{status: string, error?: string}>}
     */
    private function decode(): array
    {
        $content = $this->client->getResponse()->getContent();
        self::assertIsString($content);

        /** @var array{status: string, checks: array<string, array{status: string, error?: string}>} $decoded */
        $decoded = json_decode($content, true, 512, \JSON_THROW_ON_ERROR);

        return $decoded;
    }
}
