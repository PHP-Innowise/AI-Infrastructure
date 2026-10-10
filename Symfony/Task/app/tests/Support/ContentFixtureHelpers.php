<?php

declare(strict_types=1);

namespace App\Tests\Support;

use App\Billing\Entity\PaymentRecord;
use App\Content\Entity\ContentItem;
use App\Content\Entity\ContentProgress;
use App\Content\Entity\Drill;
use App\Content\Entity\Playlist;
use App\Content\Entity\PlaylistAccessGrant;
use App\Content\Entity\PlaylistAssignment;
use App\Content\Entity\PlaylistItem;
use App\Crm\Entity\Label;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Content-specific (Epic-04) fixture helpers, built on FixtureHelpers
 * (AppFixtures lookups, tenant activation) — the same "set up via the
 * entity layer, exercise the feature under test via HTTP" split
 * SchedulingFixtureHelpers/CrmFixtureHelpers already establish.
 *
 * Requires the including test case to also `use FixtureHelpers` and expose
 * `self::getContainer()`. The tenant must already be active
 * (`activateTenant($trainer)`) before calling any of these — every entity
 * built here is trainer-scoped or carries a trainer reference that RLS'
 * WITH CHECK clause validates on INSERT.
 */
trait ContentFixtureHelpers
{
    /**
     * @param array{title?: string, description?: ?string, isPublic?: bool, audience?: string} $overrides
     */
    protected function createLearnPlaylist(Trainer $trainer, array $overrides = []): Playlist
    {
        return $this->createPlaylist($trainer, Playlist::PILLAR_LEARN, $overrides);
    }

    /**
     * @param array{title?: string, description?: ?string, isPublic?: bool, audience?: string} $overrides
     */
    protected function createPracticePlaylist(Trainer $trainer, array $overrides = []): Playlist
    {
        return $this->createPlaylist($trainer, Playlist::PILLAR_PRACTICE, $overrides);
    }

    /**
     * @param array{title?: string, description?: ?string, isPublic?: bool, audience?: string} $overrides
     */
    private function createPlaylist(Trainer $trainer, string $pillar, array $overrides): Playlist
    {
        $playlist = new Playlist(
            $trainer,
            $pillar,
            $overrides['title'] ?? 'Test Playlist '.uniqid(),
            $overrides['description'] ?? null,
        );

        if (($overrides['isPublic'] ?? false) || isset($overrides['audience'])) {
            $playlist->setVisibility(
                $overrides['isPublic'] ?? false,
                $overrides['audience'] ?? Playlist::AUDIENCE_PLAYERS_AND_COACHES,
                new \DateTimeImmutable(),
            );
        }

        $this->contentEntityManager()->persist($playlist);
        $this->contentEntityManager()->flush();

        return $playlist;
    }

    /**
     * @param array{title?: string, youtubeUrl?: string, instructions?: ?string, tags?: ?list<string>, durationSeconds?: ?int} $overrides
     */
    protected function createVideoItem(Trainer $trainer, array $overrides = []): ContentItem
    {
        $item = new ContentItem(
            $trainer,
            Playlist::PILLAR_LEARN,
            $overrides['title'] ?? 'Test Video '.uniqid(),
            $overrides['youtubeUrl'] ?? 'https://www.youtube.com/watch?v=dQw4w9WgXcQ',
            $overrides['instructions'] ?? null,
            $overrides['durationSeconds'] ?? 120,
            $overrides['tags'] ?? null,
        );

        $this->contentEntityManager()->persist($item);
        $this->contentEntityManager()->flush();

        return $item;
    }

    /**
     * @param array{title?: string, youtubeUrl?: string, difficultyLevel?: string, categories?: list<string>, instructions?: ?string, isPublic?: bool} $overrides
     */
    protected function createDrill(Trainer $trainer, array $overrides = []): Drill
    {
        $drill = new Drill(
            $trainer,
            $overrides['title'] ?? 'Test Drill '.uniqid(),
            $overrides['youtubeUrl'] ?? 'https://www.youtube.com/watch?v=dQw4w9WgXcQ',
            $overrides['difficultyLevel'] ?? Drill::DIFFICULTY_BEGINNER,
            $overrides['categories'] ?? ['Dribbling'],
            $overrides['instructions'] ?? null,
        );

        if ($overrides['isPublic'] ?? false) {
            $drill->setPublic(true, new \DateTimeImmutable());
        }

        $this->contentEntityManager()->persist($drill);
        $this->contentEntityManager()->flush();

        return $drill;
    }

