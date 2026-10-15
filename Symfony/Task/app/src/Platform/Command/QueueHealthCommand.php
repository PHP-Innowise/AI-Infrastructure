<?php

declare(strict_types=1);

namespace App\Platform\Command;

use Doctrine\DBAL\Connection;
use Symfony\Component\Console\Attribute\AsCommand;
use Symfony\Component\Console\Command\Command;
use Symfony\Component\Console\Input\InputInterface;
use Symfony\Component\Console\Input\InputOption;
use Symfony\Component\Console\Output\OutputInterface;
use Symfony\Component\Console\Style\SymfonyStyle;

/**
 * Answers "is asynchronous work actually getting done", which nothing in this
 * stack could answer before.
 *
 * Manual testing hit the gap: a worker died mid-message, the message kept the
 * `delivered_at` stamp that marks it in-flight, and it sat there — no row in
 * the `failed` transport, no error on the webhook receipt, and a container
 * still reporting healthy, because the liveness probe asks whether the
 * process is running and a restarted worker is running. A Stripe webhook went
 * unprocessed for as long as the redelivery timeout, and the only way to
 * notice was to query `messenger_messages` by hand.
 *
 * Three questions, one command:
 *
 *   - waiting: queued and not yet picked up. Normal in small numbers; a
 *     growing number with no worker consuming is the classic silent stall.
 *   - in flight: picked up and not finished. Normal for seconds. Beyond
 *     `--stuck-after`, it means whoever picked it up is gone — nothing
 *     releases the stamp except the redelivery timeout.
 *   - failed: retries exhausted. Never normal, and never surfaced anywhere
 *     else in this product.
 *
 * Stripe receipts are counted alongside because they are the payload whose
 * loss actually costs money: an unprocessed receipt is a payment the platform
 * took and has not acted on.
 *
 * Reads through DBAL rather than Messenger's own transport API on purpose:
 * this has to answer even when the worker cannot boot, which is precisely the
 * case it was written for.
 *
 * @see specs/architect-architecture.md "Synchronous and asynchronous work"
 */
#[AsCommand(
    name: 'app:queue-health',
    description: 'Report queued, in-flight and failed async messages, and unprocessed Stripe receipts',
)]
final class QueueHealthCommand extends Command
{
    private const int DEFAULT_STUCK_AFTER_SECONDS = 300;

    public function __construct(
        private readonly Connection $connection,
    ) {
        parent::__construct();
    }

    protected function configure(): void
    {
        $this
            ->addOption('strict', null, InputOption::VALUE_NONE, 'Exit non-zero when anything is stuck or failed')
            ->addOption(
                'stuck-after',
                null,
                InputOption::VALUE_REQUIRED,
                'Seconds a message may stay in flight before it counts as stuck',
                (string) self::DEFAULT_STUCK_AFTER_SECONDS,
            );
    }

    protected function execute(InputInterface $input, OutputInterface $output): int
    {
        $io = new SymfonyStyle($input, $output);
        $stuckAfter = max(1, (int) $input->getOption('stuck-after'));

        if (!$this->messengerTableExists()) {
            $io->warning('messenger_messages does not exist yet — run `make migrate`.');

            return Command::SUCCESS;
        }

        $waiting = $this->countMessages('delivered_at IS NULL');
        $inFlight = $this->countMessages('delivered_at IS NOT NULL');
        $stuck = $this->countMessages(sprintf('delivered_at IS NOT NULL AND delivered_at < now() - interval \'%d seconds\'', $stuckAfter));
        $failed = $this->countMessages("queue_name = 'failed'");
        $unprocessedReceipts = $this->unprocessedStripeReceipts();

        $io->definitionList(
            ['waiting' => (string) $waiting],
            ['in flight' => (string) $inFlight],
            [sprintf('in flight over %ds', $stuckAfter) => (string) $stuck],
            ['failed' => (string) $failed],
            ['unprocessed Stripe receipts' => (string) $unprocessedReceipts],
        );

        if (0 === $stuck && 0 === $failed) {
            $io->success('Nothing is stuck.');

            return Command::SUCCESS;
        }

        if ($stuck > 0) {
            $io->warning(sprintf(
                '%d message(s) have been in flight for over %d seconds. A message keeps its delivered_at stamp when the '
                .'worker that picked it up dies, and nothing releases it until the transport\'s redelivery timeout. '
                .'Restart the worker, then clear the stamp: '
                ."UPDATE messenger_messages SET delivered_at = NULL WHERE delivered_at < now() - interval '%d seconds';",
                $stuck,
                $stuckAfter,
                $stuckAfter,
            ));
        }

        if ($failed > 0) {
            $io->warning(sprintf('%d message(s) exhausted their retries. Inspect with `messenger:failed:show`.', $failed));
        }

        if ($unprocessedReceipts > 0) {
            $io->note(sprintf('%d Stripe webhook receipt(s) are recorded but not processed.', $unprocessedReceipts));
        }

        // Reporting is the point; failing is opt-in, so this stays safe to
        // run from an entrypoint that must not abort a container start.
        return $input->getOption('strict') ? Command::FAILURE : Command::SUCCESS;
    }

    private function countMessages(string $where): int
    {
        return (int) $this->connection->fetchOne(sprintf('SELECT count(*) FROM messenger_messages WHERE %s', $where));
    }

    private function unprocessedStripeReceipts(): int
    {
        if (!$this->tableExists('stripe_event_receipt')) {
            return 0;
        }

        return (int) $this->connection->fetchOne('SELECT count(*) FROM stripe_event_receipt WHERE processed_at IS NULL');
    }

    private function messengerTableExists(): bool
    {
        return $this->tableExists('messenger_messages');
    }

    private function tableExists(string $table): bool
    {
        return (bool) $this->connection->fetchOne('SELECT to_regclass(?) IS NOT NULL', ['public.'.$table]);
    }
}
