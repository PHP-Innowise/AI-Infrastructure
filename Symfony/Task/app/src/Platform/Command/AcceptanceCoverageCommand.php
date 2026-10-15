<?php

declare(strict_types=1);

namespace App\Platform\Command;

use Symfony\Component\Console\Attribute\AsCommand;
use Symfony\Component\Console\Command\Command;
use Symfony\Component\Console\Input\InputInterface;
use Symfony\Component\Console\Input\InputOption;
use Symfony\Component\Console\Output\OutputInterface;
use Symfony\Component\Console\Style\SymfonyStyle;
use Symfony\Component\Finder\Finder;

/**
 * Reports which acceptance criteria are covered by a named test.
 *
 * "Covered" is deliberately mechanical: an AC id must appear somewhere in the
 * test suite. That is a weaker claim than "correctly tested", but it is a claim
 * that cannot be fudged in a status report — a criterion with no test is
 * reported as not covered, every time, by a command anyone can run.
 *
 * The alternative — a hand-maintained coverage table — drifts the moment
 * someone forgets to update it, which is exactly when it matters.
 */
#[AsCommand(
    name: 'app:ac-coverage',
    description: 'Report acceptance-criteria coverage across the derived specs and the test suite',
)]
final class AcceptanceCoverageCommand extends Command
{
    public function __construct(
        private readonly string $projectDir,
    ) {
        parent::__construct();
    }

    protected function configure(): void
    {
        $this
            ->addOption('strict', null, InputOption::VALUE_NONE, 'Exit non-zero when any criterion is uncovered')
            ->addOption('epic', null, InputOption::VALUE_REQUIRED, 'Restrict the report to one epic, e.g. 01')
            ->addOption('list-uncovered', null, InputOption::VALUE_NONE, 'Print every uncovered criterion id');
    }

    protected function execute(InputInterface $input, OutputInterface $output): int
    {
        $io = new SymfonyStyle($input, $output);

        // Mounted read-only at /specs by compose; falls back to the repository
        // layout when the command is run outside the container.
        $specsDir = is_dir('/specs') ? '/specs' : \dirname($this->projectDir, 2).'/specs';
        $testsDir = $this->projectDir.'/tests';

        if (!is_dir($specsDir)) {
            $io->error(sprintf('Specs directory not found at %s', $specsDir));

            return Command::FAILURE;
        }

        $criteria = $this->collectCriteria($specsDir);

        if ([] === $criteria) {
            $io->error('No acceptance criteria found. Has the spec format changed?');

            return Command::FAILURE;
        }

        $covered = $this->collectCoveredIds($testsDir);

        $epicFilter = $input->getOption('epic');
        $rows = [];
        $totalAll = 0;
        $totalCovered = 0;
        $uncovered = [];

        foreach ($this->groupByEpic($criteria) as $epic => $ids) {
            if (\is_string($epicFilter) && $epic !== $epicFilter) {
                continue;
            }

            $hit = array_values(array_filter($ids, static fn (string $id): bool => isset($covered[$id])));
            $miss = array_values(array_filter($ids, static fn (string $id): bool => !isset($covered[$id])));

            $totalAll += \count($ids);
            $totalCovered += \count($hit);
            $uncovered = [...$uncovered, ...$miss];

            $rows[] = [
                'Epic-'.$epic,
                \count($ids),
                \count($hit),
                \count($miss),
                sprintf('%5.1f%%', 0 === \count($ids) ? 0.0 : 100 * \count($hit) / \count($ids)),
            ];
        }

        $io->title('Acceptance-criteria coverage');
        $io->table(['Epic', 'Criteria', 'Covered', 'Uncovered', 'Coverage'], $rows);

        $io->writeln(sprintf(
            '  <info>%d</info> of <info>%d</info> criteria covered by a named test (<info>%.1f%%</info>)',
            $totalCovered,
            $totalAll,
            0 === $totalAll ? 0.0 : 100 * $totalCovered / $totalAll,
        ));

        if ($input->getOption('list-uncovered') && [] !== $uncovered) {
            $io->newLine();
            $io->section('Uncovered');
            $io->writeln(implode(' ', $uncovered));
        }

        if ([] === $uncovered) {
            $io->success('Every acceptance criterion is claimed by at least one test.');

            return Command::SUCCESS;
        }

        $io->newLine();
        $io->warning(sprintf('%d criteria have no test naming them.', \count($uncovered)));

        return $input->getOption('strict') ? Command::FAILURE : Command::SUCCESS;
    }

    /**
     * @return list<string>
     */
    private function collectCriteria(string $specsDir): array
    {
        $ids = [];

        $finder = (new Finder())
            ->files()
            ->in($specsDir)
            ->name('requirements-analyst-epic-0*-spec.md');

        foreach ($finder as $file) {
            preg_match_all('/\*\*(AC-\d{2}-\d+)\*\*/', $file->getContents(), $matches);
            foreach ($matches[1] as $id) {
                $ids[$id] = true;
            }
        }

        $ids = array_keys($ids);
        usort($ids, $this->compareIds(...));

        return $ids;
    }

    /**
     * @return array<string, true>
     */
    private function collectCoveredIds(string $testsDir): array
    {
        if (!is_dir($testsDir)) {
            return [];
        }

        $covered = [];

        foreach ((new Finder())->files()->in($testsDir)->name('*.php') as $file) {
            preg_match_all('/AC-(\d{2})-(\d+)/', $file->getContents(), $matches, \PREG_SET_ORDER);

            foreach ($matches as $match) {
                $covered[sprintf('AC-%s-%s', $match[1], $match[2])] = true;
            }
        }

        return $covered;
    }

    /**
     * @param list<string> $criteria
     *
     * @return array<string, list<string>>
     */
    private function groupByEpic(array $criteria): array
    {
        $grouped = [];

        foreach ($criteria as $id) {
            $grouped[substr($id, 3, 2)][] = $id;
        }

        ksort($grouped);

        return $grouped;
    }

    private function compareIds(string $a, string $b): int
    {
        [, $epicA, $numberA] = explode('-', $a);
        [, $epicB, $numberB] = explode('-', $b);

        return [$epicA, (int) $numberA] <=> [$epicB, (int) $numberB];
    }
}
