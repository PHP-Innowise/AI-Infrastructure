<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Entity\ContentItem;
use App\Content\Entity\Drill;
use App\Content\Entity\Playlist;
use App\Content\Entity\PlaylistItem;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Test\KernelTestCase;

/**
 * "The one that will bite you": the publication exception is the single
 * declared widening of tenant isolation in the entire product
 * (architect-architecture.md "The publication exception"; "Risks — The
 * publication predicate is the one widening of tenant isolation... cheapest
 * early check: a test asserting an unpublished item is invisible
 * cross-tenant and a reverted item stays readable").
 *
 * These two tests pin the RLS predicate itself
 * (`trainer_id = current tenant OR ever_published_at IS NOT NULL`) directly
 * — mirroring `App\Tests\Platform\TenancyIsolationTest`'s own
 * KernelTestCase + raw-DQL-under-two-tenants shape, since this is exactly
 * the kind of database-level guarantee that shape is built to prove.
 *
 * @see specs/architect-architecture.md "The publication exception", Decisions "Content readability"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-5, BR-04-12
 */
final class PublicationExceptionTest extends KernelTestCase
{
    private EntityManagerInterface $entityManager;
    private TenantContext $tenantContext;

    protected function setUp(): void
    {
        self::bootKernel();

        $this->entityManager = self::getContainer()->get(EntityManagerInterface::class);
        $this->tenantContext = self::getContainer()->get(TenantContext::class);
    }

    protected function tearDown(): void
    {
        $this->tenantContext->clear();

        parent::tearDown();
    }

