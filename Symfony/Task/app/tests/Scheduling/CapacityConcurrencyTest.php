<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\RsvpRepository;
use App\Scheduling\Service\RsvpService;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\DBAL\DriverManager;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Security\Core\Authentication\Token\UsernamePasswordToken;
use Symfony\Component\Security\Core\Authentication\Token\Storage\TokenStorageInterface;

/**
 * AC-02-67, "Acceptance Criteria (Epic-Level)" § "Data Integrity": "RSVP
 * count never exceeds an event's capacity — capacity management holds even
 * under concurrent registration attempts for the same spot."
 *
 * PHPUnit runs single-process/single-threaded, so genuine two-simultaneous-
 * request concurrency cannot be reproduced directly. This proves the same
 * guarantee a real second concurrent request would rely on —
 * EventRepository::lockForUpdate()'s `SELECT ... FOR UPDATE` genuinely
 * blocking a second writer — using a second, independent DBAL connection
 * (identical credentials to the ORM's own) that takes and holds the exact
 * same row lock first. A short lock_timeout on the attempting connection
 * turns "blocks forever" into a fast, deterministic failure instead of an
 * indefinite test hang, while still proving the lock is real: an unlocked
 * (buggy) implementation would let the second attempt straight through.
 */
final class CapacityConcurrencyTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    public function testASecondRsvpAttemptBlocksWhileTheEventRowIsLocked(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Concurrency Target Session', 'capacity' => 1]);
        $pat = $this->patPlayer();
        $patAccount = $this->account('player@practiceperfect.test');

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $mainConnection = $em->getConnection();

        // A second, independent connection with identical credentials —
        // simulates a genuinely concurrent second request's own DB
        // connection (which real concurrency, two separate PHP-FPM
        // workers, would also each have).
        $secondConnection = DriverManager::getConnection($mainConnection->getParams());
        $secondConnection->executeStatement(sprintf("SET app.current_trainer = '%d'", (int) $trainer->getId()));
        $secondConnection->beginTransaction();
        // Takes and holds the exact same row lock
        // EventRepository::lockForUpdate() takes inside RsvpService::rsvp().
        $secondConnection->executeQuery('SELECT id FROM event WHERE id = ? FOR UPDATE', [$event->getId()]);

        $blocked = false;

        try {
            $mainConnection->executeStatement("SET lock_timeout = '1500ms'");

            // loginUser() only takes effect on the next HTTP request; this
            // exercises RsvpService directly (bypassing HTTP entirely) so
            // the attempt runs on $mainConnection, not a kernel-rebooted
            // one — Security::isGranted() (called inside rsvp() for the
            // child-approval bypass check) needs a token in storage to
            // avoid throwing outright for an unrelated reason.
            /** @var TokenStorageInterface $tokenStorage */
            $tokenStorage = self::getContainer()->get(TokenStorageInterface::class);
            $tokenStorage->setToken(new UsernamePasswordToken($patAccount, 'main', $patAccount->getRoles()));

            /** @var RsvpService $rsvpService */
            $rsvpService = self::getContainer()->get(RsvpService::class);

            try {
                $rsvpService->rsvp($event, $pat, $patAccount, Rsvp::METHOD_FREE);
            } catch (\Throwable) {
                // Postgres raises 55P03 (lock_not_available) once
                // lock_timeout elapses while waiting on the row
                // $secondConnection still holds — wrapped by DBAL/PDO into
                // some driver exception type; which exact class surfaces
                // is not the point being proven here, only that the
                // attempt could not proceed.
                $blocked = true;
            }
        } finally {
            $secondConnection->rollBack();
            $secondConnection->close();
        }

        self::assertTrue($blocked, 'AC-02-67: a second attempt against the same locked event row is blocked, not silently let through to double-book the last spot.');
    }

    /**
     * The positive-path complement to the test above, exercised through
     * the real HTTP flow with no competing lock: capacity is enforced
     * without any concurrency involved at all, matching AC-02-27.
     */
    public function testUncontendedRsvpsStillCorrectlyFillAndRefuseTheLastSpot(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Fills To Capacity Session', 'capacity' => 1]);
        $this->createRsvp($event, $this->patPlayer());

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        self::assertSame(1, $rsvps->countHeld($event));

        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->switchPlayerToTrainer($this->client, $trainer);
        $this->switchToChild($this->client, 'Alex');
        $this->client->request('POST', sprintf('/portal/events/%d/rsvp', $event->getId()));

        self::assertResponseStatusCodeSame(403, 'AC-02-67/27: the voter\'s own capacity pre-check already refuses once full.');

        $this->activateTenant($trainer);
        /** @var RsvpRepository $freshRsvps */
        $freshRsvps = self::getContainer()->get(RsvpRepository::class);
        self::assertSame(1, $freshRsvps->countHeld($event), 'AC-02-67: capacity is never exceeded.');
    }
}
