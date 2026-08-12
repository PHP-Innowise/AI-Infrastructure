<?php

declare(strict_types=1);

namespace App\Tests\Platform;

use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Routing\RouterInterface;

/**
 * An entity id in a URL must be constrained to digits.
 *
 * `EntityValueResolver` hands whatever the router matched straight to a
 * lookup on a bigint column, so an unconstrained `{event}` turns
 * `/trainer/events/abc` into `SQLSTATE[22P02]: invalid input syntax for type
 * bigint` — HTTP 500, with the SQL on the page in dev. A mistyped URL, or any
 * crawler, should get 404. Manual testing found it on `/trainer/players/export`,
 * a URL that does not exist and reads like it should.
 *
 * One controller already carried the constraint, on one route, from an
 * earlier encounter with this. The structural test is here so the next route
 * cannot be the one that forgets.
 */
final class RouteParameterConstraintTest extends WebTestCase
{
    /**
     * Route variables that legitimately are not integers. Named explicitly,
     * so adding one is a decision rather than an omission:
     *
     * - `code`, `token`: opaque bearer strings (ShareLinks, forms, password
     *   reset, email verification).
     * - `trainerSlug`: a slug by definition.
     * - `fontName`, `_format`, `_locale`, `code` on error previews: framework
     *   or asset paths, not entity lookups.
     */
    private const array NON_NUMERIC_PARAMETERS = [
        'code', 'token', 'trainerSlug', 'fontName', '_format', '_locale', '_route', 'path', 'filename',
    ];

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    public function testEveryEntityIdInAUrlIsConstrainedToDigits(): void
    {
        /** @var RouterInterface $router */
        $router = self::getContainer()->get(RouterInterface::class);

        $unconstrained = [];

        foreach ($router->getRouteCollection() as $name => $route) {
            // Symfony's own dev/profiler routes are not this codebase's to
            // constrain.
            if (str_starts_with($name, '_')) {
                continue;
            }

            foreach ($route->compile()->getVariables() as $variable) {
                if (\in_array($variable, self::NON_NUMERIC_PARAMETERS, true)) {
                    continue;
                }

                $requirement = $route->getRequirement($variable);

                if (null === $requirement || !str_contains($requirement, '\d')) {
                    $unconstrained[] = sprintf('%s → {%s} in %s', $name, $variable, $route->getPath());
                }
            }
        }

        self::assertSame(
            [],
            $unconstrained,
            "These routes accept a non-numeric value where an entity id is expected, which reaches the database as a\n"
            ."bigint comparison and answers 500 instead of 404. Add an inline constraint — {id<\\d+>} — or, if the\n"
            ."parameter genuinely is not an id, name it in self::NON_NUMERIC_PARAMETERS:\n  ".implode("\n  ", $unconstrained),
        );
    }

    /**
     * And the behaviour the constraint buys, on the URL that found this.
     */
    public function testANonNumericIdIsNotFoundRatherThanAServerError(): void
    {
        $this->client->loginUser($this->trainerAccount());

        foreach (['/trainer/players/export', '/trainer/events/abc', '/trainer/forms/xyz/edit'] as $path) {
            $this->client->request('GET', $path);

            self::assertSame(
                404,
                $this->client->getResponse()->getStatusCode(),
                sprintf('%s should be Not Found — it names no entity that could exist.', $path),
            );
        }
    }

    private function trainerAccount(): \App\Identity\Entity\Account
    {
        /** @var \App\Identity\Repository\AccountRepository $accounts */
        $accounts = self::getContainer()->get(\App\Identity\Repository\AccountRepository::class);
        $account = $accounts->findOneByEmail('trainer@practiceperfect.test');

        self::assertNotNull($account);

        return $account;
    }
}