    /**
     * BR-04-3/4: a playlist that has NEVER been published is invisible to
     * every other tenant — the widened predicate's `ever_published_at IS
     * NOT NULL` branch never fires for it, so only the strict
     * `trainer_id = tenant` branch could ever match, and it does not.
     * Proven for both `Playlist` and `ContentItem`/`Drill`, since both
     * carry the same widened policy.
     */
    public function testAnUnpublishedPlaylistAndContentItemAreInvisibleCrossTenant(): void
    {
        $peak = $this->trainer('peak-performance');
        $baseline = $this->trainer('baseline-athletics');

        $this->tenantContext->activateFor($peak);
        $privatePlaylist = new Playlist($peak, Playlist::PILLAR_LEARN, 'Peak Private Playlist — Never Published');
        $this->entityManager->persist($privatePlaylist);

        $privateDrill = new Drill($peak, 'Peak Private Drill — Never Published', 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', Drill::DIFFICULTY_BEGINNER, ['Dribbling']);
        $this->entityManager->persist($privateDrill);
        $this->entityManager->flush();

        $playlistId = (int) $privatePlaylist->getId();
        $drillId = (int) $privateDrill->getId();

        self::assertFalse($privatePlaylist->hasEverBeenPublished());
        self::assertFalse($privateDrill->hasEverBeenPublished());

        $this->entityManager->clear();
        $this->tenantContext->activateFor($baseline);

        $foundPlaylist = $this->entityManager->createQuery('SELECT p FROM '.Playlist::class.' p WHERE p.id = :id')
            ->setParameter('id', $playlistId)
            ->getOneOrNullResult();
        $foundDrill = $this->entityManager->createQuery('SELECT d FROM '.Drill::class.' d WHERE d.id = :id')
            ->setParameter('id', $drillId)
            ->getOneOrNullResult();

        self::assertNull($foundPlaylist, 'BR-04-3/4: an unpublished playlist must be invisible to another tenant.');
        self::assertNull($foundDrill, 'BR-04-3/4: an unpublished drill must be invisible to another tenant.');

        // Also true through the repository's own "accessible" method — the
        // one legitimate widened-read code path — not only through a bare
        // DQL SELECT.
        $this->entityManager->clear();
        $this->tenantContext->activateFor($baseline);
        /** @var \App\Content\Repository\PlaylistRepository $playlists */
        $playlists = self::getContainer()->get(\App\Content\Repository\PlaylistRepository::class);
        self::assertNull($playlists->findAccessible($baseline, $playlistId), 'findAccessible() must also deny an unpublished cross-tenant playlist.');
    }

    /**
     * BR-04-5/BR-04-12: a playlist and a content item, once published and
     * referenced by another trainer, remain readable to that trainer even
     * after the creator reverts to Private — because `ever_published_at` is
     * a one-way ratchet, never cleared by `setVisibility(false, ...)`. This
     * is what makes BR-04-12's "reference keeps working" true: the reusing
     * trainer's own `PlaylistItem` row still resolves its `content_item_id`
     * foreign key to a row RLS will still let them read.
     */
    public function testARevertedToPrivateItemStaysReadableToATrainerAlreadyUsingIt(): void
    {
        $peak = $this->trainer('peak-performance');
        $baseline = $this->trainer('baseline-athletics');

        // Peak Performance publishes a drill.
        $this->tenantContext->activateFor($peak);
        $drill = new Drill($peak, 'Peak Drill — Published Then Reverted', 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', Drill::DIFFICULTY_BEGINNER, ['Shooting']);
        $drill->setPublic(true, new \DateTimeImmutable());
        $this->entityManager->persist($drill);
        $this->entityManager->flush();
        $drillId = (int) $drill->getId();

        // Baseline Athletics reuses it by reference in one of their own
        // playlists — BR-04-12's "a reference is stored rather than a copy".
        $this->entityManager->clear();
        $this->tenantContext->activateFor($baseline);
        /** @var Trainer $baselineRef */
        $baselineRef = $this->entityManager->getReference(Trainer::class, $baseline->getId());
        $baselinePlaylist = new Playlist($baselineRef, Playlist::PILLAR_PRACTICE, 'Baseline Workout Using Peak\'s Drill');
        $this->entityManager->persist($baselinePlaylist);
        $this->entityManager->flush();

        /** @var ContentItem $drillRefForBaseline */
        $drillRefForBaseline = $this->entityManager->getReference(ContentItem::class, $drillId);
        $playlistItem = new PlaylistItem($baselineRef, $baselinePlaylist, $drillRefForBaseline, 1);
        $this->entityManager->persist($playlistItem);
        $this->entityManager->flush();
        $playlistItemId = (int) $playlistItem->getId();

        // Peak Performance reverts the drill back to Private.
        $this->entityManager->clear();
        $this->tenantContext->activateFor($peak);
        $reloadedDrill = $this->entityManager->find(Drill::class, $drillId);
        self::assertNotNull($reloadedDrill);
        $reloadedDrill->setPublic(false, new \DateTimeImmutable());
        $this->entityManager->flush();

        self::assertFalse($reloadedDrill->isPublic(), 'The drill now reads as private...');
        self::assertTrue($reloadedDrill->hasEverBeenPublished(), '...but the one-way "ever published" fact is never cleared.');

        // Baseline Athletics' own reference must STILL resolve — this is
        // the actual guarantee under test.
        $this->entityManager->clear();
        $this->tenantContext->activateFor($baseline);

        $stillFound = $this->entityManager->createQuery('SELECT d FROM '.Drill::class.' d WHERE d.id = :id')
            ->setParameter('id', $drillId)
            ->getOneOrNullResult();
        self::assertNotNull($stillFound, 'BR-04-5: the reverted drill must STAY readable to a trainer already referencing it.');
        self::assertFalse($stillFound->isPublic(), 'It reads as private (no longer independently discoverable)...');

        $reloadedPlaylistItem = $this->entityManager->find(PlaylistItem::class, $playlistItemId);
        self::assertNotNull($reloadedPlaylistItem, 'BR-04-12: the reusing trainer\'s own reference row is untouched.');
        self::assertSame($drillId, $reloadedPlaylistItem->getContentItem()->getId(), '...and its FK still resolves to the (now-private) drill, not a broken/null reference.');
        self::assertFalse($reloadedPlaylistItem->getContentItem()->isDeleted(), 'BR-04-5 (revert) is distinct from BR-04-12 (delete): reverting never marks the row deleted/"Unavailable" — only an actual delete does.');

        // And it is no longer discoverable in PUBLIC search — is_public is
        // what gates DISCOVERY, not readability. Searched as Baseline
        // (excludingTrainer=baseline just excludes Baseline's OWN drills,
        // unrelated to Peak's drill or to Baseline's own PlaylistItem
        // reference to it — the only thing under test here is whether
        // is_public=false correctly drops it from the result set).
        $this->entityManager->clear();
        $this->tenantContext->activateFor($baseline);
        /** @var \App\Content\Repository\DrillRepository $drills */
        $drills = self::getContainer()->get(\App\Content\Repository\DrillRepository::class);
        $publicResults = $drills->searchPublic($baseline, 'Peak Drill — Published Then Reverted', null, null, null, null);
        self::assertSame([], $publicResults, 'AC-04-19/BR-04-5: reverted content no longer appears in public discovery.');
    }

    private function trainer(string $slug): Trainer
    {
        /** @var TrainerRepository $repository */
        $repository = self::getContainer()->get(TrainerRepository::class);
        $trainer = $repository->findOneBySlug($slug);

        self::assertInstanceOf(Trainer::class, $trainer, sprintf('Fixture trainer "%s" is missing.', $slug));

        return $trainer;
    }
}
