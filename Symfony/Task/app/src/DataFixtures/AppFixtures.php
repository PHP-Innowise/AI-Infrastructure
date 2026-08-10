<?php

declare(strict_types=1);

namespace App\DataFixtures;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\ParentChildLink;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Entity\ShareLink;
use App\Platform\Entity\AccountTrainerLink;
use App\Platform\Entity\PublicTenantCode;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\Bundle\FixturesBundle\Fixture;
use Doctrine\Persistence\ObjectManager;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * A reviewer needs to log in as each of the four MVP roles, and needs a second
 * trainer to exist so cross-tenant isolation is demonstrable rather than
 * theoretical. Also seeds one real ShareLink per trainer (with its matching
 * PublicTenantCode) and a parent/child relationship spanning two trainers, so
 * Epic-01's multi-trainer, multi-child scenarios have real data behind them
 * rather than only being reachable by creating everything by hand.
 *
 * Every password is `password`. This is development seed data and is never
 * loaded outside dev/test.
 */
final class AppFixtures extends Fixture
{
    public function __construct(
        private readonly UserPasswordHasherInterface $passwordHasher,
        private readonly TenantContext $tenantContext,
    ) {
    }

    public function load(ObjectManager $manager): void
    {
        $superAdmin = $this->account($manager, 'admin@practiceperfect.test', AccountRole::SuperAdmin, 'Dale', 'Administrator');

        // --- Tenant A ---------------------------------------------------------
        $trainerAccountA = $this->account($manager, 'trainer@practiceperfect.test', AccountRole::Trainer, 'Tina', 'Trainer');
        $trainerA = new Trainer($trainerAccountA, 'Peak Performance Basketball', 'peak-performance');
        $manager->persist($trainerA);

        // --- Tenant B, so isolation has something to isolate from -------------
        $trainerAccountB = $this->account($manager, 'trainer-b@practiceperfect.test', AccountRole::Trainer, 'Bruno', 'Baseline');
        $trainerB = new Trainer($trainerAccountB, 'Baseline Athletics', 'baseline-athletics');
        $manager->persist($trainerB);

        $coachAccount = $this->account($manager, 'coach@practiceperfect.test', AccountRole::Coach, 'Casey', 'Coach');
        $playerAccount = $this->account($manager, 'player@practiceperfect.test', AccountRole::Player, 'Pat', 'Parent');

        $manager->flush();

        // Links are what the resolver reads; they are global on purpose.
        $manager->persist(new AccountTrainerLink($trainerAccountA, $trainerA, 'trainer'));
        $manager->persist(new AccountTrainerLink($trainerAccountB, $trainerB, 'trainer'));
        $manager->persist(new AccountTrainerLink($coachAccount, $trainerA, 'coach'));
        $manager->persist(new AccountTrainerLink($playerAccount, $trainerA, 'player'));
        $manager->flush();

        // Pat is a self-training player under Trainer A AND a parent of two
        // children associated with two DIFFERENT trainers — "a parent
        // account is itself treated as a player account" made concrete,
        // alongside AC-01-15's "separated contexts".
        $patSelf = new PlayerProfile('Pat', new \DateTimeImmutable('1988-02-20'), $playerAccount);
        $manager->persist($patSelf);
        $childA = new PlayerProfile('Alex', new \DateTimeImmutable('2012-04-18'));
        $manager->persist($childA);
        $childB = new PlayerProfile('Blake', new \DateTimeImmutable('2011-09-02'));
        $manager->persist($childB);
        $manager->flush();

        $manager->persist(new ParentChildLink($playerAccount, $childA));
        $manager->persist(new ParentChildLink($playerAccount, $childB));
        $manager->flush();

        // Trainer-scoped rows must be written under the tenant they belong to:
        // the RLS WITH CHECK clause rejects anything else. Writing one row per
        // tenant here is also what makes the isolation test meaningful.
        $this->tenantContext->activateFor($trainerA);
        $staticLinkA = new ShareLink($trainerA, 'join-peak-performance', ShareLink::TYPE_STATIC_PLAYER, $trainerAccountA);
        $manager->persist($staticLinkA);
        $manager->persist(new CoachMembership($trainerA, $coachAccount, CoachMembership::STATUS_ACTIVE));
        $manager->flush();
        $manager->persist(new PlayerTrainerMembership($trainerA, $patSelf, PlayerTrainerMembership::SOURCE_SHARELINK, $staticLinkA));
        $manager->persist(new PlayerTrainerMembership($trainerA, $childA, PlayerTrainerMembership::SOURCE_SHARELINK, $staticLinkA));
        $manager->persist(new PublicTenantCode($staticLinkA->getCode(), $trainerA, PublicTenantCode::KIND_SHARELINK, (int) $staticLinkA->getId()));
        $manager->flush();

        $this->tenantContext->activateFor($trainerB);
        $staticLinkB = new ShareLink($trainerB, 'join-baseline-athletics', ShareLink::TYPE_STATIC_PLAYER, $trainerAccountB);
        $manager->persist($staticLinkB);
        $manager->flush();
        $manager->persist(new PlayerTrainerMembership($trainerB, $childB, PlayerTrainerMembership::SOURCE_SHARELINK, $staticLinkB));
        $manager->persist(new PublicTenantCode($staticLinkB->getCode(), $trainerB, PublicTenantCode::KIND_SHARELINK, (int) $staticLinkB->getId()));
        $manager->flush();

        $this->tenantContext->clear();

        unset($superAdmin);
    }

    private function account(
        ObjectManager $manager,
        string $email,
        AccountRole $role,
        string $firstName,
        string $lastName,
    ): Account {
        $account = new Account($email, '', $role);
        $account->changePasswordHash($this->passwordHasher->hashPassword($account, 'password'));
        $account->verifyEmail();

        $manager->persist($account);
        $manager->persist(new AccountProfile($account, $firstName, $lastName));

        return $account;
    }
}
