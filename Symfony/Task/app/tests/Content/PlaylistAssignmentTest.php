<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Entity\Playlist;
use App\Content\Repository\PlaylistAssignmentRepository;
use App\Content\Service\ContentAssignmentResolver;
use App\Content\Service\PlaylistAssignmentService;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.05 — Trainer Assigns Playlist to Players.
 */
final class PlaylistAssignmentTest extends WebTestCase
{
    use FixtureHelpers;
    use ContentFixtureHelpers;
    use CrmFixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-04-13/BR-04-13: individual multi-select assignment, with an
     * optional due date and note.
     */
    public function testTrainerAssignsPlaylistToIndividualPlayers(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Handles 101']);
        $pat = $this->patPlayer();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/assign', $playlist->getId()));
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Assign')->form();
        $form['assign_playlist[targetType]'] = 'player';
        $form['assign_playlist[targetPlayers]'] = [(string) $pat->getId()];
        $form['assign_playlist[dueDate]'] = (new \DateTimeImmutable('+7 days'))->format('Y-m-d');
        $form['assign_playlist[note]'] = 'Please complete before next practice.';
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Playlist assigned');

        $this->activateTenant($trainer);
        /** @var PlaylistAssignmentRepository $assignments */
        $assignments = self::getContainer()->get(PlaylistAssignmentRepository::class);
        $rows = $assignments->findForPlaylist($playlist);
        self::assertCount(1, $rows);
        self::assertSame($pat->getId(), $rows[0]->getTargetPlayer()?->getId());
        self::assertSame('Please complete before next practice.', $rows[0]->getNote());
        self::assertNotNull($rows[0]->getDueDate());
        self::assertSame($this->account('trainer@practiceperfect.test')->getId(), $rows[0]->getAssignedByAccount()->getId(), 'AC-04-14: records who assigned it.');
    }

    /**
     * AC-04-13/BR-04-13: assignment by a label group.
     */
    public function testTrainerAssignsPlaylistToALabelGroup(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createPracticePlaylist($trainer, ['title' => 'Beginner Conditioning']);
        // A unique-per-run name: labels are unique per (trainer, name) and
        // this whole suite shares one database with no per-test reset — a
        // fixed "Beginners" name risks colliding with another test file's
        // own label under the same fixture trainer.
        $label = $this->createLabel($trainer, 'Content Assignment Test Label '.uniqid());
        $pat = $this->patPlayer();
        $this->applyLabelToPlayer($trainer, $pat, $label, $this->account('trainer@practiceperfect.test'));

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/assign', $playlist->getId()));
        $form = $crawler->selectButton('Assign')->form();
        $form['assign_playlist[targetType]'] = 'label';
        $form['assign_playlist[targetLabel]'] = (string) $label->getId();
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var PlaylistAssignmentRepository $assignments */
        $assignments = self::getContainer()->get(PlaylistAssignmentRepository::class);
        $rows = $assignments->findForPlaylist($playlist);
        self::assertCount(1, $rows);
        self::assertSame('label', $rows[0]->getTargetType());
        self::assertSame($label->getId(), $rows[0]->getTargetLabel()?->getId());

        /** @var ContentAssignmentResolver $resolver */
        $resolver = self::getContainer()->get(ContentAssignmentResolver::class);
        self::assertTrue($resolver->isAssignedToPlayer($trainer, $playlist, $pat), 'BR-04-13: the labeled player is covered by the assignment.');
    }

    /**
     * AC-04-13/BR-04-13: assignment by a skill-level filter.
     */
    public function testTrainerAssignsPlaylistBySkillLevelFilter(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Advanced Ball Handling']);
        $pat = $this->patPlayer();
        $this->membershipFor($trainer, $pat)->setSkillLevel('Advanced');
        $this->flush();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/assign', $playlist->getId()));
        $form = $crawler->selectButton('Assign')->form();
        $form['assign_playlist[targetType]'] = 'skill_level';
        $form['assign_playlist[targetSkillLevel]'] = 'Advanced';
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var ContentAssignmentResolver $resolver */
        $resolver = self::getContainer()->get(ContentAssignmentResolver::class);
        self::assertTrue($resolver->isAssignedToPlayer($trainer, $playlist, $pat));
    }

    /**
     * AC-04-14: assigning notifies by email, and the assignment records who
     * assigned it, when, and to whom.
     */
    public function testAssigningNotifiesThePlayerByEmail(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'New Content Playlist']);
        $pat = $this->patPlayer();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/content/playlists/%d/assign', $playlist->getId()));
        $form = $crawler->selectButton('Assign')->form();
        $form['assign_playlist[targetType]'] = 'player';
        $form['assign_playlist[targetPlayers]'] = [(string) $pat->getId()];
        $this->client->submit($form);

        self::assertResponseRedirects();

        // self::assertQueuedEmailCount()/getMailerMessage() are unreliable
        // under this suite's kernel-reboot-per-request setup (see
        // EventCancellationTest's own note) — checked directly instead.
        $subjects = array_map(
            static fn ($m) => method_exists($m, 'getSubject') ? $m->getSubject() : '',
            self::getMailerMessages(),
        );
        self::assertTrue(
            (bool) array_filter($subjects, static fn (string $s): bool => str_contains($s, 'New content assigned: New Content Playlist')),
            'AC-04-14: the player was notified by email.',
        );
    }

    /**
     * AC-04-15: assignment status counts ("Assigned to N players, M
     * completed, K in progress").
     */
    public function testTrainerViewsAssignmentStatusCounts(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Progress Tracked Playlist']);
        $video = $this->createVideoItem($trainer);
        $this->addItemToPlaylist($trainer, $playlist, $video, 1);
        $pat = $this->patPlayer();

        /** @var PlaylistAssignmentService $assignmentService */
        $assignmentService = self::getContainer()->get(PlaylistAssignmentService::class);
        $assignmentService->assign($trainer, $playlist, $this->account('trainer@practiceperfect.test'), 'player', $pat, null, null, null, null);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', sprintf('/trainer/content/playlists/%d', $playlist->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Assigned to 1 player');
    }

    /**
     * AC-04-16/BR-04-14: the same playlist can be assigned to different
     * players multiple times, with no limit, each tracked separately.
     */
    public function testSamePlaylistCanBeAssignedMultipleTimes(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Repeatedly Assigned']);
        $pat = $this->patPlayer();
        $alex = $this->alexPlayer();
        $actor = $this->account('trainer@practiceperfect.test');

        /** @var PlaylistAssignmentService $assignmentService */
        $assignmentService = self::getContainer()->get(PlaylistAssignmentService::class);
        $assignmentService->assign($trainer, $playlist, $actor, 'player', $pat, null, null, null, null);
        $assignmentService->assign($trainer, $playlist, $actor, 'player', $alex, null, null, null, null);

        /** @var PlaylistAssignmentRepository $assignments */
        $assignments = self::getContainer()->get(PlaylistAssignmentRepository::class);
        $rows = $assignments->findForPlaylist($playlist);
        self::assertCount(2, $rows, 'BR-04-14: each assignment is tracked as its own separate row.');
    }
}
