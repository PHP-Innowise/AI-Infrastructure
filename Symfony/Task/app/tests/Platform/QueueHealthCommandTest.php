<?php

declare(strict_types=1);

namespace App\Tests\Platform;

use Doctrine\DBAL\Connection;
use Symfony\Bundle\FrameworkBundle\Console\Application;
use Symfony\Bundle\FrameworkBundle\Test\KernelTestCase;
use Symfony\Component\Console\Command\Command;
use Symfony\Component\Console\Tester\CommandTester;

/**
 * The visibility that was missing when a worker died holding a message.
 *
 * Manual testing found a Stripe webhook stalled with nothing to notice it by:
 * the message kept the `delivered_at` stamp that marks it in flight, the
 * `failed` transport was empty because retries had not been exhausted, the
 * receipt carried no error, and the container reported healthy because the
 * liveness probe asks whether a process is running. The only way to see it
 * was to query `messenger_messages` by hand.
 *
 * These tests assert the two states that matter — nothing stuck, and a
 * message stranded by a dead worker — and that only the second one can fail
 * a build.
 */
final class QueueHealthCommandTest extends KernelTestCase
{
    private CommandTester $command;

    protected function setUp(): void
    {
        $application = new Application(self::bootKernel());
        $this->command = new CommandTester($application->find('app:queue-health'));
        $this->clearMessages();
    }

    protected function tearDown(): void
    {
        $this->clearMessages();
        parent::tearDown();
    }

    public function testAQuietQueueReportsNothingStuck(): void
    {
        $this->command->execute([]);

        $this->command->assertCommandIsSuccessful();
        self::assertStringContainsString('Nothing is stuck', $this->command->getDisplay());
    }

    /**
     * The exact shape the defect left behind: picked up, never finished,
     * never retried, never surfaced.
     */
    public function testAMessageStrandedByADeadWorkerIsReported(): void
    {
        $this->insertMessage(deliveredMinutesAgo: 20);

        $this->command->execute([]);
        $display = $this->command->getDisplay();

        self::assertStringContainsString('in flight over 300s', $display);
        self::assertStringContainsString('have been in flight for over 300 seconds', $display);
        self::assertStringContainsString('delivered_at', $display, 'The report names the stamp an operator has to clear.');
    }

    /**
     * Reporting must not fail a container start, so the exit code is opt-in.
     */
    public function testStuckWorkIsOnlyAFailureWhenAskedToBe(): void
    {
        $this->insertMessage(deliveredMinutesAgo: 20);

        $this->command->execute([]);
        self::assertSame(Command::SUCCESS, $this->command->getStatusCode(), 'Plain reporting is safe to run from an entrypoint.');

        $this->command->execute(['--strict' => true]);
        self::assertSame(Command::FAILURE, $this->command->getStatusCode(), '--strict is what a monitor or CI would use.');
    }

    /**
     * A message picked up a moment ago is a worker doing its job, not a
     * stall — the threshold exists so the report does not cry wolf.
     */
    public function testAMessageJustPickedUpIsNotCalledStuck(): void
    {
        $this->insertMessage(deliveredMinutesAgo: 0);

        $this->command->execute(['--strict' => true]);

        $this->command->assertCommandIsSuccessful();
        self::assertStringContainsString('Nothing is stuck', $this->command->getDisplay());
    }

    public function testTheThresholdIsAdjustable(): void
    {
        $this->insertMessage(deliveredMinutesAgo: 2);

        $this->command->execute(['--stuck-after' => '60', '--strict' => true]);

        self::assertSame(Command::FAILURE, $this->command->getStatusCode(), 'Two minutes is stuck when the threshold is one.');
        self::assertStringContainsString('in flight over 60s', $this->command->getDisplay());
    }

    private function insertMessage(int $deliveredMinutesAgo): void
    {
        $this->connection()->executeStatement(
            sprintf(
                "INSERT INTO messenger_messages (body, headers, queue_name, created_at, available_at, delivered_at)
                 VALUES ('{}', '{}', 'queue-health-test', now(), now(), now() - interval '%d minutes')",
                $deliveredMinutesAgo,
            ),
        );
    }

    /**
     * Only this test's own rows: the suite shares one database, and deleting
     * whatever else happens to be queued would break a neighbouring test for
     * reasons impossible to trace back here.
     */
    private function clearMessages(): void
    {
        $this->connection()->executeStatement("DELETE FROM messenger_messages WHERE queue_name = 'queue-health-test'");
    }

    private function connection(): Connection
    {
        /** @var Connection $connection */
        $connection = self::getContainer()->get('doctrine.dbal.default_connection');

        return $connection;
    }
}