    protected function addItemToPlaylist(
        Trainer $trainer,
        Playlist $playlist,
        ContentItem $item,
        int $sequenceOrder,
        ?string $trainerNotes = null,
    ): PlaylistItem {
        $playlistItem = new PlaylistItem($trainer, $playlist, $item, $sequenceOrder, true, $trainerNotes);
        $this->contentEntityManager()->persist($playlistItem);
        $this->contentEntityManager()->flush();

        return $playlistItem;
    }

    /**
     * Epic-05: `PlaylistAccessGrant` requires a real, persisted
     * `PaymentRecord` (the schema's own NOT NULL tightening, once
     * `payment_record` exists) — this builds a minimal, already-completed
     * one (1 token, matching a fixture-typical free-form grant) so tests
     * that only care about the ACCESS fact, not the purchase mechanics,
     * do not need to hand-construct one themselves.
     */
    protected function grantPlaylistAccess(Trainer $trainer, Playlist $playlist, PlayerProfile $player, Account $parentAccount): PlaylistAccessGrant
    {
        $paymentRecord = new PaymentRecord(
            $trainer,
            PaymentRecord::TYPE_CONTENT_PURCHASE,
            PaymentRecord::METHOD_TOKEN,
            1,
            $parentAccount->getEmail(),
            $parentAccount->getEmail(),
            $parentAccount,
        );
        $paymentRecord->attachRelatedPlaylist($playlist);
        $paymentRecord->applyFee(500, 0);
        $paymentRecord->markCompleted();
        $this->contentEntityManager()->persist($paymentRecord);
        $this->contentEntityManager()->flush();

        $grant = new PlaylistAccessGrant($trainer, $playlist, $player, $parentAccount, $paymentRecord);
        $this->contentEntityManager()->persist($grant);
        $this->contentEntityManager()->flush();

        return $grant;
    }

    protected function markContentCompleted(Trainer $trainer, PlayerProfile $player, ContentItem $item, ?\DateTimeImmutable $at = null): ContentProgress
    {
        $progress = new ContentProgress($trainer, $player, $item);
        $progress->recordEngagementStarted($at ?? new \DateTimeImmutable());
        $this->contentEntityManager()->persist($progress);
        $this->contentEntityManager()->flush();

        return $progress;
    }

    protected function assignPlaylistToPlayer(
        Trainer $trainer,
        Playlist $playlist,
        PlayerProfile $player,
        Account $assignedBy,
        ?\DateTimeImmutable $dueDate = null,
        ?string $note = null,
    ): PlaylistAssignment {
        $assignment = new PlaylistAssignment($trainer, $playlist, PlaylistAssignment::TARGET_PLAYER, $assignedBy, targetPlayer: $player, dueDate: $dueDate, note: $note);
        $this->contentEntityManager()->persist($assignment);
        $this->contentEntityManager()->flush();

        return $assignment;
    }

    protected function assignPlaylistToLabel(Trainer $trainer, Playlist $playlist, Label $label, Account $assignedBy): PlaylistAssignment
    {
        $assignment = new PlaylistAssignment($trainer, $playlist, PlaylistAssignment::TARGET_LABEL, $assignedBy, targetLabel: $label);
        $this->contentEntityManager()->persist($assignment);
        $this->contentEntityManager()->flush();

        return $assignment;
    }

    protected function assignPlaylistToSkillLevel(Trainer $trainer, Playlist $playlist, string $skillLevel, Account $assignedBy): PlaylistAssignment
    {
        $assignment = new PlaylistAssignment($trainer, $playlist, PlaylistAssignment::TARGET_SKILL_LEVEL, $assignedBy, targetSkillLevel: $skillLevel);
        $this->contentEntityManager()->persist($assignment);
        $this->contentEntityManager()->flush();

        return $assignment;
    }

    private function contentEntityManager(): EntityManagerInterface
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        return $entityManager;
    }
}
